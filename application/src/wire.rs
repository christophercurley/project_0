//! Version-one transport encoding. Decimal strings preserve full-width integers
//! in vanilla JavaScript; dates use ISO calendar strings, never timestamps.
use crate::error::{ApiError, Result};
use daymark_domain::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub fn integer(value: &str) -> Result<u64> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ApiError::bad());
    }
    value.parse().map_err(|_| ApiError::bad())
}
pub fn date(value: &str) -> Result<Date> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return Err(ApiError::bad());
    }
    Ok(Date::new(
        value[..4].parse().map_err(|_| ApiError::bad())?,
        value[5..7].parse().map_err(|_| ApiError::bad())?,
        value[8..].parse().map_err(|_| ApiError::bad())?,
    )?)
}
fn hours(value: &str) -> Result<Hours> {
    Ok(value.parse::<Hours>()?)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInput {
    pub dates: Vec<String>,
    pub activity: ActivityInput,
    pub notes: String,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActivityInput {
    Grant { bucket: Bucket, hours: String },
    Adjustment { bucket: Bucket, delta: String },
    Use { bucket: Bucket, hours: String },
    CompWork { work: WorkInput },
    HolidayUse { holiday: String },
    HolidayWork { holiday: String, work: WorkInput },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkInput {
    pub hours_worked: Option<String>,
    pub multiplier: Option<Multiplier>,
    pub credited_comp: String,
}
impl WorkInput {
    fn domain(self) -> Result<Work> {
        Ok(Work {
            hours_worked: self.hours_worked.map(|h| hours(&h)).transpose()?,
            multiplier: self.multiplier,
            credited_comp: hours(&self.credited_comp)?,
        })
    }
}
impl SourceInput {
    pub fn domain(self) -> Result<Source> {
        if self.notes.len() > 8192 || self.dates.len() > 366 {
            return Err(ApiError::bad());
        }
        let activity = match self.activity {
            ActivityInput::Grant { bucket, hours: h } => Activity::Grant {
                bucket,
                hours: hours(&h)?,
            },
            ActivityInput::Use { bucket, hours: h } => Activity::Use {
                bucket,
                hours: hours(&h)?,
            },
            ActivityInput::Adjustment { bucket, delta } => {
                let digits = delta.strip_prefix('-').unwrap_or(&delta);
                integer(digits)?;
                Activity::Adjustment {
                    bucket,
                    delta: delta.parse().map_err(|_| ApiError::bad())?,
                }
            }
            ActivityInput::CompWork { work } => Activity::CompWork(work.domain()?),
            ActivityInput::HolidayUse { holiday } => Activity::HolidayUse {
                holiday: HolidayId(integer(&holiday)?),
            },
            ActivityInput::HolidayWork { holiday, work } => Activity::HolidayWork {
                holiday: HolidayId(integer(&holiday)?),
                work: work.domain()?,
            },
        };
        Ok(Source {
            dates: self.dates.iter().map(|d| date(d)).collect::<Result<_>>()?,
            activity,
            notes: self.notes,
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    pub id: String,
    pub source: SourceInput,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditInput {
    pub revision: String,
    pub source: SourceInput,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteInput {
    pub revision: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarInput {
    pub revision: Option<String>,
    pub holidays: Vec<HolidayInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HolidayInput {
    pub id: String,
    pub date: String,
    pub name: String,
}
impl CalendarInput {
    pub fn domain(self, year: Year) -> Result<(Option<i64>, Calendar)> {
        if self.holidays.len() > 10 || self.holidays.iter().any(|h| h.name.len() > 200) {
            return Err(ApiError::bad());
        }
        let revision = self
            .revision
            .map(|v| integer(&v).and_then(|n| i64::try_from(n).map_err(|_| ApiError::bad())))
            .transpose()?;
        let holidays = self
            .holidays
            .into_iter()
            .map(|h| {
                Ok(Holiday {
                    id: HolidayId(integer(&h.id)?),
                    date: date(&h.date)?,
                    name: h.name,
                })
            })
            .collect::<Result<_>>()?;
        Ok((revision, Calendar { year, holidays }))
    }
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}
impl Page {
    pub fn bounds(&self) -> Result<(u32, u32)> {
        let limit = self.limit.unwrap_or(50);
        let offset = self.offset.unwrap_or(0);
        if !(1..=100).contains(&limit) || offset > 1_000_000 {
            return Err(ApiError::bad());
        }
        Ok((offset, limit))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryInput {
    pub bucket: Option<Bucket>,
    pub classification: Option<Classification>,
    pub from: Option<String>,
    pub through: Option<String>,
    pub notes: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}
impl QueryInput {
    pub fn domain(self) -> Result<(Filter, Page)> {
        if self.notes.as_ref().is_some_and(|n| n.len() > 512) {
            return Err(ApiError::bad());
        }
        Ok((
            Filter {
                bucket: self.bucket,
                classification: self.classification,
                from: self.from.map(|d| date(&d)).transpose()?,
                through: self.through.map(|d| date(&d)).transpose()?,
                notes: self.notes,
            },
            Page {
                offset: self.offset,
                limit: self.limit,
            },
        ))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsInput {
    pub hire_date: Option<String>,
    pub annual_pto_allowance: Option<String>,
}
impl SettingsInput {
    pub fn value(self) -> Result<Value> {
        let hire = self
            .hire_date
            .map(|d| date(&d).map(|d| d.to_string()))
            .transpose()?;
        let allowance = self
            .annual_pto_allowance
            .map(|h| hours(&h).map(|h| h.get().to_string()))
            .transpose()?;
        Ok(json!({"hire_date":hire,"annual_pto_allowance":allowance}))
    }
}

pub fn output(value: impl Serialize) -> Result<Value> {
    let mut value = serde_json::to_value(value).map_err(|_| ApiError::busy())?;
    transform(&mut value, "");
    Ok(value)
}
fn transform(value: &mut Value, key: &str) {
    match value {
        Value::Number(n) if !matches!(key, "year" | "at") => {
            *value = Value::String(n.to_string());
        }
        Value::Array(items) if key == "date" && items.len() == 3 => {
            if let (Some(y), Some(m), Some(d)) =
                (items[0].as_u64(), items[1].as_u64(), items[2].as_u64())
            {
                *value = json!(format!("{y:04}-{m:02}-{d:02}"));
            }
        }
        Value::Array(items) => {
            for item in items {
                transform(item, if key == "dates" { "date" } else { key });
            }
        }
        Value::Object(map) => {
            for (key, value) in map {
                transform(value, key);
            }
        }
        _ => {}
    }
}
