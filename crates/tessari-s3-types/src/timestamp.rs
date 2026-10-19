//! [`Timestamp`]: an instant, written in the two forms S3 clients parse.

/// An instant as seconds and nanoseconds since 1970-01-01T00:00:00Z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    /// Whole seconds since the epoch.
    pub seconds: i64,
    /// The sub-second part.
    pub nanos: u32,
}

impl Timestamp {
    /// The current instant by this machine's clock (the epoch when the clock is before it).
    #[must_use]
    pub fn now() -> Self {
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
            nanos: since.subsec_nanos(),
        }
    }

    /// `2009-10-12T17:50:30.000Z` — the XML `LastModified` / `CreationDate` form, milliseconds and `Z`.
    #[must_use]
    pub fn iso8601_millis(self) -> String {
        let (year, month, day, hour, minute, second) = self.civil();
        format!(
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{:03}Z",
            self.nanos / 1_000_000
        )
    }

    /// `Mon, 12 Oct 2009 17:50:30 GMT` — the IMF-fixdate of `Last-Modified`.
    #[must_use]
    pub fn http_date(self) -> String {
        const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let (year, month, day, hour, minute, second) = self.civil();
        let weekday = self.seconds.div_euclid(86_400).rem_euclid(7);
        let weekday = usize::try_from(weekday)
            .ok()
            .and_then(|i| DAYS.get(i))
            .copied()
            .unwrap_or("Thu");
        let month_name = usize::try_from(month.saturating_sub(1))
            .ok()
            .and_then(|i| MONTHS.get(i))
            .copied()
            .unwrap_or("Jan");
        format!("{weekday}, {day:02} {month_name} {year:04} {hour:02}:{minute:02}:{second:02} GMT")
    }

    /// Parses `2009-10-12T17:50:30Z` or the same with a fraction of one to nine digits (`.000Z`, `.123456Z`), in
    /// UTC; anything else is `None`.
    #[must_use]
    pub fn parse_iso8601(text: &str) -> Option<Self> {
        let text = text.trim().strip_suffix('Z')?;
        let (clock, nanos) = match text.split_once('.') {
            Some((clock, fraction))
                if (1..=9).contains(&fraction.len())
                    && fraction.bytes().all(|b| b.is_ascii_digit()) =>
            {
                let padded = format!("{fraction:0<9}");
                (clock, padded.parse::<u32>().ok()?)
            }
            Some(_) => return None,
            None => (text, 0),
        };
        let (date, time) = clock.split_once('T')?;
        let number = |t: &str| -> Option<i64> {
            t.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| t.parse().ok())?
        };
        let mut ymd = date.split('-').map(number);
        let (year, month, day) = (ymd.next()??, ymd.next()??, ymd.next()??);
        let mut hms = time.split(':').map(number);
        let (hour, minute, second) = (hms.next()??, hms.next()??, hms.next()??);
        let valid = (1..=12).contains(&month)
            && (1..=31).contains(&day)
            && (0..24).contains(&hour)
            && (0..60).contains(&minute)
            && (0..60).contains(&second);
        if !valid || ymd.next().is_some() || hms.next().is_some() {
            return None;
        }
        let seconds = days_from_civil(year, month, day)
            .checked_mul(86_400)?
            .checked_add(hour.checked_mul(3_600)?)?
            .checked_add(minute.checked_mul(60)?)?
            .checked_add(second)?;
        Some(Self { seconds, nanos })
    }

    /// Parses an IMF-fixdate (`Mon, 12 Oct 2009 17:50:30 GMT`), the form HTTP requires senders to use; anything else
    /// is `None`, and a conditional header carrying it is ignored as RFC 9110 says.
    #[must_use]
    pub fn parse_http_date(text: &str) -> Option<Self> {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let parts: Vec<&str> = text.trim().split(' ').collect();
        let [_weekday, day, month, year, clock, "GMT"] = parts.as_slice() else {
            return None;
        };
        let month = MONTHS
            .iter()
            .position(|m| m == month)
            .and_then(|i| i64::try_from(i).ok())?
            .checked_add(1)?;
        let number = |t: &str| t.parse::<i64>().ok();
        let (day, year) = (number(day)?, number(year)?);
        let mut hms = clock.split(':').map(number);
        let (hour, minute, second) = (hms.next()??, hms.next()??, hms.next()??);
        let valid = (1..=31).contains(&day)
            && (0..24).contains(&hour)
            && (0..60).contains(&minute)
            && (0..61).contains(&second);
        if !valid || hms.next().is_some() {
            return None;
        }
        let days = days_from_civil(year, month, day);
        let seconds = days
            .checked_mul(86_400)?
            .checked_add(hour.checked_mul(3_600)?)?
            .checked_add(minute.checked_mul(60)?)?
            .checked_add(second)?;
        Some(Self { seconds, nanos: 0 })
    }

    /// Year, month, day, hour, minute, second in UTC (the days-to-civil algorithm for the proleptic Gregorian
    /// calendar). Inputs are bounded by `i64` seconds, so the saturating steps never saturate in practice; they keep
    /// the no-bare-arithmetic rule.
    fn civil(self) -> (i64, i64, i64, i64, i64, i64) {
        let days = self.seconds.div_euclid(86_400);
        let in_day = self.seconds.rem_euclid(86_400);
        let shifted = days.saturating_add(719_468);
        let era = shifted.div_euclid(146_097);
        let day_of_era = shifted.saturating_sub(era.saturating_mul(146_097));
        let year_of_era = day_of_era
            .saturating_sub(day_of_era.div_euclid(1_460))
            .saturating_add(day_of_era.div_euclid(36_524))
            .saturating_sub(day_of_era.div_euclid(146_096))
            .div_euclid(365);
        let day_of_year = day_of_era.saturating_sub(
            year_of_era
                .saturating_mul(365)
                .saturating_add(year_of_era.div_euclid(4))
                .saturating_sub(year_of_era.div_euclid(100)),
        );
        let month_from_march = day_of_year
            .saturating_mul(5)
            .saturating_add(2)
            .div_euclid(153);
        let day = day_of_year
            .saturating_sub(
                month_from_march
                    .saturating_mul(153)
                    .saturating_add(2)
                    .div_euclid(5),
            )
            .saturating_add(1);
        let month = if month_from_march < 10 {
            month_from_march.saturating_add(3)
        } else {
            month_from_march.saturating_sub(9)
        };
        let year = year_of_era
            .saturating_add(era.saturating_mul(400))
            .saturating_add(i64::from(month <= 2));
        (
            year,
            month,
            day,
            in_day.div_euclid(3_600),
            in_day.rem_euclid(3_600).div_euclid(60),
            in_day.rem_euclid(60),
        )
    }
}

/// Days from 1970-01-01 to a proleptic Gregorian date (civil-to-days). Bounded inputs keep the saturating steps from
/// ever saturating; they keep the no-bare-arithmetic rule.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let shifted_year = if month <= 2 {
        year.saturating_sub(1)
    } else {
        year
    };
    let era = shifted_year.div_euclid(400);
    let year_of_era = shifted_year.saturating_sub(era.saturating_mul(400));
    let month_from_march = if month > 2 {
        month.saturating_sub(3)
    } else {
        month.saturating_add(9)
    };
    let day_of_year = month_from_march
        .saturating_mul(153)
        .saturating_add(2)
        .div_euclid(5)
        .saturating_add(day)
        .saturating_sub(1);
    let day_of_era = year_of_era
        .saturating_mul(365)
        .saturating_add(year_of_era.div_euclid(4))
        .saturating_sub(year_of_era.div_euclid(100))
        .saturating_add(day_of_year);
    era.saturating_mul(146_097)
        .saturating_add(day_of_era)
        .saturating_sub(719_468)
}

#[cfg(test)]
#[path = "timestamp_tests.rs"]
mod tests;
