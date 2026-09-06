use std::collections::{BTreeMap, BTreeSet};

use crate::*;

/// Pure, deterministic reference model. All fields are private; mutations commit
/// only after validation. IDs are scoped to users (holiday IDs to years).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ledger {
    calendars: BTreeMap<Year, Calendar>,
    records: BTreeMap<(UserId, EventId), Record>,
    used_ids: BTreeSet<(UserId, EventId)>,
    history: BTreeMap<UserId, Vec<Change>>,
    calendar_history: Vec<CalendarChange>,
}

impl Ledger {
    pub fn calendar(&self, year: Year) -> Option<&Calendar> {
        self.calendars.get(&year)
    }
    pub fn calendar_history(&self) -> &[CalendarChange] {
        &self.calendar_history
    }
    pub fn record(&self, user: UserId, id: EventId) -> Result<&Record, Error> {
        self.records.get(&(user, id)).ok_or(Error::NotFound)
    }
    pub fn history(&self, user: UserId) -> &[Change] {
        self.history.get(&user).map_or(&[], Vec::as_slice)
    }

    /// Privileged global operation. The application must authorize the actor.
    /// Historical references remain in audit snapshots; only deliberately
    /// resolved (edited/deleted) active references release a date restriction.
    pub fn configure(
        &mut self,
        actor: UserId,
        at: AuditTime,
        calendar: Calendar,
    ) -> Result<(), Error> {
        calendar.validate()?;
        let before = self.calendars.get(&calendar.year).cloned();
        if let Some(old) = &before {
            for holiday in &old.holidays {
                let referenced = self.records.values().any(|r| {
                    r.source.dates[0].year() == old.year && r.source.holiday() == Some(holiday.id)
                });
                if referenced
                    && !calendar
                        .holidays
                        .iter()
                        .any(|h| h.id == holiday.id && h.date == holiday.date)
                {
                    return Err(Error::ReferencedHoliday);
                }
            }
        }
        self.calendars.insert(calendar.year, calendar.clone());
        self.calendar_history.push(CalendarChange {
            actor,
            at,
            before,
            after: calendar,
        });
        Ok(())
    }

    pub fn create(
        &mut self,
        user: UserId,
        at: AuditTime,
        id: EventId,
        source: Source,
    ) -> Result<(), Error> {
        if self.used_ids.contains(&(user, id)) {
            return Err(Error::AlreadyExists);
        }
        self.change(
            user,
            at,
            id,
            Some(Record {
                reference: SourceRef { id, revision: 1 },
                source,
            }),
        )
    }

    pub fn edit(
        &mut self,
        user: UserId,
        at: AuditTime,
        id: EventId,
        expected_revision: u64,
        source: Source,
    ) -> Result<(), Error> {
        let old = self.record(user, id)?;
        if old.reference.revision != expected_revision {
            return Err(Error::StaleRevision);
        }
        let revision = expected_revision.checked_add(1).ok_or(Error::Overflow)?;
        self.change(
            user,
            at,
            id,
            Some(Record {
                reference: SourceRef { id, revision },
                source,
            }),
        )
    }

    pub fn delete(
        &mut self,
        user: UserId,
        at: AuditTime,
        id: EventId,
        expected_revision: u64,
    ) -> Result<(), Error> {
        if self.record(user, id)?.reference.revision != expected_revision {
            return Err(Error::StaleRevision);
        }
        self.change(user, at, id, None)
    }

    fn change(
        &mut self,
        user: UserId,
        at: AuditTime,
        id: EventId,
        mut after: Option<Record>,
    ) -> Result<(), Error> {
        if let Some(record) = &mut after {
            record.source.validate()?;
            record.source.dates.sort();
        }
        let before = self.records.get(&(user, id)).cloned();
        let years: BTreeSet<_> = before
            .iter()
            .chain(after.iter())
            .map(|r| r.source.dates[0].year())
            .collect();
        let before_effects = years
            .iter()
            .map(|y| self.snapshot(user, *y))
            .collect::<Result<Vec<_>, _>>()?;
        let mut candidate = self.clone();
        if let Some(record) = &after {
            candidate.records.insert((user, id), record.clone());
        } else {
            candidate.records.remove(&(user, id));
        }
        let after_effects = years
            .iter()
            .map(|y| candidate.snapshot(user, *y))
            .collect::<Result<Vec<_>, _>>()?;
        candidate.used_ids.insert((user, id));
        candidate.history.entry(user).or_default().push(Change {
            actor: user,
            at,
            id,
            before,
            after,
            before_effects,
            after_effects,
        });
        *self = candidate;
        Ok(())
    }

    /// Annual net effective balance, including future-dated records. There is
    /// intentionally no clock or chronological running-balance validation.
    pub fn snapshot(&self, user: UserId, year: Year) -> Result<Snapshot, Error> {
        let mut effects = Vec::new();
        if let Some(calendar) = self.calendars.get(&year) {
            for holiday in &calendar.holidays {
                effects.push(Effect {
                    dates: vec![holiday.date],
                    bucket: Bucket::Holiday,
                    delta: 8,
                    classification: Classification::Grant,
                    origin: Origin::Entitlement {
                        holiday: holiday.id,
                    },
                });
            }
        }
        let mut worked: BTreeMap<HolidayId, Vec<SourceRef>> = BTreeMap::new();
        let mut taken = BTreeSet::new();
        for ((owner, _), record) in &self.records {
            if *owner != user || record.source.dates[0].year() != year {
                continue;
            }
            let source = &record.source;
            let mut add = |bucket, delta, classification| {
                effects.push(Effect {
                    dates: source.dates.clone(),
                    bucket,
                    delta,
                    classification,
                    origin: Origin::Source(record.reference),
                });
            };
            match &source.activity {
                Activity::Grant { bucket, hours } => {
                    add(*bucket, hours.get(), Classification::Grant)
                }
                Activity::Adjustment { bucket, delta } => {
                    add(*bucket, *delta, Classification::Adjustment)
                }
                Activity::Use { bucket, hours } => add(*bucket, -hours.get(), Classification::Use),
                Activity::CompWork(work) => add(
                    Bucket::Comp,
                    work.credited_comp.get(),
                    Classification::Accrual,
                ),
                Activity::HolidayUse { holiday } => {
                    self.validate_holiday(year, *holiday, source.dates[0])?;
                    if !taken.insert(*holiday) {
                        return Err(Error::HolidayConflict);
                    }
                    add(Bucket::Holiday, -8, Classification::Use);
                }
                Activity::HolidayWork { holiday, work } => {
                    self.validate_holiday(year, *holiday, source.dates[0])?;
                    worked.entry(*holiday).or_default().push(record.reference);
                    add(
                        Bucket::Comp,
                        work.credited_comp.get(),
                        Classification::Accrual,
                    );
                }
            }
        }
        for (holiday, supports) in worked {
            if taken.contains(&holiday) {
                return Err(Error::HolidayConflict);
            }
            let date = self.calendars[&year]
                .holidays
                .iter()
                .find(|h| h.id == holiday)
                .ok_or(Error::UnknownHoliday)?
                .date;
            for (bucket, delta) in [(Bucket::Holiday, -8), (Bucket::Floater, 8)] {
                effects.push(Effect {
                    dates: vec![date],
                    bucket,
                    delta,
                    classification: Classification::Conversion,
                    origin: Origin::WorkedHoliday {
                        holiday,
                        supports: supports.clone(),
                    },
                });
            }
        }
        let mut balances = [Hours::default(); 4];
        for (index, bucket) in Bucket::ALL.iter().enumerate() {
            // A wider accumulator avoids imposing accidental chronological or
            // insertion-order restrictions on individually representable effects.
            let total = effects
                .iter()
                .filter(|e| e.bucket == *bucket)
                .try_fold(0_i128, |sum, e| {
                    sum.checked_add(i128::from(e.delta)).ok_or(Error::Overflow)
                })?;
            if total < 0 {
                return Err(Error::NegativeBalance {
                    year,
                    bucket: *bucket,
                });
            }
            balances[index] = Hours::new(i64::try_from(total).map_err(|_| Error::Overflow)?)?;
        }
        effects.sort_by(|a, b| b.dates[0].cmp(&a.dates[0]));
        Ok(Snapshot {
            year,
            holiday_configured: self.calendars.contains_key(&year),
            effects,
            balances,
        })
    }

    fn validate_holiday(&self, year: Year, id: HolidayId, date: Date) -> Result<(), Error> {
        let calendar = self
            .calendars
            .get(&year)
            .ok_or(Error::CalendarNotConfigured)?;
        if !calendar
            .holidays
            .iter()
            .any(|h| h.id == id && h.date == date)
        {
            return Err(Error::UnknownHoliday);
        }
        Ok(())
    }

    /// Filters effective effects, not independent calendar-cell deductions.
    /// Date filtering matches any explicitly included date. Notes on a shared
    /// conversion match any currently supporting source; other effects stay out.
    pub fn query(&self, user: UserId, year: Year, filter: &Filter) -> Result<Vec<Effect>, Error> {
        if matches!((filter.from, filter.through), (Some(a), Some(b)) if a > b) {
            return Err(Error::InvalidDates);
        }
        let needle = filter.notes.as_ref().map(|s| s.to_lowercase());
        Ok(self
            .snapshot(user, year)?
            .effects
            .into_iter()
            .filter(|effect| {
                if filter.bucket.is_some_and(|b| effect.bucket != b)
                    || filter
                        .classification
                        .is_some_and(|c| effect.classification != c)
                {
                    return false;
                }
                if !effect.dates.iter().any(|d| {
                    filter.from.is_none_or(|a| *d >= a) && filter.through.is_none_or(|b| *d <= b)
                }) {
                    return false;
                }
                if let Some(needle) = &needle {
                    let matches = |reference: &SourceRef| {
                        self.records
                            .get(&(user, reference.id))
                            .is_some_and(|r| r.source.notes.to_lowercase().contains(needle))
                    };
                    match &effect.origin {
                        Origin::Entitlement { .. } => return needle.is_empty(),
                        Origin::Source(reference) => return matches(reference),
                        Origin::WorkedHoliday { supports, .. } => {
                            return supports.iter().any(matches);
                        }
                    }
                }
                true
            })
            .collect())
    }
}
