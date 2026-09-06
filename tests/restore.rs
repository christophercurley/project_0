use daymark_domain::*;

fn record(id: u64, delta: i64) -> Record {
    Record {
        reference: SourceRef {
            id: EventId(id),
            revision: 3,
        },
        source: Source {
            dates: vec![Date::new(2026, 1, 1).unwrap()],
            activity: Activity::Adjustment {
                bucket: Bucket::Pto,
                delta,
            },
            notes: String::new(),
        },
    }
}

#[test]
fn restore_validates_final_net_without_replaying_in_insertion_order() {
    let owner = UserId(1);
    let mut ledger = Ledger::restore(
        [],
        [(owner, record(1, -8)), (owner, record(2, 8))],
        [(owner, EventId(3))],
    )
    .unwrap();
    assert!(ledger.history(owner).is_empty());
    assert!(ledger.calendar_history().is_empty());
    assert_eq!(
        ledger.record(owner, EventId(1)).unwrap().reference.revision,
        3
    );
    assert_eq!(
        ledger.create(owner, AuditTime(0), EventId(3), record(3, 1).source),
        Err(Error::AlreadyExists)
    );
    ledger
        .edit(owner, AuditTime(1), EventId(1), 3, record(1, -4).source)
        .unwrap();
    assert_eq!(ledger.history(owner).len(), 1);
    assert_eq!(
        ledger.history(owner)[0]
            .before
            .as_ref()
            .unwrap()
            .reference
            .revision,
        3
    );
}

#[test]
fn restore_rejects_invalid_sources_duplicates_revisions_and_net_balances() {
    let owner = UserId(1);
    assert!(Ledger::restore([], [(owner, record(1, -1))], []).is_err());
    assert!(Ledger::restore([], [(owner, record(1, 1)), (owner, record(1, 2))], []).is_err());
    let mut bad = record(1, 1);
    bad.reference.revision = 0;
    assert!(Ledger::restore([], [(owner, bad)], []).is_err());
    let mut bad = record(1, 1);
    bad.source.dates.clear();
    assert!(Ledger::restore([], [(owner, bad)], []).is_err());
    let mut bad = record(1, 1);
    bad.source.activity = Activity::Grant {
        bucket: Bucket::Holiday,
        hours: Hours::new(8).unwrap(),
    };
    assert!(Ledger::restore([], [(owner, bad)], []).is_err());
}
