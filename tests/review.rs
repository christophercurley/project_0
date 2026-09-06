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

// A small independent state table: plain Comp, work on either holiday, or
// Holiday Use. It deliberately does not construct or sum generated effects.
fn case_source(kind: usize) -> Source {
    let (holiday, credit, taken) = match kind {
        0 => (0, 0, false),
        1 => (0, 3, false),
        2 => (1, 0, false),
        3 => (1, 3, false),
        4 => (2, 3, false),
        5 => (1, 0, true),
        6 => (2, 0, true),
        _ => unreachable!(),
    };
    let work = Work {
        hours_worked: None,
        multiplier: None,
        credited_comp: Hours::new(credit).unwrap(),
    };
    let activity = if taken {
        Activity::HolidayUse {
            holiday: HolidayId(holiday),
        }
    } else if holiday != 0 {
        Activity::HolidayWork {
            holiday: HolidayId(holiday),
            work,
        }
    } else {
        Activity::CompWork(work)
    };
    let mut result = source(2026, activity);
    result.dates = vec![Date::new(2026, holiday.max(1) as u8, 1).unwrap()];
    result
}

fn oracle(kinds: [Option<usize>; 2], comp_spent: i64, floater_spent: i64) -> Option<[i64; 4]> {
    let mut work = [false; 2];
    let mut taken = [0; 2];
    let mut comp = 0;
    for kind in kinds.into_iter().flatten() {
        match kind {
            0 => {}
            1 => comp += 3,
            2 => work[0] = true,
            3 => {
                work[0] = true;
                comp += 3;
            }
            4 => {
                work[1] = true;
                comp += 3;
            }
            5 => taken[0] += 1,
            6 => taken[1] += 1,
            _ => unreachable!(),
        }
    }
    if (0..2).any(|i| taken[i] > 1 || (taken[i] != 0 && work[i])) {
        return None;
    }
    let converted = work.into_iter().filter(|w| *w).count() as i64;
    let comp = comp - comp_spent;
    let floater = converted * 8 - floater_spent;
    (comp >= 0 && floater >= 0).then_some([
        0,
        comp,
        80 - 8 * (converted + taken.iter().sum::<i64>()),
        floater,
    ])
}

#[test]
fn exhaustive_source_replacements_match_independent_spent_balance_oracle() {
    for first in 0..7 {
        for second in 0..7 {
            for comp_spent in [0, 3, 6] {
                for floater_spent in [0, 8, 16] {
                    let kinds = [Some(first), Some(second)];
                    if oracle(kinds, comp_spent, floater_spent).is_none() {
                        continue;
                    }
                    let mut base = Ledger::default();
                    base.configure(UserId(99), TIME, calendar(2026)).unwrap();
                    for (index, kind) in [first, second].into_iter().enumerate() {
                        base.create(USER, TIME, EventId(index as u64), case_source(kind))
                            .unwrap();
                    }
                    for (id, bucket, amount) in [
                        (10, Bucket::Comp, comp_spent),
                        (11, Bucket::Floater, floater_spent),
                    ] {
                        if amount != 0 {
                            base.create(
                                USER,
                                TIME,
                                EventId(id),
                                source(
                                    2026,
                                    Activity::Use {
                                        bucket,
                                        hours: Hours::new(amount).unwrap(),
                                    },
                                ),
                            )
                            .unwrap();
                        }
                    }
                    base.create(UserId(2), TIME, EventId(0), worked(2026))
                        .unwrap();
                    for index in 0..2 {
                        for replacement in (0..7).map(Some).chain([None]) {
                            let mut ledger = base.clone();
                            let mut changed = kinds;
                            changed[index] = replacement;
                            let expected = oracle(changed, comp_spent, floater_spent);
                            let id = EventId(index as u64);
                            let result = match replacement {
                                Some(kind) => {
                                    ledger.edit(USER, AuditTime(43), id, 1, case_source(kind))
                                }
                                None => ledger.delete(USER, AuditTime(43), id, 1),
                            };
                            assert_eq!(
                                result.is_ok(),
                                expected.is_some(),
                                "{kinds:?} -> {changed:?}, spent {comp_spent}/{floater_spent}"
                            );
                            let Some(balances) = expected else {
                                assert_eq!(ledger, base);
                                continue;
                            };
                            let snapshot = ledger.snapshot(USER, year(2026)).unwrap();
                            assert_eq!(snapshot.balances.map(Hours::get), balances);
                            assert_eq!(
                                ledger.snapshot(UserId(2), year(2026)),
                                base.snapshot(UserId(2), year(2026))
                            );
                            assert_eq!(ledger.history(UserId(2)), base.history(UserId(2)));
                            let history = ledger.history(USER);
                            assert_eq!(&history[..history.len() - 1], base.history(USER));
                            let audit = history.last().unwrap();
                            assert_eq!(audit.before.as_ref(), Some(base.record(USER, id).unwrap()));
                            assert_eq!(audit.after.as_ref(), ledger.record(USER, id).ok());
                            assert_eq!(
                                audit.before_effects,
                                vec![base.snapshot(USER, year(2026)).unwrap()]
                            );
                            assert_eq!(audit.after_effects, vec![snapshot.clone()]);
                            assert_eq!(audit.at, AuditTime(43));
                            for holiday in [1, 2] {
                                let supports: Vec<_> = changed
                                    .iter()
                                    .enumerate()
                                    .filter_map(|(i, kind)| {
                                        let supports = match holiday {
                                            1 => matches!(kind, Some(2 | 3)),
                                            _ => *kind == Some(4),
                                        };
                                        supports.then_some(SourceRef {
                                            id: EventId(i as u64),
                                            revision: if i == index { 2 } else { 1 },
                                        })
                                    })
                                    .collect();
                                let effects: Vec<_> = snapshot.effects.iter().filter(|effect| matches!(effect.origin, Origin::WorkedHoliday { holiday: h, .. } if h == HolidayId(holiday))).collect();
                                assert_eq!(effects.len(), if supports.is_empty() { 0 } else { 2 });
                                for effect in effects {
                                    assert_eq!(
                                        effect.origin,
                                        Origin::WorkedHoliday {
                                            holiday: HolidayId(holiday),
                                            supports: supports.clone()
                                        }
                                    );
                                }
                            }
                            let accepted = ledger.clone();
                            let stale_error = if replacement.is_some() {
                                Error::StaleRevision
                            } else {
                                Error::NotFound
                            };
                            assert_eq!(
                                ledger.edit(USER, TIME, id, 1, case_source(0)),
                                Err(stale_error.clone())
                            );
                            assert_eq!(ledger.delete(USER, TIME, id, 1), Err(stale_error));
                            assert_eq!(ledger, accepted);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn boundary_adjustment_edits_use_wide_net_arithmetic_for_every_manual_bucket() {
    for bucket in [Bucket::Pto, Bucket::Comp, Bucket::Floater] {
        for delta in [i64::MIN, -i64::MAX, -1, 1, i64::MAX] {
            let mut ledger = Ledger::default();
            let make = |delta| source(2026, Activity::Adjustment { bucket, delta });
            ledger
                .create(USER, TIME, EventId(3), make(i64::MAX))
                .unwrap();
            ledger.create(USER, TIME, EventId(2), make(-1)).unwrap();
            ledger.create(USER, TIME, EventId(1), make(1)).unwrap();
            let before = ledger.clone();
            let result = ledger.edit(USER, TIME, EventId(2), 1, make(delta));
            let net = i128::from(i64::MAX) + 1 + i128::from(delta);
            if (0..=i128::from(i64::MAX)).contains(&net) {
                result.unwrap();
                assert_eq!(
                    ledger
                        .snapshot(USER, year(2026))
                        .unwrap()
                        .balance(bucket)
                        .get(),
                    net as i64
                );
            } else {
                assert_eq!(result, Err(Error::Overflow));
                assert_eq!(ledger, before);
            }
        }
    }
}

#[test]
fn effective_projection_is_independent_of_source_insertion_order() {
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut expected = None;
    for order in orders {
        let mut ledger = Ledger::default();
        ledger.configure(UserId(99), TIME, calendar(2026)).unwrap();
        for id in order {
            ledger
                .create(USER, TIME, EventId(id), case_source([3, 3, 4][id as usize]))
                .unwrap();
        }
        let snapshot = ledger.snapshot(USER, year(2026)).unwrap();
        if let Some(expected) = &expected {
            assert_eq!(&snapshot, expected);
        } else {
            expected = Some(snapshot);
        }
    }
}
