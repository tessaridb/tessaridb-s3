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

#[cfg(test)]
mod tests {
    use super::{RangeRequest, resolve};

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
}
