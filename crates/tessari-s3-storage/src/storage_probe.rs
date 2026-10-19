//! How long the metadata store takes to answer: one statement that reads nothing, timed from the send to the answer.

use std::time::{Duration, Instant};

use super::Storage;
use crate::Result;

impl Storage {
    /// The time the metadata store took to answer one statement now, the connection's own wait included.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn probe(&self) -> Result<Duration> {
        let started = Instant::now();
        self.pool.run("RETURN 1;", Vec::new()).await?;
        Ok(started.elapsed())
    }
}
