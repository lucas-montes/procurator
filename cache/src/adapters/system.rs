//! The real clock and the OS random source.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::application::ports::{Clock, Entropy};

/// Wall clock and OS randomness.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEnv;

impl Clock for SystemEnv {
    fn now(&self) -> i64 {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        i64::try_from(secs).unwrap_or(i64::MAX)
    }
}

impl Entropy for SystemEnv {
    fn fill(&self, buf: &mut [u8]) {
        // Why: a token minted from predictable bytes is worse than a crash.
        getrandom::getrandom(buf).expect("OS random source is unavailable");
    }
}
