//! Same-origin application boundary; accepted domain rules live in the domain crate.
mod db;
mod error;
mod http;
pub mod operator;
mod security;
mod wire;

#[cfg(test)]
extern crate self as daymark_api;

pub use error::ApiError;
use error::Result;
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> i64;
}
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs().min(i64::MAX as u64) as i64)
    }
}
#[derive(Clone)]
pub struct Config {
    origin: String,
    local: bool,
}
impl Config {
    pub fn https(origin: impl Into<String>) -> Result<Self> {
        Self::validate(origin.into(), false)
    }
    pub fn local(origin: impl Into<String>) -> Result<Self> {
        Self::validate(origin.into(), true)
    }
    fn validate(origin: String, local: bool) -> Result<Self> {
        let uri: axum::http::Uri = origin.parse().map_err(|_| ApiError::bad())?;
        let host = uri.host().ok_or_else(ApiError::bad)?;
        if uri.scheme_str() != Some(if local { "http" } else { "https" })
            || uri.authority().is_none_or(|a| a.as_str().contains('@'))
            || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
            || origin.ends_with('/')
            || origin.contains(['#', '?'])
            || (local && !matches!(host, "localhost" | "127.0.0.1" | "[::1]"))
        {
            return Err(ApiError::bad());
        }
        Ok(Self { origin, local })
    }
    fn cookie_name(&self) -> &'static str {
        if self.local {
            "daymark-local"
        } else {
            "__Host-daymark"
        }
    }
    fn cookie(&self, token: &str, logout: bool) -> String {
        format!(
            "{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
            self.cookie_name(),
            token,
            if logout { 0 } else { db::SESSION_SECONDS },
            if self.local { "" } else { "; Secure" }
        )
    }
}

#[derive(Clone)]
pub struct App(Arc<Inner>);
struct Inner {
    db: Arc<Mutex<db::Db>>,
    database_slot: Arc<Semaphore>,
    database_queue: Arc<Semaphore>,
    hashing: Arc<Semaphore>,
    requests: Arc<Semaphore>,
    clock: Arc<dyn Clock>,
    config: Config,
    dummy_hash: String,
}
impl App {
    pub async fn open(path: impl AsRef<Path>, config: Config) -> Result<Self> {
        Self::with_clock(path, config, Arc::new(SystemClock)).await
    }
    pub async fn with_clock(
        path: impl AsRef<Path>,
        config: Config,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        let path = path.as_ref().to_owned();
        let (db, dummy_hash) = tokio::task::spawn_blocking(move || -> Result<_> {
            Ok((db::Db::open(&path)?, security::hash(&security::token()?)?))
        })
        .await
        .map_err(|_| ApiError::busy())??;
        Ok(Self(Arc::new(Inner {
            db: Arc::new(Mutex::new(db)),
            database_slot: Arc::new(Semaphore::new(1)),
            database_queue: Arc::new(Semaphore::new(32)),
            hashing: Arc::new(Semaphore::new(2)),
            requests: Arc::new(Semaphore::new(64)),
            clock,
            config,
            dummy_hash,
        })))
    }
    pub fn router(&self) -> axum::Router {
        http::router(self.clone())
    }
    async fn database<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut db::Db, i64) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let queued = self
            .0
            .database_queue
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::busy())?;
        let slot = self
            .0
            .database_slot
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| ApiError::busy())?;
        let db = self.0.db.clone();
        let clock = self.0.clock.clone();
        tokio::task::spawn_blocking(move || {
            // Permits survive cancellation of the requesting future and are held
            // until the actual blocking operation finishes.
            let (_queued, _slot) = (queued, slot);
            let mut db = db.lock().map_err(|_| ApiError::busy())?;
            work(&mut db, clock.now())
        })
        .await
        .map_err(|_| ApiError::busy())?
    }
    async fn protected<T: Send + 'static>(
        &self,
        session: db::Session,
        mutation: bool,
        work: impl FnOnce(&mut db::Db, db::Principal, i64) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        self.database(move |db, now| {
            let principal = db.authenticate(&session, mutation, now)?;
            work(db, principal, now)
        })
        .await
    }
    async fn hashing<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let permit = self
            .0
            .hashing
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::busy())?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(|_| ApiError::busy())?
    }
    /// Local operator boundary, never exposed as an HTTP route. Fails if an
    /// administrator already exists; never promotes a registered account.
    pub async fn bootstrap(&self, username: String, password: String) -> Result<serde_json::Value> {
        let password = Zeroizing::new(password);
        let username = security::username(&username)?;
        security::password(&password)?;
        let hash = self.hashing(move || security::hash(&password)).await?;
        self.database(move |db, now| db.account(&username, &hash, true, now))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn cancelled_database_work_retains_admission_lock_and_logout_ordering() {
        use daymark_domain::{Activity, AuditTime, Bucket, Date, EventId, Source, UserId, Year};
        use std::{future::Future, task::Poll, time::Duration};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancel.sqlite");
        let app = App::open(&path, Config::https("https://daymark.test").unwrap())
            .await
            .unwrap();
        let hash = app
            .hashing(|| security::hash("synthetic password 42"))
            .await
            .unwrap();
        let (token, view) = app
            .database(move |db, now| {
                db.account("alice", &hash, false, now)?;
                db.login("alice", &hash, None, now)
            })
            .await
            .unwrap();
        let owner = UserId(view["id"].as_str().unwrap().parse().unwrap());
        let session = db::Session {
            token,
            csrf: Some(view["csrf"].as_str().unwrap().into()),
        };
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let worker = app.clone();
        let credential = session.clone();
        let task = tokio::spawn(async move {
            worker
                .protected(credential, true, move |db, p, now| {
                    assert_eq!(p.owner, owner);
                    started.send(()).unwrap();
                    wait.recv_timeout(Duration::from_secs(20)).unwrap();
                    db.store
                        .create(
                            p.owner,
                            AuditTime(now),
                            EventId(1),
                            Source {
                                dates: vec![Date::new(2026, 1, 1).unwrap()],
                                activity: Activity::Adjustment {
                                    bucket: Bucket::Pto,
                                    delta: 8,
                                },
                                notes: "Cancelled caller, committed worker".into(),
                            },
                        )
                        .map_err(Into::into)
                })
                .await
        });
        ready.await.unwrap();
        let mut readers = Vec::new();
        for _ in 0..30 {
            let mut pending = Box::pin(app.protected(
                session.clone(),
                false,
                |_, _, _| -> Result<()> { panic!("cancelled queued reader ran") },
            ));
            std::future::poll_fn(|cx| {
                assert!(pending.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            readers.push(pending);
        }
        let revoked = session.clone();
        let mut logout =
            Box::pin(app.protected(session.clone(), true, move |db, _, _| db.logout(&revoked)));
        std::future::poll_fn(|cx| {
            assert!(logout.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert_eq!(app.database(|_, _| Ok(())).await.unwrap_err().0, 503);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(app.0.database_queue.available_permits(), 0);
        drop(readers);
        assert_eq!(app.0.database_queue.available_permits(), 30);
        assert_eq!(app.0.database_slot.available_permits(), 0);
        assert!(
            App::open(&path, Config::https("https://daymark.test").unwrap())
                .await
                .is_err()
        );
        release.send(()).unwrap();
        logout.await.unwrap();
        assert_eq!(
            app.protected(session, true, |_, _, _| -> Result<()> {
                panic!("revoked authority reached persistence")
            })
            .await
            .unwrap_err()
            .0,
            401
        );
        let (snapshot, history) = app
            .database(move |db, _| {
                Ok((
                    db.store.snapshot(owner, Year::new(2026).unwrap())?,
                    db.store.history(owner, EventId(1))?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(snapshot.balance(Bucket::Pto).get(), 8);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].actor, owner);
        assert_eq!(app.0.database_queue.available_permits(), 32);
        assert_eq!(app.0.database_slot.available_permits(), 1);
    }

    #[tokio::test]
    async fn hashing_is_bounded_even_when_requesting_futures_are_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(
            dir.path().join("test.sqlite"),
            Config::https("https://daymark.test").unwrap(),
        )
        .await
        .unwrap();
        let mut releases = Vec::new();
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let (started, ready) = tokio::sync::oneshot::channel();
            let (release, wait) = std::sync::mpsc::channel();
            let worker = app.clone();
            tasks.push(tokio::spawn(async move {
                worker
                    .hashing(move || {
                        started.send(()).unwrap();
                        wait.recv().unwrap();
                        Ok(())
                    })
                    .await
            }));
            ready.await.unwrap();
            releases.push(release);
        }
        for task in &tasks {
            task.abort();
        }
        assert_eq!(
            app.hashing(|| Ok(())).await.unwrap_err().0,
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        let response = app
            .router()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        for release in releases {
            release.send(()).unwrap();
        }
        // Acquiring both permits is a deterministic completion handshake.
        let _done = app.0.hashing.acquire_many(2).await.unwrap();
    }

    #[tokio::test]
    async fn database_queue_is_bounded_and_health_remains_responsive() {
        use crate::http::router;
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(
            dir.path().join("test.sqlite"),
            Config::https("https://daymark.test").unwrap(),
        )
        .await
        .unwrap();
        let held = app.0.database_queue.acquire_many(32).await.unwrap();
        let error = app.database(|_, _| Ok(())).await.unwrap_err();
        assert_eq!(error.0, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        let response = router(app.clone())
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        drop(held);
        assert!(app.database(|_, _| Ok(())).await.is_ok());
    }

    #[tokio::test]
    async fn queued_protected_work_rechecks_expiry_and_revocation_before_execution() {
        use std::{
            future::Future,
            sync::atomic::{AtomicI64, Ordering},
            task::Poll,
        };
        struct TestClock(AtomicI64);
        impl Clock for TestClock {
            fn now(&self) -> i64 {
                self.0.load(Ordering::SeqCst)
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock(AtomicI64::new(10000)));
        let app = App::with_clock(
            dir.path().join("test.sqlite"),
            Config::https("https://daymark.test").unwrap(),
            clock.clone(),
        )
        .await
        .unwrap();
        let hash = security::hash("synthetic password 42").unwrap();
        let setup_hash = hash.clone();
        app.database(move |db, now| db.account("alice", &setup_hash, false, now))
            .await
            .unwrap();
        for expired in [true, false] {
            let hash = hash.clone();
            let (token, view) = app
                .database(move |db, now| db.login("alice", &hash, None, now))
                .await
                .unwrap();
            let session = db::Session {
                token,
                csrf: Some(view["csrf"].as_str().unwrap().into()),
            };
            let held = app.0.database_slot.acquire().await.unwrap();
            let mut pending = Box::pin(app.protected(
                session.clone(),
                true,
                |_, _, _| -> Result<()> { panic!("stale session executed protected operation") },
            ));
            std::future::poll_fn(|cx| {
                assert!(pending.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            if expired {
                clock.0.fetch_add(db::IDLE_SECONDS, Ordering::SeqCst);
            } else {
                // Test-only control of the serialized state at the queue boundary.
                app.0.db.lock().unwrap().logout(&session).unwrap();
            }
            drop(held);
            assert_eq!(
                pending.await.unwrap_err().0,
                axum::http::StatusCode::UNAUTHORIZED
            );
        }
    }

    #[test]
    fn reset_between_credential_read_and_session_issuance_rejects_old_verified_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = db::Db::open(&dir.path().join("test.sqlite")).unwrap();
        let old = security::hash("synthetic old password 42").unwrap();
        let new = security::hash("synthetic new password 42").unwrap();
        let account = db.account("alice", &old, false, 10000).unwrap();
        let admin = db.account("operator", &new, true, 10000).unwrap();
        let user = daymark_domain::UserId(account["id"].as_str().unwrap().parse().unwrap());
        let operator = daymark_domain::UserId(admin["id"].as_str().unwrap().parse().unwrap());
        let verified = db.credentials("alice").unwrap().unwrap();
        assert!(security::verify("synthetic old password 42", &verified));
        let (token, _) = db.login("alice", &verified, None, 10000).unwrap();
        db.reset(operator, user, &new, 10001).unwrap();
        assert_eq!(
            db.login("alice", &verified, None, 10002).unwrap_err().0,
            axum::http::StatusCode::UNAUTHORIZED
        );
        assert!(
            db.authenticate(&db::Session { token, csrf: None }, false, 10002)
                .is_err()
        );
        assert!(db.login("alice", &new, None, 10002).is_ok());
    }
}
