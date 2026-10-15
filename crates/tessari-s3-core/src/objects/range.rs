//! A `Range: bytes=…` header resolved against an object's size: S3 serves a single range, ignores a malformed or
//! multi-range header (answering the whole object), and refuses an unsatisfiable one with 416.

/// What a Range header asks for, once the object's size is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeRequest {
    /// The whole object (no header, or one S3 ignores).
    Whole,
    /// Bytes `start..=end`, both within the object.
    Part {
        /// First byte.
        start: u64,
        /// Last byte, inclusive.
        end: u64,
    },
    /// The range cannot be satisfied: 416 InvalidRange.
    Unsatisfiable,
}

/// Resolves `header` (the raw `Range` value, if any) against an object of `size` bytes.
#[must_use]
pub fn resolve(header: Option<&str>, size: u64) -> RangeRequest {
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return RangeRequest::Whole;
    };
    if spec.contains(',') {
        return RangeRequest::Whole;
    }
    let Some((first, last)) = spec.split_once('-') else {
        return RangeRequest::Whole;
    };
    let number = |text: &str| -> Option<u64> {
        (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| text.parse().ok())
            .flatten()
    };
    let last_byte = size.checked_sub(1);
    match (first.trim(), last.trim()) {
        ("", suffix) => match (number(suffix), last_byte) {
            (Some(0), _) | (Some(_), None) => RangeRequest::Unsatisfiable,
            (Some(length), Some(end)) => RangeRequest::Part {
                start: size.saturating_sub(length),
                end,
            },
            (None, _) => RangeRequest::Whole,
        },
        (start, end) => {
            let Some(start) = number(start) else {
                return RangeRequest::Whole;
            };
            let end = if end.is_empty() {
                Some(u64::MAX)
            } else {
                number(end)
            };
            match (end, last_byte) {
                (None, _) => RangeRequest::Whole,
                (Some(end), _) if end < start => RangeRequest::Whole,
                (Some(_), None) => RangeRequest::Unsatisfiable,
                (Some(_), Some(last)) if start > last => RangeRequest::Unsatisfiable,
                (Some(end), Some(last)) => RangeRequest::Part {
                    start,
                    end: end.min(last),
                },
            }
        }
    }
}

/// One part's share of a byte range of an object made of consecutive parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// The part's position among the object's parts, from 0.
    pub index: usize,
    /// First byte within the part.
    pub start: u64,
    /// Last byte within the part, inclusive.
    pub end: u64,
}

/// The parts, in order, holding bytes `start..=end` of an object whose parts have `sizes`; empty parts are skipped.
#[must_use]
pub fn segments(sizes: &[u64], start: u64, end: u64) -> Vec<Segment> {
    let mut found = Vec::new();
    let mut offset: u64 = 0;
    for (index, size) in sizes.iter().copied().enumerate() {
        let Some(last) = offset
            .checked_add(size)
            .and_then(|next| next.checked_sub(1))
        else {
            continue;
        };
        if size > 0 && start <= last && end >= offset {
            found.push(Segment {
                index,
                start: start.saturating_sub(offset),
                end: end.min(last).saturating_sub(offset),
            });
        }
        offset = offset.saturating_add(size);
    }
    found
}

/// Bytes `start..=end` of the object that part `number` (from 1) covers, by position; `None` past the last part or
/// for an empty one.
#[must_use]
pub fn part_bytes(sizes: &[u64], number: usize) -> Option<(u64, u64)> {
    let index = number.checked_sub(1)?;
    let size = sizes.get(index).copied().filter(|size| *size > 0)?;
    let start = sizes
        .get(..index)?
        .iter()
        .try_fold(0_u64, |sum, s| sum.checked_add(*s))?;
    Some((start, start.checked_add(size)?.checked_sub(1)?))
}

#[cfg(test)]
mod tests {
    use super::{RangeRequest, Segment, part_bytes, resolve, segments};

    fn part(start: u64, end: u64) -> RangeRequest {
        RangeRequest::Part { start, end }
    }

    #[test]
    fn the_three_single_range_forms_resolve() {
        assert_eq!(resolve(Some("bytes=0-9"), 100), part(0, 9));
        assert_eq!(resolve(Some("bytes=90-"), 100), part(90, 99));
        assert_eq!(resolve(Some("bytes=-10"), 100), part(90, 99));
        assert_eq!(
            resolve(Some("bytes=95-500"), 100),
            part(95, 99),
            "an end past the object is clamped"
        );
        assert_eq!(
            resolve(Some("bytes=-500"), 100),
            part(0, 99),
            "a suffix longer than the object is all of it"
        );
    }

    #[test]
    fn a_start_past_the_end_or_a_zero_suffix_is_unsatisfiable() {
        assert_eq!(
            resolve(Some("bytes=100-"), 100),
            RangeRequest::Unsatisfiable
        );
        assert_eq!(resolve(Some("bytes=-0"), 100), RangeRequest::Unsatisfiable);
        assert_eq!(
            resolve(Some("bytes=0-0"), 0),
            RangeRequest::Unsatisfiable,
            "any range on an empty object"
        );
    }

    #[test]
    fn absent_malformed_or_multiple_ranges_mean_the_whole_object() {
        for header in [
            None,
            Some("bytes=5-1"),
            Some("items=0-1"),
            Some("bytes=a-b"),
            Some("bytes=0-1,4-5"),
            Some("bytes="),
        ] {
            assert_eq!(resolve(header, 100), RangeRequest::Whole, "{header:?}");
        }
    }

    #[test]
    fn a_range_splits_across_the_parts_it_covers() {
        let sizes = [10, 0, 5, 10];
        let seg = |index, start, end| Segment { index, start, end };
        assert_eq!(
            segments(&sizes, 0, 24),
            [seg(0, 0, 9), seg(2, 0, 4), seg(3, 0, 9)]
        );
        assert_eq!(
            segments(&sizes, 8, 11),
            [seg(0, 8, 9), seg(2, 0, 1)],
            "straddles a boundary"
        );
        assert_eq!(segments(&sizes, 15, 15), [seg(3, 0, 0)]);
        assert_eq!(segments(&sizes, 12, 13), [seg(2, 2, 3)], "inside one part");
        assert_eq!(part_bytes(&sizes, 1), Some((0, 9)));
        assert_eq!(part_bytes(&sizes, 3), Some((10, 14)));
        assert_eq!(part_bytes(&sizes, 4), Some((15, 24)));
        assert_eq!(part_bytes(&sizes, 2), None, "an empty part covers no bytes");
        assert_eq!(part_bytes(&sizes, 5), None);
        assert_eq!(part_bytes(&sizes, 0), None);
    }
}
