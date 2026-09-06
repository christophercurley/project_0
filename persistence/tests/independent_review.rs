use daymark_domain::*;
use daymark_persistence::{Store, StoreError};
use std::sync::{Arc, Barrier};
use std::thread;

const OWNER: UserId = UserId(0);
const OTHER: UserId = UserId(u64::MAX);
const TIME: AuditTime = AuditTime(-1);

fn year() -> Year {
    Year::new(2026).unwrap()
}

fn calendar() -> Calendar {
    Calendar {
        year: year(),
        holidays: (1..=10)
            .map(|month| Holiday {
                id: HolidayId(u64::from(month)),
                date: Date::new(2026, month, 1).unwrap(),
                name: format!("Review holiday {month}"),
            })
            .collect(),
    }
}

fn source(case: usize) -> Source {
    let work = Work {
        hours_worked: Some(Hours::new(1).unwrap()),
        multiplier: Some(Multiplier::OneAndAHalf),
        credited_comp: Hours::new(3).unwrap(),
    };
    let activity = match case {
        0 => Activity::CompWork(work),
        1 | 2 => Activity::HolidayWork {
            holiday: HolidayId(case as u64),
            work,
        },
        3 | 4 => Activity::HolidayUse {
            holiday: HolidayId((case - 2) as u64),
        },
        5 => Activity::Adjustment {
            bucket: Bucket::Comp,
            delta: -3,
        },
        6 => Activity::Adjustment {
            bucket: Bucket::Floater,
            delta: -8,
        },
        _ => unreachable!(),
    };
    Source {
        dates: vec![Date::new(2026, if matches!(case, 2 | 4) { 2 } else { 1 }, 1).unwrap()],
        activity,
        notes: format!("Review case {case}: apostrophe ' and Unicode 雪;\nsecond line."),
    }
}

#[test]
fn replacement_matrix_preserves_domain_results_and_complete_audit_on_reopen() {
    // Differential proof of the adapter contract, not an independent domain
    // oracle: every replacement/deletion of either of two shared supports.
    for spent in [false, true] {
        for target in [0, 1] {
            for replacement in 0..8 {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("review.sqlite");
                let mut store = Store::open(&path).unwrap();
                let mut domain = Ledger::default();
                store.configure(OTHER, TIME, None, calendar()).unwrap();
                domain.configure(OTHER, TIME, calendar()).unwrap();
                for (owner, id, case) in [(OWNER, 0, 1), (OWNER, 1, 1), (OTHER, 0, 2)] {
                    store
                        .create(owner, TIME, EventId(id), source(case))
                        .unwrap();
                    domain
                        .create(owner, TIME, EventId(id), source(case))
                        .unwrap();
                }
                if spent {
                    for (id, case) in [(2, 5), (3, 6)] {
                        store
                            .create(OWNER, TIME, EventId(id), source(case))
                            .unwrap();
                        domain
                            .create(OWNER, TIME, EventId(id), source(case))
                            .unwrap();
                    }
                }
                let at = AuditTime(i64::MAX);
                let (actual, expected) = if replacement == 7 {
                    (
                        store.delete(OWNER, year(), at, EventId(target), 1),
                        domain.delete(OWNER, at, EventId(target), 1),
                    )
                } else {
                    (
                        store.edit(OWNER, year(), at, EventId(target), 1, source(replacement)),
                        domain.edit(OWNER, at, EventId(target), 1, source(replacement)),
                    )
                };
                match (actual, expected) {
                    (Ok(change), Ok(())) => {
                        assert_eq!(&change, domain.history(OWNER).last().unwrap())
                    }
                    (Err(StoreError::Domain(a)), Err(b)) => assert_eq!(a, b),
                    other => panic!("adapter disagreement: {other:?}"),
                }
                drop(store);
                let mut store = Store::open(&path).unwrap();
                for owner in [OWNER, OTHER] {
                    assert_eq!(
                        store.snapshot(owner, year()).unwrap(),
                        domain.snapshot(owner, year()).unwrap()
                    );
                    for id in 0..4 {
                        let expected: Vec<_> = domain
                            .history(owner)
                            .iter()
                            .filter(|c| c.id == EventId(id))
                            .cloned()
                            .collect();
                        assert_eq!(store.history(owner, EventId(id)).unwrap(), expected);
                    }
                    for case in 0..7 {
                        let filter = Filter {
                            notes: Some(format!("case {case}")),
                            ..Filter::default()
                        };
                        assert_eq!(
                            store.query(owner, year(), &filter).unwrap(),
                            domain.query(owner, year(), &filter).unwrap()
                        );
                    }
                }
                let raw = rusqlite::Connection::open(&path).unwrap();
                assert!(
                    !raw.prepare("PRAGMA foreign_key_check")
                        .unwrap()
                        .exists([])
                        .unwrap()
                );
            }
        }
    }
}

#[test]
fn concurrent_first_reference_and_calendar_date_correction_cannot_both_commit() {
    for case in [1, 3] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("review.sqlite");
        let mut store = Store::open(&path).unwrap();
        store.configure(OTHER, TIME, None, calendar()).unwrap();
        let mut writer = Store::open(&path).unwrap();
        let mut admin = Store::open(&path).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let start = barrier.clone();
        let reference = thread::spawn(move || {
            start.wait();
            writer.create(OWNER, TIME, EventId(0), source(case))
        });
        let correction = thread::spawn(move || {
            let mut changed = calendar();
            changed.holidays[0].date = Date::new(2026, 1, 2).unwrap();
            barrier.wait();
            admin.configure(OTHER, TIME, Some(1), changed)
        });
        match (reference.join().unwrap(), correction.join().unwrap()) {
            (Ok(_), Err(StoreError::Domain(Error::ReferencedHoliday))) => {
                assert_eq!(store.calendar_history(year()).unwrap().len(), 1);
                assert_eq!(store.history(OWNER, EventId(0)).unwrap().len(), 1);
            }
            (Err(StoreError::Domain(Error::UnknownHoliday)), Ok(2)) => {
                assert_eq!(store.calendar_history(year()).unwrap().len(), 2);
                assert!(store.history(OWNER, EventId(0)).unwrap().is_empty());
            }
            other => panic!("nonserializable outcome: {other:?}"),
        }
        store.snapshot(OWNER, year()).unwrap();
    }
}

#[test]
fn simultaneous_initialization_is_atomic_and_retryable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("review.sqlite");
    let barrier = Arc::new(Barrier::new(4));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                match Store::open(&path) {
                    Ok(_) => {}
                    // WAL activation can legitimately report SQLITE_BUSY before the
                    // migration transaction starts. The complete open is retryable.
                    Err(StoreError::Storage(rusqlite::Error::SqliteFailure(error, _)))
                        if error.code == rusqlite::ErrorCode::DatabaseBusy =>
                    {
                        Store::open(&path).unwrap();
                    }
                    Err(error) => panic!("unexpected initialization failure: {error:?}"),
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store.snapshot(OWNER, year()).unwrap().balances,
        [Hours::default(); 4]
    );
    let raw = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        raw.query_row("SELECT count(*) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        raw.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}
