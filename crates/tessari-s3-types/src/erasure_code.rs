//! An erasure code RS(k, m): k data shards and m parity shards, any k of which hold the object.

use tessari_s3_constants::ERASURE_MAX_WIDTH as WIDEST;

/// Why a code was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CodeError {
    /// Not written as `k+m` with two decimal counts.
    #[error("an erasure code is written as k+m, for example 4+2")]
    Syntax,
    /// No data shard, no parity shard, or more than sixteen shards.
    #[error("{data}+{parity} is not a code: k and m must each be at least 1 and k+m at most 16")]
    Shape {
        /// The data shard count asked for.
        data: u8,
        /// The parity shard count asked for.
        parity: u8,
    },
}

/// RS(k, m): `data` shards carry the bytes, `parity` shards let any `data` of the `data + parity` rebuild them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Code {
    data: u8,
    parity: u8,
}

impl Code {
    /// A code of `data` data shards and `parity` parity shards.
    ///
    /// # Errors
    /// [`CodeError::Shape`] when either count is zero or the total exceeds sixteen.
    pub fn new(data: u8, parity: u8) -> Result<Self, CodeError> {
        let fits = data
            .checked_add(parity)
            .is_some_and(|width| width <= WIDEST);
        if data == 0 || parity == 0 || !fits {
            return Err(CodeError::Shape { data, parity });
        }
        Ok(Self { data, parity })
    }

    /// A code written as `k+m`.
    ///
    /// # Errors
    /// [`CodeError::Syntax`] when the text is not two counts joined by `+`; [`CodeError::Shape`] as [`Code::new`].
    pub fn parse(text: &str) -> Result<Self, CodeError> {
        let count = |part: &str| {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(CodeError::Syntax);
            }
            part.parse::<u8>().map_err(|_| CodeError::Syntax)
        };
        let (data, parity) = text.split_once('+').ok_or(CodeError::Syntax)?;
        Self::new(count(data)?, count(parity)?)
    }

    /// k, the data shards.
    #[must_use]
    pub const fn data(self) -> u8 {
        self.data
    }

    /// m, the parity shards.
    #[must_use]
    pub const fn parity(self) -> u8 {
        self.parity
    }

    /// k + m, the shards one object is cut into.
    #[must_use]
    pub const fn width(self) -> u8 {
        // At most sixteen, checked by the constructor.
        self.data.saturating_add(self.parity)
    }

    /// The verified shards a read needs: k.
    #[must_use]
    pub const fn read_quorum(self) -> u8 {
        self.data
    }

    /// The durable shards a write needs before it is acknowledged: k, and k + 1 when k = m, so two writers can never
    /// each hold a disjoint half of the set.
    #[must_use]
    pub const fn write_quorum(self) -> u8 {
        if self.data == self.parity {
            self.data.saturating_add(1)
        } else {
            self.data
        }
    }
}

#[cfg(test)]
#[path = "erasure_code_tests.rs"]
mod tests;
