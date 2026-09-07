use crate::{
    App,
    db::{Db, Session},
    error::{ApiError, Result},
    security,
    wire::*,
};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use daymark_domain::{EventId, UserId, Year};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{net::SocketAddr, time::Duration};
use zeroize::Zeroizing;

pub fn router(app: App) -> Router {
    Router::new()
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/api/v1/register", post(register))
        .route("/api/v1/login", post(login))
        .route("/api/v1/logout", post(logout))
        .route("/api/v1/session", get(session_info))
        .route("/api/v1/settings", get(settings).put(set_settings))
        .route("/api/v1/years/{year}/snapshot", get(snapshot))
        .route("/api/v1/years/{year}/ledger", get(ledger))
        .route("/api/v1/sources", post(create))
        .route(
            "/api/v1/years/{year}/sources/{id}",
            get(record).put(edit).delete(delete),
        )
        .route("/api/v1/sources/{id}/history", get(history))
        .route(
            "/api/v1/years/{year}/holidays",
            get(calendar).put(configure),
        )
        .route("/api/v1/admin/accounts", get(accounts))
        .route("/api/v1/admin/accounts/{id}/password", post(reset))
        .fallback(|| async { ApiError::missing() })
        .layer(middleware::from_fn_with_state(app.clone(), boundary))
        .with_state(app)
}

async fn boundary(State(app): State<App>, mut req: Request, next: Next) -> Response {
    let result = async {
        let _permit = app
            .0
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::busy())?;
        if req.uri().to_string().len() > 4096
            || req
                .headers()
                .iter()
                .map(|(k, v)| k.as_str().len() + v.len())
                .sum::<usize>()
                > 16384
        {
            return Err(ApiError::bad());
        }
        let mutation = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
        let origin = single(req.headers(), "origin")?;
        if origin.is_some_and(|o| o != app.0.config.origin)
            || (mutation && origin != Some(app.0.config.origin.as_str()))
        {
            return Err(ApiError::forbidden());
        }
        if single(req.headers(), "sec-fetch-site")?
            .is_some_and(|s| !matches!(s, "same-origin" | "none"))
        {
            return Err(ApiError::forbidden());
        }
        if mutation
            && (single(req.headers(), "x-daymark-request")? != Some("1")
                || single(req.headers(), "content-type")?.is_none_or(|v| {
                    !v.split(';')
                        .next()
                        .is_some_and(|m| m.trim() == "application/json")
                }))
        {
            return Err(ApiError::forbidden());
        }
        let body = std::mem::replace(req.body_mut(), Body::empty());
        let bytes = tokio::time::timeout(Duration::from_secs(10), to_bytes(body, 32 * 1024))
            .await
            .map_err(|_| {
                ApiError(
                    StatusCode::REQUEST_TIMEOUT,
                    "body_timeout",
                    "Request body timed out.".into(),
                )
            })?
            .map_err(|_| {
                ApiError(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "body_limit",
                    "Request body exceeds 32 KiB or is invalid.".into(),
                )
            })?;
        *req.body_mut() = Body::from(bytes);
        Ok::<_, ApiError>(next.run(req).await)
    }
    .await;
    let mut response = result.unwrap_or_else(IntoResponse::into_response);
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        (
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'; base-uri 'none'",
        ),
    ] {
        response.headers_mut().insert(name, value.parse().unwrap());
    }
    if !app.0.config.local {
        response.headers_mut().insert(
            "strict-transport-security",
            "max-age=31536000".parse().unwrap(),
        );
    }
    response
}
fn single<'a>(headers: &'a HeaderMap, key: &str) -> Result<Option<&'a str>> {
    let mut values = headers.get_all(key).iter();
    let value = values
        .next()
        .map(|v| v.to_str().map_err(|_| ApiError::bad()))
        .transpose()?;
    if values.next().is_some() {
        return Err(ApiError::bad());
    }
    Ok(value)
}
fn session(app: &App, headers: &HeaderMap) -> Result<Session> {
    optional_session(app, headers)?.ok_or_else(ApiError::unauthorized)
}
fn optional_session(app: &App, headers: &HeaderMap) -> Result<Option<Session>> {
    let csrf = single(headers, "x-csrf-token")?.map(str::to_owned);
    let mut token = None;
    for header in headers.get_all("cookie") {
        for cookie in header
            .to_str()
            .map_err(|_| ApiError::unauthorized())?
            .split(';')
        {
            if let Some((key, value)) = cookie.trim().split_once('=')
                && key == app.0.config.cookie_name()
            {
                if token.is_some() || !security::valid_token(value) {
                    return Err(ApiError::unauthorized());
                }
                token = Some(value.to_owned());
            }
        }
    }
    Ok(token.map(|token| Session { token, csrf }))
}
fn parse<T: DeserializeOwned>(body: Input<T>) -> Result<T> {
    body.map(|Json(value)| value).map_err(|_| ApiError::bad())
}
// A separate alias keeps extractor rejection types distinct from API Result.
type Input<T> = std::result::Result<Json<T>, axum::extract::rejection::JsonRejection>;
fn query<T>(
    value: std::result::Result<Query<T>, axum::extract::rejection::QueryRejection>,
) -> Result<T> {
    value.map(|Query(q)| q).map_err(|_| ApiError::bad())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    username: String,
    password: String,
}
async fn register(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    body: Input<Credentials>,
) -> Result<(StatusCode, Json<Value>)> {
    let input = parse(body)?;
    let password = Zeroizing::new(input.password);
    let username = security::username(&input.username)?;
    security::password(&password)?;
    let name = username.clone();
    app.database(move |db, now| db.throttle(true, &name, &peer.ip().to_string(), now))
        .await?;
    let hash = app.hashing(move || security::hash(&password)).await?;
    let account = app
        .database(move |db, now| db.account(&username, &hash, false, now))
        .await?;
    Ok((StatusCode::CREATED, Json(account)))
}
async fn login(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Input<Credentials>,
) -> Result<Response> {
    // Missing or expired sessions may log in, but ambiguous credentials must
    // never be silently discarded and bypass presented-session revocation.
    let old = optional_session(&app, &headers)?;
    let input = parse(body)?;
    let password = Zeroizing::new(input.password);
    let username = security::username(&input.username).map_err(|_| ApiError::unauthorized())?;
    if password.len() > 128 || password.is_empty() {
        return Err(ApiError::unauthorized());
    }
    let name = username.clone();
    let hash = app
        .database(move |db, now| {
            db.throttle(false, &name, &peer.ip().to_string(), now)?;
            db.credentials(&name)
        })
        .await?;
    let exists = hash.is_some();
    let hash = hash.unwrap_or_else(|| app.0.dummy_hash.clone());
    let verified = app
        .hashing(move || {
            if security::verify(&password, &hash) && exists {
                Ok(hash)
            } else {
                Err(ApiError::unauthorized())
            }
        })
        .await?;
    let (token, value) = app
        .database(move |db, now| db.login(&username, &verified, old, now))
        .await?;
    let mut response = Json(value).into_response();
    response.headers_mut().insert(
        "set-cookie",
        app.0
            .config
            .cookie(&token, false)
            .parse()
            .map_err(|_| ApiError::busy())?,
    );
    Ok(response)
}
async fn logout(State(app): State<App>, headers: HeaderMap) -> Result<Response> {
    let session = session(&app, &headers)?;
    app.protected(session.clone(), true, move |db, _, _| db.logout(&session))
        .await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert("set-cookie", app.0.config.cookie("", true).parse().unwrap());
    Ok(response)
}
async fn session_info(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>> {
    Ok(Json(
        app.protected(session(&app, &headers)?, false, |_, p, _| Ok(p.view()))
            .await?,
    ))
}
async fn settings(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>> {
    Ok(Json(
        app.protected(session(&app, &headers)?, false, |db, p, _| {
            db.settings(p.owner)
        })
        .await?,
    ))
}
async fn set_settings(
    State(app): State<App>,
    headers: HeaderMap,
    body: Input<SettingsInput>,
) -> Result<Json<Value>> {
    let value = parse(body)?.value()?;
    Ok(Json(
        app.protected(session(&app, &headers)?, true, move |db, p, now| {
            db.set_settings(p.owner, value, now)
        })
        .await?,
    ))
}
async fn snapshot(
    State(app): State<App>,
    headers: HeaderMap,
    Path(year): Path<u16>,
) -> Result<Json<Value>> {
    let year = Year::new(year)?;
    Ok(Json(
        app.protected(session(&app, &headers)?, false, move |db, p, _| {
            output(db.store.snapshot(p.owner, year)?)
        })
        .await?,
    ))
}
async fn ledger(
    State(app): State<App>,
    headers: HeaderMap,
    Path(year): Path<u16>,
    params: std::result::Result<Query<QueryInput>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<Value>> {
    let year = Year::new(year)?;
    let (filter, page) = query(params)?.domain()?;
    let (offset, limit) = page.bounds()?;
    Ok(Json(
        app.protected(session(&app, &headers)?, false, move |db, p, _| {
            let effects = db.store.query(p.owner, year, &filter)?;
            output(
                effects
                    .into_iter()
                    .skip(offset as usize)
                    .take(limit as usize)
                    .collect::<Vec<_>>(),
            )
        })
        .await?,
    ))
}
async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    body: Input<CreateInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let input = parse(body)?;
    let id = EventId(integer(&input.id)?);
    let source = input.source.domain()?;
    Ok((
        StatusCode::CREATED,
        Json(
            app.protected(session(&app, &headers)?, true, move |db, p, now| {
                output(db.store.create(p.owner, Db::time(now), id, source)?)
            })
            .await?,
        ),
    ))
}
async fn record(
    State(app): State<App>,
    headers: HeaderMap,
    Path((year, id)): Path<(u16, String)>,
) -> Result<Json<Value>> {
    let (year, id) = (Year::new(year)?, EventId(integer(&id)?));
    Ok(Json(
        app.protected(session(&app, &headers)?, false, move |db, p, _| {
            output(db.store.record(p.owner, year, id)?)
        })
        .await?,
    ))
}
async fn edit(
    State(app): State<App>,
    headers: HeaderMap,
    Path((year, id)): Path<(u16, String)>,
    body: Input<EditInput>,
) -> Result<Json<Value>> {
    let input = parse(body)?;
    let (year, id, revision, source) = (
        Year::new(year)?,
        EventId(integer(&id)?),
        integer(&input.revision)?,
        input.source.domain()?,
    );
    Ok(Json(
        app.protected(session(&app, &headers)?, true, move |db, p, now| {
            output(
                db.store
                    .edit(p.owner, year, Db::time(now), id, revision, source)?,
            )
        })
        .await?,
    ))
}
async fn delete(
    State(app): State<App>,
    headers: HeaderMap,
    Path((year, id)): Path<(u16, String)>,
    body: Input<DeleteInput>,
) -> Result<Json<Value>> {
    let revision = integer(&parse(body)?.revision)?;
    let (year, id) = (Year::new(year)?, EventId(integer(&id)?));
    Ok(Json(
        app.protected(session(&app, &headers)?, true, move |db, p, now| {
            output(
                db.store
                    .delete(p.owner, year, Db::time(now), id, revision)?,
            )
        })
        .await?,
    ))
}
async fn history(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    params: std::result::Result<Query<Page>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<Value>> {
    let id = integer(&id)?;
    let (offset, limit) = query(params)?.bounds()?;
    Ok(Json(
        app.protected(session(&app, &headers)?, false, move |db, p, _| {
            db.history(p.owner, id, offset, limit)
        })
        .await?,
    ))
}
async fn calendar(
    State(app): State<App>,
    headers: HeaderMap,
    Path(year): Path<u16>,
) -> Result<Json<Value>> {
    let year = Year::new(year)?;
    Ok(Json(
        app.protected(session(&app, &headers)?, false, move |db, _, _| {
            match db.store.calendar(year)? {
                Some(c) => {
                    Ok(json!({"revision":c.revision.to_string(),"calendar":output(c.calendar)?}))
                }
                None => Ok(Value::Null),
            }
        })
        .await?,
    ))
}
async fn configure(
    State(app): State<App>,
    headers: HeaderMap,
    Path(year): Path<u16>,
    body: Input<CalendarInput>,
) -> Result<Json<Value>> {
    let (revision, calendar) = parse(body)?.domain(Year::new(year)?)?;
    Ok(Json(app.protected(session(&app,&headers)?,true,move |db,p,now| {p.admin()?;Ok(json!({"revision":db.store.configure(p.owner,Db::time(now),revision,calendar)?.to_string()}))}).await?))
}
async fn accounts(
    State(app): State<App>,
    headers: HeaderMap,
    params: std::result::Result<Query<Page>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<Value>> {
    let (offset, limit) = query(params)?.bounds()?;
    Ok(Json(
        app.protected(session(&app, &headers)?, false, move |db, p, _| {
            p.admin()?;
            db.accounts(offset, limit)
        })
        .await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetInput {
    password: String,
}
async fn reset(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Input<ResetInput>,
) -> Result<StatusCode> {
    let target = UserId(integer(&id)?);
    let password = Zeroizing::new(parse(body)?.password);
    security::password(&password)?;
    let session = session(&app, &headers)?;
    // Do not let ordinary or stale sessions consume expensive hashing resources.
    app.protected(session.clone(), true, |_, p, _| p.admin())
        .await?;
    let hash = app.hashing(move || security::hash(&password)).await?;
    app.protected(session, true, move |db, p, now| {
        p.admin()?;
        db.reset(p.owner, target, &hash, now)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
