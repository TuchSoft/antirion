//! The non-streaming equivalent of a Responses stream: the response of its terminal event
//! (`response.completed`, `response.incomplete`, `response.failed`), or, for a stream that ended
//! without one, the announced response with the output items finished so far and the text of the
//! ones still open.

use std::collections::BTreeMap;

use serde_json::{Value, json};

#[derive(Debug, Default)]
pub(crate) struct ResponsesAssembler {
    /// The response of `response.created` or `response.in_progress`.
    started: Option<Value>,
    /// The response of the terminal event.
    ended: Option<Value>,
    /// Finished output items, by `output_index`.
    items: BTreeMap<u64, Value>,
    /// Output text of items not finished yet, by `output_index`.
    text: BTreeMap<u64, String>,
}

impl ResponsesAssembler {
    /// Takes one parsed event.
    pub(crate) fn push(&mut self, p: &Value) {
        let index = p.get("output_index").and_then(Value::as_u64).unwrap_or(0);
        match p.get("type").and_then(Value::as_str) {
            Some("response.created" | "response.in_progress") => self.started = p.get("response").cloned(),
            Some("response.completed" | "response.incomplete" | "response.failed") => {
                self.ended = p.get("response").cloned();
            }
            Some("response.output_item.done") => {
                if let Some(item) = p.get("item") {
                    self.items.insert(index, item.clone());
                    self.text.remove(&index);
                }
            }
            Some("response.output_text.delta") => {
                if let Some(d) = p.get("delta").and_then(Value::as_str) {
                    self.text.entry(index).or_default().push_str(d);
                }
            }
            _ => {}
        }
    }

    /// The assembled `response` object.
    pub(crate) fn finish(self) -> Value {
        if let Some(done) = self.ended {
            return done;
        }
        let mut response = self
            .started
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({"object": "response"}));
        let mut output = self.items;
        for (i, t) in self.text {
            output.entry(i).or_insert_with(|| {
                json!({"type": "message", "role": "assistant", "status": "incomplete",
                    "content": [{"type": "output_text", "text": t, "annotations": []}]})
            });
        }
        if let Some(o) = response.as_object_mut() {
            o.insert("status".into(), Value::from("incomplete"));
            o.insert("output".into(), Value::Array(output.into_values().collect()));
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_terminal_response_wins() {
        let mut a = ResponsesAssembler::default();
        a.push(
            &json!({"type": "response.created", "response": {"id": "r", "object": "response", "output": []}}),
        );
        a.push(&json!({"type": "response.output_text.delta", "output_index": 0, "delta": "hi"}));
        a.push(&json!({"type": "response.completed", "response": {"id": "r", "status": "completed", "output": [1]}}));
        assert_eq!(a.finish()["status"], "completed");
    }

    #[test]
    fn a_cut_stream_keeps_what_was_sent() {
        let mut a = ResponsesAssembler::default();
        a.push(
            &json!({"type": "response.created", "response": {"id": "r", "object": "response", "output": []}}),
        );
        a.push(&json!({"type": "response.output_text.delta", "output_index": 0, "delta": "Hel"}));
        a.push(&json!({"type": "response.output_text.delta", "output_index": 0, "delta": "lo"}));
        let v = a.finish();
        assert_eq!(v["id"], "r");
        assert_eq!(v["status"], "incomplete");
        assert_eq!(v["output"][0]["content"][0]["text"], "Hello");
    }
}
