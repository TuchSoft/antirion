//! The background reader of one upstream session: while no request is in flight it holds the
//! session's server-to-client stream, answers the `ping`s the server sends there, invalidates the
//! lists a `list_changed` notification names, and hands every other message to the client's own
//! server-to-client stream.
//!
//! The reader is the only consumer of the session's standalone stream, so a client's `GET` never
//! opens a second one; it subscribes to this reader instead. It reads from that stream only, never
//! from the stream of a request in flight, so the two never contend for the same bytes.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::http::Env;
use crate::link::Link;
use crate::lists::Kind;
use crate::sse::ReadError;
use crate::upstream::Source;

/// Silence after which the reader looks at its cancellation token.
const IDLE: Duration = Duration::from_secs(3600);
/// How many times a stream that ended is re-opened before the reader gives up; a session the server
/// has ended answers the `GET` with `404` and the reader stops after the last attempt.
const REOPENS: u8 = 3;
/// The pause before re-opening an ended stream.
const REOPEN_DELAY: Duration = Duration::from_secs(1);

/// A message a client's stream receives, with the SSE event id it came with.
pub(crate) type Item = (Value, Option<String>);

/// One upstream session's background reader.
#[derive(Debug)]
pub(crate) struct Watch {
    sid: String,
    /// The client streams that receive this session's messages.
    subscribers: Mutex<Vec<mpsc::UnboundedSender<Item>>>,
    stop: CancellationToken,
}

impl Watch {
    /// A receiver of the messages the reader consumes; `None` once it has stopped.
    pub(crate) fn subscribe(&self) -> Option<Messages> {
        if self.stop.is_cancelled() {
            return None;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        self.subscribers.lock().push(tx);
        Some(Messages(rx))
    }

    /// Hands a message to every subscribed client stream, dropping the ones that went away.
    fn dispatch(&self, msg: Value, id: Option<String>) {
        self.subscribers
            .lock()
            .retain(|tx| tx.send((msg.clone(), id.clone())).is_ok());
    }

    /// Stops the reader.
    pub(crate) fn stop(&self) {
        self.stop.cancel();
    }
}

/// The messages of one session a client's server-to-client stream receives.
#[derive(Debug)]
pub(crate) struct Messages(mpsc::UnboundedReceiver<Item>);

impl Messages {
    /// The next message with the SSE event id it came with.
    pub(crate) async fn next(&mut self, idle: Duration) -> Result<Item, ReadError> {
        match tokio::time::timeout(idle, self.0.recv()).await {
            Err(_) => Err(ReadError::Idle),
            Ok(None) => Err(ReadError::Ended),
            Ok(Some(m)) => Ok(m),
        }
    }
}

/// Opens the server-to-client stream of `link`'s session and reads it in the background. `None` when
/// the link has no upstream session, when the server offers no such stream, or when the session
/// already has a reader.
pub(crate) async fn start(env: &Env, link: &Link) -> Option<Arc<Watch>> {
    let sid = link.sid.clone()?;
    if env.state.watch(&sid).is_some() {
        return None;
    }
    // Bounded like the session's own initialization: a server that never answers its stream costs
    // the session no reader, not a delayed `initialize`.
    let src = match tokio::time::timeout(link.server.init_timeout, link.listen(env, None)).await {
        Ok(Some(src)) => src,
        _ => {
            tracing::debug!(
                server = %link.server.name,
                "the server offers no server-to-client stream; the session has no background reader"
            );
            return None;
        }
    };
    let watch = Arc::new(Watch {
        sid: sid.clone(),
        subscribers: Mutex::new(Vec::new()),
        stop: env.shutdown.child_token(),
    });
    if !env.state.set_watch(sid, Arc::clone(&watch)) {
        return None;
    }
    env.tasks
        .spawn(pump(env.clone(), link.clone(), Arc::clone(&watch), src));
    Some(watch)
}

/// Reads `src` until the reader is stopped or the server ends the session, re-opening the stream
/// (after the last event seen) a bounded number of times.
async fn pump(env: Env, link: Link, watch: Arc<Watch>, mut src: Source) {
    let mut last: Option<String> = None;
    let mut reopens = 0u8;
    'stream: loop {
        loop {
            let next = tokio::select! {
                () = watch.stop.cancelled() => break 'stream,
                next = src.next_event(IDLE) => next,
            };
            match next {
                Ok((msg, id)) => {
                    reopens = 0;
                    if id.is_some() {
                        last = id.clone();
                    }
                    if link.answer_ping(&env, &msg).await {
                        continue;
                    }
                    if let Some(method) = msg.get("method").and_then(Value::as_str) {
                        for kind in Kind::of_notification(method) {
                            env.state.invalidate(link.server.generation, kind.method());
                        }
                    }
                    watch.dispatch(msg, id);
                }
                Err(ReadError::Idle) => {}
                Err(ReadError::Ended) => break,
            }
        }
        if reopens >= REOPENS {
            break;
        }
        reopens += 1;
        let opened = tokio::select! {
            () = watch.stop.cancelled() => break,
            () = tokio::time::sleep(REOPEN_DELAY) => link.listen(&env, last.as_deref()).await,
        };
        match opened {
            Some(next) => src = next,
            None => break,
        }
    }
    env.state.drop_watch(&watch.sid, &watch);
    watch.stop();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_message_goes_to_every_subscriber_and_a_gone_one_is_dropped() {
        let watch = Watch {
            sid: "s".into(),
            subscribers: Mutex::new(Vec::new()),
            stop: CancellationToken::new(),
        };
        let mut first = watch.subscribe().expect("a subscriber");
        let mut second = watch.subscribe().expect("a subscriber");
        watch.dispatch(
            json!({"jsonrpc": "2.0", "method": "notifications/message"}),
            Some("7".into()),
        );
        let (a, id) = first.0.try_recv().expect("the first subscriber");
        assert_eq!(a["method"], "notifications/message");
        assert_eq!(id.as_deref(), Some("7"));
        assert!(second.0.try_recv().is_ok());
        drop(first);
        watch.dispatch(
            json!({"jsonrpc": "2.0", "method": "notifications/progress"}),
            None,
        );
        assert!(
            watch.subscribers.lock().len() == 1,
            "a closed subscriber is forgotten"
        );
        assert!(second.0.try_recv().is_ok());
    }

    #[test]
    fn a_stopped_reader_accepts_no_subscriber() {
        let watch = Watch {
            sid: "s".into(),
            subscribers: Mutex::new(Vec::new()),
            stop: CancellationToken::new(),
        };
        watch.stop();
        assert!(watch.subscribe().is_none());
    }
}
