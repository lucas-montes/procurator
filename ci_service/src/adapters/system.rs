//! The real clock.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::application::ports::Clock;

/// Wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        i64::try_from(secs).unwrap_or(i64::MAX)
    }
}
