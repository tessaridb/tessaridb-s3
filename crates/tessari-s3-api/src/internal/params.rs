//! The parts of an internal shard request: the data id and shard index from the path, and decimal query parameters
//! from a fixed set per route — an unknown, repeated or malformed parameter is refused, never ignored.

use super::refusal::Refusal;

/// The data id: 32 lowercase hex digits.
pub(super) fn data_id(text: &str) -> Result<[u8; 16], Refusal> {
    let invalid = Refusal::Invalid("the data id is not 32 lowercase hex digits");
    let digits = text.as_bytes();
    let lowercase_hex = digits.len() == 32
        && digits
            .iter()
            .all(|digit| matches!(digit, b'0'..=b'9' | b'a'..=b'f'));
    if !lowercase_hex {
        return Err(invalid);
    }
    let mut id = [0_u8; 16];
    for (byte, pair) in id.iter_mut().zip(digits.as_chunks::<2>().0) {
        let pair = std::str::from_utf8(pair).map_err(|_| invalid)?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| invalid)?;
    }
    Ok(id)
}

/// The shard index.
pub(super) fn index(text: &str) -> Result<u16, Refusal> {
    decimal(text).ok_or(Refusal::Invalid("the shard index is not a number"))
}

/// The query's parameters, each named in `allowed` at most once, by name.
pub(super) struct Params<'a> {
    pairs: Vec<(&'a str, &'a str)>,
}

impl<'a> Params<'a> {
    /// Parses `query` against the names a route takes.
    pub(super) fn parse(query: Option<&'a str>, allowed: &[&str]) -> Result<Self, Refusal> {
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        for pair in query
            .unwrap_or("")
            .split('&')
            .filter(|pair| !pair.is_empty())
        {
            let (name, value) = pair
                .split_once('=')
                .ok_or(Refusal::Invalid("a query parameter has no value"))?;
            if !allowed.contains(&name) || pairs.iter().any(|(seen, _)| *seen == name) {
                return Err(Refusal::Invalid("an unknown or repeated query parameter"));
            }
            pairs.push((name, value));
        }
        Ok(Self { pairs })
    }

    /// The decimal value of `name`, when present.
    pub(super) fn optional<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>, Refusal> {
        self.pairs
            .iter()
            .find(|(seen, _)| *seen == name)
            .map(|(_, value)| {
                decimal(value).ok_or(Refusal::Invalid("a query parameter is not a number"))
            })
            .transpose()
    }

    /// The decimal value of `name`, which must be present.
    pub(super) fn required<T: std::str::FromStr>(&self, name: &str) -> Result<T, Refusal> {
        self.optional(name)?
            .ok_or(Refusal::Invalid("a required query parameter is missing"))
    }
}

/// Digits only: no sign, no space, no leading plus, which `FromStr` would otherwise take.
fn decimal<T: std::str::FromStr>(text: &str) -> Option<T> {
    let digits = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    digits.then(|| text.parse().ok()).flatten()
}
