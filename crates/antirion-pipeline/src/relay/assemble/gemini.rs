//! The non-streaming equivalent of a Gemini stream: each candidate's parts joined (adjacent text
//! parts of the same kind become one), its last finish reason, and the chunk-level members
//! (`usageMetadata`, `modelVersion`, `responseId`) as the stream last reported them.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

#[derive(Debug, Default)]
struct Candidate {
    role: Option<Value>,
    parts: Vec<Value>,
    /// Every other member (`finishReason`, `safetyRatings`, `index`, …), the latest kept.
    fields: Map<String, Value>,
}

#[derive(Debug, Default)]
pub(crate) struct GeminiAssembler {
    candidates: BTreeMap<u64, Candidate>,
    top: Map<String, Value>,
}

impl GeminiAssembler {
    /// Takes one parsed chunk.
    pub(crate) fn push(&mut self, chunk: &Value) {
        let Some(o) = chunk.as_object() else {
            return;
        };
        for (k, v) in o.iter().filter(|(k, _)| k.as_str() != "candidates") {
            self.top.insert(k.clone(), v.clone());
        }
        for (n, c) in o
            .get("candidates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let index = c
                .get("index")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| u64::try_from(n).unwrap_or(0));
            let candidate = self.candidates.entry(index).or_default();
            for (k, v) in c.as_object().into_iter().flatten() {
                if k != "content" {
                    candidate.fields.insert(k.clone(), v.clone());
                }
            }
            let Some(content) = c.get("content") else {
                continue;
            };
            if let Some(r) = content.get("role") {
                candidate.role = Some(r.clone());
            }
            for p in content
                .get("parts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                join(&mut candidate.parts, p);
            }
        }
    }

    /// The assembled `GenerateContentResponse`.
    pub(crate) fn finish(self) -> Value {
        let candidates: Vec<Value> = self
            .candidates
            .into_values()
            .map(|c| {
                let mut out = Map::new();
                out.insert(
                    "content".into(),
                    json!({"role": c.role.unwrap_or_else(|| Value::from("model")), "parts": c.parts}),
                );
                out.extend(c.fields);
                Value::Object(out)
            })
            .collect();
        let mut out = Map::new();
        out.insert("candidates".into(), Value::Array(candidates));
        out.extend(self.top);
        Value::Object(out)
    }
}

/// Whether a part is plain text (or thought text), with its thought flag.
fn text_part(p: &Value) -> Option<bool> {
    let o = p.as_object()?;
    o.get("text")?.as_str()?;
    o.keys()
        .all(|k| matches!(k.as_str(), "text" | "thought" | "thoughtSignature"))
        .then(|| o.get("thought").and_then(Value::as_bool).unwrap_or(false))
}

/// Appends a part, joining text onto the previous part of the same kind.
fn join(parts: &mut Vec<Value>, p: &Value) {
    let Some(thought) = text_part(p) else {
        parts.push(p.clone());
        return;
    };
    let joinable = parts
        .last()
        .is_some_and(|last| text_part(last) == Some(thought) && last.get("thoughtSignature").is_none());
    match parts
        .last_mut()
        .and_then(Value::as_object_mut)
        .filter(|_| joinable)
    {
        Some(last) => {
            let text = p.get("text").and_then(Value::as_str).unwrap_or_default();
            if let Some(Value::String(s)) = last.get_mut("text") {
                s.push_str(text);
            }
            if let Some(sig) = p.get("thoughtSignature") {
                last.insert("thoughtSignature".into(), sig.clone());
            }
        }
        None => parts.push(p.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_joined_and_the_last_report_kept() {
        let mut a = GeminiAssembler::default();
        a.push(
            &json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "Hel"}]}, "index": 0}],
            "modelVersion": "g"}),
        );
        a.push(
            &json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "lo"}]}, "index": 0}]}),
        );
        a.push(&json!({"candidates": [{"content": {"role": "model", "parts": [{"functionCall": {"name": "f", "args": {}}}]},
            "finishReason": "STOP", "index": 0}], "usageMetadata": {"promptTokenCount": 2, "candidatesTokenCount": 3}}));
        let v = a.finish();
        assert_eq!(v["candidates"][0]["content"]["parts"][0]["text"], "Hello");
        assert_eq!(
            v["candidates"][0]["content"]["parts"][1]["functionCall"]["name"],
            "f"
        );
        assert_eq!(v["candidates"][0]["finishReason"], "STOP");
        assert_eq!(v["usageMetadata"]["candidatesTokenCount"], 3);
        assert_eq!(v["modelVersion"], "g");
    }

    #[test]
    fn thoughts_stay_apart_from_the_answer() {
        let mut a = GeminiAssembler::default();
        a.push(&json!({"candidates": [{"content": {"parts": [{"text": "hmm", "thought": true}]}}]}));
        a.push(&json!({"candidates": [{"content": {"parts": [{"text": "yes"}]}}]}));
        let v = a.finish();
        assert_eq!(
            v["candidates"][0]["content"]["parts"].as_array().unwrap().len(),
            2
        );
        assert_eq!(v["candidates"][0]["content"]["role"], "model");
    }
}
