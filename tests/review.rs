use daymark_domain::*;

const USER: UserId = UserId(1);
const TIME: AuditTime = AuditTime(42);

fn year(value: u16) -> Year {
    Year::new(value).unwrap()
}

fn source(y: u16, activity: Activity) -> Source {
    Source {
        dates: vec![Date::new(y, 1, 1).unwrap()],
        activity,
        notes: "Independent review fixture".into(),
    }
}

fn adjustment(y: u16, delta: i64) -> Source {
    source(
        y,
        Activity::Adjustment {
            bucket: Bucket::Pto,
            delta,
        },
    )
}

fn worked(y: u16) -> Source {
    source(
        y,
        Activity::HolidayWork {
            holiday: HolidayId(1),
            work: Work {
                hours_worked: Some(Hours::new(1).unwrap()),
                multiplier: None,
                credited_comp: Hours::default(),
            },
        },
    )
}

fn calendar(y: u16) -> Calendar {
    Calendar {
        year: year(y),
        holidays: (1..=10)
            .map(|month| Holiday {
                id: HolidayId(u64::from(month)),
                date: Date::new(y, month, 1).unwrap(),
                name: format!("Configured day {month}"),
            })
            .collect(),
    }
}

#[test]
fn moving_debit_to_unfunded_year_rolls_back_both_years_and_history() {
    let mut ledger = Ledger::default();
    ledger
        .create(USER, TIME, EventId(1), adjustment(2026, 8))
        .unwrap();
    ledger
        .create(USER, TIME, EventId(2), adjustment(2026, -8))
        .unwrap();
    let before = ledger.clone();
    assert_eq!(
        ledger.edit(USER, TIME, EventId(2), 1, adjustment(2027, -8)),
        Err(Error::NegativeBalance {
            year: year(2027),
            bucket: Bucket::Pto
        })
    );
    assert_eq!(ledger, before);
}

#[test]
fn deleting_debit_cannot_overflow_remaining_balance_or_erase_audit() {
    let mut ledger = Ledger::default();
    for (id, delta) in [(1, i64::MAX), (2, -1), (3, 1)] {
        ledger
            .create(USER, TIME, EventId(id), adjustment(2026, delta))
            .unwrap();
    }
    let before = ledger.clone();
    assert_eq!(
        ledger.delete(USER, TIME, EventId(2), 1),
        Err(Error::Overflow)
    );
    assert_eq!(ledger, before);
}

#[test]
fn shared_support_split_across_years_preserves_old_conversion_and_revision_links() {
    let mut ledger = Ledger::default();
    for y in [2026, 2027] {
        ledger.configure(UserId(99), TIME, calendar(y)).unwrap();
    }
    for id in [1, 2] {
        ledger
            .create(USER, TIME, EventId(id), worked(2026))
            .unwrap();
    }
    ledger
        .edit(USER, TIME, EventId(1), 1, worked(2027))
        .unwrap();
    for (y, id, revision) in [(2026, 2, 1), (2027, 1, 2)] {
        let snapshot = ledger.snapshot(USER, year(y)).unwrap();
        assert_eq!(snapshot.balance(Bucket::Floater).get(), 8);
        assert_eq!(snapshot.balance(Bucket::Holiday).get(), 72);
        for effect in snapshot
            .effects
            .iter()
            .filter(|e| e.classification == Classification::Conversion)
        {
            assert_eq!(
                effect.origin,
                Origin::WorkedHoliday {
                    holiday: HolidayId(1),
                    supports: vec![SourceRef {
                        id: EventId(id),
                        revision
                    }]
                }
            );
        }
    }
    let audit = ledger.history(USER).last().unwrap();
    assert_eq!(audit.before_effects.len(), 2);
    assert_eq!(audit.after_effects.len(), 2);
    assert_eq!(audit.before_effects[1].balance(Bucket::Floater).get(), 0);
}

#[test]
fn calendar_date_remains_locked_until_every_users_active_reference_is_resolved() {
    let mut ledger = Ledger::default();
    ledger.configure(UserId(99), TIME, calendar(2026)).unwrap();
    for user in [USER, UserId(2)] {
        ledger.create(user, TIME, EventId(1), worked(2026)).unwrap();
    }
    ledger.delete(USER, TIME, EventId(1), 1).unwrap();
    let mut changed = calendar(2026);
    changed.holidays[0].date = Date::new(2026, 1, 2).unwrap();
    let before = ledger.clone();
    assert_eq!(
        ledger.configure(UserId(99), TIME, changed.clone()),
        Err(Error::ReferencedHoliday)
    );
    assert_eq!(ledger, before);
    ledger.delete(UserId(2), TIME, EventId(1), 1).unwrap();
    ledger.configure(UserId(99), TIME, changed).unwrap();
    for user in [USER, UserId(2)] {
        assert_eq!(
            ledger.history(user)[0].after.as_ref().unwrap().source.dates,
            vec![Date::new(2026, 1, 1).unwrap()]
        );
    }
}
