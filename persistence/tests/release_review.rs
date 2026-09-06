use daymark_domain::*;
use daymark_persistence::{Store, StoreError};
use rusqlite::Connection;

const OWNER: UserId = UserId(u64::MAX);
const TIME: AuditTime = AuditTime(i64::MIN);

fn year() -> Year {
    Year::new(2026).unwrap()
}

fn source(delta: i64) -> Source {
    Source {
        dates: vec![Date::new(2026, 1, 1).unwrap()],
        activity: Activity::Adjustment {
            bucket: Bucket::Pto,
            delta,
        },
        notes: "Release review".into(),
    }
}

fn calendar() -> Calendar {
    Calendar {
        year: year(),
        holidays: (1..=10)
            .map(|month| Holiday {
                id: HolidayId(u64::MAX - u64::from(month)),
                date: Date::new(2026, month, 1).unwrap(),
                name: format!("Day {month}"),
            })
            .collect(),
    }
}

fn worked(month: u8) -> Source {
    Source {
        dates: vec![Date::new(2026, month, 1).unwrap()],
        activity: Activity::HolidayWork {
            holiday: HolidayId(u64::MAX - u64::from(month)),
            work: Work {
                hours_worked: Some(Hours::new(i64::MAX).unwrap()),
                multiplier: None,
                credited_comp: Hours::new(1).unwrap(),
            },
        },
        notes: format!("Support {month}"),
    }
}

fn dump(conn: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
    let mut result = Vec::new();
    for table in [
        "calendars",
        "holidays",
        "sources",
        "effects",
        "effect_supports",
        "source_history",
        "calendar_history",
        "sqlite_sequence",
    ] {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = stmt.column_count();
        result.extend(
            stmt.query_map([], |row| {
                (0..columns)
                    .map(|i| row.get(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap(),
        );
    }
    result
}

#[test]
fn deferred_calendar_commit_failure_restores_configuration_and_audit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("review.sqlite");
    let mut store = Store::open(&path).unwrap();
    store.configure(UserId(0), TIME, None, calendar()).unwrap();
    store.create(OWNER, TIME, EventId(0), worked(1)).unwrap();
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch("CREATE TRIGGER fail_calendar_commit AFTER INSERT ON calendar_history BEGIN UPDATE sources SET holiday_id=zeroblob(8); END;").unwrap();
    let before = dump(&raw);
    let mut next = calendar();
    next.holidays[0].name = "Corrected".into();
    assert!(
        matches!(store.configure(UserId(0), TIME, Some(1), next.clone()), Err(StoreError::Storage(rusqlite::Error::SqliteFailure(e, _))) if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY)
    );
    assert_eq!(dump(&raw), before);
    raw.execute_batch("DROP TRIGGER fail_calendar_commit")
        .unwrap();
    assert_eq!(
        store
            .configure(UserId(0), TIME, Some(1), next.clone())
            .unwrap(),
        2
    );
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.calendar(year()).unwrap().unwrap().calendar, next);
    store.snapshot(OWNER, year()).unwrap();
}

// Run this same integration-test executable as an independent process. Pipe
// handshakes control phases; no timing sleeps decide lock or commit outcomes.
#[test]
fn subprocess_worker() {
    use std::io::{BufRead, Write};
    let Some(path) = std::env::var_os("DAYMARK_REVIEW_DB") else {
        return;
    };
    let mode = std::env::var("DAYMARK_REVIEW_MODE").unwrap();
    let mut store = Store::open(&path).unwrap();
    let mut input = std::io::stdin().lock().lines();
    let signal = |value| {
        println!("REVIEW_{value}");
        std::io::stdout().flush().unwrap();
    };
    if mode == "busy" {
        signal("READY");
        input.next().unwrap().unwrap();
        assert!(
            matches!(store.create(OWNER, TIME, EventId(2), source(-8)), Err(StoreError::Storage(rusqlite::Error::SqliteFailure(e, _))) if e.code == rusqlite::ErrorCode::DatabaseBusy)
        );
        signal("BUSY");
        input.next().unwrap().unwrap();
        assert!(matches!(
            store.create(OWNER, TIME, EventId(2), source(-8)),
            Err(StoreError::Domain(Error::NegativeBalance {
                bucket: Bucket::Pto,
                ..
            }))
        ));
        signal("REVALIDATED");
    } else {
        assert_eq!(mode, "crash");
        drop(store);
        let raw = Connection::open(path).unwrap();
        raw.execute_batch("PRAGMA foreign_keys=ON; BEGIN IMMEDIATE")
            .unwrap();
        let mut ledger = Ledger::default();
        ledger.create(OWNER, TIME, EventId(1), source(8)).unwrap();
        let change = &ledger.history(OWNER)[0];
        let record = change.after.as_ref().unwrap();
        raw.execute(
            "INSERT INTO sources VALUES (?1,?2,?2,2026,NULL,?3)",
            rusqlite::params![
                OWNER.0.to_be_bytes(),
                1_u64.to_be_bytes(),
                serde_json::to_string(record).unwrap()
            ],
        )
        .unwrap();
        raw.execute(
            "INSERT INTO effects VALUES (?1,2026,0,?2)",
            rusqlite::params![
                OWNER.0.to_be_bytes(),
                serde_json::to_string(&change.after_effects[0].effects[0]).unwrap()
            ],
        )
        .unwrap();
        raw.execute(
            "INSERT INTO effect_supports VALUES (?1,2026,0,?2,?2)",
            rusqlite::params![OWNER.0.to_be_bytes(), 1_u64.to_be_bytes()],
        )
        .unwrap();
        raw.execute(
            "INSERT INTO source_history(owner,source_id,at,payload) VALUES (?1,?2,?3,?4)",
            rusqlite::params![
                OWNER.0.to_be_bytes(),
                1_u64.to_be_bytes(),
                TIME.0,
                serde_json::to_string(change).unwrap()
            ],
        )
        .unwrap();
        signal("UNCOMMITTED");
        input.next().unwrap().unwrap();
        // Deliberately bypass Rust Drop/rollback, including SQLite connection close.
        std::process::exit(0);
    }
}

struct Worker {
    child: std::process::Child,
    lines: std::sync::mpsc::Receiver<String>,
}

impl Worker {
    fn start(path: &std::path::Path, mode: &str) -> Self {
        use std::io::BufRead;
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "subprocess_worker", "--nocapture"])
            .env("DAYMARK_REVIEW_DB", path)
            .env("DAYMARK_REVIEW_MODE", mode)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let output = child.stdout.take().unwrap();
        let (send, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(output).lines() {
                if send.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Self { child, lines }
    }

    fn expect(&self, signal: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let line = self
                .lines
                .recv_timeout(remaining)
                .expect("worker failed or timed out");
            if line.contains(&format!("REVIEW_{signal}")) {
                break;
            }
        }
    }

    fn proceed(&mut self) {
        use std::io::Write;
        writeln!(self.child.stdin.as_mut().unwrap(), "go").unwrap();
    }

    fn finish(&mut self) {
        assert!(self.child.wait().unwrap().success());
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn independent_process_busy_timeout_and_retry_reload_committed_balance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("review.sqlite");
    let mut store = Store::open(&path).unwrap();
    store.create(OWNER, TIME, EventId(1), source(8)).unwrap();
    let mut worker = Worker::start(&path, "busy");
    worker.expect("READY");
    let raw = Connection::open(&path).unwrap();
    let before = dump(&raw);
    raw.execute_batch("BEGIN IMMEDIATE").unwrap();
    worker.proceed();
    worker.expect("BUSY");
    assert_eq!(dump(&raw), before);
    raw.execute_batch("ROLLBACK").unwrap();
    store.create(OWNER, TIME, EventId(3), source(-8)).unwrap();
    worker.proceed();
    worker.expect("REVALIDATED");
    worker.finish();
    assert!(store.history(OWNER, EventId(2)).unwrap().is_empty());
    assert_eq!(
        store
            .snapshot(OWNER, year())
            .unwrap()
            .balance(Bucket::Pto)
            .get(),
        0
    );
}

#[test]
fn process_exit_without_drop_discards_uncommitted_sources_effects_and_audit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("review.sqlite");
    let store = Store::open(&path).unwrap();
    drop(store);
    let raw = Connection::open(&path).unwrap();
    let before = dump(&raw);
    let mut worker = Worker::start(&path, "crash");
    worker.expect("UNCOMMITTED");
    assert_eq!(dump(&raw), before);
    worker.proceed();
    worker.finish();
    let mut store = Store::open(&path).unwrap();
    assert_eq!(dump(&raw), before);
    assert!(store.history(OWNER, EventId(1)).unwrap().is_empty());
    store.create(OWNER, TIME, EventId(1), source(8)).unwrap();
    assert_eq!(
        store
            .snapshot(OWNER, year())
            .unwrap()
            .balance(Bucket::Pto)
            .get(),
        8
    );
}

#[test]
fn mutations_must_not_silently_repair_drift_and_write_an_inaccurate_before_audit() {
    for operation in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("review.sqlite");
        let mut store = Store::open(&path).unwrap();
        store.create(OWNER, TIME, EventId(1), source(8)).unwrap();
        let mut moved = source(8);
        moved.dates = vec![Date::new(2027, 1, 1).unwrap()];
        store
            .create(OWNER, TIME, EventId(3), moved.clone())
            .unwrap();
        let drift_year = if operation == 4 { 2027 } else { 2026 };
        let raw = Connection::open(&path).unwrap();
        raw.execute(
            "UPDATE effects SET payload=json_set(payload,'$.delta',7) WHERE year=?1",
            [drift_year],
        )
        .unwrap();
        let before = dump(&raw);
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert!(matches!(
            store.snapshot(OWNER, Year::new(drift_year).unwrap()),
            Err(StoreError::CorruptState)
        ));
        let result = match operation {
            0 => store.create(OWNER, TIME, EventId(2), source(1)),
            1 => store.edit(OWNER, year(), TIME, EventId(1), 1, source(9)),
            2 => store.delete(OWNER, year(), TIME, EventId(1), 1),
            _ => store.edit(OWNER, year(), TIME, EventId(1), 1, moved),
        };
        assert!(
            matches!(result, Err(StoreError::CorruptState)),
            "operation {operation}: {result:?}"
        );
        assert_eq!(dump(&raw), before);
    }
}

#[test]
fn relational_support_drift_is_detected_even_when_sqlite_foreign_keys_pass() {
    for corruption in [
        "DELETE FROM effect_supports WHERE source_id=x'FFFFFFFFFFFFFFFF'",
        // This other source exists, with the same owner/year/revision. An FK
        // proves existence, not that it actually generated the linked effect.
        "UPDATE effect_supports SET source_id=x'0000000000000000' WHERE source_id=x'FFFFFFFFFFFFFFFF' AND ordinal=(SELECT min(ordinal) FROM effect_supports WHERE source_id=x'FFFFFFFFFFFFFFFF')",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("review.sqlite");
        let mut store = Store::open(&path).unwrap();
        store.configure(UserId(0), TIME, None, calendar()).unwrap();
        store
            .create(OWNER, TIME, EventId(u64::MAX), worked(1))
            .unwrap();
        store.create(OWNER, TIME, EventId(0), source(1)).unwrap();
        store.create(OWNER, TIME, EventId(1), worked(1)).unwrap();
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        raw.execute(corruption, []).unwrap();
        assert!(
            !raw.prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap()
        );
        let before = dump(&raw);
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert!(matches!(
            store.snapshot(OWNER, year()),
            Err(StoreError::CorruptState)
        ));
        assert!(matches!(
            store.query(OWNER, year(), &Filter::default()),
            Err(StoreError::CorruptState)
        ));
        assert!(matches!(
            store.edit(OWNER, year(), TIME, EventId(0), 1, source(2)),
            Err(StoreError::CorruptState)
        ));
        assert_eq!(dump(&raw), before);
    }
}

#[test]
fn durable_order_and_full_width_ids_match_domain_across_every_insertion_order() {
    let mut expected = None;
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("review.sqlite");
        let mut store = Store::open(&path).unwrap();
        let mut domain = Ledger::default();
        let mut c = calendar();
        c.holidays.reverse();
        store.configure(UserId(0), TIME, None, c.clone()).unwrap();
        domain.configure(UserId(0), TIME, c).unwrap();
        for index in order {
            let id = EventId([u64::MAX, 0, 1 << 63][index]);
            let input = worked([1, 1, 2][index]);
            let change = store.create(OWNER, TIME, id, input.clone()).unwrap();
            domain.create(OWNER, TIME, id, input).unwrap();
            assert_eq!(&change, domain.history(OWNER).last().unwrap());
            drop(store);
            store = Store::open(&path).unwrap();
            assert_eq!(
                store.snapshot(OWNER, year()).unwrap(),
                domain.snapshot(OWNER, year()).unwrap()
            );
        }
        let snapshot = store.snapshot(OWNER, year()).unwrap();
        if let Some(expected) = &expected {
            assert_eq!(&snapshot, expected);
        } else {
            expected = Some(snapshot);
        }
    }
}

#[test]
fn signed_minimum_adjustment_round_trips_in_sources_effects_and_audit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("review.sqlite");
    let mut store = Store::open(&path).unwrap();
    let mut domain = Ledger::default();
    for (id, delta) in [(3, i64::MAX), (2, -1), (1, 1)] {
        store
            .create(OWNER, TIME, EventId(id), source(delta))
            .unwrap();
        domain
            .create(OWNER, TIME, EventId(id), source(delta))
            .unwrap();
    }
    let change = store
        .edit(OWNER, year(), TIME, EventId(2), 1, source(i64::MIN))
        .unwrap();
    domain
        .edit(OWNER, TIME, EventId(2), 1, source(i64::MIN))
        .unwrap();
    assert_eq!(&change, domain.history(OWNER).last().unwrap());
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store.record(OWNER, year(), EventId(2)).unwrap(),
        *domain.record(OWNER, EventId(2)).unwrap()
    );
    assert_eq!(store.history(OWNER, EventId(2)).unwrap()[1], change);
    assert_eq!(
        store
            .snapshot(OWNER, year())
            .unwrap()
            .balance(Bucket::Pto)
            .get(),
        0
    );
    let raw = Connection::open(&path).unwrap();
    let before = dump(&raw);
    assert!(matches!(
        store.delete(OWNER, year(), TIME, EventId(2), 2),
        Err(StoreError::Domain(Error::Overflow))
    ));
    assert_eq!(dump(&raw), before);
}

#[test]
fn source_revision_encoding_crosses_signed_boundary_and_rejects_unsigned_overflow() {
    // Synthetic restored heads avoid pretending to execute 2^64 edits. The
    // fixture tests current-state encoding and subsequent real audited writes.
    for revision in [i64::MAX as u64, u64::MAX - 1, u64::MAX] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("review.sqlite");
        drop(Store::open(&path).unwrap());
        let record = Record {
            reference: SourceRef {
                id: EventId(u64::MAX),
                revision,
            },
            source: source(8),
        };
        let mut domain = Ledger::restore([], [(OWNER, record.clone())], []).unwrap();
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch("PRAGMA foreign_keys=ON; BEGIN IMMEDIATE")
            .unwrap();
        raw.execute(
            "INSERT INTO sources VALUES (?1,?1,?2,2026,NULL,?3)",
            rusqlite::params![
                OWNER.0.to_be_bytes(),
                revision.to_be_bytes(),
                serde_json::to_string(&record).unwrap()
            ],
        )
        .unwrap();
        raw.execute(
            "INSERT INTO effects VALUES (?1,2026,0,?2)",
            rusqlite::params![
                OWNER.0.to_be_bytes(),
                serde_json::to_string(&domain.snapshot(OWNER, year()).unwrap().effects[0]).unwrap()
            ],
        )
        .unwrap();
        raw.execute(
            "INSERT INTO effect_supports VALUES (?1,2026,0,?1,?2)",
            rusqlite::params![OWNER.0.to_be_bytes(), revision.to_be_bytes()],
        )
        .unwrap();
        raw.execute_batch("COMMIT").unwrap();
        let mut store = Store::open(&path).unwrap();
        assert_eq!(
            store.record(OWNER, year(), EventId(u64::MAX)).unwrap(),
            record
        );
        assert_eq!(
            store.snapshot(OWNER, year()).unwrap(),
            domain.snapshot(OWNER, year()).unwrap()
        );
        let before = dump(&raw);
        let actual = store.edit(OWNER, year(), TIME, EventId(u64::MAX), revision, source(9));
        let expected = domain.edit(OWNER, TIME, EventId(u64::MAX), revision, source(9));
        let current_revision = if revision == u64::MAX {
            assert_eq!(expected, Err(Error::Overflow));
            assert!(matches!(actual, Err(StoreError::Domain(Error::Overflow))));
            assert_eq!(dump(&raw), before);
            revision
        } else {
            expected.unwrap();
            assert_eq!(&actual.unwrap(), domain.history(OWNER).last().unwrap());
            revision + 1
        };
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert_eq!(
            store.snapshot(OWNER, year()).unwrap(),
            domain.snapshot(OWNER, year()).unwrap()
        );
        assert_eq!(
            store
                .record(OWNER, year(), EventId(u64::MAX))
                .unwrap()
                .reference
                .revision,
            current_revision
        );
        let change = store
            .delete(OWNER, year(), TIME, EventId(u64::MAX), current_revision)
            .unwrap();
        domain
            .delete(OWNER, TIME, EventId(u64::MAX), current_revision)
            .unwrap();
        assert_eq!(&change, domain.history(OWNER).last().unwrap());
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert!(matches!(
            store.create(OWNER, TIME, EventId(u64::MAX), source(1)),
            Err(StoreError::Domain(Error::AlreadyExists))
        ));
        assert_eq!(
            store.history(OWNER, EventId(u64::MAX)).unwrap(),
            domain.history(OWNER)
        );
    }
}
