mod common;
use common::*;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::SocketAddr,
    time::Duration,
};

struct Server {
    address: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn start(app: &daymark_api::App) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = app.router();
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        Self { address, task }
    }
    async fn exchange(&self, input: String) -> String {
        let address = self.address;
        tokio::task::spawn_blocking(move || {
            let mut stream =
                std::net::TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(15)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream.write_all(input.as_bytes()).unwrap();
            let mut response = String::new();
            stream
                .take(2 * 1024 * 1024)
                .read_to_string(&mut response)
                .unwrap();
            response
        })
        .await
        .unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn raw_request(method: &str, path: &str, headers: &str, body: &str) -> String {
    format!(
        "{method} {path} HTTP/1.1\r\nHost: daymark.test\r\nConnection: close\r\n{headers}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
}
fn security_headers(client: Option<&Client>) -> String {
    let mut headers =
        format!("Origin: {ORIGIN}\r\nContent-Type: application/json\r\nX-Daymark-Request: 1\r\n");
    if let Some(client) = client {
        headers.push_str(&format!(
            "Cookie: {}\r\nX-CSRF-Token: {}\r\n",
            client.cookie, client.csrf
        ));
    }
    headers
}
fn status(response: &str) -> u16 {
    response
        .split_whitespace()
        .nth(1)
        .expect("HTTP status")
        .parse()
        .unwrap()
}

#[tokio::test]
async fn tcp_login_rejects_ambiguous_session_headers_without_issuing_or_revoking_sessions() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    let bob = f.user("bob").await;
    let server = Server::start(&f.app).await;
    let body = json!({"username":"alice","password":PASSWORD}).to_string();
    for extra in [
        format!("Cookie: {}\r\n", alice.cookie),
        format!("Cookie: {}\r\n", bob.cookie),
        format!("X-CSRF-Token: {}\r\n", alice.csrf),
        "X-CSRF-Token: wrong\r\n".into(),
        "Cookie: __Host-daymark=malformed\r\n".into(),
    ] {
        let headers = security_headers(Some(&alice)) + &extra;
        let response = server
            .exchange(raw_request("POST", "/api/v1/login", &headers, &body))
            .await;
        assert!(
            (400..500).contains(&status(&response)),
            "ambiguous login accepted: {response}"
        );
        assert!(!response.contains("set-cookie:"));
        assert_eq!(
            f.raw()
                .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        for client in [&alice, &bob] {
            assert_eq!(
                send(&f.app, "GET", "/api/v1/session", Some(client), Value::Null)
                    .await
                    .status,
                200
            );
        }
    }
    // A valid stale cookie must still allow password authentication and receive
    // a fresh token; ambiguity rejection is not an expiry-related login lockout.
    f.clock.advance(1800);
    let fresh = login(&f.app, "alice", PASSWORD, Some(&alice)).await;
    assert_ne!(fresh.cookie, alice.cookie);
}

#[tokio::test]
async fn tcp_security_headers_and_malformed_framing_cannot_reach_a_mutation() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    let bob = f.user("bob").await;
    let server = Server::start(&f.app).await;
    let body = json!({"id":"1","source":source(8,"never accepted")}).to_string();
    let valid = security_headers(Some(&alice));
    let mut cases = Vec::new();
    for origin in [
        "null",
        "https://foreign.test",
        "https://daymark.test.foreign.test",
        "https://daymark.test@foreign.test",
        "https://daymark.test/",
        "https://daymark.test, https://foreign.test",
        "https://daymark.test https://foreign.test",
    ] {
        cases.push(valid.replace(ORIGIN, origin));
    }
    for line in [
        format!("Origin: {ORIGIN}\r\n"),
        "X-Daymark-Request: 1\r\n".into(),
        "Content-Type: application/json\r\n".into(),
        format!("X-CSRF-Token: {}\r\n", alice.csrf),
    ] {
        cases.push(valid.replace(&line, ""));
        cases.push(valid.clone() + &line);
    }
    for extra in [
        "Sec-Fetch-Site: cross-site\r\n",
        "Sec-Fetch-Site: same-site\r\n",
        "Sec-Fetch-Site: same-origin\r\nSec-Fetch-Site: none\r\n",
        "Origin: \u{0080}\r\n",
    ] {
        cases.push(valid.clone() + extra);
    }
    for content in [
        "text/plain",
        "application/x-www-form-urlencoded",
        "multipart/form-data; boundary=x",
        "application/json, text/plain",
    ] {
        cases.push(valid.replace("application/json", content));
    }
    cases.push(valid.replace(&alice.csrf, &bob.csrf));
    cases.push(valid.replace(&alice.cookie, &format!("{}; {}", alice.cookie, bob.cookie)));
    cases.push(valid.clone() + &format!("Cookie: {}\r\n", bob.cookie));
    for headers in cases {
        let response = server
            .exchange(raw_request("POST", "/api/v1/sources", &headers, &body))
            .await;
        assert!(
            (400..500).contains(&status(&response)),
            "accepted hostile headers: {headers:?}; {response}"
        );
    }
    for framing in [
        "Content-Length: 1\r\nContent-Length: 2\r\n",
        "Content-Length: -1\r\n",
        "Content-Length: +1\r\n",
        "Transfer-Encoding: chunked\r\nTransfer-Encoding: chunked\r\n",
    ] {
        let input = format!(
            "POST /api/v1/sources HTTP/1.1\r\nHost: daymark.test\r\nConnection: close\r\n{valid}{framing}\r\n0\r\n\r\n"
        );
        let response = server.exchange(input).await;
        assert!(
            (400..500).contains(&status(&response)),
            "{framing:?}: {response}"
        );
    }
    for method in [
        "GET", "HEAD", "OPTIONS", "TRACE", "PATCH", "CONNECT", "post",
    ] {
        let response = server
            .exchange(raw_request(method, "/api/v1/sources", &valid, &body))
            .await;
        assert!(
            (400..500).contains(&status(&response)),
            "{method}: {response}"
        );
    }
    let oversized = "x".repeat(32769);
    let chunked = format!(
        "POST /api/v1/sources HTTP/1.1\r\nHost: daymark.test\r\nConnection: close\r\n{valid}Transfer-Encoding: chunked\r\n\r\n{:x}\r\n{oversized}\r\n0\r\n\r\n",
        oversized.len()
    );
    assert_eq!(status(&server.exchange(chunked).await), 413);
    assert_eq!(
        f.raw()
            .query_row("SELECT count(*) FROM sources", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.raw()
            .query_row("SELECT count(*) FROM source_history", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn tcp_chunked_requests_trailers_and_concurrent_spends_preserve_cookie_owner() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    let bob = f.user("bob").await;
    let server = Server::start(&f.app).await;
    for (index, bucket) in ["Pto", "Comp", "Floater"].into_iter().enumerate() {
        let id = (index * 10).to_string();
        let opening = json!({"id":id,"source":{"dates":["2026-01-01"],"activity":{"kind":"grant","bucket":bucket,"hours":"8"},"notes":"alice only"}}).to_string();
        let headers = security_headers(Some(&alice));
        // Trailer fields cannot replace the already validated identity or CSRF.
        let input = format!(
            "POST /api/v1/sources HTTP/1.1\r\nHost: daymark.test\r\nConnection: close\r\n{headers}Transfer-Encoding: chunked\r\n\r\n{:x}\r\n{opening}\r\n0\r\nCookie: {}\r\nX-CSRF-Token: {}\r\n\r\n",
            opening.len(),
            bob.cookie,
            bob.csrf
        );
        let response = server.exchange(input).await;
        assert_eq!(status(&response), 201, "{response}");
        let change: Value =
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(change["actor"], alice.id);
        let spend = |id: usize| {
            json!({"id":id.to_string(),"source":{"dates":["2026-01-02"],"activity":{"kind":"use","bucket":bucket,"hours":"8"},"notes":"contending spend"}}).to_string()
        };
        let (first, second) = tokio::join!(
            server.exchange(raw_request(
                "POST",
                "/api/v1/sources",
                &headers,
                &spend(index * 10 + 1)
            )),
            server.exchange(raw_request(
                "POST",
                "/api/v1/sources",
                &headers,
                &spend(index * 10 + 2)
            ))
        );
        let mut statuses = [status(&first), status(&second)];
        statuses.sort();
        assert_eq!(statuses, [201, 422]);
        for client in [&alice, &bob] {
            let snapshot = send(
                &f.app,
                "GET",
                "/api/v1/years/2026/snapshot",
                Some(client),
                Value::Null,
            )
            .await;
            assert_eq!(snapshot.status, 200);
            assert_eq!(snapshot.value["balances"], json!(["0", "0", "0", "0"]));
        }
        assert_eq!(
            send(
                &f.app,
                "GET",
                &format!("/api/v1/sources/{id}/history"),
                Some(&bob),
                Value::Null
            )
            .await
            .value,
            json!([])
        );
    }
    assert_eq!(
        f.raw()
            .query_row("SELECT count(*) FROM sources", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        6
    );
    assert_eq!(
        f.raw()
            .query_row("SELECT count(*) FROM source_history", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        6
    );
}

#[tokio::test]
async fn tcp_incomplete_bodies_exhaust_request_admission_then_release_it() {
    let f = Fixture::new().await;
    let server = Server::start(&f.app).await;
    let address = server.address;
    let mut held = tokio::task::spawn_blocking(move || {
        let mut held = Vec::new();
        for _ in 0..64 {
            let mut stream = std::net::TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
            write!(stream, "POST /api/v1/sources HTTP/1.1\r\nHost: daymark.test\r\nConnection: close\r\n{}Content-Length: 2\r\nExpect: 100-continue\r\n\r\n", security_headers(None)).unwrap();
            // Hyper sends Continue when middleware first polls the body, after
            // the request permit is acquired. This is an admission handshake.
            let mut interim = vec![0; b"HTTP/1.1 100 Continue\r\n\r\n".len()];
            stream.read_exact(&mut interim).unwrap();
            assert_eq!(interim, b"HTTP/1.1 100 Continue\r\n\r\n");
            held.push(stream);
        }
        held
    }).await.unwrap();
    assert_eq!(
        status(&server.exchange(raw_request("GET", "/health", "", "")).await),
        503
    );
    tokio::task::spawn_blocking(move || {
        for stream in &mut held {
            stream.write_all(b"{}").unwrap();
        }
        for mut stream in held {
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert_eq!(status(&response), 400);
        }
    })
    .await
    .unwrap();
    assert_eq!(
        status(&server.exchange(raw_request("GET", "/health", "", "")).await),
        200
    );
    // A client that never finishes its admitted body also loses its permit.
    let response = server.exchange(format!("POST /api/v1/sources HTTP/1.1\r\nHost: daymark.test\r\nConnection: close\r\n{}Content-Length: 2\r\n\r\n", security_headers(None))).await;
    assert_eq!(status(&response), 408);
    assert_eq!(
        status(&server.exchange(raw_request("GET", "/health", "", "")).await),
        200
    );
}

#[tokio::test]
async fn year_moves_and_storage_corruption_fail_safely_at_the_http_boundary() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    let bob = f.user("bob").await;
    assert_eq!(
        create(&f.app, &alice, "1", 8, "alice private").await.status,
        201
    );
    assert_eq!(
        create(&f.app, &bob, "1", 80, "bob private").await.status,
        201
    );
    let mut moved = source(8, "deliberate move");
    moved["dates"] = json!(["2027-01-01"]);
    let response = send(
        &f.app,
        "PUT",
        "/api/v1/years/2026/sources/1",
        Some(&alice),
        json!({"revision":"1","source":moved}),
    )
    .await;
    assert_eq!(response.status, 200);
    assert_eq!(response.value["actor"], alice.id);
    for (path, expected) in [
        ("/api/v1/years/2026/sources/1", 404),
        ("/api/v1/years/2027/sources/1", 409),
    ] {
        assert_eq!(
            send(
                &f.app,
                "DELETE",
                path,
                Some(&alice),
                json!({"revision":"1"})
            )
            .await
            .status,
            expected
        );
    }
    let history = send(
        &f.app,
        "GET",
        "/api/v1/sources/1/history",
        Some(&alice),
        Value::Null,
    )
    .await;
    assert_eq!(history.value.as_array().unwrap().len(), 2);
    assert!(!history.text.contains("bob private"));
    let raw = f.raw();
    let owner = alice.id.parse::<u64>().unwrap().to_be_bytes();
    raw.execute(
        "UPDATE effects SET payload=json_set(payload,'$.delta',7) WHERE owner=?1",
        [owner],
    )
    .unwrap();
    for path in ["/api/v1/years/2027/snapshot", "/api/v1/years/2027/ledger"] {
        let response = send(&f.app, "GET", path, Some(&alice), Value::Null).await;
        assert_eq!(response.status, 503);
        assert!(!response.text.contains("private"));
    }
    let mut next = source(9, "must not repair drift");
    next["dates"] = json!(["2027-01-01"]);
    assert_eq!(
        send(
            &f.app,
            "PUT",
            "/api/v1/years/2027/sources/1",
            Some(&alice),
            json!({"revision":"2","source":next})
        )
        .await
        .status,
        503
    );
    assert_eq!(
        send(
            &f.app,
            "GET",
            "/api/v1/sources/1/history",
            Some(&alice),
            Value::Null
        )
        .await
        .value,
        history.value
    );
    assert_eq!(
        send(
            &f.app,
            "GET",
            "/api/v1/years/2026/snapshot",
            Some(&bob),
            Value::Null
        )
        .await
        .value["balances"][0],
        "80"
    );
}

#[tokio::test]
async fn normalization_races_and_utf8_password_bounds_do_not_alias_accounts_or_secrets() {
    let f = Fixture::new().await;
    let (first, second) = tokio::join!(
        send(
            &f.app,
            "POST",
            "/api/v1/register",
            None,
            json!({"username":"Collision","password":PASSWORD})
        ),
        send(
            &f.app,
            "POST",
            "/api/v1/register",
            None,
            json!({"username":"COLLISION","password":PASSWORD})
        )
    );
    let mut statuses = [first.status.as_u16(), second.status.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [201, 409]);
    assert_eq!(
        f.raw()
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let alice = login(&f.app, "cOlLiSiOn", PASSWORD, None).await;
    for username in [
        "collision ",
        " collision",
        "collision\0",
        "collisİon",
        "ｃollision",
        "co\nllision",
    ] {
        assert_eq!(
            send(
                &f.app,
                "POST",
                "/api/v1/login",
                None,
                json!({"username":username,"password":PASSWORD})
            )
            .await
            .status,
            401
        );
        assert_eq!(
            send(
                &f.app,
                "POST",
                "/api/v1/register",
                None,
                json!({"username":username,"password":PASSWORD})
            )
            .await
            .status,
            400
        );
    }
    let password = "é".repeat(64);
    assert_eq!(password.len(), 128);
    assert_eq!(
        send(
            &f.app,
            "POST",
            "/api/v1/register",
            None,
            json!({"username":"unicode","password":password})
        )
        .await
        .status,
        201
    );
    let unicode = login(&f.app, "unicode", &password, None).await;
    assert_ne!(unicode.id, alice.id);
    assert_eq!(
        send(
            &f.app,
            "POST",
            "/api/v1/login",
            None,
            json!({"username":"unicode","password":format!("{password}x")})
        )
        .await
        .status,
        401
    );
    assert_eq!(
        send(
            &f.app,
            "POST",
            "/api/v1/register",
            None,
            json!({"username":"toolong","password":format!("{password}x")})
        )
        .await
        .status,
        400
    );
    // Typed JSON values cannot be used to construct duplicate object keys.
    let duplicate =
        format!(r#"{{"username":"collision","username":"unicode","password":"{PASSWORD}"}}"#);
    assert_eq!(
        request(
            f.app.router(),
            build("POST", "/api/v1/login", None, duplicate)
        )
        .await
        .status,
        400
    );
    assert_eq!(
        send(&f.app, "GET", "/api/v1/session", Some(&alice), Value::Null)
            .await
            .value["id"],
        alice.id
    );
}

#[tokio::test]
async fn separate_server_process_cannot_bypass_live_lock_with_path_aliases() {
    let f = Fixture::new().await;
    let paths = [
        f.dir.path().join("test.sqlite"),
        f.dir.path().join(".").join("test.sqlite"),
        f.dir.path().join("test.sqlite").canonicalize().unwrap(),
    ];
    tokio::task::spawn_blocking(move || {
        for path in paths {
            let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_daymark-api"))
                .arg("serve")
                .arg(path)
                .args([ORIGIN, "127.0.0.1:0"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(!status.success());
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("second server failed to reject a live database lock");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        send(&f.app, "GET", "/health", None, Value::Null)
            .await
            .status,
        200
    );
}
