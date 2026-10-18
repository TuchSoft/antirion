//! The non-streaming equivalent of a Bedrock ConverseStream: the content blocks rebuilt from their
//! deltas (text, tool use, reasoning), the stop reason of `messageStop` and the usage and metrics of
//! `metadata`.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

#[derive(Debug, Default)]
struct Block {
    text: Option<String>,
    /// `toolUseId`, `name` and the argument fragments.
    tool: Option<(Value, Value, String)>,
    reasoning: Option<String>,
    signature: Option<Value>,
    redacted: Option<Value>,
}

#[derive(Debug, Default)]
pub(crate) struct ConverseAssembler {
    role: Option<Value>,
    blocks: BTreeMap<u64, Block>,
    /// `stopReason`, `additionalModelResponseFields`, `usage`, `metrics`, `trace`.
    fields: Map<String, Value>,
}

impl ConverseAssembler {
    /// Takes one event, by its `:event-type`.
    pub(crate) fn push(&mut self, kind: &str, p: &Value) {
        let index = p.get("contentBlockIndex").and_then(Value::as_u64).unwrap_or(0);
        match kind {
            "messageStart" => self.role = p.get("role").cloned(),
            "contentBlockStart" => {
                if let Some(t) = p.pointer("/start/toolUse") {
                    let block = self.blocks.entry(index).or_default();
                    block.tool = Some((
                        t.get("toolUseId").cloned().unwrap_or(Value::Null),
                        t.get("name").cloned().unwrap_or(Value::Null),
                        String::new(),
                    ));
                }
            }
            "contentBlockDelta" => {
                let Some(d) = p.get("delta") else {
                    return;
                };
                let block = self.blocks.entry(index).or_default();
                if let Some(t) = d.get("text").and_then(Value::as_str) {
                    block.text.get_or_insert_default().push_str(t);
                }
                if let Some(i) = d.pointer("/toolUse/input").and_then(Value::as_str) {
                    block
                        .tool
                        .get_or_insert_with(|| (Value::Null, Value::Null, String::new()))
                        .2
                        .push_str(i);
                }
                if let Some(r) = d.get("reasoningContent") {
                    if let Some(t) = r.get("text").and_then(Value::as_str) {
                        block.reasoning.get_or_insert_default().push_str(t);
                    }
                    if let Some(s) = r.get("signature") {
                        block.signature = Some(s.clone());
                    }
                    if let Some(c) = r.get("redactedContent") {
                        block.redacted = Some(c.clone());
                    }
                }
            }
            "messageStop" | "metadata" => {
                for (k, v) in p.as_object().into_iter().flatten() {
                    self.fields.insert(k.clone(), v.clone());
                }
            }
            _ => {}
        }
    }

    /// The assembled `Converse` response.
    pub(crate) fn finish(self) -> Value {
        let content: Vec<Value> = self.blocks.into_values().filter_map(block).collect();
        let mut out = Map::new();
        out.insert(
            "output".into(),
            json!({"message": {"role": self.role.unwrap_or_else(|| Value::from("assistant")), "content": content}}),
        );
        out.extend(self.fields);
        Value::Object(out)
    }
}

fn block(b: Block) -> Option<Value> {
    if let Some((id, name, input)) = b.tool {
        let input = if input.is_empty() {
            json!({})
        } else {
            serde_json::from_str(&input).unwrap_or(Value::String(input))
        };
        return Some(json!({"toolUse": {"toolUseId": id, "name": name, "input": input}}));
    }
    if let Some(c) = b.redacted {
        return Some(json!({"reasoningContent": {"redactedContent": c}}));
    }
    if b.reasoning.is_some() || b.signature.is_some() {
        let mut r = Map::new();
        r.insert("text".into(), Value::from(b.reasoning.unwrap_or_default()));
        if let Some(s) = b.signature {
            r.insert("signature".into(), s);
        }
        return Some(json!({"reasoningContent": {"reasoningText": r}}));
    }
    b.text.map(|t| json!({"text": t}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_are_rebuilt() {
        let mut a = ConverseAssembler::default();
        a.push("messageStart", &json!({"role": "assistant"}));
        a.push(
            "contentBlockDelta",
            &json!({"contentBlockIndex": 0, "delta": {"text": "Hel"}}),
        );
        a.push(
            "contentBlockDelta",
            &json!({"contentBlockIndex": 0, "delta": {"text": "lo"}}),
        );
        a.push("contentBlockStop", &json!({"contentBlockIndex": 0}));
        a.push(
            "contentBlockStart",
            &json!({"contentBlockIndex": 1, "start": {"toolUse": {"toolUseId": "t", "name": "f"}}}),
        );
        a.push(
            "contentBlockDelta",
            &json!({"contentBlockIndex": 1, "delta": {"toolUse": {"input": "{\"a\":"}}}),
        );
        a.push(
            "contentBlockDelta",
            &json!({"contentBlockIndex": 1, "delta": {"toolUse": {"input": "1}"}}}),
        );
        a.push("messageStop", &json!({"stopReason": "tool_use"}));
        a.push(
            "metadata",
            &json!({"usage": {"inputTokens": 1, "outputTokens": 2}, "metrics": {"latencyMs": 3}}),
        );
        let v = a.finish();
        assert_eq!(v["output"]["message"]["content"][0]["text"], "Hello");
        assert_eq!(
            v["output"]["message"]["content"][1]["toolUse"]["input"],
            json!({"a": 1})
        );
        assert_eq!(v["stopReason"], "tool_use");
        assert_eq!(v["usage"]["outputTokens"], 2);
    }
}
