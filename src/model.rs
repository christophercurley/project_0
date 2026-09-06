use std::{collections::BTreeSet, fmt, str::FromStr};

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidDate,
    InvalidHours,
    InvalidDates,
    InvalidCalendar,
    CalendarNotConfigured,
    UnknownHoliday,
    ReferencedHoliday,
    HolidayConflict,
    ManualHolidayForbidden,
    AlreadyExists,
    NotFound,
    StaleRevision,
    Overflow,
    NegativeBalance { year: Year, bucket: Bucket },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDate => write!(f, "Enter a valid calendar date in years 1 through 9999."),
            Self::InvalidHours => write!(
                f,
                "Enter whole hours; grants, uses and adjustments must be nonzero."
            ),
            Self::InvalidDates => write!(f, "Include distinct dates in one calendar year."),
            Self::InvalidCalendar => write!(
                f,
                "Configure exactly ten named holidays with distinct IDs and dates in the selected year."
            ),
            Self::CalendarNotConfigured => write!(
                f,
                "Configure the year's holidays before using or working a holiday."
            ),
            Self::UnknownHoliday => {
                write!(f, "Select a configured holiday on its configured date.")
            }
            Self::ReferencedHoliday => write!(
                f,
                "Resolve active records referencing this holiday before changing its date or removing it."
            ),
            Self::HolidayConflict => write!(
                f,
                "This holiday is already taken off or worked; correct the conflicting record first."
            ),
            Self::ManualHolidayForbidden => write!(
                f,
                "Holiday supports only configured entitlement, Holiday Use and Holiday-to-Floater conversion."
            ),
            Self::AlreadyExists => write!(
                f,
                "This source ID already exists, including in deletion history."
            ),
            Self::NotFound => write!(f, "The requested active record was not found."),
            Self::StaleRevision => write!(f, "The record changed; reload it before editing."),
            Self::Overflow => write!(
                f,
                "The amount or revision exceeds the supported integer range."
            ),
            Self::NegativeBalance { year, bucket } => write!(
                f,
                "This change would make {bucket:?} negative in {}.",
                year.get()
            ),
        }
    }
}

impl std::error::Error for Error {}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", serde(try_from = "u16", into = "u16"))]
pub struct Year(u16);

impl Year {
    pub fn new(value: u16) -> Result<Self, Error> {
        if (1..=9999).contains(&value) {
            Ok(Self(value))
        } else {
            Err(Error::InvalidDate)
        }
    }
    pub fn get(self) -> u16 {
        self.0
    }
}

/// Proleptic Gregorian date, without timezone or time-of-day.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(
    feature = "serde",
    serde(try_from = "(u16, u8, u8)", into = "(u16, u8, u8)")
)]
pub struct Date {
    year: Year,
    month: u8,
    day: u8,
}

impl Date {
    pub fn new(year: u16, month: u8, day: u8) -> Result<Self, Error> {
        let year = Year::new(year)?;
        let leap =
            year.0.is_multiple_of(4) && (!year.0.is_multiple_of(100) || year.0.is_multiple_of(400));
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return Err(Error::InvalidDate),
        };
        if day == 0 || day > days {
            return Err(Error::InvalidDate);
        }
        Ok(Self { year, month, day })
    }
    pub fn year(self) -> Year {
        self.year
    }
    pub fn month(self) -> u8 {
        self.month
    }
    pub fn day(self) -> u8 {
        self.day
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year.0, self.month, self.day)
    }
}

/// Nonnegative whole hours. Zero is useful for explicitly confirmed no Comp.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", serde(try_from = "i64", into = "i64"))]
pub struct Hours(i64);

impl Hours {
    pub fn new(value: i64) -> Result<Self, Error> {
        if value >= 0 {
            Ok(Self(value))
        } else {
            Err(Error::InvalidHours)
        }
    }
    pub fn get(self) -> i64 {
        self.0
    }
    /// Exact day display, with no floating point even for very large balances.
    pub fn days(self) -> String {
        let whole = self.0 / 8;
        let fraction =
            ["", ".125", ".25", ".375", ".5", ".625", ".75", ".875"][(self.0 % 8) as usize];
        format!("{whole}{fraction}")
    }
}

impl FromStr for Hours {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::InvalidHours);
        }
        Self::new(s.parse().map_err(|_| Error::InvalidHours)?)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserId(pub u64);
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(pub u64);
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HolidayId(pub u64);

/// UTC seconds since the Unix epoch, supplied by the trusted application clock.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditTime(pub i64);

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bucket {
    Pto,
    Comp,
    Holiday,
    Floater,
}
impl Bucket {
    pub const ALL: [Self; 4] = [Self::Pto, Self::Comp, Self::Holiday, Self::Floater];
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classification {
    Grant,
    Use,
    Accrual,
    Conversion,
    Adjustment,
}
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Multiplier {
    OneToOne,
    OneAndAHalf,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Work {
    pub hours_worked: Option<Hours>,
    pub multiplier: Option<Multiplier>,
    pub credited_comp: Hours,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Activity {
    Grant { bucket: Bucket, hours: Hours },
    Adjustment { bucket: Bucket, delta: i64 },
    Use { bucket: Bucket, hours: Hours },
    CompWork(Work),
    HolidayUse { holiday: HolidayId },
    HolidayWork { holiday: HolidayId, work: Work },
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// One explicit total for all included dates. No daily allocation is inferred.
    pub dates: Vec<Date>,
    pub activity: Activity,
    pub notes: String,
}

impl Source {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let Some(first) = self.dates.first() else {
            return Err(Error::InvalidDates);
        };
        if self.dates.iter().any(|d| d.year() != first.year())
            || self.dates.iter().collect::<BTreeSet<_>>().len() != self.dates.len()
        {
            return Err(Error::InvalidDates);
        }
        if !matches!(self.activity, Activity::Use { .. }) && self.dates.len() != 1 {
            return Err(Error::InvalidDates);
        }
        match self.activity {
            Activity::Grant {
                bucket: Bucket::Holiday,
                ..
            }
            | Activity::Adjustment {
                bucket: Bucket::Holiday,
                ..
            }
            | Activity::Use {
                bucket: Bucket::Holiday,
                ..
            } => return Err(Error::ManualHolidayForbidden),
            Activity::Grant { hours, .. } | Activity::Use { hours, .. } if hours.get() == 0 => {
                return Err(Error::InvalidHours);
            }
            Activity::Adjustment { delta: 0, .. } => return Err(Error::InvalidHours),
            _ => {}
        }
        Ok(())
    }
    pub(crate) fn holiday(&self) -> Option<HolidayId> {
        match self.activity {
            Activity::HolidayUse { holiday } | Activity::HolidayWork { holiday, .. } => {
                Some(holiday)
            }
            _ => None,
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holiday {
    pub id: HolidayId,
    pub date: Date,
    pub name: String,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Calendar {
    pub year: Year,
    pub holidays: Vec<Holiday>,
}

impl Calendar {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.holidays.len() != 10
            || self
                .holidays
                .iter()
                .any(|h| h.date.year() != self.year || h.name.trim().is_empty())
            || self
                .holidays
                .iter()
                .map(|h| h.id)
                .collect::<BTreeSet<_>>()
                .len()
                != 10
            || self
                .holidays
                .iter()
                .map(|h| h.date)
                .collect::<BTreeSet<_>>()
                .len()
                != 10
        {
            return Err(Error::InvalidCalendar);
        }
        Ok(())
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceRef {
    pub id: EventId,
    pub revision: u64,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub reference: SourceRef,
    pub source: Source,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Entitlement {
        holiday: HolidayId,
    },
    Source(SourceRef),
    /// Shared state, with every supporting source revision; no permanent owner.
    WorkedHoliday {
        holiday: HolidayId,
        supports: Vec<SourceRef>,
    },
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effect {
    pub dates: Vec<Date>,
    pub bucket: Bucket,
    pub delta: i64,
    pub classification: Classification,
    pub origin: Origin,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub year: Year,
    pub holiday_configured: bool,
    pub effects: Vec<Effect>,
    pub balances: [Hours; 4],
}
impl Snapshot {
    pub fn balance(&self, bucket: Bucket) -> Hours {
        self.balances[Bucket::ALL
            .iter()
            .position(|b| *b == bucket)
            .expect("fixed bucket")]
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub actor: UserId,
    pub at: AuditTime,
    pub id: EventId,
    pub before: Option<Record>,
    pub after: Option<Record>,
    /// Complete affected-year projections make support changes reconstructable.
    pub before_effects: Vec<Snapshot>,
    pub after_effects: Vec<Snapshot>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarChange {
    pub actor: UserId,
    pub at: AuditTime,
    pub before: Option<Calendar>,
    pub after: Calendar,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub bucket: Option<Bucket>,
    pub classification: Option<Classification>,
    pub from: Option<Date>,
    pub through: Option<Date>,
    pub notes: Option<String>,
}
