//! The rungs of an adaptive stream: video heights from [`VIDEO_SIZES`], each at most once, spelled ascending and
//! joined by `+` (`360p+720p+1080p`).

use super::VIDEO_SIZES;

/// A non-empty set of [`VIDEO_SIZES`], bit `i` standing for `VIDEO_SIZES[i]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ladder(u8);

impl Ladder {
    /// The ladder spelled `name`: rungs `<lines>p` from the list, strictly ascending, joined by `+`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        let mut bits = 0_u8;
        let mut highest = None;
        for rung in name.split('+') {
            let lines: u32 = rung.strip_suffix('p')?.parse().ok()?;
            let index = VIDEO_SIZES.iter().position(|size| *size == lines)?;
            if highest.is_some_and(|below| index <= below) {
                return None;
            }
            highest = Some(index);
            bits |= 1_u8.checked_shl(u32::try_from(index).ok()?)?;
        }
        (bits != 0).then_some(Self(bits))
    }

    /// The heights, ascending.
    pub fn rungs(self) -> impl Iterator<Item = u32> {
        VIDEO_SIZES
            .into_iter()
            .enumerate()
            .filter(move |(index, _)| {
                u32::try_from(*index)
                    .ok()
                    .and_then(|index| 1_u8.checked_shl(index))
                    .is_some_and(|bit| self.0 & bit != 0)
            })
            .map(|(_, lines)| lines)
    }

    /// Its spelling.
    #[must_use]
    pub fn name(self) -> String {
        self.rungs()
            .map(|lines| format!("{lines}p"))
            .collect::<Vec<_>>()
            .join("+")
    }
}
