#![allow(dead_code)]
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
};
use daymark_api::{App, Clock, Config};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};
use tower::ServiceExt;

pub const ORIGIN: &str = "https://daymark.test";
pub const PASSWORD: &str = "synthetic test password 42";
pub struct TestClock(pub AtomicI64);
impl Clock for TestClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
impl TestClock {
    pub fn advance(&self, seconds: i64) {
        self.0.fetch_add(seconds, Ordering::SeqCst);
    }
}
pub struct Fixture {
    pub app: App,
    pub dir: tempfile::TempDir,
    pub clock: Arc<TestClock>,
}
impl Fixture {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock(AtomicI64::new(1_800_000_000)));
        let app = App::with_clock(
            dir.path().join("test.sqlite"),
            Config::https(ORIGIN).unwrap(),
            clock.clone(),
        )
        .await
        .unwrap();
        Self { app, dir, clock }
    }
    pub fn raw(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.dir.path().join("test.sqlite")).unwrap()
    }
    pub async fn user(&self, name: &str) -> Client {
        let r = send(
            &self.app,
            "POST",
            "/api/v1/register",
            None,
            json!({"username":name,"password":PASSWORD}),
        )
        .await;
        assert_eq!(r.status, 201, "{}", r.text);
        login(&self.app, name, PASSWORD, None).await
    }
    pub async fn admin(&self) -> Client {
        self.app
            .bootstrap("operator".into(), PASSWORD.into())
            .await
            .unwrap();
        login(&self.app, "operator", PASSWORD, None).await
    }
}
#[derive(Clone)]
pub struct Client {
    pub cookie: String,
    pub csrf: String,
    pub id: String,
}
pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub value: Value,
    pub text: String,
}
pub async fn request(router: Router, request: Request<Body>) -> Reply {
    let response = router.oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let bytes = to_bytes(body, 16 * 1024 * 1024).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Reply {
        status: parts.status,
        headers: parts.headers,
        value,
        text,
    }
}
pub fn build(method: &str, path: &str, client: Option<&Client>, body: String) -> Request<Body> {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("origin", ORIGIN)
        .header("content-type", "application/json")
        .header("x-daymark-request", "1")
        .extension(ConnectInfo(
            "192.0.2.1:12345".parse::<SocketAddr>().unwrap(),
        ));
    if let Some(c) = client {
        req = req
            .header("cookie", &c.cookie)
            .header("x-csrf-token", &c.csrf);
    }
    req.body(Body::from(body)).unwrap()
}
pub async fn send(
    app: &App,
    method: &str,
    path: &str,
    client: Option<&Client>,
    value: Value,
) -> Reply {
    request(app.router(), build(method, path, client, value.to_string())).await
}
pub async fn login(app: &App, name: &str, password: &str, old: Option<&Client>) -> Client {
    let r = send(
        app,
        "POST",
        "/api/v1/login",
        old,
        json!({"username":name,"password":password}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text);
    Client {
        cookie: r.headers["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .into(),
        csrf: r.value["csrf"].as_str().unwrap().into(),
        id: r.value["id"].as_str().unwrap().into(),
    }
}
pub fn source(delta: i64, notes: &str) -> Value {
    json!({"dates":["2026-01-01"],"activity":{"kind":"adjustment","bucket":"Pto","delta":delta.to_string()},"notes":notes})
}
pub async fn create(app: &App, client: &Client, id: &str, delta: i64, notes: &str) -> Reply {
    send(
        app,
        "POST",
        "/api/v1/sources",
        Some(client),
        json!({"id":id,"source":source(delta,notes)}),
    )
    .await
}
pub fn calendar(revision: Value) -> Value {
    json!({"revision":revision,"holidays":(1..=10).map(|m|json!({"id":m.to_string(),"date":format!("2026-{m:02}-01"),"name":format!("Holiday {m}")})).collect::<Vec<_>>()})
}
