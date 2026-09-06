use daymark_domain::*;
use daymark_persistence::{Store, StoreError};
use rusqlite::Connection;
use std::{
    sync::{Arc, Barrier},
    thread,
};
use tempfile::TempDir;

const A: UserId = UserId(1);
const B: UserId = UserId(u64::MAX);
const ADMIN: UserId = UserId(99);
const T: AuditTime = AuditTime(1_800_000_000);
fn y(n: u16) -> Year {
    Year::new(n).unwrap()
}
fn h(n: i64) -> Hours {
    Hours::new(n).unwrap()
}
fn source(year: u16, activity: Activity) -> Source {
    Source {
        dates: vec![Date::new(year, 1, 1).unwrap()],
        activity,
        notes: "Several sentences of retained context. A second sentence. Final details.".into(),
    }
}
fn adjustment(year: u16, bucket: Bucket, delta: i64) -> Source {
    source(year, Activity::Adjustment { bucket, delta })
}
fn worked(year: u16, comp: i64) -> Source {
    source(
        year,
        Activity::HolidayWork {
            holiday: HolidayId(1),
            work: Work {
                hours_worked: Some(h(1)),
                multiplier: Some(Multiplier::OneAndAHalf),
                credited_comp: h(comp),
            },
        },
    )
}
fn calendar(year: u16) -> Calendar {
    Calendar {
        year: y(year),
        holidays: (1..=10)
            .map(|month| Holiday {
                id: HolidayId(u64::from(month)),
                date: Date::new(year, month, 1).unwrap(),
                name: format!("Holiday {month}"),
            })
            .collect(),
    }
}
fn database() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("ledger.sqlite")).unwrap();
    (dir, store)
}
fn raw(dir: &TempDir) -> Connection {
    let conn = Connection::open(dir.path().join("ledger.sqlite")).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    conn
}
fn dump(conn: &Connection) -> Vec<String> {
    let mut result = Vec::new();
    for table in [
        "schema_migrations",
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
        let rows = stmt
            .query_map([], |r| {
                (0..columns)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap();
        result.push(format!(
            "{table}: {:?}",
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        ));
    }
    result
}
fn domain_error<T: std::fmt::Debug>(result: daymark_persistence::Result<T>, expected: Error) {
    assert!(
        matches!(result, Err(StoreError::Domain(ref error)) if *error == expected),
        "{result:?}"
    );
}

#[test]
fn empty_migration_reopen_and_schema_versions() {
    let (dir, mut store) = database();
    assert_eq!(store.snapshot(A, y(2026)).unwrap().balances, [h(0); 4]);
    let conn = raw(&dir);
    assert_eq!(
        conn.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    assert_eq!(
        conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    let before = dump(&conn);
    drop(store);
    let store = Store::open(dir.path().join("ledger.sqlite")).unwrap();
    assert_eq!(dump(&conn), before);
    drop(store);
    conn.execute("INSERT INTO schema_migrations VALUES (2,'future')", [])
        .unwrap();
    assert!(matches!(
        Store::open(dir.path().join("ledger.sqlite")),
        Err(StoreError::IncompatibleSchema)
    ));
    conn.execute("DELETE FROM schema_migrations WHERE version=2", [])
        .unwrap();
    conn.execute("UPDATE schema_migrations SET sql='modified'", [])
        .unwrap();
    assert!(matches!(
        Store::open(dir.path().join("ledger.sqlite")),
        Err(StoreError::IncompatibleSchema)
    ));
}

#[test]
fn failed_migration_does_not_leave_partial_schema() {
    let dir = tempfile::tempdir().unwrap();
    let conn = raw(&dir);
    conn.execute_batch("CREATE TABLE effects(dummy TEXT);")
        .unwrap();
    assert!(Store::open(dir.path().join("ledger.sqlite")).is_err());
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(tables, ["effects"]);
}

#[test]
fn durable_shared_conversion_audit_and_last_support_deletion() {
    let (dir, mut store) = database();
    store.configure(ADMIN, T, None, calendar(2026)).unwrap();
    let first = store.create(A, T, EventId(1), worked(2026, 9)).unwrap();
    store.create(A, T, EventId(2), worked(2026, 3)).unwrap();
    store
        .create(A, T, EventId(3), adjustment(2026, Bucket::Floater, -8))
        .unwrap();
    store
        .create(A, T, EventId(4), adjustment(2026, Bucket::Comp, -3))
        .unwrap();
    let deleted = store
        .delete(A, y(2026), AuditTime(T.0 + 1), EventId(1), 1)
        .unwrap();
    assert_eq!(deleted.after_effects[0].balance(Bucket::Floater), h(0));
    assert!(deleted.after_effects[0].effects.iter().any(|e| e.origin
        == Origin::WorkedHoliday {
            holiday: HolidayId(1),
            supports: vec![SourceRef {
                id: EventId(2),
                revision: 1
            }]
        }));
    let conn = raw(&dir);
    let before = dump(&conn);
    assert!(store.delete(A, y(2026), T, EventId(2), 1).is_err());
    assert_eq!(dump(&conn), before);
    drop(store);
    let mut store = Store::open(dir.path().join("ledger.sqlite")).unwrap();
    assert_eq!(store.history(A, EventId(1)).unwrap(), [first, deleted]);
    assert_eq!(
        store.snapshot(A, y(2026)).unwrap().balance(Bucket::Holiday),
        h(72)
    );
    store.delete(A, y(2026), T, EventId(3), 1).unwrap();
    // Spent Comp independently prevents deleting its remaining source.
    assert!(store.delete(A, y(2026), T, EventId(2), 1).is_err());
    store.delete(A, y(2026), T, EventId(4), 1).unwrap();
    store.delete(A, y(2026), T, EventId(2), 1).unwrap();
    let snapshot = store.snapshot(A, y(2026)).unwrap();
    assert_eq!(snapshot.balances, [h(0), h(0), h(80), h(0)]);
    domain_error(
        store.create(A, T, EventId(1), worked(2026, 0)),
        Error::AlreadyExists,
    );
}

#[test]
fn revisions_year_moves_and_colliding_owner_ids_survive_reopen() {
    let (dir, mut store) = database();
    for owner in [A, B] {
        store
            .create(
                owner,
                T,
                EventId(u64::MAX),
                adjustment(2026, Bucket::Pto, 16),
            )
            .unwrap();
    }
    store
        .create(A, T, EventId(2), adjustment(2026, Bucket::Pto, -8))
        .unwrap();
    let conn = raw(&dir);
    let before = dump(&conn);
    assert!(
        store
            .edit(
                A,
                y(2026),
                T,
                EventId(u64::MAX),
                1,
                adjustment(2027, Bucket::Pto, 16)
            )
            .is_err()
    );
    assert!(
        store
            .edit(
                A,
                y(2026),
                T,
                EventId(2),
                1,
                adjustment(2027, Bucket::Pto, -8)
            )
            .is_err()
    );
    assert_eq!(dump(&conn), before);
    store.delete(A, y(2026), T, EventId(2), 1).unwrap();
    let changed = store
        .edit(
            A,
            y(2026),
            T,
            EventId(u64::MAX),
            1,
            adjustment(2027, Bucket::Pto, 16),
        )
        .unwrap();
    assert_eq!(changed.after_effects.len(), 2);
    domain_error(store.record(A, y(2026), EventId(u64::MAX)), Error::NotFound);
    domain_error(
        store.delete(A, y(2026), T, EventId(u64::MAX), 2),
        Error::NotFound,
    );
    domain_error(
        store.edit(
            A,
            y(2027),
            T,
            EventId(u64::MAX),
            1,
            adjustment(2027, Bucket::Pto, 1),
        ),
        Error::StaleRevision,
    );
    domain_error(
        store.delete(A, y(2027), T, EventId(u64::MAX), 1),
        Error::StaleRevision,
    );
    assert!(store.history(ADMIN, EventId(u64::MAX)).unwrap().is_empty());
    drop(store);
    let mut store = Store::open(dir.path().join("ledger.sqlite")).unwrap();
    assert_eq!(
        store.snapshot(A, y(2026)).unwrap().balance(Bucket::Pto),
        h(0)
    );
    assert_eq!(
        store.snapshot(A, y(2027)).unwrap().balance(Bucket::Pto),
        h(16)
    );
    assert_eq!(
        store.snapshot(B, y(2026)).unwrap().balance(Bucket::Pto),
        h(16)
    );
    assert_eq!(store.history(A, EventId(u64::MAX)).unwrap()[1], changed);
}

#[test]
fn guessed_ids_search_and_effect_foreign_keys_are_owner_scoped() {
    let (dir, mut store) = database();
    let mut private = adjustment(2026, Bucket::Pto, 8);
    private.notes = "unique private phrase".into();
    store.create(B, T, EventId(7), private).unwrap();
    domain_error(store.record(A, y(2026), EventId(7)), Error::NotFound);
    domain_error(
        store.edit(
            A,
            y(2026),
            T,
            EventId(7),
            1,
            adjustment(2026, Bucket::Pto, 8),
        ),
        Error::NotFound,
    );
    domain_error(store.delete(A, y(2026), T, EventId(7), 1), Error::NotFound);
    assert!(store.history(A, EventId(7)).unwrap().is_empty());
    let filter = Filter {
        notes: Some("PRIVATE".into()),
        ..Filter::default()
    };
    assert!(store.query(A, y(2026), &filter).unwrap().is_empty());
    assert_eq!(store.query(B, y(2026), &filter).unwrap().len(), 1);
    assert!(store.query(B, y(2027), &filter).unwrap().is_empty());
    let conn = raw(&dir);
    assert!(
        conn.execute("UPDATE effect_supports SET owner=?1", [A.0.to_be_bytes()])
            .is_err()
    );
    assert!(
        conn.execute("UPDATE effect_supports SET year=2027", [])
            .is_err()
    );
    assert!(
        conn.execute(
            "UPDATE effect_supports SET revision=?1",
            [2u64.to_be_bytes()]
        )
        .is_err()
    );
}

#[test]
fn holiday_corrections_check_all_users_and_preserve_historical_configuration() {
    let (dir, mut store) = database();
    store.configure(ADMIN, T, None, calendar(2026)).unwrap();
    for owner in [A, B] {
        store.create(owner, T, EventId(1), worked(2026, 0)).unwrap();
    }
    let mut named = calendar(2026);
    named.holidays[0].name = "Corrected name".into();
    assert_eq!(
        store.configure(ADMIN, T, Some(1), named.clone()).unwrap(),
        2
    );
    let conn = raw(&dir);
    let before = dump(&conn);
    assert!(matches!(
        store.configure(ADMIN, T, Some(1), calendar(2026)),
        Err(StoreError::StaleCalendar)
    ));
    let mut invalid = named.clone();
    invalid.holidays.pop();
    domain_error(
        store.configure(ADMIN, T, Some(2), invalid),
        Error::InvalidCalendar,
    );
    let mut moved = named.clone();
    moved.holidays[0].date = Date::new(2026, 1, 2).unwrap();
    domain_error(
        store.configure(ADMIN, T, Some(2), moved.clone()),
        Error::ReferencedHoliday,
    );
    let mut replaced = named;
    replaced.holidays[0].id = HolidayId(50);
    domain_error(
        store.configure(ADMIN, T, Some(2), replaced),
        Error::ReferencedHoliday,
    );
    assert_eq!(dump(&conn), before);
    store.delete(A, y(2026), T, EventId(1), 1).unwrap();
    domain_error(
        store.configure(ADMIN, T, Some(2), moved.clone()),
        Error::ReferencedHoliday,
    );
    store.delete(B, y(2026), T, EventId(1), 1).unwrap();
    store.configure(ADMIN, T, Some(2), moved.clone()).unwrap();
    drop(store);
    let store = Store::open(dir.path().join("ledger.sqlite")).unwrap();
    assert_eq!(store.calendar(y(2026)).unwrap().unwrap().calendar, moved);
    let history = store.calendar_history(y(2026)).unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(
        history[1].before.as_ref().unwrap().holidays[0].name,
        "Holiday 1"
    );
    assert_eq!(
        history[2].before.as_ref().unwrap().holidays[0].date,
        Date::new(2026, 1, 1).unwrap()
    );
    assert_eq!(
        store.history(B, EventId(1)).unwrap()[0]
            .after
            .as_ref()
            .unwrap()
            .source,
        worked(2026, 0)
    );
}

#[test]
fn storage_failure_after_source_and_effect_writes_rolls_back_create_edit_delete() {
    let (dir, mut store) = database();
    store.configure(ADMIN, T, None, calendar(2026)).unwrap();
    store.create(A, T, EventId(1), worked(2026, 9)).unwrap();
    let conn = raw(&dir);
    conn.execute_batch("CREATE TRIGGER fail_audit BEFORE INSERT ON source_history BEGIN SELECT RAISE(ABORT,'injected audit failure'); END;").unwrap();
    let before = dump(&conn);
    for operation in 0..3 {
        let result = match operation {
            0 => store.create(A, T, EventId(2), worked(2026, 3)),
            1 => store.edit(A, y(2026), T, EventId(1), 1, worked(2026, 3)),
            _ => store.delete(A, y(2026), T, EventId(1), 1),
        };
        assert!(matches!(result, Err(StoreError::Storage(_))));
        assert_eq!(dump(&conn), before);
    }
    conn.execute_batch("DROP TRIGGER fail_audit;").unwrap();
    store.create(A, T, EventId(2), worked(2026, 3)).unwrap();
    assert_eq!(
        store.snapshot(A, y(2026)).unwrap().balance(Bucket::Floater),
        h(8)
    );
}

#[test]
fn deferred_commit_failure_rolls_back_and_connection_remains_usable() {
    let (dir, mut store) = database();
    let conn = raw(&dir);
    conn.execute_batch("CREATE TRIGGER fail_commit AFTER INSERT ON source_history BEGIN UPDATE sources SET holiday_id=x'00000000000000FF' WHERE owner=NEW.owner AND id=NEW.source_id; END;").unwrap();
    let before = dump(&conn);
    assert!(matches!(
        store.create(A, T, EventId(1), adjustment(2026, Bucket::Pto, 8)),
        Err(StoreError::Storage(_))
    ));
    assert_eq!(dump(&conn), before);
    conn.execute_batch("DROP TRIGGER fail_commit;").unwrap();
    store
        .create(A, T, EventId(1), adjustment(2026, Bucket::Pto, 8))
        .unwrap();
}

#[test]
fn calendar_storage_failure_is_atomic_and_audits_are_immutable() {
    let (dir, mut store) = database();
    store.configure(ADMIN, T, None, calendar(2026)).unwrap();
    store.create(A, T, EventId(1), worked(2026, 9)).unwrap();
    let conn = raw(&dir);
    conn.execute_batch("CREATE TRIGGER fail_calendar BEFORE INSERT ON calendar_history BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    let before = dump(&conn);
    let mut updated = calendar(2026);
    updated.holidays[0].name = "New name".into();
    assert!(store.configure(ADMIN, T, Some(1), updated).is_err());
    assert_eq!(dump(&conn), before);
    for table in ["source_history", "calendar_history"] {
        assert!(
            conn.execute(&format!("UPDATE {table} SET at=0"), [])
                .is_err()
        );
        assert!(conn.execute(&format!("DELETE FROM {table}"), []).is_err());
    }
    assert_eq!(dump(&conn), before);
}

#[test]
fn two_writers_cannot_spend_the_same_remaining_balance() {
    for bucket in [Bucket::Pto, Bucket::Comp, Bucket::Floater] {
        let (dir, mut store) = database();
        store
            .create(A, T, EventId(1), adjustment(2026, bucket, 8))
            .unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let writers: Vec<_> = (2..=3)
            .map(|id| {
                let mut connection = Store::open(dir.path().join("ledger.sqlite")).unwrap();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    connection.create(A, T, EventId(id), adjustment(2026, bucket, -8))
                })
            })
            .collect();
        let results: Vec<_> = writers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(StoreError::Domain(Error::NegativeBalance { .. }))))
                .count(),
            1
        );
        assert_eq!(store.snapshot(A, y(2026)).unwrap().balance(bucket), h(0));
        assert_eq!(
            store.history(A, EventId(2)).unwrap().len()
                + store.history(A, EventId(3)).unwrap().len(),
            1
        );
    }
}

#[test]
fn concurrent_stale_edits_and_calendar_submissions_have_one_winner() {
    let (dir, mut store) = database();
    store
        .create(A, T, EventId(1), adjustment(2026, Bucket::Pto, 8))
        .unwrap();
    for calendar_update in [false, true] {
        let barrier = Arc::new(Barrier::new(2));
        let writers: Vec<_> = (0..2)
            .map(|index| {
                let mut connection = Store::open(dir.path().join("ledger.sqlite")).unwrap();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    if calendar_update {
                        let mut c = calendar(2026);
                        c.holidays[0].name = format!("Submission {index}");
                        connection.configure(ADMIN, T, None, c).map(|_| ())
                    } else if index == 0 {
                        connection
                            .edit(
                                A,
                                y(2026),
                                T,
                                EventId(1),
                                1,
                                adjustment(2026, Bucket::Pto, 16),
                            )
                            .map(|_| ())
                    } else {
                        connection
                            .edit(
                                A,
                                y(2026),
                                T,
                                EventId(1),
                                1,
                                adjustment(2026, Bucket::Pto, 24),
                            )
                            .map(|_| ())
                    }
                })
            })
            .collect();
        let results: Vec<_> = writers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(results.iter().any(|r| if calendar_update {
            matches!(r, Err(StoreError::StaleCalendar))
        } else {
            matches!(r, Err(StoreError::Domain(Error::StaleRevision)))
        }));
    }
    assert_eq!(store.calendar_history(y(2026)).unwrap().len(), 1);
    assert_eq!(store.history(A, EventId(1)).unwrap().len(), 2);
}

#[test]
fn multi_day_and_extreme_net_state_reload_without_replay_order() {
    let (dir, mut store) = database();
    for (id, delta) in [(3, i64::MAX), (1, -1), (2, 1)] {
        store
            .create(A, T, EventId(id), adjustment(2026, Bucket::Pto, delta))
            .unwrap();
    }
    drop(store);
    let mut store = Store::open(dir.path().join("ledger.sqlite")).unwrap();
    assert_eq!(
        store.snapshot(A, y(2026)).unwrap().balance(Bucket::Pto),
        h(i64::MAX)
    );
    let conn = raw(&dir);
    let before = dump(&conn);
    domain_error(store.delete(A, y(2026), T, EventId(1), 1), Error::Overflow);
    assert_eq!(dump(&conn), before);
    let mut vacation = source(
        2026,
        Activity::Use {
            bucket: Bucket::Pto,
            hours: h(40),
        },
    );
    vacation.dates = vec![
        Date::new(2026, 3, 3).unwrap(),
        Date::new(2026, 3, 1).unwrap(),
    ];
    store.create(A, T, EventId(4), vacation).unwrap();
    assert_eq!(
        store.snapshot(A, y(2026)).unwrap().balance(Bucket::Pto),
        h(i64::MAX - 40)
    );
    let filter = Filter {
        classification: Some(Classification::Use),
        from: Some(Date::new(2026, 3, 2).unwrap()),
        through: Some(Date::new(2026, 3, 2).unwrap()),
        ..Filter::default()
    };
    assert!(store.query(A, y(2026), &filter).unwrap().is_empty());
    store.delete(A, y(2026), T, EventId(4), 1).unwrap();
    assert_eq!(
        store.snapshot(A, y(2026)).unwrap().balance(Bucket::Pto),
        h(i64::MAX)
    );
}

#[test]
fn invalid_encoded_primitives_are_rejected_and_effect_drift_is_detected() {
    assert!(serde_json::from_str::<Hours>("-1").is_err());
    assert!(serde_json::from_str::<Year>("0").is_err());
    assert!(serde_json::from_str::<Date>("[2026,2,29]").is_err());
    let (dir, mut store) = database();
    store
        .create(A, T, EventId(1), adjustment(2026, Bucket::Pto, 8))
        .unwrap();
    let conn = raw(&dir);
    conn.execute(
        "UPDATE effects SET payload=json_set(payload,'$.delta',7)",
        [],
    )
    .unwrap();
    assert!(matches!(
        store.snapshot(A, y(2026)),
        Err(StoreError::CorruptState)
    ));
}

#[test]
fn concurrent_shared_support_creation_and_deletion_preserve_one_conversion() {
    let (dir, mut store) = database();
    store.configure(ADMIN, T, None, calendar(2026)).unwrap();
    for deleting in [false, true] {
        let barrier = Arc::new(Barrier::new(2));
        let writers: Vec<_> = (1..=2)
            .map(|id| {
                let mut writer = Store::open(dir.path().join("ledger.sqlite")).unwrap();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    if deleting {
                        writer.delete(A, y(2026), T, EventId(id), 1)
                    } else {
                        writer.create(A, T, EventId(id), worked(2026, 0))
                    }
                })
            })
            .collect();
        let results: Vec<_> = writers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            if deleting { 1 } else { 2 }
        );
        if deleting {
            assert!(results.iter().any(|r| matches!(
                r,
                Err(StoreError::Domain(Error::NegativeBalance {
                    bucket: Bucket::Floater,
                    ..
                }))
            )));
        }
        let snapshot = store.snapshot(A, y(2026)).unwrap();
        assert_eq!(snapshot.balance(Bucket::Holiday), h(72));
        assert_eq!(
            snapshot
                .effects
                .iter()
                .filter(|e| matches!(e.origin, Origin::WorkedHoliday { .. }))
                .count(),
            2
        );
        if !deleting {
            store
                .create(A, T, EventId(3), adjustment(2026, Bucket::Floater, -8))
                .unwrap();
        }
    }
}

#[test]
fn existing_calendar_concurrent_corrections_reject_stale_submission() {
    let (dir, mut store) = database();
    store.configure(ADMIN, T, None, calendar(2026)).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let writers: Vec<_> = (0..2)
        .map(|index| {
            let mut writer = Store::open(dir.path().join("ledger.sqlite")).unwrap();
            let mut submission = writer.calendar(y(2026)).unwrap().unwrap();
            submission.calendar.holidays[index].name = format!("Correction {index}");
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                writer.configure(ADMIN, T, Some(submission.revision), submission.calendar)
            })
        })
        .collect();
    let results: Vec<_> = writers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(StoreError::StaleCalendar)))
            .count(),
        1
    );
    assert_eq!(store.calendar_history(y(2026)).unwrap().len(), 2);
}

#[test]
fn shared_support_year_move_and_replacement_keep_durable_links() {
    let (dir, mut store) = database();
    for year in [2026, 2027] {
        store.configure(ADMIN, T, None, calendar(year)).unwrap();
    }
    for id in [1, 2] {
        store.create(A, T, EventId(id), worked(2026, 3)).unwrap();
    }
    let moved = store
        .edit(A, y(2026), T, EventId(1), 1, worked(2027, 3))
        .unwrap();
    drop(store);
    let mut store = Store::open(dir.path().join("ledger.sqlite")).unwrap();
    for (year, id, revision) in [(2026, 2, 1), (2027, 1, 2)] {
        let snapshot = store.snapshot(A, y(year)).unwrap();
        assert_eq!(snapshot.balance(Bucket::Floater), h(8));
        assert!(snapshot.effects.iter().any(|e| e.origin
            == Origin::WorkedHoliday {
                holiday: HolidayId(1),
                supports: vec![SourceRef {
                    id: EventId(id),
                    revision
                }]
            }));
    }
    assert_eq!(store.history(A, EventId(1)).unwrap()[1], moved);
    store
        .edit(
            A,
            y(2027),
            T,
            EventId(1),
            2,
            source(
                2027,
                Activity::HolidayUse {
                    holiday: HolidayId(1),
                },
            ),
        )
        .unwrap();
    assert_eq!(
        store.snapshot(A, y(2027)).unwrap().balance(Bucket::Floater),
        h(0)
    );
    assert_eq!(
        store.snapshot(A, y(2026)).unwrap().balance(Bucket::Floater),
        h(8)
    );
    let conn = raw(&dir);
    assert!(
        !conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
}

#[test]
fn bounded_state_machine_survives_reopen_after_every_attempt() {
    // Independent Boolean oracle, 64 sequences / 192 transitions. Operations
    // toggle either work support, Holiday Use, or a fully spent Floater.
    for sequence in 0..64 {
        let (dir, mut store) = database();
        store.configure(ADMIN, T, None, calendar(2026)).unwrap();
        store.create(B, T, EventId(1), worked(2026, 3)).unwrap();
        let other = store.snapshot(B, y(2026)).unwrap();
        let mut active = [false; 4];
        let mut ids = [EventId(0); 4];
        let mut code = sequence;
        for next in 1..=3 {
            let op = code % 4;
            code /= 4;
            let count = usize::from(active[0]) + usize::from(active[1]);
            let expected = match (op, active[op]) {
                (0 | 1, false) => !active[2],
                (0 | 1, true) => count > 1 || !active[3],
                (2, false) => count == 0,
                (3, false) => count > 0,
                _ => true,
            };
            let conn = raw(&dir);
            let before = dump(&conn);
            let result = if active[op] {
                store.delete(A, y(2026), T, ids[op], 1)
            } else {
                let input = match op {
                    0 | 1 => worked(2026, 0),
                    2 => source(
                        2026,
                        Activity::HolidayUse {
                            holiday: HolidayId(1),
                        },
                    ),
                    _ => adjustment(2026, Bucket::Floater, -8),
                };
                ids[op] = EventId(next);
                store.create(A, T, ids[op], input)
            };
            assert_eq!(result.is_ok(), expected, "sequence {sequence} step {next}");
            if expected {
                active[op] = !active[op];
            } else {
                assert_eq!(dump(&conn), before);
            }
            drop(store);
            store = Store::open(dir.path().join("ledger.sqlite")).unwrap();
            let snapshot = store.snapshot(A, y(2026)).unwrap();
            let converted = active[0] || active[1];
            assert_eq!(
                snapshot.balance(Bucket::Holiday),
                h(if converted || active[2] { 72 } else { 80 })
            );
            assert_eq!(
                snapshot.balance(Bucket::Floater),
                h(8 * i64::from(converted) - 8 * i64::from(active[3]))
            );
            assert_eq!(store.snapshot(B, y(2026)).unwrap(), other);
        }
    }
}

#[test]
fn migration_identity_is_independent_of_checkout_line_endings() {
    let (dir, store) = database();
    drop(store);
    let conn = raw(&dir);
    let sql: String = conn
        .query_row("SELECT sql FROM schema_migrations", [], |r| r.get(0))
        .unwrap();
    conn.execute(
        "UPDATE schema_migrations SET sql=?1",
        [sql.replace('\n', "\r\n")],
    )
    .unwrap();
    Store::open(dir.path().join("ledger.sqlite")).unwrap();
}

#[test]
fn calendar_name_storage_preserves_the_domain_string_contract() {
    let (dir, mut store) = database();
    let mut c = calendar(2026);
    c.holidays[0].name = "\0Retained name".into();
    // The accepted domain requires a nonempty trimmed Rust string, not SQLite's
    // NUL-terminated text length. Persistence must not add a second name policy.
    let mut domain = Ledger::default();
    domain.configure(ADMIN, T, c.clone()).unwrap();
    store.configure(ADMIN, T, None, c.clone()).unwrap();
    drop(store);
    let store = Store::open(dir.path().join("ledger.sqlite")).unwrap();
    assert_eq!(store.calendar(y(2026)).unwrap().unwrap().calendar, c);
}
