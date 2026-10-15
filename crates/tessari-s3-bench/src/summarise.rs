//! Several runs of one set-up summarised per phase: the median and the range of each percentile and of the rate,
//! so a reader sees the spread between runs and not one run's luck.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::body::SizeClass;
use crate::error::{Error, Result};
use crate::report::{Report, millis};

/// Median, lowest and highest of one figure across runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spread {
    /// The median (the lower middle value for an even count).
    pub median: f64,
    /// The lowest.
    pub low: f64,
    /// The highest.
    pub high: f64,
}

/// The spread of `values`; `None` when there are none.
#[must_use]
pub fn spread(values: &mut [f64]) -> Option<Spread> {
    values.sort_by(f64::total_cmp);
    let middle = values.len().saturating_sub(1) / 2;
    Some(Spread {
        median: *values.get(middle)?,
        low: *values.first()?,
        high: *values.last()?,
    })
}

/// Refuses reports that are not like-for-like, then renders one row per phase.
///
/// # Errors
/// [`Error::Unlike`] when the reports differ in server build, endpoint, host or workload.
pub fn summarise(reports: &[Report]) -> Result<String> {
    let Some(first) = reports.first() else {
        return Ok(String::new());
    };
    for report in reports {
        let differs = [
            ("server build", report.server_build != first.server_build),
            ("endpoint", report.endpoint != first.endpoint),
            ("host", report.host != first.host),
            (
                "workload",
                (report.ops, report.warmup, report.prefixes)
                    != (first.ops, first.warmup, first.prefixes),
            ),
        ];
        if let Some((what, _)) = differs.iter().find(|(_, differs)| *differs) {
            return Err(Error::Unlike { what });
        }
    }
    let mut groups: BTreeMap<(SizeClass, String, usize), Vec<[f64; 4]>> = BTreeMap::new();
    for phase in reports.iter().flat_map(|report| &report.phases) {
        if phase.skipped.is_none() {
            groups
                .entry((phase.class, phase.op.clone(), phase.concurrency))
                .or_default()
                .push([
                    millis(phase.p50_us),
                    millis(phase.p99_us),
                    millis(phase.p999_us),
                    phase.ops_per_sec,
                ]);
        }
    }
    let mut out = String::new();
    let valid = reports.iter().all(Report::valid);
    // Writing to a String cannot fail.
    let _header = writeln!(
        out,
        "{} runs · server `{}` at `{}` · {} · all valid: {valid}\n",
        reports.len(),
        first.server_build,
        first.endpoint,
        first.host.note
    );
    out.push_str("| class | op | conc | p50 ms (median, range) | p99 ms | p99.9 ms | ops/s |\n|---|---|---:|---|---|---|---|\n");
    for ((class, op, concurrency), rows) in &groups {
        let mut cells = Vec::new();
        for column in 0..4 {
            let mut values: Vec<f64> = rows
                .iter()
                .filter_map(|row| row.get(column).copied())
                .collect();
            cells.push(spread(&mut values).map_or_else(String::new, |s| {
                format!("{:.2} ({:.2}–{:.2})", s.median, s.low, s.high)
            }));
        }
        let _row = writeln!(
            out,
            "| {class:?} | {op} | {concurrency} | {} |",
            cells.join(" | ")
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{Spread, spread};

    #[test]
    fn the_spread_is_median_and_range_whatever_the_order() {
        let mut odd = [3.0, 1.0, 2.0];
        assert_eq!(
            spread(&mut odd),
            Some(Spread {
                median: 2.0,
                low: 1.0,
                high: 3.0
            })
        );
        let mut even = [4.0, 1.0, 3.0, 2.0];
        assert_eq!(
            spread(&mut even),
            Some(Spread {
                median: 2.0,
                low: 1.0,
                high: 4.0
            })
        );
        assert_eq!(spread(&mut []), None);
    }
}
