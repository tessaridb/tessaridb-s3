//! SigV4's timestamp `YYYYMMDD'T'HHMMSS'Z'`, parsed to seconds since the Unix epoch without a clock or a calendar
//! dependency: the server's clock is passed in by the caller, so this module stays pure.

use super::{AuthError, AuthResult};

/// A SigV4 request timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmzDateTime {
    text: String,
    unix_secs: i64,
}

impl AmzDateTime {
    /// Parses `YYYYMMDDTHHMMSSZ`.
    ///
    /// # Errors
    /// [`AuthError::InvalidDate`] when the text is not that shape or not a real time of day.
    pub fn parse(text: &str) -> AuthResult<Self> {
        let bytes = text.as_bytes();
        let shape_ok = bytes.len() == 16
            && bytes.get(8) == Some(&b'T')
            && bytes.get(15) == Some(&b'Z')
            && bytes
                .iter()
                .enumerate()
                .all(|(i, b)| i == 8 || i == 15 || b.is_ascii_digit());
        if !shape_ok {
            return Err(AuthError::InvalidDate);
        }
        let number = |from: usize, to: usize| -> AuthResult<i64> {
            text.get(from..to)
                .and_then(|digits| digits.parse().ok())
                .ok_or(AuthError::InvalidDate)
        };
        let (year, month, day) = (number(0, 4)?, number(4, 6)?, number(6, 8)?);
        let (hour, minute, second) = (number(9, 11)?, number(11, 13)?, number(13, 15)?);
        let valid = (1..=12).contains(&month)
            && (1..=days_in_month(year, month)).contains(&day)
            && (0..24).contains(&hour)
            && (0..60).contains(&minute)
            && (0..60).contains(&second);
        if !valid {
            return Err(AuthError::InvalidDate);
        }
        let unix_secs = days_from_civil(year, month, day)
            .checked_mul(86_400)
            .and_then(|s| s.checked_add(hour.checked_mul(3_600)?))
            .and_then(|s| s.checked_add(minute.checked_mul(60)?))
            .and_then(|s| s.checked_add(second))
            .ok_or(AuthError::InvalidDate)?;
        Ok(Self {
            text: text.to_owned(),
            unix_secs,
        })
    }

    /// The timestamp exactly as the client wrote it; it is part of the string to sign.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The `YYYYMMDD` date, which must equal the date in the credential scope.
    #[must_use]
    pub fn date(&self) -> &str {
        self.text.get(..8).unwrap_or_default()
    }

    /// Seconds since 1970-01-01T00:00:00Z.
    #[must_use]
    pub const fn unix_secs(&self) -> i64 {
        self.unix_secs
    }
}

/// Days from 1970-01-01 to the given proleptic Gregorian date (the civil-from-days inverse, valid for years 0-9999).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    // Bounded inputs (year 0-9999, month 1-12, day 1-31) keep every intermediate far inside i64, so saturating
    // arithmetic never saturates; it only satisfies the no-bare-arithmetic rule.
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

/// The number of days in `month` of `year`.
const fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::AmzDateTime;
    use crate::auth::AuthError;

    #[test]
    fn known_instants_convert_to_unix_seconds() {
        let cases = [
            ("19700101T000000Z", 0),
            ("20130524T000000Z", 1_369_353_600),
            ("20000229T235959Z", 951_868_799),
        ];
        for (text, secs) in cases {
            assert_eq!(
                AmzDateTime::parse(text).map(|t| t.unix_secs()),
                Ok(secs),
                "{text}"
            );
        }
    }

    #[test]
    fn the_date_part_is_the_first_eight_characters() {
        assert_eq!(
            AmzDateTime::parse("20130524T000000Z").map(|t| t.date().to_owned()),
            Ok("20130524".to_owned())
        );
    }

    #[test]
    fn malformed_or_impossible_times_are_refused() {
        for text in [
            "2013-05-24T000000Z",
            "20130524T000000",
            "20130230T000000Z",
            "20130524T240000Z",
            "20130524 000000Z",
        ] {
            assert_eq!(
                AmzDateTime::parse(text),
                Err(AuthError::InvalidDate),
                "{text}"
            );
        }
    }
}
