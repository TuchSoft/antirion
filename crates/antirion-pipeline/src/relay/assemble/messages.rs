//! The non-streaming equivalent of an Anthropic Messages stream: the `message_start` message with
//! its content blocks rebuilt from their deltas, and the stop reason and usage of `message_delta`.

use serde_json::{Map, Value};

#[derive(Debug, Default)]
pub(crate) struct MessageAssembler {
    message: Map<String, Value>,
    blocks: Vec<Value>,
    /// The `input_json_delta` fragments of each block not yet joined into its `input`.
    partial: Vec<String>,
    /// A tool call's arguments were not JSON.
    broken: bool,
}

impl MessageAssembler {
    /// Takes one parsed event.
    pub(crate) fn push(&mut self, p: &Value) {
        let index = p
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|i| usize::try_from(i).ok());
        match p.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(m) = p.get("message").and_then(Value::as_object) {
                    self.message = m.clone();
                }
            }
            Some("content_block_start") => {
                if let (Some(i), Some(b)) = (index, p.get("content_block")) {
                    if self.blocks.len() <= i {
                        self.blocks.resize(i + 1, Value::Null);
                        self.partial.resize(i + 1, String::new());
                    }
                    if let Some(slot) = self.blocks.get_mut(i) {
                        *slot = b.clone();
                    }
                }
            }
            Some("content_block_delta") => {
                if let (Some(i), Some(d)) = (index, p.get("delta")) {
                    self.delta(i, d);
                }
            }
            Some("content_block_stop") => {
                if let Some(i) = index {
                    self.close(i);
                }
            }
            Some("message_delta") => {
                if let Some(d) = p.get("delta").and_then(Value::as_object) {
                    for (k, v) in d {
                        self.message.insert(k.clone(), v.clone());
                    }
                }
                if let Some(u) = p.get("usage").and_then(Value::as_object) {
                    let mut usage = self
                        .message
                        .get("usage")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    for (k, v) in u {
                        usage.insert(k.clone(), v.clone());
                    }
                    self.message.insert("usage".into(), Value::Object(usage));
                }
            }
            _ => {}
        }
    }

    fn delta(&mut self, i: usize, d: &Value) {
        let Some(block) = self.blocks.get_mut(i).and_then(Value::as_object_mut) else {
            return;
        };
        let text = |field: &str| d.get(field).and_then(Value::as_str).unwrap_or_default();
        match d.get("type").and_then(Value::as_str) {
            Some("text_delta") => append(block, "text", text("text")),
            Some("thinking_delta") => append(block, "thinking", text("thinking")),
            Some("signature_delta") => {
                block.insert(
                    "signature".into(),
                    d.get("signature").cloned().unwrap_or(Value::Null),
                );
            }
            Some("citations_delta") => {
                if let Some(c) = d.get("citation") {
                    match block.get_mut("citations").and_then(Value::as_array_mut) {
                        Some(list) => list.push(c.clone()),
                        None => {
                            block.insert("citations".into(), Value::Array(vec![c.clone()]));
                        }
                    }
                }
            }
            Some("input_json_delta") => {
                if let Some(s) = self.partial.get_mut(i) {
                    s.push_str(text("partial_json"));
                }
            }
            _ => {}
        }
    }

    /// Joins a tool call's argument fragments into its `input`.
    fn close(&mut self, i: usize) {
        let Some(text) = self.partial.get_mut(i).map(std::mem::take) else {
            return;
        };
        let Some(block) = self.blocks.get_mut(i).and_then(Value::as_object_mut) else {
            return;
        };
        let tool = matches!(
            block.get("type").and_then(Value::as_str),
            Some("tool_use" | "server_tool_use")
        );
        if !tool || text.is_empty() {
            return;
        }
        let input = serde_json::from_str::<Value>(&text).unwrap_or_else(|_| {
            self.broken = true;
            Value::String(text)
        });
        block.insert("input".into(), input);
    }

    /// The assembled `message`; `Err` carries it too when a tool call's arguments were not JSON
    /// (they are kept as a string).
    pub(crate) fn finish(mut self) -> Result<Value, Value> {
        for i in 0..self.partial.len() {
            self.close(i);
        }
        let mut message = self.message;
        message.insert(
            "content".into(),
            Value::Array(self.blocks.into_iter().filter(|b| !b.is_null()).collect()),
        );
        let message = Value::Object(message);
        if self.broken { Err(message) } else { Ok(message) }
    }
}

fn append(block: &mut Map<String, Value>, field: &str, text: &str) {
    match block.get_mut(field) {
        Some(Value::String(s)) => s.push_str(text),
        _ => {
            block.insert(field.into(), Value::from(text));
        }
    }
}
