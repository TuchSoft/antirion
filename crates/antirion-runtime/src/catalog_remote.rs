//! `catalog.remote`: fetches models.dev's model data periodically, through the egress policy, and
//! puts it in place of the bundled snapshot. The serving snapshot is rebuilt with it, so prices
//! and model metadata follow at once. A fetch that fails keeps the data in use; turning the setting
//! off goes back to the bundled snapshot.

use std::sync::Arc;
use std::time::{Duration, Instant};

use antirion_catalog::modelsdev::{self, ModelsDev};
use antirion_egress::{OutboundRequest, Payload, TrustClass};
use http::{HeaderMap, HeaderValue, Method, Uri};
use http_body_util::BodyExt;
use serde_json::Value;

use crate::Runtime;

/// How often the setting is read.
const POLL: Duration = Duration::from_secs(5);
/// The longest a fetch waits for its answer.
const TIMEOUT: Duration = Duration::from_secs(60);
/// The largest document read.
const MAX_BODY: usize = 64 << 20;
/// What a fetch waits after a failure before trying again.
const RETRY_AFTER: Duration = Duration::from_secs(15 * 60);

/// The settings of `catalog.remote` when it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub url: String,
    pub refresh: Duration,
}

impl Remote {
    /// The settings of a document, when `catalog.remote.enabled` is true.
    #[must_use]
    pub fn of(doc: &Value) -> Option<Remote> {
        let r = doc.pointer("/catalog/remote")?;
        if r.get("enabled").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        Some(Remote {
            url: r
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or(modelsdev::SOURCE_URL)
                .to_owned(),
            refresh: r
                .get("refresh")
                .and_then(Value::as_str)
                .and_then(antirion_config::scalar::parse_duration)
                .unwrap_or(Duration::from_secs(86_400)),
        })
    }
}

/// When the last fetch of a URL was made and whether it worked.
#[derive(Clone, Debug)]
struct Last {
    url: String,
    at: Instant,
    ok: bool,
}

/// Whether a fetch of `remote` is due after `last`.
fn due(remote: &Remote, last: Option<&Last>, now: Instant) -> bool {
    match last {
        None => true,
        Some(l) if l.url != remote.url => true,
        Some(l) => {
            let wait = if l.ok {
                remote.refresh
            } else {
                RETRY_AFTER.min(remote.refresh)
            };
            now.duration_since(l.at) >= wait
        }
    }
}

/// Fetches and reads the document at `url` through the serving snapshot's egress policy.
async fn fetch(rt: &Runtime, url: &str) -> Result<ModelsDev, String> {
    let snap = rt.gens.load();
    let uri: Uri = url.parse().map_err(|_| "the URL is not valid".to_owned())?;
    let tls = match snap.security.egress_tls.clone() {
        Some(t) => t,
        None => antirion_egress::TlsProfile::default_profile().map_err(|e| e.to_string())?,
    };
    let mut headers = HeaderMap::new();
    headers.insert(http::header::ACCEPT, HeaderValue::from_static("application/json"));
    let request = OutboundRequest {
        method: Method::GET,
        uri,
        headers,
        body: Payload::from(bytes::Bytes::new()),
        response_timeout: TIMEOUT,
        connect_timeout: None,
        fresh_connection: false,
        trust: TrustClass::Operator,
        tls,
        policy: Arc::clone(&snap.security.egress),
        resend_when_closed: true,
    };
    let resp = tokio::time::timeout(TIMEOUT, rt.egress.send(request))
        .await
        .map_err(|_| "no answer in time".to_owned())?
        .map_err(|e| e.to_string())?;
    if !resp.status.is_success() {
        return Err(format!("the server answered {}", resp.status.as_u16()));
    }
    let mut body = resp.body;
    let mut read = Vec::new();
    let reading = async {
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|e| e.to_string())?;
            if let Ok(data) = frame.into_data() {
                read.extend_from_slice(&data);
                if read.len() > MAX_BODY {
                    return Err("the document is too large".to_owned());
                }
            }
        }
        Ok(())
    };
    tokio::time::timeout(TIMEOUT, reading)
        .await
        .map_err(|_| "the document did not arrive in time".to_owned())??;
    let v: Value = serde_json::from_slice(&read).map_err(|_| "the document is not JSON".to_owned())?;
    let today = rt.now().to_rfc3339_seconds().chars().take(10).collect::<String>();
    ModelsDev::from_api(&v, &today).map_err(|e| e.to_string())
}

/// Follows `catalog.remote` until shutdown.
pub async fn run(rt: Arc<Runtime>) {
    let mut last: Option<Last> = None;
    let mut installed = false;
    loop {
        let snap = rt.gens.load();
        match Remote::of(&snap.config.doc) {
            None => {
                last = None;
                if installed {
                    modelsdev::install(None);
                    installed = false;
                    crate::secretwatch::rebuild(
                        &rt,
                        snap.generation,
                        "the bundled models.dev data is back in use",
                    )
                    .await;
                }
            }
            Some(remote) if due(&remote, last.as_ref(), Instant::now()) => {
                let result = fetch(&rt, &remote.url).await;
                let ok = result.is_ok();
                match result {
                    Ok(data) => {
                        let models = data.len();
                        modelsdev::install(Some(data));
                        installed = true;
                        let snap = rt.gens.load();
                        crate::secretwatch::rebuild(
                            &rt,
                            snap.generation,
                            "the models.dev data was refreshed",
                        )
                        .await;
                        tracing::info!(url = %remote.url, models, "the models.dev data was refreshed");
                    }
                    Err(why) => {
                        tracing::warn!(url = %remote.url, reason = %why, "the models.dev data could not be fetched; the data in use stays");
                    }
                }
                last = Some(Last {
                    url: remote.url,
                    at: Instant::now(),
                    ok,
                });
            }
            Some(_) => {}
        }
        tokio::select! {
            () = rt.shutdown.cancelled() => return,
            () = tokio::time::sleep(POLL) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_setting_is_off_unless_enabled() {
        assert_eq!(Remote::of(&json!({})), None);
        assert_eq!(
            Remote::of(&json!({"catalog": {"remote": {"enabled": false}}})),
            None
        );
        assert_eq!(
            Remote::of(&json!({"catalog": {"remote": {"enabled": true}}})),
            Some(Remote {
                url: modelsdev::SOURCE_URL.to_owned(),
                refresh: Duration::from_secs(86_400)
            })
        );
        let r = Remote::of(
            &json!({"catalog": {"remote": {"enabled": true, "url": "https://m/api.json",
                                                         "refresh": "6h"}}}),
        )
        .unwrap();
        assert_eq!(
            (r.url.as_str(), r.refresh),
            ("https://m/api.json", Duration::from_secs(6 * 3600))
        );
    }

    #[test]
    fn fetches_are_due_after_the_refresh_or_a_shorter_retry() {
        let r = Remote {
            url: "https://m".into(),
            refresh: Duration::from_secs(3600 * 24),
        };
        let now = Instant::now();
        assert!(due(&r, None, now));
        let at = |ago: u64, ok: bool, url: &str| Last {
            url: url.into(),
            at: now.checked_sub(Duration::from_secs(ago)).unwrap(),
            ok,
        };
        assert!(!due(&r, Some(&at(3600, true, "https://m")), now));
        assert!(due(&r, Some(&at(3600 * 24, true, "https://m")), now));
        assert!(
            due(&r, Some(&at(15 * 60, false, "https://m")), now),
            "a failure retries sooner"
        );
        assert!(
            due(&r, Some(&at(1, true, "https://other")), now),
            "a new URL is fetched at once"
        );
    }
}
