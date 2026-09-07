mod common;
use axum::{body::Body, http::StatusCode};
use common::*;
use daymark_api::{App, Config};
use serde_json::{Value, json};

#[tokio::test]
async fn registration_password_secrecy_normalization_and_no_first_user_promotion() {
    let f = Fixture::new().await;
    let alice = f.user("Alice").await;
    let bob = f.user("bob").await;
    assert_ne!(alice.id, bob.id);
    assert_ne!(alice.cookie, bob.cookie);
    let raw = f.raw();
    let hashes: Vec<String> = raw
        .prepare("SELECT password_hash FROM accounts")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(hashes.len(), 2);
    assert_ne!(hashes[0], hashes[1]);
    for hash in hashes {
        assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(!hash.contains(PASSWORD));
    }
    assert_eq!(
        raw.query_row("SELECT sum(admin) FROM accounts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let duplicate = send(
        &f.app,
        "POST",
        "/api/v1/register",
        None,
        json!({"username":"ALICE","password":PASSWORD}),
    )
    .await;
    assert_eq!(duplicate.status, 409);
    for bad in [
        json!({"username":"newuser","password":"short"}),
        json!({"username":"newuser","password":PASSWORD,"admin":true}),
        json!({"username":"newuser","password":PASSWORD,"email":"unused"}),
        json!({"username":"' OR 1=1--","password":PASSWORD}),
    ] {
        assert_eq!(
            send(&f.app, "POST", "/api/v1/register", None, bad)
                .await
                .status,
            400
        );
    }
    let r = send(
        &f.app,
        "POST",
        "/api/v1/login",
        None,
        json!({"username":"alice","password":PASSWORD}),
    )
    .await;
    assert_eq!(r.status, 200);
    let cookie = r.headers["set-cookie"].to_str().unwrap();
    for flag in [
        "__Host-daymark=",
        "Secure",
        "HttpOnly",
        "SameSite=Strict",
        "Path=/",
        "Max-Age=43200",
    ] {
        assert!(cookie.contains(flag));
    }
    assert!(!cookie.contains("Domain="));
    assert_eq!(r.headers["cache-control"], "no-store");
    assert!(r.headers.contains_key("strict-transport-security"));
    assert!(!r.text.contains(PASSWORD));
    assert!(!r.text.contains("argon2"));
    let absent = send(
        &f.app,
        "POST",
        "/api/v1/login",
        None,
        json!({"username":"absent","password":PASSWORD}),
    )
    .await;
    let wrong = send(
        &f.app,
        "POST",
        "/api/v1/login",
        None,
        json!({"username":"alice","password":"wrong password 42"}),
    )
    .await;
    assert_eq!(absent.status, 401);
    assert_eq!(absent.text, wrong.text);
    let tokens: Vec<Vec<u8>> = raw
        .prepare("SELECT token_hash FROM sessions")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(tokens.iter().all(|t| t.len() == 32));
    assert!(!format!("{tokens:?}").contains(&alice.cookie));
}

#[tokio::test]
async fn two_users_colliding_and_guessed_ids_never_cross_read_search_edit_delete_or_history() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    let bob = f.user("bob").await;
    let admin = f.admin().await;
    for (c, amount, note) in [
        (&alice, 8, "alice private"),
        (&bob, 80, "bob hidden snow 雪"),
    ] {
        assert_eq!(
            create(&f.app, c, "18446744073709551615", amount, note)
                .await
                .status,
            201
        );
    }
    assert_eq!(
        create(&f.app, &bob, "7", 7, "bob secret needle")
            .await
            .status,
        201
    );
    for c in [&alice, &admin] {
        let r = send(
            &f.app,
            "GET",
            "/api/v1/years/2026/sources/7",
            Some(c),
            Value::Null,
        )
        .await;
        assert_eq!(r.status, 404);
        for (method, body) in [
            ("PUT", json!({"revision":"1","source":source(1,"attacker")})),
            ("DELETE", json!({"revision":"1"})),
        ] {
            assert_eq!(
                send(
                    &f.app,
                    method,
                    "/api/v1/years/2026/sources/7",
                    Some(c),
                    body
                )
                .await
                .status,
                404
            );
        }
        for path in [
            "/api/v1/years/2026/ledger?notes=bob",
            "/api/v1/sources/7/history",
        ] {
            let r = send(&f.app, "GET", path, Some(c), Value::Null).await;
            assert_eq!(r.status, 200);
            assert_eq!(r.value, json!([]));
        }
        let r = send(
            &f.app,
            "GET",
            &format!("/api/v1/admin/accounts/{}/ledger", bob.id),
            Some(c),
            Value::Null,
        )
        .await;
        assert_eq!(r.status, 404);
    }
    let r = send(
        &f.app,
        "GET",
        "/api/v1/years/2026/sources/18446744073709551615",
        Some(&alice),
        Value::Null,
    )
    .await;
    assert_eq!(r.value["source"]["notes"], "alice private");
    assert_eq!(r.value["reference"]["id"], "18446744073709551615");
    assert_eq!(r.value["source"]["dates"], json!(["2026-01-01"]));
    let r = send(
        &f.app,
        "PUT",
        "/api/v1/years/2026/sources/18446744073709551615",
        Some(&alice),
        json!({"revision":"1","source":source(9,"changed alice")}),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(
        send(
            &f.app,
            "DELETE",
            "/api/v1/years/2026/sources/18446744073709551615",
            Some(&alice),
            json!({"revision":"2"})
        )
        .await
        .status,
        200
    );
    let own_history = send(
        &f.app,
        "GET",
        "/api/v1/sources/18446744073709551615/history?limit=1&offset=1",
        Some(&alice),
        Value::Null,
    )
    .await;
    assert_eq!(own_history.value.as_array().unwrap().len(), 1);
    assert_eq!(
        own_history.value[0]["before"]["source"]["notes"],
        "alice private"
    );
    assert!(!own_history.text.contains("bob"));
    let bob_record = send(
        &f.app,
        "GET",
        "/api/v1/years/2026/sources/18446744073709551615",
        Some(&bob),
        Value::Null,
    )
    .await;
    assert_eq!(bob_record.value["source"]["notes"], "bob hidden snow 雪");
    for path in [
        format!("/api/v1/years/2026/ledger?owner={}", bob.id),
        format!("/api/v1/sources/7/history?owner={}", bob.id),
    ] {
        assert_eq!(
            send(&f.app, "GET", &path, Some(&alice), Value::Null)
                .await
                .status,
            400
        );
    }
    assert_eq!(
        send(
            &f.app,
            "POST",
            "/api/v1/sources",
            Some(&alice),
            json!({"owner":bob.id,"id":"7","source":source(1,"attack")})
        )
        .await
        .status,
        400
    );
    assert_eq!(
        send(
            &f.app,
            "POST",
            "/api/v1/sources",
            None,
            json!({"id":"7","source":source(1,"attack")})
        )
        .await
        .status,
        401
    );
}

#[tokio::test]
async fn bootstrap_and_admin_reset_are_narrow_atomic_and_revoke_every_old_session() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    let second = login(&f.app, "alice", PASSWORD, None).await;
    let admin = f.admin().await;
    assert!(
        f.app
            .bootstrap("another".into(), PASSWORD.into())
            .await
            .is_err()
    );
    assert!(
        f.app
            .bootstrap("alice".into(), PASSWORD.into())
            .await
            .is_err()
    );
    let path = format!("/api/v1/admin/accounts/{}/password", alice.id);
    for c in [None, Some(&alice)] {
        assert_eq!(
            send(
                &f.app,
                "POST",
                &path,
                c,
                json!({"password":"changed synthetic password"})
            )
            .await
            .status,
            if c.is_none() { 401 } else { 403 }
        );
    }
    assert_eq!(
        send(
            &f.app,
            "GET",
            "/api/v1/admin/accounts",
            Some(&alice),
            Value::Null
        )
        .await
        .status,
        403
    );
    let listed = send(
        &f.app,
        "GET",
        "/api/v1/admin/accounts",
        Some(&admin),
        Value::Null,
    )
    .await;
    assert_eq!(listed.status, 200);
    assert!(!listed.text.contains("password"));
    assert!(!listed.text.contains("settings"));
    assert_eq!(
        send(
            &f.app,
            "POST",
            &path,
            Some(&admin),
            json!({"password":"changed synthetic password"})
        )
        .await
        .status,
        204
    );
    for stale in [&alice, &second] {
        assert_eq!(
            send(&f.app, "GET", "/api/v1/session", Some(stale), Value::Null)
                .await
                .status,
            401
        );
        assert_eq!(
            create(&f.app, stale, "1", 8, "must reject").await.status,
            401
        );
    }
    assert_eq!(
        send(
            &f.app,
            "POST",
            "/api/v1/login",
            None,
            json!({"username":"alice","password":PASSWORD})
        )
        .await
        .status,
        401
    );
    login(&f.app, "alice", "changed synthetic password", None).await;
    assert_eq!(
        send(
            &f.app,
            "POST",
            &format!("/api/v1/admin/accounts/{}/password", admin.id),
            Some(&admin),
            json!({"password":"changed synthetic password"})
        )
        .await
        .status,
        404
    );
    let raw = f.raw();
    assert_eq!(
        raw.query_row(
            "SELECT count(*) FROM account_history WHERE action='password_reset'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert!(raw.execute("DELETE FROM account_history", []).is_err());
    assert!(
        raw.execute("UPDATE accounts SET admin=1 WHERE username='alice'", [])
            .is_err()
    );
}

#[tokio::test]
async fn session_fixation_rotation_logout_idle_absolute_expiry_and_cap() {
    let f = Fixture::new().await;
    let original = f.user("alice").await;
    let fixed = Client {
        cookie: format!("__Host-daymark={}", "a".repeat(64)),
        csrf: "b".repeat(64),
        id: original.id.clone(),
    };
    assert_eq!(
        send(&f.app, "GET", "/api/v1/session", Some(&fixed), Value::Null)
            .await
            .status,
        401
    );
    let fresh = login(&f.app, "alice", PASSWORD, Some(&fixed)).await;
    assert_ne!(fixed.cookie, fresh.cookie);
    let rotated = login(&f.app, "alice", PASSWORD, Some(&fresh)).await;
    assert_ne!(fresh.cookie, rotated.cookie);
    assert_ne!(fresh.csrf, rotated.csrf);
    assert_eq!(
        send(&f.app, "GET", "/api/v1/session", Some(&fresh), Value::Null)
            .await
            .status,
        401
    );
    assert_eq!(
        send(&f.app, "POST", "/api/v1/logout", Some(&rotated), json!({}))
            .await
            .status,
        204
    );
    assert_eq!(
        send(
            &f.app,
            "GET",
            "/api/v1/session",
            Some(&rotated),
            Value::Null
        )
        .await
        .status,
        401
    );
    f.clock.advance(1800);
    assert_eq!(
        send(
            &f.app,
            "GET",
            "/api/v1/session",
            Some(&original),
            Value::Null
        )
        .await
        .status,
        401
    );
    let absolute = login(&f.app, "alice", PASSWORD, None).await;
    for _ in 0..43 {
        f.clock.advance(1000);
        assert_eq!(
            send(
                &f.app,
                "GET",
                "/api/v1/session",
                Some(&absolute),
                Value::Null
            )
            .await
            .status,
            200
        );
    }
    f.clock.advance(200);
    assert_eq!(
        send(
            &f.app,
            "GET",
            "/api/v1/session",
            Some(&absolute),
            Value::Null
        )
        .await
        .status,
        401
    );
    for _ in 0..6 {
        login(&f.app, "alice", PASSWORD, None).await;
        f.clock.advance(1);
    }
    assert_eq!(
        f.raw()
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        5
    );
}

#[tokio::test]
async fn csrf_origin_fetch_metadata_and_ambiguous_cookies_fail_closed() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    for (header, value) in [
        ("origin", Some("https://evil.test")),
        ("origin", None),
        ("origin", Some("null")),
        ("x-daymark-request", None),
        ("x-csrf-token", None),
        ("x-csrf-token", Some("wrong")),
        ("content-type", Some("text/plain")),
        ("sec-fetch-site", Some("same-site")),
    ] {
        let mut req = build(
            "POST",
            "/api/v1/sources",
            Some(&alice),
            json!({"id":"1","source":source(8,"blocked")}).to_string(),
        );
        req.headers_mut().remove(header);
        if let Some(value) = value {
            req.headers_mut().insert(header, value.parse().unwrap());
        }
        assert_eq!(
            request(f.app.router(), req).await.status,
            403,
            "{header} {value:?}"
        );
    }
    let mut req = build(
        "POST",
        "/api/v1/login",
        None,
        json!({"username":"alice","password":PASSWORD}).to_string(),
    );
    req.headers_mut().remove("origin");
    assert_eq!(request(f.app.router(), req).await.status, 403);
    let mut req = build("GET", "/api/v1/session", Some(&alice), "".into());
    req.headers_mut()
        .append("cookie", alice.cookie.parse().unwrap());
    assert_eq!(request(f.app.router(), req).await.status, 401);
    let mut req = build("POST", "/api/v1/logout", Some(&alice), "{}".into());
    req.headers_mut().append("origin", ORIGIN.parse().unwrap());
    assert_eq!(request(f.app.router(), req).await.status, 400);
    assert_eq!(
        send(
            &f.app,
            "GET",
            "/api/v1/years/2026/snapshot",
            Some(&alice),
            Value::Null
        )
        .await
        .value["balances"],
        json!(["0", "0", "0", "0"])
    );
}

#[tokio::test]
async fn admin_calendar_import_stale_corrections_and_private_reference_errors() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    let admin = f.admin().await;
    let path = "/api/v1/years/2026/holidays";
    assert_eq!(
        send(&f.app, "PUT", path, Some(&alice), calendar(Value::Null))
            .await
            .status,
        403
    );
    assert_eq!(
        send(&f.app, "PUT", path, Some(&admin), calendar(Value::Null))
            .await
            .status,
        200
    );
    let old = send(&f.app, "GET", path, Some(&alice), Value::Null)
        .await
        .value;
    assert_eq!(
        send(&f.app, "PUT", path, Some(&admin), calendar(Value::Null))
            .await
            .status,
        409
    );
    let mut invalid = calendar(json!("1"));
    invalid["holidays"].as_array_mut().unwrap().pop();
    assert_eq!(
        send(&f.app, "PUT", path, Some(&admin), invalid)
            .await
            .status,
        422
    );
    assert_eq!(
        send(&f.app, "GET", path, Some(&alice), Value::Null)
            .await
            .value,
        old
    );
    let worked = json!({"id":"1","source":{"dates":["2026-01-01"],"activity":{"kind":"holiday_work","holiday":"1","work":{"hours_worked":"1","multiplier":"OneAndAHalf","credited_comp":"9"}},"notes":"alice deeply private"}});
    assert_eq!(
        send(&f.app, "POST", "/api/v1/sources", Some(&alice), worked)
            .await
            .status,
        201
    );
    let mut moved = calendar(json!("1"));
    moved["holidays"][0]["date"] = json!("2026-01-02");
    let r = send(&f.app, "PUT", path, Some(&admin), moved).await;
    assert_eq!(r.status, 409);
    assert!(!r.text.contains("alice"));
    assert!(!r.text.contains(&alice.id));
    let mut named = calendar(json!("1"));
    named["holidays"][0]["name"] = json!("Corrected");
    assert_eq!(
        send(&f.app, "PUT", path, Some(&admin), named).await.value["revision"],
        "2"
    );
    let r = send(
        &f.app,
        "GET",
        "/api/v1/years/2026/snapshot",
        Some(&alice),
        Value::Null,
    )
    .await;
    assert_eq!(r.value["balances"], json!(["0", "9", "72", "8"]));
    assert_eq!(
        send(
            &f.app,
            "GET",
            "/api/v1/years/2026/snapshot",
            Some(&admin),
            Value::Null
        )
        .await
        .value["balances"],
        json!(["0", "0", "80", "0"])
    );
}

#[tokio::test]
async fn settings_stay_informational_and_owned_with_durable_accounts_sessions_and_data() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    let bob = f.user("bob").await;
    let settings = json!({"hire_date":"2000-01-01","annual_pto_allowance":"160"});
    assert_eq!(
        send(
            &f.app,
            "PUT",
            "/api/v1/settings",
            Some(&alice),
            settings.clone()
        )
        .await
        .status,
        200
    );
    assert_eq!(
        send(
            &f.app,
            "GET",
            "/api/v1/years/2026/snapshot",
            Some(&alice),
            Value::Null
        )
        .await
        .value["balances"][0],
        "0"
    );
    assert_eq!(
        send(&f.app, "GET", "/api/v1/settings", Some(&bob), Value::Null)
            .await
            .value,
        json!({})
    );
    assert_eq!(create(&f.app, &alice, "1", 8, "durable").await.status, 201);
    let Fixture { app, dir, clock } = f;
    assert!(
        App::with_clock(
            dir.path().join("test.sqlite"),
            Config::https(ORIGIN).unwrap(),
            clock.clone()
        )
        .await
        .is_err()
    );
    drop(app);
    let app = App::with_clock(
        dir.path().join("test.sqlite"),
        Config::https(ORIGIN).unwrap(),
        clock,
    )
    .await
    .unwrap();
    assert_eq!(
        send(&app, "GET", "/api/v1/session", Some(&alice), Value::Null)
            .await
            .status,
        200
    );
    assert_eq!(
        send(&app, "GET", "/api/v1/settings", Some(&alice), Value::Null)
            .await
            .value,
        settings
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/v1/years/2026/snapshot",
            Some(&alice),
            Value::Null
        )
        .await
        .value["balances"][0],
        "8"
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/v1/years/2027/snapshot",
            Some(&alice),
            Value::Null
        )
        .await
        .value["balances"][0],
        "0"
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/v1/sources/1/history",
            Some(&bob),
            Value::Null
        )
        .await
        .value,
        json!([])
    );
    login(&app, "alice", PASSWORD, None).await;
}

#[tokio::test]
async fn http_concurrent_overspending_and_stale_source_revisions_are_atomic() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    assert_eq!(create(&f.app, &alice, "1", 8, "opening").await.status, 201);
    let (a, b) = tokio::join!(
        create(&f.app, &alice, "2", -8, "spend a"),
        create(&f.app, &alice, "3", -8, "spend b")
    );
    assert_eq!(
        [a.status, b.status].iter().filter(|s| **s == 201).count(),
        1
    );
    let failure = if a.status == 201 { b } else { a };
    assert_eq!(failure.status, 422);
    assert_eq!(failure.value["error"], "negative_balance");
    let path = "/api/v1/years/2026/sources/1";
    let (a, b) = tokio::join!(
        send(
            &f.app,
            "PUT",
            path,
            Some(&alice),
            json!({"revision":"1","source":source(16,"edit a")})
        ),
        send(
            &f.app,
            "PUT",
            path,
            Some(&alice),
            json!({"revision":"1","source":source(24,"edit b")})
        )
    );
    assert_eq!(
        [a.status, b.status].iter().filter(|s| **s == 200).count(),
        1
    );
    let failure = if a.status == 200 { b } else { a };
    assert_eq!(failure.status, 409);
    assert_eq!(failure.value["error"], "stale_revision");
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
        409
    );
    assert_eq!(
        send(
            &f.app,
            "DELETE",
            "/api/v1/years/2025/sources/1",
            Some(&alice),
            json!({"revision":"2"})
        )
        .await
        .status,
        404
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
        .value
        .as_array()
        .unwrap()
        .len(),
        2
    );
}

#[tokio::test]
async fn malformed_oversized_and_injection_inputs_never_mutate_or_leak_contents() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    for body in [
        "{".into(),
        "[]".into(),
        "null".into(),
        json!({"id":"1","source":source(1,"fine"),"actor":"another"}).to_string(),
    ] {
        assert_eq!(
            request(
                f.app.router(),
                build("POST", "/api/v1/sources", Some(&alice), body)
            )
            .await
            .status,
            400
        );
    }
    let mut req = build("POST", "/api/v1/sources", Some(&alice), "x".repeat(32769));
    req.headers_mut().remove("content-length");
    assert_eq!(request(f.app.router(), req).await.status, 413);
    for field in ["notes", "dates", "activity"] {
        let mut s = source(1, "fine");
        s[field] = match field {
            "notes" => json!("x".repeat(8193)),
            "dates" => json!(["2026-02-29"]),
            _ => json!({"kind":"grant","bucket":"Pto","hours":"1.5"}),
        };
        assert!(
            send(
                &f.app,
                "POST",
                "/api/v1/sources",
                Some(&alice),
                json!({"id":"1","source":s})
            )
            .await
            .status
            .is_client_error()
        );
    }
    for path in [
        "/api/v1/years/0/snapshot",
        "/api/v1/years/2026/sources/18446744073709551616",
        "/api/v1/years/2026/ledger?limit=101",
        "/api/v1/sources/1/history?offset=1000001",
    ] {
        assert!(
            send(&f.app, "GET", path, Some(&alice), Value::Null)
                .await
                .status
                .is_client_error()
        );
    }
    let note = "' OR 1=1; DROP TABLE accounts; -- <script>alert(1)</script>";
    assert_eq!(create(&f.app, &alice, "1", 8, note).await.status, 201);
    let r = send(
        &f.app,
        "GET",
        "/api/v1/years/2026/ledger?notes=%27%20OR%201%3D1",
        Some(&alice),
        Value::Null,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.value.as_array().unwrap().len(), 1);
    assert_eq!(
        f.raw()
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let mut req = build("POST", "/api/v1/logout", Some(&alice), "{}".into());
    *req.body_mut() = Body::empty();
    assert_eq!(request(f.app.router(), req).await.status, 204);
}

#[tokio::test]
async fn bounded_durable_throttles_do_not_trust_forwarded_addresses() {
    let f = Fixture::new().await;
    let _alice = f.user("alice").await;
    for attempt in 0..9 {
        let mut req = build(
            "POST",
            "/api/v1/login",
            None,
            json!({"username":"alice","password":"wrong synthetic password"}).to_string(),
        );
        req.headers_mut().insert(
            "x-forwarded-for",
            format!("198.51.100.{attempt}").parse().unwrap(),
        );
        assert_eq!(request(f.app.router(), req).await.status, 401);
    }
    let r = send(
        &f.app,
        "POST",
        "/api/v1/login",
        None,
        json!({"username":"alice","password":PASSWORD}),
    )
    .await;
    assert_eq!(r.status, 429);
    assert!(r.headers.contains_key("retry-after"));
    f.clock.advance(601);
    login(&f.app, "alice", PASSWORD, None).await;
    for n in 0..4 {
        assert_eq!(
            send(
                &f.app,
                "POST",
                "/api/v1/register",
                None,
                json!({"username":format!("user{n}"),"password":PASSWORD})
            )
            .await
            .status,
            201
        );
    }
    assert_eq!(
        send(
            &f.app,
            "POST",
            "/api/v1/register",
            None,
            json!({"username":"overflow","password":PASSWORD})
        )
        .await
        .status,
        429
    );
    let Fixture { app, dir, clock } = f;
    drop(app);
    let app = App::with_clock(
        dir.path().join("test.sqlite"),
        Config::https(ORIGIN).unwrap(),
        clock.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/v1/register",
            None,
            json!({"username":"overflow","password":PASSWORD})
        )
        .await
        .status,
        429
    );
    clock.advance(3601);
    assert_eq!(
        send(
            &app,
            "POST",
            "/api/v1/register",
            None,
            json!({"username":"overflow","password":PASSWORD})
        )
        .await
        .status,
        201
    );
}

#[tokio::test]
async fn sqlite_faults_return_safe_errors_and_retry_revalidates_current_balances() {
    let f = Fixture::new().await;
    let alice = f.user("alice").await;
    assert_eq!(create(&f.app, &alice, "1", 8, "opening").await.status, 201);
    let raw = f.raw();
    raw.execute_batch("CREATE TRIGGER fail_audit BEFORE INSERT ON source_history BEGIN SELECT RAISE(ABORT,'PRIVATE INTERNAL ERROR'); END;").unwrap();
    let r = create(&f.app, &alice, "2", -8, "spend").await;
    assert_eq!(r.status, 503);
    assert!(!r.text.contains("PRIVATE"));
    assert_eq!(
        raw.query_row("SELECT count(*) FROM sources", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    raw.execute_batch("DROP TRIGGER fail_audit").unwrap();
    assert_eq!(
        create(&f.app, &alice, "3", -8, "other spend").await.status,
        201
    );
    assert_eq!(create(&f.app, &alice, "2", -8, "retry").await.status, 422);
    assert_eq!(
        send(
            &f.app,
            "GET",
            "/api/v1/sources/2/history",
            Some(&alice),
            Value::Null
        )
        .await
        .value,
        json!([])
    );
    raw.execute_batch("BEGIN IMMEDIATE").unwrap();
    let r = create(&f.app, &alice, "4", 8, "busy").await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    raw.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        create(&f.app, &alice, "4", 8, "retry after busy")
            .await
            .status,
        201
    );
}

#[test]
fn https_and_explicit_loopback_configuration_only() {
    for origin in [
        "http://daymark.test",
        "https://daymark.test/path",
        "https://daymark.test/",
        "https://user@daymark.test",
        "https://daymark.test?x=1",
    ] {
        assert!(Config::https(origin).is_err(), "{origin}");
    }
    assert!(Config::https(ORIGIN).is_ok());
    assert!(Config::local("http://127.0.0.1:3000").is_ok());
    assert!(Config::local("http://public.test:3000").is_err());
}
