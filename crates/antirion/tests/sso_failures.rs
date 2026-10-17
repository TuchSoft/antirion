//! Failed single sign-ons over a whole node: a browser is sent back to the console's sign-in page
//! (a test sign-in: to where it was started) with a stable code and a short message, while a client
//! asking for JSON keeps the Admin API error.

use std::net::SocketAddr;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const TOKEN: &str = "sso-failures-bootstrap-token-0123456789abcdef";
const BROWSER: (&str, &str) = (
    "accept",
    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
);
const JSON: (&str, &str) = ("accept", "application/json");
const BEARER: (&str, &str) = (
    "authorization",
    "Bearer sso-failures-bootstrap-token-0123456789abcdef",
);

struct Answer {
    status: u16,
    head: String,
    body: String,
}

impl Answer {
    fn header(&self, name: &str) -> Vec<String> {
        self.head
            .lines()
            .filter_map(|l| l.split_once(':'))
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_owned())
            .collect()
    }

    fn location(&self) -> String {
        self.header("location").into_iter().next().unwrap_or_default()
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

async fn call(addr: SocketAddr, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Answer {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nhost: test\r\nconnection: close\r\ncontent-length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        request.push_str(&format!("{k}: {v}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    Answer {
        status,
        head: head.to_owned(),
        body: body.to_owned(),
    }
}

/// An OpenID provider that only publishes its discovery document.
async fn idp(listener: TcpListener) {
    let base = format!("http://{}", listener.local_addr().unwrap());
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let doc = json!({
            "issuer": base,
            "authorization_endpoint": format!("{base}/authorize"),
            "token_endpoint": format!("{base}/token"),
            "jwks_uri": format!("{base}/jwks"),
        })
        .to_string();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{doc}",
            doc.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
    }
}

/// The value of `name` in the query of `url`.
fn param(url: &str, name: &str) -> String {
    url.split_once('?')
        .map(|(_, q)| q)
        .unwrap_or_default()
        .split('&')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_owned())
        .unwrap_or_default()
}

/// The `name=value` of the cookie `name` that `answer` sets.
fn cookie(answer: &Answer, name: &str) -> String {
    answer
        .header("set-cookie")
        .into_iter()
        .filter_map(|c| c.split(';').next().map(str::to_owned))
        .find(|c| c.starts_with(&format!("{name}=")))
        .unwrap_or_default()
}

/// Asserts that `answer` sends the browser to `landing` with `code` and a message.
fn lands(answer: &Answer, landing: &str, code: &str) {
    assert_eq!(answer.status, 302, "{} {}", answer.head, answer.body);
    let to = answer.location();
    assert!(to.starts_with(landing), "{to} should start with {landing}");
    assert_eq!(param(&to, "error"), code, "{to}");
    assert!(!param(&to, "message").is_empty(), "{to}");
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_sign_ins_return_to_the_console() {
    let dir = ScratchDir::new("sso-failures");
    let mock = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", mock.local_addr().unwrap());
    let mut mock_task = JoinSet::new();
    mock_task.spawn(idp(mock));
    let config = json!({
        "version": 1,
        "server": {"listen": "127.0.0.1:0"},
        "admin": {"listen": "127.0.0.1:0"},
        "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
        "auth": {"admin": {
            "bootstrap_token": TOKEN,
            "sso": {
                "oidc": {
                    "corp": {"issuer": issuer, "client_id": "gateway", "client_secret": "s3cret"},
                    "down": {"issuer": "http://127.0.0.1:9", "client_id": "gateway", "client_secret": "s3cret"}
                },
                "oauth": {"gh": {"type": "github", "client_id": "gateway", "client_secret": "s3cret",
                                 "api_base_url": "http://127.0.0.1:9"}},
                "saml": {"idp": {"idp": {"metadata_url": "http://127.0.0.1:9/metadata"}}}
            }
        }}
    });
    let file = dir.path().join("gw.json");
    std::fs::write(&file, config.to_string()).unwrap();
    let node = start(ServeOptions {
        config: Some(file),
        test_mode: true,
    })
    .await
    .unwrap();
    let addr = node.admin_addr().unwrap();
    let shutdown = node.rt.shutdown.clone();
    let mut tasks = JoinSet::new();
    tasks.spawn(node.run());

    // OIDC: a provider that cannot be reached, an unknown provider, a callback without a sign-in.
    let down = call(
        addr,
        "GET",
        "/admin/v1/auth/oidc/down/login?return_to=/ui/",
        &[BROWSER],
        "",
    )
    .await;
    lands(&down, "/ui/login?", "sso_provider_error");
    let down = call(addr, "GET", "/admin/v1/auth/oidc/down/login", &[JSON], "").await;
    assert_eq!(down.status, 502, "{}", down.body);
    assert_eq!(down.json()["error"]["code"], "idp_unavailable");
    let unknown = call(addr, "GET", "/admin/v1/auth/oidc/nobody/login", &[], "").await;
    lands(&unknown, "/ui/login?", "sso_provider_unknown");
    let stray = call(
        addr,
        "GET",
        "/admin/v1/auth/oidc/corp/callback?code=c&state=forged",
        &[BROWSER],
        "",
    )
    .await;
    lands(&stray, "/ui/login?", "sso_state_invalid");
    let stray = call(
        addr,
        "GET",
        "/admin/v1/auth/oidc/corp/callback?code=c&state=forged",
        &[JSON],
        "",
    )
    .await;
    assert_eq!(stray.status, 400, "{}", stray.body);
    assert_eq!(stray.json()["error"]["code"], "invalid_parameter");

    // A test sign-in the provider refused returns to where it was started, with the same parameters.
    let started = call(
        addr,
        "GET",
        "/admin/v1/auth/oidc/corp/login?test=true&return_to=%2Fui%2Fsettings%3Fsection%3Dsign-in",
        &[BEARER, BROWSER],
        "",
    )
    .await;
    assert_eq!(started.status, 302, "{}", started.body);
    assert!(started.location().starts_with(&format!("{issuer}/authorize?")));
    let state = param(&started.location(), "state");
    let binding = cookie(&started, "antirion_sso");
    let refused = call(
        addr,
        "GET",
        &format!("/admin/v1/auth/oidc/corp/callback?error=access_denied&error_description=secret%20stuff&state={state}"),
        &[BROWSER, ("cookie", &binding)],
        "",
    )
    .await;
    lands(&refused, "/ui/settings?section=sign-in&error=", "sso_denied");
    assert!(!refused.location().contains("secret"), "{}", refused.location());
    assert!(
        refused
            .header("set-cookie")
            .iter()
            .any(|c| c.starts_with("antirion_sso=;")),
        "the binding is cleared: {}",
        refused.head
    );
    // A test sign-in with no return_to lands on the sign-in settings.
    let test_down = call(
        addr,
        "GET",
        "/admin/v1/auth/oidc/down/login?test=true",
        &[BEARER, BROWSER],
        "",
    )
    .await;
    lands(
        &test_down,
        "/ui/settings?section=sign-in&error=",
        "sso_provider_error",
    );

    // OAuth: a callback from another browser, then the provider's refusal.
    let started = call(
        addr,
        "GET",
        "/admin/v1/auth/oauth/gh/login?return_to=/ui/",
        &[BROWSER],
        "",
    )
    .await;
    assert_eq!(started.status, 302, "{}", started.body);
    let state = param(&started.location(), "state");
    let binding = cookie(&started, "antirion_sso");
    let elsewhere = call(
        addr,
        "GET",
        &format!("/admin/v1/auth/oauth/gh/callback?code=c&state={state}"),
        &[BROWSER],
        "",
    )
    .await;
    lands(&elsewhere, "/ui/login?", "sso_state_invalid");
    let denied = call(
        addr,
        "GET",
        &format!("/admin/v1/auth/oauth/gh/callback?error=access_denied&state={state}"),
        &[BROWSER, ("cookie", &binding)],
        "",
    )
    .await;
    lands(&denied, "/ui/login?", "sso_denied");
    let json = call(
        addr,
        "GET",
        "/admin/v1/auth/oauth/gh/callback?state=x",
        &[JSON],
        "",
    )
    .await;
    assert_eq!(json.status, 400, "{}", json.body);

    // SAML: a response that cannot be read, one that answers no sign-in, an unreachable provider.
    let form = ("content-type", "application/x-www-form-urlencoded");
    let garbled = call(
        addr,
        "POST",
        "/admin/v1/auth/saml/idp/acs",
        &[BROWSER, form],
        "SAMLResponse=%25%25",
    )
    .await;
    lands(&garbled, "/ui/login?", "sso_response_invalid");
    let garbled = call(
        addr,
        "POST",
        "/admin/v1/auth/saml/idp/acs",
        &[JSON, form],
        "SAMLResponse=%25%25",
    )
    .await;
    assert_eq!(garbled.status, 400, "{}", garbled.body);
    let response = "<samlp:Response xmlns:samlp=\"urn:oasis:names:tc:SAML:2.0:protocol\" ID=\"_r\" \
                    Version=\"2.0\" InResponseTo=\"_nothing\"/>";
    let encoded = STANDARD.encode(response);
    let orphan = call(
        addr,
        "POST",
        "/admin/v1/auth/saml/idp/acs",
        &[BROWSER, form],
        &format!("SAMLResponse={}", urlencode(&encoded)),
    )
    .await;
    lands(&orphan, "/ui/login?", "sso_state_invalid");
    let down = call(addr, "GET", "/admin/v1/auth/saml/idp/login", &[BROWSER], "").await;
    lands(&down, "/ui/login?", "sso_provider_error");
    let test_down = call(
        addr,
        "GET",
        "/admin/v1/auth/saml/idp/login?test=true&return_to=%2Fui%2Fsettings",
        &[BEARER, BROWSER],
        "",
    )
    .await;
    lands(&test_down, "/ui/settings?error=", "sso_provider_error");

    shutdown.cancel();
    while tasks.join_next().await.is_some() {}
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => char::from(b).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
