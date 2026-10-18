//! The response a stream's client was sent, for content logging: the frames the relay wrote, after
//! translation, guardrails and extensions, read back and assembled into the non-streaming body of
//! the client's own dialect, so the logged response matches the logged request.

use antirion_wire::eventstream::{self, Decoder};
use antirion_wire::sse::SseParser;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

use super::ClientDialect;
use super::assemble::{Assembler, ConverseAssembler, GeminiAssembler, MessageAssembler, ResponsesAssembler};

/// The client's framing, as the relay writes it before any re-framing for the wire.
#[derive(Debug)]
enum Reader {
    Sse(SseParser),
    EventStream(Decoder),
}

#[derive(Debug)]
enum Body {
    Chat(Assembler),
    Messages(MessageAssembler),
    Responses(ResponsesAssembler),
    Gemini(GeminiAssembler),
    /// ConverseStream events, until a `chunk` message shows an InvokeModel stream instead.
    Converse(ConverseAssembler),
}

#[derive(Debug)]
pub(crate) struct Sent {
    reader: Reader,
    body: Body,
    /// A payload was assembled.
    seen: bool,
    /// The frames stopped parsing; nothing more is read.
    broken: bool,
}

impl Sent {
    /// A reader of the frames written to a client of `client`'s dialect (`invoke`: Anthropic events
    /// the writer re-frames for an InvokeModel client).
    pub(crate) fn new(client: ClientDialect, invoke: bool) -> Sent {
        let sse = || Reader::Sse(SseParser::new(usize::MAX));
        let (reader, body) = match client {
            _ if invoke => (sse(), Body::Messages(MessageAssembler::default())),
            ClientDialect::Chat => (sse(), Body::Chat(Assembler::default())),
            ClientDialect::Anthropic => (sse(), Body::Messages(MessageAssembler::default())),
            ClientDialect::Responses => (sse(), Body::Responses(ResponsesAssembler::default())),
            ClientDialect::Gemini => (sse(), Body::Gemini(GeminiAssembler::default())),
            ClientDialect::Bedrock => (
                Reader::EventStream(Decoder::new(usize::MAX)),
                Body::Converse(ConverseAssembler::default()),
            ),
        };
        Sent {
            reader,
            body,
            seen: false,
            broken: false,
        }
    }

    /// Reads one written frame.
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        if self.broken {
            return;
        }
        match &mut self.reader {
            Reader::Sse(parser) => {
                let mut events = Vec::new();
                if parser.push(bytes, &mut events).is_err() {
                    self.broken = true;
                    return;
                }
                for ev in events {
                    let data = ev.data.trim();
                    if data.is_empty() || data == "[DONE]" {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        self.take(None, &v);
                    }
                }
            }
            Reader::EventStream(decoder) => {
                let mut messages = Vec::new();
                if decoder.push(bytes, &mut messages).is_err() {
                    self.broken = true;
                    return;
                }
                for m in &messages {
                    self.message(m);
                }
            }
        }
    }

    /// One event-stream message: a ConverseStream event, or an InvokeModel `chunk` around an
    /// Anthropic event.
    fn message(&mut self, m: &eventstream::Message) {
        if m.header(":message-type") != Some("event") {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&m.payload) else {
            return;
        };
        let kind = m.header(":event-type").unwrap_or_default();
        if kind != "chunk" {
            self.take(Some(kind), &v);
            return;
        }
        let event = v
            .get("bytes")
            .and_then(Value::as_str)
            .and_then(|b| STANDARD.decode(b).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        let Some(event) = event.filter(|e| e.get("type").is_some()) else {
            return;
        };
        if matches!(self.body, Body::Converse(_)) && !self.seen {
            self.body = Body::Messages(MessageAssembler::default());
        }
        self.take(None, &event);
    }

    fn take(&mut self, kind: Option<&str>, v: &Value) {
        self.seen = true;
        match &mut self.body {
            Body::Chat(a) => a.push(v),
            Body::Messages(a) => a.push(v),
            Body::Responses(a) => a.push(v),
            Body::Gemini(a) => a.push(v),
            Body::Converse(a) => a.push(kind.unwrap_or_default(), v),
        }
    }

    /// The assembled response, when the client was sent any event.
    pub(crate) fn finish(self) -> Option<Value> {
        if !self.seen {
            return None;
        }
        Some(match self.body {
            Body::Chat(a) => a.finish(),
            Body::Messages(a) => a.finish().unwrap_or_else(|kept| kept),
            Body::Responses(a) => a.finish(),
            Body::Gemini(a) => a.finish(),
            Body::Converse(a) => a.finish(),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::relay::render::{chat_frame, converse_frame, named};

    #[test]
    fn chat_frames_are_assembled() {
        let mut s = Sent::new(ClientDialect::Chat, false);
        s.push(b": ping\n\n");
        s.push(&chat_frame(
            &json!({"id": "c", "choices": [{"index": 0, "delta": {"content": "Hel"}}]}),
        ));
        s.push(&chat_frame(
            &json!({"choices": [{"index": 0, "delta": {"content": "lo"}, "finish_reason": "stop"}]}),
        ));
        s.push(b"data: [DONE]\n\n");
        let v = s.finish().unwrap();
        assert_eq!(v["choices"][0]["message"]["content"], "Hello");
        assert_eq!(v["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn nothing_sent_logs_nothing() {
        let mut s = Sent::new(ClientDialect::Anthropic, false);
        s.push(b": keepalive\n\n");
        assert!(s.finish().is_none());
    }

    #[test]
    fn invoke_frames_are_read_as_messages() {
        let mut s = Sent::new(ClientDialect::Bedrock, true);
        s.push(&named(
            &json!({"type": "message_start", "message": {"id": "m", "role": "assistant", "content": []}}),
        ));
        s.push(&named(&json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})));
        s.push(&named(&json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "hi"}})));
        let v = s.finish().unwrap();
        assert_eq!(v["content"][0]["text"], "hi");
    }

    #[test]
    fn event_stream_frames_are_read() {
        let mut s = Sent::new(ClientDialect::Bedrock, false);
        s.push(&converse_frame("messageStart", &json!({"role": "assistant"})));
        s.push(&converse_frame(
            "contentBlockDelta",
            &json!({"contentBlockIndex": 0, "delta": {"text": "hi"}}),
        ));
        s.push(&converse_frame("messageStop", &json!({"stopReason": "end_turn"})));
        let v = s.finish().unwrap();
        assert_eq!(v["output"]["message"]["content"][0]["text"], "hi");
        assert_eq!(v["stopReason"], "end_turn");

        let mut s = Sent::new(ClientDialect::Bedrock, false);
        let event = json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": "yo"}});
        let chunk = json!({"bytes": STANDARD.encode(event.to_string())});
        s.push(&eventstream::encode(
            &[
                (":event-type", "chunk"),
                (":content-type", "application/json"),
                (":message-type", "event"),
            ],
            chunk.to_string().as_bytes(),
        ));
        assert_eq!(s.finish().unwrap()["content"][0]["text"], "yo");
    }
}
