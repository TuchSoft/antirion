//! `POST /admin/v1/guardrails:test`: a guardrail spec that is not saved, tried on a sample: the
//! verdict, the matches with what replaces them, the redacted text and what restore gives back;
//! the validation errors of a save; nothing stored.

use std::net::SocketAddr;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinSet;

const TOKEN: &str = "guardrail-unsaved-bootstrap-token-0123456789";

/// One admin call on a fresh connection: (status, JSON body).
async fn admin(addr: SocketAddr, method: &str, path: &str, body: &Value) -> (u16, Value) {
    let body = if body.is_null() {
        String::new()
    } else {
        body.to_string()
    };
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "{method} /admin/v1{path} HTTP/1.1\r\nhost: test\r\nconnection: close\r\ncontent-type: application/json\r\n\
         authorization: Bearer {TOKEN}\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, rest) = text.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    let chunked = head.to_ascii_lowercase().contains("transfer-encoding: chunked");
    let rest = if chunked {
        let mut out = String::new();
        let mut s = rest;
        while let Some((size, r)) = s.split_once("\r\n") {
            let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
            if n == 0 {
                break;
            }
            out.push_str(&r[..n]);
            s = &r[n + 2..];
        }
        out
    } else {
        rest.to_owned()
    };
    (status, serde_json::from_str(&rest).unwrap_or(Value::Null))
}

fn paths(error: &Value) -> Vec<String> {
    error["error"]["details"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d["path"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unsaved_spec_is_tried_with_the_validation_of_a_save_and_stored_nowhere() {
    let dir = ScratchDir::new("guardrail-unsaved");
    let config = json!({
        "version": 1,
        "server": {"listen": "127.0.0.1:0"},
        "admin": {"listen": "127.0.0.1:0"},
        "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
        "auth": {"admin": {"bootstrap_token": TOKEN}},
        "guardrails": {"definitions": {
            "personal": {"type": "pii", "pii": {"entities": ["email"]}}
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

    // A reversible pii rule with a custom entity, tried at the input stage (the default).
    let text = "call +5511978399280 or +5511978399280, CPF 529.982.247-25";
    let (status, result) = admin(
        addr,
        "POST",
        "/guardrails:test",
        &json!({
            "spec": {"type": "pii", "action": "redact", "pii": {
                "entities": ["phone"], "reversible": true,
                "custom": [{"name": "cpf", "pattern": "[0-9]{3}\\.[0-9]{3}\\.[0-9]{3}-[0-9]{2}", "validate": "cpf"}]}},
            "input": {"text": text}
        }),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["verdict"], "redact", "{result}");
    assert_eq!(result["category"], "pii", "{result}");
    assert_eq!(
        result["redacted"], "call [PHONE_1] or [PHONE_1], CPF [CPF_1]",
        "{result}"
    );
    assert_eq!(result["restored"], text, "{result}");
    assert_eq!(
        result["spans"],
        json!([
            {"start": 5, "end": 19, "entity": "phone", "replacement": "[PHONE_1]"},
            {"start": 23, "end": 37, "entity": "phone", "replacement": "[PHONE_1]"},
            {"start": 43, "end": 57, "entity": "cpf", "replacement": "[CPF_1]"}
        ]),
        "{result}"
    );

    // A reversible pattern rule takes its name as the label; numbering follows the values.
    let (status, result) = admin(
        addr,
        "POST",
        "/guardrails:test",
        &json!({
            "name": "account",
            "spec": {"type": "regex", "action": "redact", "regex": {"deny": ["ACCT-[0-9]{4}"], "reversible": true}},
            "input": {"stage": "input", "text": "ACCT-0001 ACCT-0002 ACCT-0001"}
        }),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(
        result["redacted"], "[ACCOUNT_1] [ACCOUNT_2] [ACCOUNT_1]",
        "{result}"
    );
    assert_eq!(result["restored"], "ACCT-0001 ACCT-0002 ACCT-0001", "{result}");

    // Editing a saved rule: the form's values, not the saved ones; a mask replaces in place.
    let (status, result) = admin(
        addr,
        "POST",
        "/guardrails:test",
        &json!({
            "name": "personal",
            "spec": {"type": "pii", "replacement": "mask", "pii": {"entities": ["phone"]}},
            "input": {"text": "mail ana@acme.example, call +5511978399280"}
        }),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(
        result["redacted"], "mail ana@acme.example, call **************",
        "{result}"
    );
    assert_eq!(result["spans"][0]["replacement"], "**************", "{result}");
    assert!(result["restored"].is_null(), "{result}");

    // The output stage of a blocking pattern rule.
    let (status, result) = admin(
        addr,
        "POST",
        "/guardrails:test",
        &json!({
            "spec": {"type": "regex", "regex": {"deny": ["falcon"]}},
            "input": {"stage": "output", "text": "project falcon"}
        }),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["verdict"], "block", "{result}");
    assert_eq!(result["category"], "deny_pattern", "{result}");
    assert!(result["spans"][0]["replacement"].is_null(), "{result}");

    // The validation errors of a save.
    let (status, error) = admin(
        addr,
        "POST",
        "/guardrails:test",
        &json!({
            "spec": {"type": "pii", "action": "block", "pii": {"reversible": true,
                "custom": [{"name": "cpf", "pattern": "(unclosed"}]}},
            "input": {"text": "x"}
        }),
    )
    .await;
    assert_eq!(status, 400, "{error}");
    assert_eq!(
        paths(&error),
        [
            "guardrails.definitions.unsaved.pii.custom[0].pattern",
            "guardrails.definitions.unsaved.pii.reversible"
        ],
        "{error}"
    );
    let (status, error) = admin(
        addr,
        "POST",
        "/guardrails:test",
        &json!({"spec": {"type": "pii", "pii": {"custom": [{"name": "cpf"}]}}, "input": {"text": "x"}}),
    )
    .await;
    assert_eq!(status, 400, "{error}");
    let (status, error) = admin(
        addr,
        "POST",
        "/guardrails:test",
        &json!({"spec": {"type": "regex", "regex": {"deny": ["x"]}}, "input": {"text": "x", "stage": "later"}}),
    )
    .await;
    assert_eq!(status, 400, "{error}");
    assert_eq!(paths(&error), ["input.stage"], "{error}");

    // Nothing was stored, and the saved rule is unchanged.
    let (status, _) = admin(addr, "GET", "/guardrails/unsaved", &Value::Null).await;
    assert_eq!(status, 404);
    let (status, _) = admin(addr, "GET", "/guardrails/account", &Value::Null).await;
    assert_eq!(status, 404);
    let (status, saved) = admin(addr, "GET", "/guardrails/personal", &Value::Null).await;
    assert_eq!(status, 200, "{saved}");
    assert_eq!(saved["spec"]["pii"]["entities"], json!(["email"]), "{saved}");

    // The saved rule's own test reports replacements too.
    let (status, result) = admin(
        addr,
        "POST",
        "/guardrails/personal:test",
        &json!({"stage": "input", "text": "mail ana@acme.example"}),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["redacted"], "mail [EMAIL]", "{result}");
    assert_eq!(result["spans"][0]["replacement"], "[EMAIL]", "{result}");

    shutdown.cancel();
    while tasks.join_next().await.is_some() {}
}
