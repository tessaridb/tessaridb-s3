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
mod tests {
    use super::Timestamp;

    #[test]
    fn known_instants_render_in_both_forms() {
        let cases = [
            (
                0,
                0,
                "1970-01-01T00:00:00.000Z",
                "Thu, 01 Jan 1970 00:00:00 GMT",
            ),
            (
                1_255_369_830,
                123_456_789,
                "2009-10-12T17:50:30.123Z",
                "Mon, 12 Oct 2009 17:50:30 GMT",
            ),
            (
                951_782_400,
                0,
                "2000-02-29T00:00:00.000Z",
                "Tue, 29 Feb 2000 00:00:00 GMT",
            ),
            (
                4_102_444_799,
                999_999_999,
                "2099-12-31T23:59:59.999Z",
                "Thu, 31 Dec 2099 23:59:59 GMT",
            ),
        ];
        for (seconds, nanos, iso, http) in cases {
            let at = Timestamp { seconds, nanos };
            assert_eq!(
                (at.iso8601_millis().as_str(), at.http_date().as_str()),
                (iso, http),
                "{seconds}"
            );
            assert_eq!(
                Timestamp::parse_http_date(http),
                Some(Timestamp { seconds, nanos: 0 }),
                "{http}"
            );
        }
    }

    #[test]
    fn only_imf_fixdate_parses() {
        let refused = [
            "",
            "yesterday",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
            "Mon, 12 Oct 2009 25:00:00 GMT",
            "Mon, 12 Oct 2009 17:50:30 UTC",
        ];
        for text in refused {
            assert_eq!(Timestamp::parse_http_date(text), None, "{text}");
        }
    }
}
