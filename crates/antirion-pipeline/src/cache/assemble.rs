//! The non-streaming shape of a stored stream, for requests that hit it without streaming and for
//! the storability checks.

use antirion_cache::StoredEvent;
use serde_json::Value;

use crate::relay::assemble::{Assembler, MessageAssembler};

/// A tool call whose arguments are not JSON: the stream is not storable.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct BrokenToolCall;

/// The assembled response of `endpoint_id`'s stream, when its dialect has one.
pub(super) fn assemble(endpoint_id: &str, events: &[StoredEvent]) -> Result<Option<Value>, BrokenToolCall> {
    let payloads = || {
        events
            .iter()
            .filter_map(|e| serde_json::from_str::<Value>(&e.data).ok())
    };
    match endpoint_id {
        "chat_completions" => {
            let mut a = Assembler::default();
            for p in payloads() {
                a.push(&p);
            }
            Ok(Some(a.finish()))
        }
        "messages" => anthropic(payloads()).map(Some),
        "responses" => Ok(payloads()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("response.completed"))
            .find_map(|p| p.get("response").cloned())),
        _ => Ok(None),
    }
}

/// An Anthropic message from its events.
fn anthropic(payloads: impl Iterator<Item = Value>) -> Result<Value, BrokenToolCall> {
    let mut a = MessageAssembler::default();
    for p in payloads {
        a.push(&p);
    }
    a.finish().map_err(|_| BrokenToolCall)
}

/// The usage member of an assembled response, or the one a stream reported.
pub(super) fn usage(assembled: Option<&Value>, reported: Option<&Value>) -> Option<Value> {
    assembled
        .and_then(|v| v.get("usage"))
        .filter(|u| !u.is_null())
        .or(reported)
        .cloned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ev(data: Value) -> StoredEvent {
        StoredEvent {
            event: data.get("type").and_then(Value::as_str).map(str::to_owned),
            data: data.to_string(),
        }
    }

    #[test]
    fn anthropic_messages_are_assembled() {
        let events = [
            ev(
                json!({"type": "message_start", "message": {"id": "m", "role": "assistant", "content": [],
                "stop_reason": null, "usage": {"input_tokens": 5, "output_tokens": 1}}}),
            ),
            ev(
                json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            ),
            ev(
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hel"}}),
            ),
            ev(
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "lo"}}),
            ),
            ev(json!({"type": "content_block_stop", "index": 0})),
            ev(
                json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "t", "name": "f", "input": {}}}),
            ),
            ev(
                json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"a\":"}}),
            ),
            ev(
                json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "1}"}}),
            ),
            ev(json!({"type": "content_block_stop", "index": 1})),
            ev(
                json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 7}}),
            ),
            ev(json!({"type": "message_stop"})),
        ];
        let m = assemble("messages", &events).unwrap().unwrap();
        assert_eq!(m["content"][0]["text"], "Hello");
        assert_eq!(m["content"][1]["input"], json!({"a": 1}));
        assert_eq!(m["stop_reason"], "tool_use");
        assert_eq!(m["usage"], json!({"input_tokens": 5, "output_tokens": 7}));
        let broken = [
            ev(
                json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "input": {}}}),
            ),
            ev(
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "{\"a\":"}}),
            ),
            ev(json!({"type": "content_block_stop", "index": 0})),
        ];
        assert_eq!(assemble("messages", &broken), Err(BrokenToolCall));
    }

    #[test]
    fn chat_streams_are_assembled() {
        let events = [
            ev(
                json!({"id": "c", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "a"}, "finish_reason": null}]}),
            ),
            ev(
                json!({"id": "c", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "b"}, "finish_reason": "stop"}]}),
            ),
        ];
        let v = assemble("chat_completions", &events).unwrap().unwrap();
        assert_eq!(v["choices"][0]["message"]["content"], "ab");
        assert_eq!(v["object"], "chat.completion");
    }
}
