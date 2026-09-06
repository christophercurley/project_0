use daymark_domain::*;

const A: UserId = UserId(1);
const B: UserId = UserId(2);
const ADMIN: UserId = UserId(99);
const T: AuditTime = AuditTime(1_800_000_000);
fn y(n: u16) -> Year {
    Year::new(n).unwrap()
}
fn d(month: u8, day: u8) -> Date {
    Date::new(2026, month, day).unwrap()
}
fn h(n: i64) -> Hours {
    Hours::new(n).unwrap()
}
fn source(date: Date, activity: Activity) -> Source {
    Source {
        dates: vec![date],
        activity,
        notes: "A detailed explanation. More context is retained. Final sentence.".into(),
    }
}
fn grant(bucket: Bucket, amount: i64) -> Source {
    source(
        d(1, 1),
        Activity::Grant {
            bucket,
            hours: h(amount),
        },
    )
}
fn usage(bucket: Bucket, amount: i64) -> Source {
    source(
        d(1, 2),
        Activity::Use {
            bucket,
            hours: h(amount),
        },
    )
}
fn work(amount: i64) -> Work {
    Work {
        hours_worked: Some(h(6)),
        multiplier: Some(Multiplier::OneAndAHalf),
        credited_comp: h(amount),
    }
}
fn worked(holiday: u64, amount: i64) -> Source {
    source(
        d(holiday as u8, 1),
        Activity::HolidayWork {
            holiday: HolidayId(holiday),
            work: work(amount),
        },
    )
}
fn off(holiday: u64) -> Source {
    source(
        d(holiday as u8, 1),
        Activity::HolidayUse {
            holiday: HolidayId(holiday),
        },
    )
}
fn calendar(year: u16) -> Calendar {
    Calendar {
        year: y(year),
        holidays: (1..=10)
            .map(|n| Holiday {
                id: HolidayId(n),
                date: Date::new(year, n as u8, 1).unwrap(),
                name: format!("Holiday {n}"),
            })
            .collect(),
    }
}
fn configured() -> Ledger {
    let mut l = Ledger::default();
    l.configure(ADMIN, T, calendar(2026)).unwrap();
    l
}
fn add(l: &mut Ledger, user: UserId, id: u64, s: Source) {
    l.create(user, T, EventId(id), s).unwrap();
}
fn balance(l: &Ledger, user: UserId, bucket: Bucket) -> i64 {
    l.snapshot(user, y(2026)).unwrap().balance(bucket).get()
}
fn reject_unchanged(
    l: &mut Ledger,
    action: impl FnOnce(&mut Ledger) -> Result<(), Error>,
) -> Error {
    let before = l.clone();
    let error = action(l).unwrap_err();
    assert_eq!(*l, before);
    error
}

#[test]
fn unit_001_all_duration_inputs_are_whole_hours() {
    for invalid in [
        "1.5",
        "0.5",
        "-1",
        "NaN",
        "1e3",
        "",
        " 1",
        "+1",
        "١",
        "9223372036854775808",
    ] {
        assert!(invalid.parse::<Hours>().is_err(), "{invalid}");
    }
    assert_eq!("6".parse::<Hours>().unwrap(), h(6));
    assert!(Hours::new(-1).is_err());
    let w = Work {
        hours_worked: Some("1".parse().unwrap()),
        multiplier: Some(Multiplier::OneAndAHalf),
        credited_comp: "2".parse().unwrap(),
    };
    assert_eq!(w.hours_worked, Some(h(1)));
}

#[test]
fn unit_002_exact_day_conversion() {
    for (hours, days) in [
        (0, "0"),
        (1, "0.125"),
        (2, "0.25"),
        (3, "0.375"),
        (4, "0.5"),
        (5, "0.625"),
        (6, "0.75"),
        (7, "0.875"),
        (16, "2"),
    ] {
        assert_eq!(h(hours).days(), days);
    }
    assert_eq!(h(i64::MAX).days(), "1152921504606846975.875");
}

#[test]
fn dates_validate_calendar_and_century_leap_rules() {
    for (year, month, day) in [
        (0, 1, 1),
        (10000, 1, 1),
        (2026, 0, 1),
        (2026, 13, 1),
        (2026, 1, 0),
        (2026, 4, 31),
        (2026, 2, 29),
        (1900, 2, 29),
        (2100, 2, 29),
    ] {
        assert_eq!(Date::new(year, month, day), Err(Error::InvalidDate));
    }
    assert_eq!(Date::new(2000, 2, 29).unwrap().to_string(), "2000-02-29");
    assert!(Date::new(2024, 2, 29).is_ok());
    assert_eq!(
        (d(3, 7).year(), d(3, 7).month(), d(3, 7).day()),
        (y(2026), 3, 7)
    );
}

#[test]
fn bal_001_to_004_reject_overspending_each_bucket() {
    for bucket in [Bucket::Pto, Bucket::Comp, Bucket::Floater] {
        let mut l = configured();
        if bucket == Bucket::Pto {
            add(&mut l, A, 1, grant(bucket, 4));
        }
        assert_eq!(
            reject_unchanged(&mut l, |l| l.create(A, T, EventId(2), usage(bucket, 8))),
            Error::NegativeBalance {
                year: y(2026),
                bucket
            }
        );
    }
    let mut l = configured();
    for n in 1..=10 {
        add(&mut l, A, n, off(n));
    }
    assert_eq!(balance(&l, A, Bucket::Holiday), 0);
    reject_unchanged(&mut l, |l| l.create(A, T, EventId(11), off(1)));
}

#[test]
fn pto_001_002_explicit_grant_and_use() {
    let mut l = Ledger::default();
    assert_eq!(balance(&l, A, Bucket::Pto), 0);
    add(&mut l, A, 1, grant(Bucket::Pto, 160));
    add(&mut l, A, 2, usage(Bucket::Pto, 8));
    assert_eq!(balance(&l, A, Bucket::Pto), 152);
}

#[test]
fn year_001_to_004_no_rollover_or_cross_year_mixing() {
    let mut l = configured();
    for (i, bucket) in [Bucket::Pto, Bucket::Comp, Bucket::Floater]
        .into_iter()
        .enumerate()
    {
        add(&mut l, A, i as u64, grant(bucket, 120));
    }
    let old = l.snapshot(A, y(2026)).unwrap();
    assert_eq!(l.snapshot(A, y(2027)).unwrap().balances, [h(0); 4]);
    let mut opening = grant(Bucket::Pto, 100);
    opening.dates = vec![Date::new(2027, 1, 1).unwrap()];
    add(&mut l, A, 4, opening);
    l.configure(ADMIN, T, calendar(2027)).unwrap();
    assert_eq!(l.snapshot(A, y(2026)).unwrap(), old);
    assert_eq!(l.snapshot(A, y(2027)).unwrap().balance(Bucket::Pto), h(100));
    assert_eq!(
        l.snapshot(A, y(2027)).unwrap().balance(Bucket::Holiday),
        h(80)
    );
}

#[test]
fn approved_net_balance_counts_future_credit_and_usage_immediately() {
    let mut l = Ledger::default();
    let mut future = grant(Bucket::Comp, 10);
    future.dates = vec![d(12, 31)];
    add(&mut l, A, 1, future);
    add(&mut l, A, 2, usage(Bucket::Comp, 8));
    assert_eq!(balance(&l, A, Bucket::Comp), 2);
    let mut future_use = usage(Bucket::Comp, 2);
    future_use.dates = vec![d(12, 31)];
    add(&mut l, A, 3, future_use);
    assert_eq!(balance(&l, A, Bucket::Comp), 0);
}

#[test]
fn hol_001_002_003_global_ten_eight_hour_entitlements() {
    let l = configured();
    for user in [A, B, ADMIN] {
        let snapshot = l.snapshot(user, y(2026)).unwrap();
        assert!(snapshot.holiday_configured);
        assert_eq!(snapshot.balance(Bucket::Holiday), h(80));
        assert_eq!(snapshot.effects.len(), 10);
        assert!(
            snapshot
                .effects
                .iter()
                .all(|e| e.delta == 8 && matches!(e.origin, Origin::Entitlement { .. }))
        );
    }
}

#[test]
fn hol_007_invalid_configuration_is_atomic() {
    let mut l = configured();
    let mut invalids = Vec::new();
    let mut c = calendar(2026);
    c.holidays.pop();
    invalids.push(c);
    let mut c = calendar(2026);
    c.holidays.push(c.holidays[0].clone());
    invalids.push(c);
    let mut c = calendar(2026);
    c.holidays[0].name = "  ".into();
    invalids.push(c);
    let mut c = calendar(2026);
    c.holidays[0].date = d(2, 1);
    invalids.push(c);
    let mut c = calendar(2026);
    c.holidays[0].id = HolidayId(2);
    invalids.push(c);
    let mut c = calendar(2026);
    c.holidays[0].date = Date::new(2027, 1, 1).unwrap();
    invalids.push(c);
    for c in invalids {
        assert_eq!(
            reject_unchanged(&mut l, |l| l.configure(ADMIN, T, c)),
            Error::InvalidCalendar
        );
    }
}

#[test]
fn unconfigured_year_has_no_entitlement_and_rejects_holiday_activity() {
    let mut l = Ledger::default();
    assert!(!l.snapshot(A, y(2026)).unwrap().holiday_configured);
    for s in [off(1), worked(1, 9)] {
        assert_eq!(
            reject_unchanged(&mut l, |l| l.create(A, T, EventId(1), s)),
            Error::CalendarNotConfigured
        );
    }
    add(&mut l, A, 1, grant(Bucket::Pto, 8));
    l.configure(ADMIN, T, calendar(2026)).unwrap();
    add(&mut l, A, 2, off(1));
}

#[test]
fn hol_004_005_006_use_is_date_specific_and_once_only() {
    let mut l = configured();
    let mut wrong = off(1);
    wrong.dates = vec![d(1, 2)];
    assert_eq!(
        reject_unchanged(&mut l, |l| l.create(A, T, EventId(1), wrong)),
        Error::UnknownHoliday
    );
    add(&mut l, A, 1, off(1));
    assert_eq!(balance(&l, A, Bucket::Holiday), 72);
    for s in [off(1), worked(1, 9)] {
        assert_eq!(
            reject_unchanged(&mut l, |l| l.create(A, T, EventId(2), s)),
            Error::HolidayConflict
        );
    }
    add(&mut l, B, 1, off(1));
    assert_eq!(balance(&l, B, Bucket::Holiday), 72);
}

#[test]
fn manual_holiday_paths_cannot_bypass_entitlement() {
    let mut l = configured();
    for activity in [
        Activity::Grant {
            bucket: Bucket::Holiday,
            hours: h(8),
        },
        Activity::Adjustment {
            bucket: Bucket::Holiday,
            delta: 8,
        },
        Activity::Adjustment {
            bucket: Bucket::Holiday,
            delta: -8,
        },
        Activity::Use {
            bucket: Bucket::Holiday,
            hours: h(8),
        },
    ] {
        assert_eq!(
            reject_unchanged(&mut l, |l| l.create(
                A,
                T,
                EventId(1),
                source(d(1, 1), activity)
            )),
            Error::ManualHolidayForbidden
        );
    }
}

#[test]
fn flt_001_002_combo_001_partial_work_converts_exactly_eight() {
    for hours in [None, Some(h(1)), Some(h(6)), Some(h(20))] {
        let mut l = configured();
        let mut w = work(9);
        w.hours_worked = hours;
        add(
            &mut l,
            A,
            1,
            source(
                d(1, 1),
                Activity::HolidayWork {
                    holiday: HolidayId(1),
                    work: w,
                },
            ),
        );
        assert_eq!(
            (
                balance(&l, A, Bucket::Holiday),
                balance(&l, A, Bucket::Floater),
                balance(&l, A, Bucket::Comp)
            ),
            (72, 8, 9)
        );
        let conversions = l
            .query(
                A,
                y(2026),
                &Filter {
                    classification: Some(Classification::Conversion),
                    ..Filter::default()
                },
            )
            .unwrap();
        assert_eq!(conversions.len(), 2);
        assert!(conversions.iter().all(|e| matches!(&e.origin, Origin::WorkedHoliday { supports, .. } if supports == &vec![SourceRef { id:EventId(1),revision:1 }])));
    }
}

#[test]
fn flt_003_shared_conversion_survives_deletion_of_first_support() {
    let mut l = configured();
    add(&mut l, A, 1, worked(1, 9));
    add(&mut l, A, 2, worked(1, 3));
    assert_eq!(
        (
            balance(&l, A, Bucket::Floater),
            balance(&l, A, Bucket::Comp)
        ),
        (8, 12)
    );
    let before = l.snapshot(A, y(2026)).unwrap();
    assert!(
        before.effects.iter().any(
            |e| matches!(&e.origin,Origin::WorkedHoliday { supports,.. } if supports.len()==2)
        )
    );
    l.delete(A, T, EventId(1), 1).unwrap();
    assert_eq!(
        (
            balance(&l, A, Bucket::Holiday),
            balance(&l, A, Bucket::Floater),
            balance(&l, A, Bucket::Comp)
        ),
        (72, 8, 3)
    );
    assert!(l.snapshot(A,y(2026)).unwrap().effects.iter().any(|e| matches!(&e.origin,Origin::WorkedHoliday { supports,.. } if supports == &vec![SourceRef { id:EventId(2),revision:1 }])));
    l.delete(A, T, EventId(2), 1).unwrap();
    assert_eq!(
        (
            balance(&l, A, Bucket::Holiday),
            balance(&l, A, Bucket::Floater),
            balance(&l, A, Bucket::Comp)
        ),
        (80, 0, 0)
    );
}

#[test]
fn last_support_deletion_rejects_spent_floater_atomically() {
    let mut l = configured();
    add(&mut l, A, 1, worked(1, 9));
    add(&mut l, A, 2, worked(1, 3));
    add(&mut l, A, 3, usage(Bucket::Floater, 8));
    l.delete(A, T, EventId(1), 1).unwrap();
    assert_eq!(
        reject_unchanged(&mut l, |l| l.delete(A, T, EventId(2), 1)),
        Error::NegativeBalance {
            year: y(2026),
            bucket: Bucket::Floater
        }
    );
    l.delete(A, T, EventId(3), 1).unwrap();
    l.delete(A, T, EventId(2), 1).unwrap();
}

#[test]
fn support_deletion_rejects_spent_independent_comp_even_when_conversion_remains() {
    let mut l = configured();
    add(&mut l, A, 1, worked(1, 9));
    add(&mut l, A, 2, worked(1, 0));
    add(&mut l, A, 3, usage(Bucket::Comp, 9));
    assert_eq!(
        reject_unchanged(&mut l, |l| l.delete(A, T, EventId(1), 1)),
        Error::NegativeBalance {
            year: y(2026),
            bucket: Bucket::Comp
        }
    );
}

#[test]
fn flt_004_two_conversions_and_use() {
    let mut l = configured();
    add(&mut l, A, 1, worked(1, 0));
    add(&mut l, A, 2, worked(2, 0));
    add(&mut l, A, 3, usage(Bucket::Floater, 8));
    assert_eq!(balance(&l, A, Bucket::Floater), 8);
}

#[test]
fn all_ten_holidays_can_convert_without_extra_entitlement() {
    let mut l = configured();
    for n in 1..=10 {
        add(&mut l, A, n, worked(n, 1));
        add(&mut l, A, n + 10, worked(n, 2));
    }
    assert_eq!(
        (
            balance(&l, A, Bucket::Holiday),
            balance(&l, A, Bucket::Floater),
            balance(&l, A, Bucket::Comp)
        ),
        (0, 80, 30)
    );
}

#[test]
fn comp_001_to_005_context_never_calculates_credit_and_notes_survive() {
    for (worked_hours, credited) in [(6, 6), (6, 9), (1, 7)] {
        for multiplier in [
            None,
            Some(Multiplier::OneToOne),
            Some(Multiplier::OneAndAHalf),
        ] {
            let mut l = configured();
            let s = source(
                d(1, 1),
                Activity::CompWork(Work {
                    hours_worked: Some(h(worked_hours)),
                    multiplier,
                    credited_comp: h(credited),
                }),
            );
            add(&mut l, A, 1, s.clone());
            assert_eq!(balance(&l, A, Bucket::Comp), credited);
            assert_eq!(balance(&l, A, Bucket::Floater), 0);
            assert_eq!(l.record(A, EventId(1)).unwrap().source, s);
        }
    }
}

#[test]
fn work_can_have_no_duration_or_context_and_zero_confirmed_comp() {
    let mut l = configured();
    add(
        &mut l,
        A,
        1,
        source(
            d(1, 1),
            Activity::HolidayWork {
                holiday: HolidayId(1),
                work: Work {
                    hours_worked: None,
                    multiplier: None,
                    credited_comp: h(0),
                },
            },
        ),
    );
    assert_eq!(balance(&l, A, Bucket::Floater), 8);
    assert_eq!(balance(&l, A, Bucket::Comp), 0);
}

#[test]
fn combo_002_edit_comp_preserves_conversion_and_prior_support_revision() {
    let mut l = configured();
    add(&mut l, A, 1, worked(1, 9));
    l.edit(A, AuditTime(T.0 + 1), EventId(1), 1, worked(1, 6))
        .unwrap();
    assert_eq!(balance(&l, A, Bucket::Comp), 6);
    assert_eq!(balance(&l, A, Bucket::Floater), 8);
    let audit = &l.history(A)[1];
    assert_eq!(audit.before.as_ref().unwrap().reference.revision, 1);
    assert_eq!(audit.after.as_ref().unwrap().reference.revision, 2);
    assert!(audit.after_effects[0].effects.iter().any(
        |e| matches!(&e.origin,Origin::WorkedHoliday { supports,.. } if supports[0].revision == 2)
    ));
}

#[test]
fn moving_support_between_holidays_reconciles_both_states() {
    let mut l = configured();
    add(&mut l, A, 1, worked(1, 9));
    add(&mut l, A, 2, worked(1, 3));
    l.edit(A, T, EventId(1), 1, worked(2, 9)).unwrap();
    assert_eq!(
        (
            balance(&l, A, Bucket::Holiday),
            balance(&l, A, Bucket::Floater),
            balance(&l, A, Bucket::Comp)
        ),
        (64, 16, 12)
    );
    l.edit(A, T, EventId(2), 1, worked(2, 3)).unwrap();
    assert_eq!(
        (
            balance(&l, A, Bucket::Holiday),
            balance(&l, A, Bucket::Floater)
        ),
        (72, 8)
    );
}

#[test]
fn moving_support_into_taken_holiday_rejects_all_changes() {
    let mut l = configured();
    add(&mut l, A, 1, worked(1, 9));
    add(&mut l, A, 2, off(2));
    assert_eq!(
        reject_unchanged(&mut l, |l| l.edit(A, T, EventId(1), 1, worked(2, 3))),
        Error::HolidayConflict
    );
}

#[test]
fn explicit_edit_corrects_taken_holiday_to_work_and_back() {
    let mut l = configured();
    add(&mut l, A, 1, off(1));
    l.edit(A, T, EventId(1), 1, worked(1, 9)).unwrap();
    assert_eq!(balance(&l, A, Bucket::Floater), 8);
    l.edit(A, T, EventId(1), 2, off(1)).unwrap();
    assert_eq!(
        (
            balance(&l, A, Bucket::Holiday),
            balance(&l, A, Bucket::Floater),
            balance(&l, A, Bucket::Comp)
        ),
        (72, 0, 0)
    );
}

#[test]
fn multi_001_to_003_single_deduction_with_explicit_included_dates() {
    let mut l = configured();
    add(&mut l, A, 1, grant(Bucket::Pto, 80));
    let mut s = usage(Bucket::Pto, 40);
    s.dates = vec![d(3, 7), d(3, 3), d(3, 4), d(3, 5), d(3, 6)];
    add(&mut l, A, 2, s.clone());
    assert_eq!(balance(&l, A, Bucket::Pto), 40);
    let entries = l
        .query(
            A,
            y(2026),
            &Filter {
                classification: Some(Classification::Use),
                ..Filter::default()
            },
        )
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].dates.len(), 5);
    assert_eq!(entries[0].delta, -40);
    s.dates = vec![d(3, 3), d(3, 7)];
    l.edit(A, T, EventId(2), 1, s).unwrap();
    assert_eq!(balance(&l, A, Bucket::Pto), 40);
    l.delete(A, T, EventId(2), 2).unwrap();
    assert_eq!(balance(&l, A, Bucket::Pto), 80);
}

#[test]
fn included_dates_reject_empty_duplicate_cross_year_and_multiday_work() {
    let mut l = configured();
    for dates in [
        vec![],
        vec![d(1, 1), d(1, 1)],
        vec![d(12, 31), Date::new(2027, 1, 1).unwrap()],
    ] {
        let mut s = usage(Bucket::Pto, 8);
        s.dates = dates;
        assert_eq!(
            reject_unchanged(&mut l, |l| l.create(A, T, EventId(1), s)),
            Error::InvalidDates
        );
    }
    let mut s = worked(1, 9);
    s.dates.push(d(1, 2));
    assert_eq!(
        reject_unchanged(&mut l, |l| l.create(A, T, EventId(1), s)),
        Error::InvalidDates
    );
}

#[test]
fn multiple_buckets_and_overlaps_have_only_explicit_deductions() {
    let mut l = Ledger::default();
    for (i, bucket) in [Bucket::Pto, Bucket::Comp, Bucket::Floater]
        .into_iter()
        .enumerate()
    {
        add(&mut l, A, i as u64, grant(bucket, 30));
        add(&mut l, A, i as u64 + 10, usage(bucket, 20));
        add(&mut l, A, i as u64 + 20, usage(bucket, 5));
        assert_eq!(balance(&l, A, bucket), 5);
    }
}

#[test]
fn ledger_001_to_006_filters_are_effect_precise_and_owned() {
    let mut l = configured();
    add(&mut l, A, 1, grant(Bucket::Pto, 40));
    add(&mut l, A, 2, worked(2, 9));
    add(&mut l, B, 1, grant(Bucket::Pto, 200));
    let mut use_s = usage(Bucket::Pto, 8);
    use_s.dates = vec![d(3, 1), d(3, 3)];
    use_s.notes = "Distinctive SEARCH phrase".into();
    add(&mut l, A, 3, use_s);
    let all = l.query(A, y(2026), &Filter::default()).unwrap();
    assert!(
        all.windows(2)
            .all(|pair| pair[0].dates[0] >= pair[1].dates[0])
    );
    let filter = Filter {
        bucket: Some(Bucket::Pto),
        classification: Some(Classification::Use),
        from: Some(d(3, 3)),
        through: Some(d(3, 3)),
        notes: Some("search".into()),
    };
    let result = l.query(A, y(2026), &filter).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].delta, -8);
    assert!(l.query(B, y(2026), &filter).unwrap().is_empty());
    assert!(l.query(A, y(2027), &Filter::default()).unwrap().is_empty());
    let gap = Filter {
        from: Some(d(3, 2)),
        through: Some(d(3, 2)),
        classification: Some(Classification::Use),
        ..Filter::default()
    };
    assert!(l.query(A, y(2026), &gap).unwrap().is_empty());
    assert_eq!(
        l.query(
            A,
            y(2026),
            &Filter {
                from: Some(d(3, 3)),
                through: Some(d(3, 1)),
                ..Filter::default()
            }
        ),
        Err(Error::InvalidDates)
    );
}

#[test]
fn shared_conversion_search_links_all_current_supports_only() {
    let mut l = configured();
    add(&mut l, A, 1, worked(1, 9));
    let mut s = worked(1, 3);
    s.notes = "Second support".into();
    add(&mut l, A, 2, s);
    let f = Filter {
        bucket: Some(Bucket::Floater),
        notes: Some("second".into()),
        ..Filter::default()
    };
    assert_eq!(l.query(A, y(2026), &f).unwrap().len(), 1);
    assert!(l.query(B, y(2026), &f).unwrap().is_empty());
    l.delete(A, T, EventId(2), 1).unwrap();
    assert!(l.query(A, y(2026), &f).unwrap().is_empty());
    assert_eq!(balance(&l, A, Bucket::Floater), 8);
}

#[test]
fn audit_001_to_004_revisions_deletions_and_ownership() {
    let mut l = Ledger::default();
    add(&mut l, A, 1, grant(Bucket::Pto, 160));
    l.edit(
        A,
        AuditTime(T.0 + 1),
        EventId(1),
        1,
        grant(Bucket::Pto, 120),
    )
    .unwrap();
    l.delete(A, AuditTime(T.0 + 2), EventId(1), 2).unwrap();
    assert!(l.query(A, y(2026), &Filter::default()).unwrap().is_empty());
    assert_eq!(l.record(A, EventId(1)), Err(Error::NotFound));
    assert!(l.history(B).is_empty());
    assert!(l.history(ADMIN).is_empty());
    let changes = l.history(A);
    assert_eq!(changes.len(), 3);
    assert!(changes[0].before.is_none());
    assert_eq!(changes[1].before, changes[0].after);
    assert_eq!(changes[2].before, changes[1].after);
    assert!(changes[2].after.is_none());
    assert_eq!(changes[2].at, AuditTime(T.0 + 2));
    assert_eq!(changes[0].after_effects[0].balance(Bucket::Pto), h(160));
    assert_eq!(changes[1].after_effects[0].balance(Bucket::Pto), h(120));
    assert_eq!(balance(&l, A, Bucket::Pto), 0);
    assert_eq!(
        reject_unchanged(&mut l, |l| l.create(
            A,
            T,
            EventId(1),
            grant(Bucket::Pto, 8)
        )),
        Error::AlreadyExists
    );
}

#[test]
fn isolation_guessed_ids_cannot_read_edit_delete_or_spend_other_owner_records() {
    let mut l = configured();
    add(&mut l, B, 7, grant(Bucket::Pto, 100));
    assert_eq!(l.record(A, EventId(7)), Err(Error::NotFound));
    assert_eq!(
        reject_unchanged(&mut l, |l| l.edit(
            A,
            T,
            EventId(7),
            1,
            grant(Bucket::Pto, 1)
        )),
        Error::NotFound
    );
    assert_eq!(
        reject_unchanged(&mut l, |l| l.delete(A, T, EventId(7), 1)),
        Error::NotFound
    );
    reject_unchanged(&mut l, |l| {
        l.create(A, T, EventId(8), usage(Bucket::Pto, 1))
    });
    add(&mut l, A, 7, grant(Bucket::Pto, 5));
    assert_eq!(balance(&l, B, Bucket::Pto), 100);
    assert_eq!(balance(&l, A, Bucket::Pto), 5);
}

#[test]
fn stale_revisions_and_duplicate_ids_fail_without_audit_mutation() {
    let mut l = Ledger::default();
    add(&mut l, A, 1, grant(Bucket::Pto, 8));
    assert_eq!(
        reject_unchanged(&mut l, |l| l.create(
            A,
            T,
            EventId(1),
            grant(Bucket::Pto, 16)
        )),
        Error::AlreadyExists
    );
    l.edit(A, T, EventId(1), 1, grant(Bucket::Pto, 16)).unwrap();
    assert_eq!(
        reject_unchanged(&mut l, |l| l.edit(
            A,
            T,
            EventId(1),
            1,
            grant(Bucket::Pto, 4)
        )),
        Error::StaleRevision
    );
    assert_eq!(
        reject_unchanged(&mut l, |l| l.delete(A, T, EventId(1), 1)),
        Error::StaleRevision
    );
}

#[test]
fn grant_reduction_and_deletion_reject_spent_credit() {
    let mut l = Ledger::default();
    add(&mut l, A, 1, grant(Bucket::Pto, 10));
    add(&mut l, A, 2, usage(Bucket::Pto, 8));
    reject_unchanged(&mut l, |l| {
        l.edit(A, T, EventId(1), 1, grant(Bucket::Pto, 7))
    });
    reject_unchanged(&mut l, |l| l.delete(A, T, EventId(1), 1));
    assert_eq!(balance(&l, A, Bucket::Pto), 2);
}

#[test]
fn edit_moving_year_checks_old_and_new_year_atomically() {
    let mut l = configured();
    l.configure(ADMIN, T, calendar(2027)).unwrap();
    add(&mut l, A, 1, worked(1, 9));
    add(&mut l, A, 2, usage(Bucket::Floater, 8));
    let mut moved = worked(1, 9);
    moved.dates = vec![Date::new(2027, 1, 1).unwrap()];
    reject_unchanged(&mut l, |l| l.edit(A, T, EventId(1), 1, moved.clone()));
    l.delete(A, T, EventId(2), 1).unwrap();
    l.edit(A, T, EventId(1), 1, moved).unwrap();
    assert_eq!(balance(&l, A, Bucket::Floater), 0);
    assert_eq!(
        l.snapshot(A, y(2027)).unwrap().balance(Bucket::Floater),
        h(8)
    );
    assert_eq!(l.history(A).last().unwrap().after_effects.len(), 2);
}

#[test]
fn signed_adjustments_are_explicit_and_checked() {
    for bucket in [Bucket::Pto, Bucket::Comp, Bucket::Floater] {
        let mut l = Ledger::default();
        add(
            &mut l,
            A,
            1,
            source(d(1, 1), Activity::Adjustment { bucket, delta: 100 }),
        );
        add(
            &mut l,
            A,
            2,
            source(d(1, 1), Activity::Adjustment { bucket, delta: -20 }),
        );
        assert_eq!(balance(&l, A, bucket), 80);
        reject_unchanged(&mut l, |l| {
            l.create(
                A,
                T,
                EventId(3),
                source(d(1, 1), Activity::Adjustment { bucket, delta: -81 }),
            )
        });
    }
}

#[test]
fn zero_amounts_rejected_for_grants_uses_adjustments() {
    let mut l = Ledger::default();
    for s in [
        grant(Bucket::Pto, 0),
        usage(Bucket::Pto, 0),
        source(
            d(1, 1),
            Activity::Adjustment {
                bucket: Bucket::Pto,
                delta: 0,
            },
        ),
    ] {
        assert_eq!(
            reject_unchanged(&mut l, |l| l.create(A, T, EventId(1), s)),
            Error::InvalidHours
        );
    }
}

#[test]
fn overflow_and_extreme_debits_reject_without_panics_or_partial_conversion() {
    let mut l = configured();
    add(&mut l, A, 1, grant(Bucket::Comp, i64::MAX));
    assert_eq!(
        reject_unchanged(&mut l, |l| l.create(A, T, EventId(2), worked(1, 1))),
        Error::Overflow
    );
    assert_eq!(balance(&l, A, Bucket::Holiday), 80);
    assert_eq!(balance(&l, A, Bucket::Floater), 0);
    assert_eq!(
        reject_unchanged(&mut l, |l| l.create(
            A,
            T,
            EventId(2),
            source(
                d(1, 1),
                Activity::Adjustment {
                    bucket: Bucket::Comp,
                    delta: i64::MIN
                }
            )
        )),
        Error::NegativeBalance {
            year: y(2026),
            bucket: Bucket::Comp
        }
    );
}

#[test]
fn large_net_total_does_not_depend_on_intermediate_sum_order() {
    let mut l = Ledger::default();
    add(&mut l, A, 1, grant(Bucket::Pto, i64::MAX));
    add(&mut l, A, 3, usage(Bucket::Pto, 10));
    add(&mut l, A, 2, grant(Bucket::Pto, 10));
    assert_eq!(balance(&l, A, Bucket::Pto), i64::MAX);
}

#[test]
fn holiday_name_correction_preserves_activity_and_audits_old_name() {
    let mut l = configured();
    add(&mut l, A, 1, worked(1, 9));
    let before = l.snapshot(A, y(2026)).unwrap();
    let record = l.record(A, EventId(1)).unwrap().clone();
    let mut c = calendar(2026);
    c.holidays[0].name = "Corrected holiday".into();
    l.configure(ADMIN, T, c).unwrap();
    assert_eq!(l.snapshot(A, y(2026)).unwrap(), before);
    assert_eq!(l.record(A, EventId(1)).unwrap(), &record);
    assert_eq!(
        l.calendar_history()[1].before.as_ref().unwrap().holidays[0].name,
        "Holiday 1"
    );
    assert_eq!(
        l.calendar(y(2026)).unwrap().holidays[0].name,
        "Corrected holiday"
    );
}

#[test]
fn referenced_dates_and_identity_removal_are_blocked_for_any_user() {
    for event in [off(1), worked(1, 9)] {
        let mut l = configured();
        add(&mut l, B, 1, event);
        let mut c = calendar(2026);
        c.holidays[0].date = d(1, 2);
        assert_eq!(
            reject_unchanged(&mut l, |l| l.configure(ADMIN, T, c)),
            Error::ReferencedHoliday
        );
        let mut c = calendar(2026);
        c.holidays[0].id = HolidayId(50);
        assert_eq!(
            reject_unchanged(&mut l, |l| l.configure(ADMIN, T, c)),
            Error::ReferencedHoliday
        );
    }
}

#[test]
fn unreferenced_dates_can_change_after_deliberate_resolution() {
    let mut l = configured();
    add(&mut l, A, 1, worked(1, 9));
    add(&mut l, A, 2, worked(1, 1));
    l.delete(A, T, EventId(1), 1).unwrap();
    let mut c = calendar(2026);
    c.holidays[0].date = d(1, 2);
    reject_unchanged(&mut l, |l| l.configure(ADMIN, T, c.clone()));
    l.delete(A, T, EventId(2), 1).unwrap();
    l.configure(ADMIN, T, c).unwrap();
    assert_eq!(l.calendar(y(2026)).unwrap().holidays[0].date, d(1, 2));
    assert_eq!(
        l.history(A)[0].after.as_ref().unwrap().source.dates,
        vec![d(1, 1)]
    );
    let mut s = worked(1, 0);
    s.dates = vec![d(1, 2)];
    add(&mut l, A, 3, s);
}

#[test]
fn moving_a_support_to_plain_comp_releases_only_its_holiday_support() {
    let mut l = configured();
    add(&mut l, A, 1, worked(1, 9));
    add(&mut l, A, 2, worked(1, 3));
    l.edit(
        A,
        T,
        EventId(1),
        1,
        source(d(1, 1), Activity::CompWork(work(9))),
    )
    .unwrap();
    assert_eq!(balance(&l, A, Bucket::Floater), 8);
    l.edit(
        A,
        T,
        EventId(2),
        1,
        source(d(1, 1), Activity::CompWork(work(3))),
    )
    .unwrap();
    assert_eq!(balance(&l, A, Bucket::Floater), 0);
    assert_eq!(balance(&l, A, Bucket::Comp), 12);
}

#[test]
fn deleting_holiday_use_restores_entitlement_without_floater() {
    let mut l = configured();
    add(&mut l, A, 1, off(1));
    l.delete(A, T, EventId(1), 1).unwrap();
    assert_eq!(
        (
            balance(&l, A, Bucket::Holiday),
            balance(&l, A, Bucket::Floater)
        ),
        (80, 0)
    );
    add(&mut l, A, 2, worked(1, 0));
    assert_eq!(balance(&l, A, Bucket::Floater), 8);
}

#[test]
fn exhaustive_short_sequences_match_independent_shared_state_oracle() {
    // Enumerate create/delete/off/use orderings. The oracle tracks support bits,
    // one holiday-use flag and one Floater-use flag, not projected effects.
    for sequence in 0_u32..6_u32.pow(4) {
        let mut l = configured();
        let mut supports = [false; 2];
        let mut taken = false;
        let mut spent = false;
        let mut next_id = 10;
        let mut support_ids = [EventId(0); 2];
        let mut off_id = EventId(0);
        let mut use_id = EventId(0);
        let mut code = sequence;
        for _ in 0..4 {
            let op = code % 6;
            code /= 6;
            let old = l.clone();
            let count = supports.iter().filter(|v| **v).count();
            let expected;
            let result = match op {
                0 | 1 => {
                    let i = op as usize;
                    if supports[i] {
                        expected = count > 1 || !spent;
                        let result = l.delete(A, T, support_ids[i], 1);
                        if expected {
                            supports[i] = false;
                        }
                        result
                    } else {
                        expected = !taken;
                        let id = EventId(next_id);
                        next_id += 1;
                        let result = l.create(A, T, id, worked(1, 0));
                        if expected {
                            supports[i] = true;
                            support_ids[i] = id;
                        }
                        result
                    }
                }
                2 | 3 => {
                    if taken {
                        expected = true;
                        let result = l.delete(A, T, off_id, 1);
                        taken = false;
                        result
                    } else {
                        expected = count == 0;
                        let id = EventId(next_id);
                        next_id += 1;
                        let result = l.create(A, T, id, off(1));
                        if expected {
                            taken = true;
                            off_id = id;
                        }
                        result
                    }
                }
                _ => {
                    if spent {
                        expected = true;
                        let result = l.delete(A, T, use_id, 1);
                        spent = false;
                        result
                    } else {
                        expected = count > 0;
                        let id = EventId(next_id);
                        next_id += 1;
                        let result = l.create(A, T, id, usage(Bucket::Floater, 8));
                        if expected {
                            spent = true;
                            use_id = id;
                        }
                        result
                    }
                }
            };
            assert_eq!(result.is_ok(), expected, "sequence {sequence}, op {op}");
            if !expected {
                assert_eq!(l, old);
            }
            let converted = supports.iter().any(|v| *v);
            assert_eq!(
                balance(&l, A, Bucket::Holiday),
                if converted || taken { 72 } else { 80 }
            );
            assert_eq!(
                balance(&l, A, Bucket::Floater),
                8 * i64::from(converted) - 8 * i64::from(spent)
            );
            assert_eq!(balance(&l, B, Bucket::Holiday), 80);
            assert_eq!(balance(&l, B, Bucket::Floater), 0);
        }
    }
}

#[test]
fn errors_are_useful_without_private_record_contents() {
    let message = Error::NegativeBalance {
        year: y(2026),
        bucket: Bucket::Floater,
    }
    .to_string();
    assert!(
        message.contains("2026") && message.contains("Floater") && message.contains("negative")
    );
    assert!(Error::ReferencedHoliday.to_string().contains("Resolve"));
}
