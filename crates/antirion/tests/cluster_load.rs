//! Three cluster nodes under concurrent load charge one budget exactly, leave nothing held, and
//! each comes to hold every node's ledger lines; the Admin API of any node reads, adjusts, resets
//! and rebuilds the budget counter requests charge. Run when both
//! `ANTIRION_TEST_POSTGRES_URL` names a PostgreSQL server the tests may create databases on and
//! `ANTIRION_TEST_VALKEY_URL` names a Valkey (or Redis) server:
//!
//! ```text
//! ANTIRION_TEST_POSTGRES_URL=postgres://antirion:antirion@127.0.0.1:55432/antirion \
//! ANTIRION_TEST_VALKEY_URL=redis://127.0.0.1:16379 cargo test -p antirion --test cluster_load
//! ```

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_postgres::NoTls;

const TOKEN: &str = "cluster-load-bootstrap-token-0123456789abcdef";
const ENCRYPTION_KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const NODES: usize = 3;
const REQUESTS: usize = 2000;
const IN_FLIGHT: usize = 64;

/// A fresh database on the test server.
async fn fresh_database(drivers: &mut JoinSet<()>) -> Option<String> {
    let base = std::env::var("ANTIRION_TEST_POSTGRES_URL").ok()?;
    let name = format!("load_{:x}", antirion_core::ResourceUid::random().0);
    let (client, conn) = tokio_postgres::connect(&base, NoTls).await.ok()?;
    drivers.spawn(async move {
        let _ = conn.await;
    });
    client
        .batch_execute(&format!("CREATE DATABASE {name}"))
        .await
        .ok()?;
    let (prefix, _) = base.rsplit_once('/')?;
    Some(format!("{prefix}/{name}"))
}

/// One request on a fresh connection: status, headers (lower-cased) and JSON body.
async fn call(addr: SocketAddr, method: &str, path: &str, auth: &str, body: &str) -> (u16, String, Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nhost: test\r\nconnection: close\r\nauthorization: Bearer {auth}\r\n\
         content-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let (head, rest) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let head = head.to_ascii_lowercase();
    let rest = if head.contains("transfer-encoding: chunked") {
        dechunk(rest)
    } else {
        rest.to_owned()
    };
    (status, head, serde_json::from_str(&rest).unwrap_or(Value::Null))
}

fn dechunk(mut s: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = s.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&rest[..n]);
        s = &rest[n + 2..];
    }
    out
}

/// An upstream that answers every chat completion with the same usage.
async fn upstream(listener: TcpListener) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        connections.spawn(async move {
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen);
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len: usize = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if body.len() >= len {
                        break;
                    }
                }
            }
            let reply = r#"{"id":"c1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                reply.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

/// A node's startup file: the shared store and counter store, a project with a monthly budget, and
/// a model whose every request costs whole dollars (3 input tokens at $1, 1 output token at $2).
fn config(
    dir: &ScratchDir,
    i: usize,
    store: &str,
    valkey: &str,
    prefix: &str,
    mock: SocketAddr,
) -> std::path::PathBuf {
    let doc = json!({
        "version": 1,
        "server": {"listen": "127.0.0.1:0"},
        "admin": {"listen": "127.0.0.1:0"},
        "cluster": {
            "mode": "cluster",
            "node_id": format!("load-{i}"),
            "counters": {"backend": "redis", "redis": {"url": valkey, "key_prefix": prefix}}
        },
        "store": {
            "backend": "postgres",
            "embedded": {"path": dir.path().join(format!("node-{i}")).to_string_lossy()},
            "postgres": {"url": store}
        },
        "security": {"secrets": {"encryption_keys": [ENCRYPTION_KEY]}},
        "auth": {"admin": {"bootstrap_token": TOKEN}},
        "providers": {"p": {"type": "openai", "base_url": format!("http://{mock}/v1"), "auth": {"api_key": "sk-up"}}},
        "deployments": {"p": {"provider": "p", "model": "up-model", "price": {"input": "1000000", "output": "2000000"}}},
        "models": {"m": {"target": {"deployment": "p"}}},
        "organizations": {"o": {}},
        "teams": {"t": {"organization": "o"}},
        "projects": {"p": {"team": "t", "policies": {"budgets": {
            "monthly": {"unit": "usd", "amount": "1000000000", "period": "monthly"}
        }}}}
    });
    let file = dir.path().join(format!("node-{i}.json"));
    std::fs::write(&file, doc.to_string()).unwrap();
    file
}

/// The project budget's settled spend and outstanding holds, as the counter store holds them: the
/// one budget counter under `prefix`.
async fn budget(valkey: &str, prefix: &str) -> (u128, u128) {
    let client = redis::Client::open(valkey).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let (_, keys): (u64, Vec<String>) = redis::cmd("SCAN")
        .arg(0)
        .arg("MATCH")
        .arg(format!("{prefix}b:*"))
        .arg("COUNT")
        .arg(100_000)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(keys.len(), 1, "one budget counter: {keys:?}");
    let (spent, reserved): (Option<String>, Option<String>) = redis::cmd("HMGET")
        .arg(&keys[0])
        .arg("s")
        .arg("r")
        .query_async(&mut conn)
        .await
        .unwrap();
    let num = |v: Option<String>| v.map_or(0, |v| v.parse::<u128>().unwrap());
    (num(spent), num(reserved))
}

/// A node's startup file and the node started from it: (data, admin) addresses and its shutdown.
async fn node(
    dir: &ScratchDir,
    i: usize,
    store: &str,
    valkey: &str,
    prefix: &str,
    mock: SocketAddr,
    tasks: &mut JoinSet<()>,
) -> ((SocketAddr, SocketAddr), Arc<antirion_runtime::Runtime>) {
    let file = config(dir, i, store, valkey, prefix, mock);
    let node = start(ServeOptions {
        config: Some(file),
        test_mode: false,
        no_config: None,
    })
    .await
    .unwrap();
    let addrs = (node.data_addr().unwrap(), node.admin_addr().unwrap());
    let rt = Arc::clone(&node.rt);
    tasks.spawn(async move {
        let _ = node.run().await;
    });
    (addrs, rt)
}

/// A USD amount as the Admin API writes it, in the counter's units (nano-dollars).
fn usd_units(text: &str) -> u128 {
    let (int, frac) = text.split_once('.').unwrap_or((text, ""));
    let frac = format!("{frac:0<9}");
    int.parse::<u128>().unwrap() * 1_000_000_000 + frac.parse::<u128>().unwrap()
}

/// The project budget's settled spend and outstanding holds, as a node's Admin API reads them.
async fn budget_via_admin(admin: SocketAddr) -> (u128, u128) {
    let (status, _, body) = call(
        admin,
        "GET",
        "/admin/v1/runtime/budgets/project/p/monthly",
        TOKEN,
        "",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    (
        usd_units(body["spent"].as_str().unwrap()),
        usd_units(body["reserved"].as_str().unwrap()),
    )
}

/// Requests in the ledger lines a node holds in memory.
async fn ledger_requests(admin: SocketAddr) -> u64 {
    let (_, _, summary) = call(admin, "GET", "/admin/v1/reports/spend/summary", TOKEN, "").await;
    summary["items"].as_array().map_or(0, |items| {
        items.iter().map(|r| r["requests"].as_u64().unwrap_or(0)).sum()
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn three_nodes_under_concurrent_load_charge_one_budget_exactly_and_share_their_ledgers() {
    let mut drivers = JoinSet::new();
    let (Some(store), Ok(valkey)) = (
        fresh_database(&mut drivers).await,
        std::env::var("ANTIRION_TEST_VALKEY_URL"),
    ) else {
        return;
    };
    let dir = ScratchDir::new("cluster-load");
    let prefix = format!("load{:x}:", antirion_core::ResourceUid::random().0);
    let mock = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = mock.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(upstream(mock));

    let mut nodes = Vec::new();
    for i in 0..NODES {
        nodes.push(node(&dir, i, &store, &valkey, &prefix, mock_addr, &mut tasks).await);
    }
    let admin = nodes[0].0.1;
    let limits = json!({
        "rpm": {"unit": "requests", "limit": 1_000_000_000u64, "per": "1m"},
        "tpm": {"unit": "tokens", "limit": 1_000_000_000_000u64, "per": "1m"}
    });
    let spec = json!({"name": "k", "spec": {"project": "p", "policies": {"limits": limits}}});
    let (status, _, key) = call(admin, "POST", "/admin/v1/keys?wait=10s", TOKEN, &spec.to_string()).await;
    assert_eq!(status, 201, "{key}");
    let secret: Arc<str> = key["secret"].as_str().unwrap().into();
    let chat = r#"{"model":"m","messages":[{"role":"user","content":"hello"}]}"#;

    // One request alone gives the cost of every request (they all have the same usage).
    let (status, _, body) = call(nodes[1].0.0, "POST", "/v1/chat/completions", &secret, chat).await;
    assert_eq!(status, 200, "{body}");
    let (one, held) = budget(&valkey, &prefix).await;
    assert!(one > 0 && held == 0, "spent {one}, reserved {held}");

    let gate = Arc::new(Semaphore::new(IN_FLIGHT));
    let mut load = JoinSet::new();
    for i in 1..REQUESTS {
        let permit = Arc::clone(&gate).acquire_owned().await.unwrap();
        let data = nodes[i % NODES].0.0;
        let secret = Arc::clone(&secret);
        load.spawn(async move {
            let (status, head, body) = call(data, "POST", "/v1/chat/completions", &secret, chat).await;
            drop(permit);
            (status, head, body)
        });
    }
    // A store call that misses its deadline under the burst refuses budgeted requests with 503
    // `limits_unavailable` while the node probes the store again; nothing else may fail.
    let mut answered = 1u64;
    while let Some(done) = load.join_next().await {
        let (status, head, body) = done.unwrap();
        match status {
            200 => answered += 1,
            503 => {
                assert_eq!(body["antirion"]["reason"], "limits_unavailable", "{body}");
                assert!(head.contains("x-antirion-degraded"), "{head}");
            }
            _ => panic!("{status}: {body}"),
        }
    }

    // The one budget counter every node charged: settled exactly once per answered request, nothing
    // left held (settlements queued while a node had the store down are replayed once it is back).
    let mut counter = (0, 0);
    for _ in 0..100 {
        counter = budget(&valkey, &prefix).await;
        if counter == (one * u128::from(answered), 0) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(counter, (one * u128::from(answered), 0), "{answered} answered");
    // Every node's ledger window comes to hold every node's lines.
    for ((_, admin), _) in &nodes {
        let mut seen = 0;
        for _ in 0..100 {
            seen = ledger_requests(*admin).await;
            if seen == answered {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(seen, answered, "the ledger of the node at {admin}");
    }

    for (_, rt) in &nodes {
        rt.shutdown.cancel();
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

/// A tenant's budget is grouped under its organization in the counter store. The Admin API names
/// the budget by its owner and must reach that same counter: reads show the spend requests
/// charged, `:adjust` and `:reset` change it, and a node rebuilding the counter from the ledger
/// writes it where requests will look.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_admin_api_reads_adjusts_resets_and_rebuilds_the_counter_requests_charge() {
    const SPENT: usize = 4;
    let mut drivers = JoinSet::new();
    let (Some(store), Ok(valkey)) = (
        fresh_database(&mut drivers).await,
        std::env::var("ANTIRION_TEST_VALKEY_URL"),
    ) else {
        return;
    };
    let dir = ScratchDir::new("cluster-budgets");
    let prefix = format!("bud{:x}:", antirion_core::ResourceUid::random().0);
    let mock = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = mock.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(upstream(mock));
    let mut nodes = Vec::new();
    for i in 0..2 {
        nodes.push(node(&dir, i, &store, &valkey, &prefix, mock_addr, &mut tasks).await);
    }
    let (data, admin) = (nodes[1].0.0, nodes[0].0.1);
    let spec = json!({"name": "k", "spec": {"project": "p"}});
    let (status, _, key) = call(admin, "POST", "/admin/v1/keys?wait=10s", TOKEN, &spec.to_string()).await;
    assert_eq!(status, 201, "{key}");
    let secret = key["secret"].as_str().unwrap().to_owned();
    let chat = r#"{"model":"m","messages":[{"role":"user","content":"hello"}]}"#;
    for _ in 0..SPENT {
        let (status, _, body) = call(data, "POST", "/v1/chat/completions", &secret, chat).await;
        assert_eq!(status, 200, "{body}");
    }
    let (spent, held) = budget(&valkey, &prefix).await;
    assert!(spent > 0 && held == 0, "spent {spent}, reserved {held}");

    // Another node's Admin API reads what the requests charged, not an empty counter beside it.
    assert_eq!(budget_via_admin(admin).await, (spent, 0));
    assert_eq!(budget(&valkey, &prefix).await, (spent, 0), "one budget counter");

    // A node that rebuilds the counter (here: the store lost it) writes it where requests charge.
    let mut seen = 0;
    for _ in 0..100 {
        seen = ledger_requests(admin).await;
        if seen == SPENT as u64 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(seen, SPENT as u64, "the lines reached the store");
    let client = redis::Client::open(valkey.as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let (_, keys): (u64, Vec<String>) = redis::cmd("SCAN")
        .arg(0)
        .arg("MATCH")
        .arg(format!("{prefix}b:*"))
        .arg("COUNT")
        .arg(100_000)
        .query_async(&mut conn)
        .await
        .unwrap();
    let _: () = redis::cmd("DEL").arg(&keys).query_async(&mut conn).await.unwrap();
    nodes.push(node(&dir, 2, &store, &valkey, &prefix, mock_addr, &mut tasks).await);
    let fresh = nodes[2].0.1;
    assert_eq!(budget_via_admin(fresh).await, (spent, 0));
    assert_eq!(budget(&valkey, &prefix).await, (spent, 0));

    // `:adjust` and `:reset` act on that counter, and every node reads the result.
    let adjust = json!({"amount": "2.5", "reason": "credit check"}).to_string();
    let path = "/admin/v1/runtime/budgets/project/p/monthly:adjust";
    let (status, _, body) = call(admin, "POST", path, TOKEN, &adjust).await;
    assert!(status == 200, "{status}: {body}");
    let adjusted = spent + 2_500_000_000;
    assert_eq!(budget(&valkey, &prefix).await, (adjusted, 0));
    assert_eq!(budget_via_admin(nodes[1].0.1).await, (adjusted, 0));
    let path = "/admin/v1/runtime/budgets/project/p/monthly:reset";
    let (status, _, body) = call(fresh, "POST", path, TOKEN, "").await;
    assert!(status == 200, "{status}: {body}");
    assert_eq!(budget(&valkey, &prefix).await, (0, 0));
    assert_eq!(budget_via_admin(admin).await, (0, 0));

    for (_, rt) in &nodes {
        rt.shutdown.cancel();
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}
