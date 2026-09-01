use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CivilDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl CivilDate {
    pub fn from_naive(d: NaiveDate) -> Self {
        Self {
            year: d.year(),
            month: d.month() as u8,
            day: d.day() as u8,
        }
    }

    pub fn today_local() -> Self {
        Self::from_naive(Local::now().date_naive())
    }

    pub(crate) fn to_naive(self) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(self.year, self.month as u32, self.day as u32)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DateRange {
    pub start_day: Option<CivilDate>,
    pub end_day: Option<CivilDate>,
}

impl DateRange {
    /// Inclusive local-calendar window of `n` days ending on `today` (`n = 1` is today).
    pub fn last_n_days(today: CivilDate, n: i64) -> Self {
        let start = today.to_naive().and_then(|d| {
            d.checked_sub_signed(ChronoDuration::days((n - 1).max(0)))
                .map(CivilDate::from_naive)
        });
        Self {
            start_day: start,
            end_day: Some(today),
        }
    }
}
