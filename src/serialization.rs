//! Optional value serialization; decoding never bypasses validated constructors.
use crate::{Date, Error, Hours, Year};

impl TryFrom<u16> for Year {
    type Error = Error;
    fn try_from(value: u16) -> Result<Self, Error> {
        Self::new(value)
    }
}
impl From<Year> for u16 {
    fn from(value: Year) -> Self {
        value.get()
    }
}
impl TryFrom<i64> for Hours {
    type Error = Error;
    fn try_from(value: i64) -> Result<Self, Error> {
        Self::new(value)
    }
}
impl From<Hours> for i64 {
    fn from(value: Hours) -> Self {
        value.get()
    }
}
impl TryFrom<(u16, u8, u8)> for Date {
    type Error = Error;
    fn try_from((year, month, day): (u16, u8, u8)) -> Result<Self, Error> {
        Self::new(year, month, day)
    }
}
impl From<Date> for (u16, u8, u8) {
    fn from(value: Date) -> Self {
        (value.year().get(), value.month(), value.day())
    }
}
