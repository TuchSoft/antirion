//! The non-streaming equivalent of a stream, for content logging and the response cache. This
//! module assembles Chat streams: text deltas concatenated per choice, tool-call fragments joined,
//! finish reasons and usage attached; its submodules the other dialects a client reads.

mod converse;
mod gemini;
mod messages;
mod responses;

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

pub(crate) use self::converse::ConverseAssembler;
pub(crate) use self::gemini::GeminiAssembler;
pub(crate) use self::messages::MessageAssembler;
pub(crate) use self::responses::ResponsesAssembler;

#[derive(Debug, Default)]
struct ToolCall {
    id: Option<String>,
    kind: Option<String>,
    name: String,
    arguments: String,
}

#[derive(Debug, Default)]
struct Choice {
    role: Option<String>,
    content: String,
    refusal: String,
    tool_calls: BTreeMap<u64, ToolCall>,
    finish: Option<Value>,
}

#[derive(Debug, Default)]
pub(crate) struct Assembler {
    id: Option<Value>,
    model: Option<Value>,
    created: Option<Value>,
    choices: BTreeMap<u64, Choice>,
    usage: Option<Value>,
    /// The chunks were legacy Completions (`text_completion`, text in `choices[].text`).
    completion: bool,
}

impl Assembler {
    /// An assembler whose body names `model`, the name the client was sent, instead of the
    /// upstream chunks' own.
    pub(crate) fn naming(model: Option<&str>) -> Assembler {
        Assembler {
            model: model.map(Value::from),
            ..Assembler::default()
        }
    }

    /// Takes one parsed chunk.
    pub(crate) fn push(&mut self, chunk: &Value) {
        for (field, slot) in [
            ("id", &mut self.id),
            ("model", &mut self.model),
            ("created", &mut self.created),
        ] {
            if slot.is_none() {
                *slot = chunk.get(field).cloned();
            }
        }
        if let Some(u) = chunk.get("usage").filter(|u| !u.is_null()) {
            self.usage = Some(u.clone());
        }
        for c in chunk
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let index = c.get("index").and_then(Value::as_u64).unwrap_or(0);
            let choice = self.choices.entry(index).or_default();
            if let Some(f) = c.get("finish_reason").filter(|f| !f.is_null()) {
                choice.finish = Some(f.clone());
            }
            if let Some(t) = c.get("text").and_then(Value::as_str) {
                self.completion = true;
                choice.content.push_str(t);
                continue;
            }
            let Some(delta) = c.get("delta") else {
                continue;
            };
            if let Some(r) = delta.get("role").and_then(Value::as_str) {
                choice.role = Some(r.to_owned());
            }
            if let Some(t) = delta.get("content").and_then(Value::as_str) {
                choice.content.push_str(t);
            }
            if let Some(t) = delta.get("refusal").and_then(Value::as_str) {
                choice.refusal.push_str(t);
            }
            for tc in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let i = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
                let call = choice.tool_calls.entry(i).or_default();
                if let Some(id) = tc.get("id").and_then(Value::as_str) {
                    call.id = Some(id.to_owned());
                }
                if let Some(k) = tc.get("type").and_then(Value::as_str) {
                    call.kind = Some(k.to_owned());
                }
                if let Some(f) = tc.get("function") {
                    if let Some(n) = f.get("name").and_then(Value::as_str) {
                        call.name.push_str(n);
                    }
                    if let Some(a) = f.get("arguments").and_then(Value::as_str) {
                        call.arguments.push_str(a);
                    }
                }
            }
        }
    }

    /// The assembled `chat.completion` body (`text_completion` for Completions chunks).
    pub(crate) fn finish(self) -> Value {
        if self.completion {
            let choices: Vec<Value> = self
                .choices
                .into_iter()
                .map(|(index, c)| json!({"index": index, "text": c.content, "finish_reason": c.finish}))
                .collect();
            return json!({
                "id": self.id,
                "object": "text_completion",
                "created": self.created,
                "model": self.model,
                "choices": choices,
                "usage": self.usage,
            });
        }
        let choices: Vec<Value> = self
            .choices
            .into_iter()
            .map(|(index, c)| {
                let mut message = Map::new();
                message.insert(
                    "role".into(),
                    Value::from(c.role.unwrap_or_else(|| "assistant".to_owned())),
                );
                let content = if c.content.is_empty() && !c.tool_calls.is_empty() {
                    Value::Null
                } else {
                    Value::String(c.content)
                };
                message.insert("content".into(), content);
                if !c.refusal.is_empty() {
                    message.insert("refusal".into(), Value::String(c.refusal));
                }
                if !c.tool_calls.is_empty() {
                    let calls: Vec<Value> = c
                        .tool_calls
                        .into_values()
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "type": t.kind.unwrap_or_else(|| "function".to_owned()),
                                "function": {"name": t.name, "arguments": t.arguments},
                            })
                        })
                        .collect();
                    message.insert("tool_calls".into(), Value::Array(calls));
                }
                json!({"index": index, "message": message, "finish_reason": c.finish})
            })
            .collect();
        json!({
            "id": self.id,
            "object": "chat.completion",
            "created": self.created,
            "model": self.model,
            "choices": choices,
            "usage": self.usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_fragments_are_joined() {
        let mut a = Assembler::default();
        a.push(&json!({"id": "c", "model": "m", "choices": [{"index": 0, "delta": {"role": "assistant", "content": null}}]}));
        a.push(&json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "weather", "arguments": ""}}]}}]}));
        a.push(&json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{\"ci"}}]}}]}));
        a.push(&json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": "ty\":\"Paris\"}"}}]}}]}));
        a.push(&json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}));
        let body = a.finish();
        let call = &body["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(call["function"]["name"], "weather");
        assert_eq!(call["function"]["arguments"], "{\"city\":\"Paris\"}");
        assert_eq!(body["choices"][0]["finish_reason"], "tool_calls");
        assert!(body["choices"][0]["message"]["content"].is_null());
    }

    #[test]
    fn completions_chunks_keep_their_shape_and_the_client_model() {
        let mut a = Assembler::naming(Some("public"));
        a.push(&json!({"id": "c", "object": "text_completion", "model": "up", "choices": [{"index": 0, "text": "hel", "finish_reason": null}]}));
        a.push(&json!({"id": "c", "object": "text_completion", "model": "up", "choices": [{"index": 0, "text": "lo", "finish_reason": "stop"}]}));
        let body = a.finish();
        assert_eq!(body["object"], "text_completion");
        assert_eq!(body["model"], "public");
        assert_eq!(body["choices"][0]["text"], "hello");
        assert_eq!(body["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn text_is_concatenated() {
        let mut a = Assembler::default();
        a.push(&json!({"choices": [{"index": 0, "delta": {"content": "hel"}}]}));
        a.push(&json!({"choices": [{"index": 0, "delta": {"content": "lo"}, "finish_reason": "stop"}]}));
        a.push(&json!({"choices": [], "usage": {"prompt_tokens": 1}}));
        let body = a.finish();
        assert_eq!(body["choices"][0]["message"]["content"], "hello");
        assert_eq!(body["usage"]["prompt_tokens"], 1);
    }
}
