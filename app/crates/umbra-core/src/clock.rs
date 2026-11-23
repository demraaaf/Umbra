//! The recording clock. Video frames and audio packets are both placed on one timeline
//! that starts when the recorder starts, so they stay in sync in the saved clip.

use std::time::Instant;

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub epoch: Instant,
    /// The epoch on the QPC clock, in 100 ns units. WASAPI stamps audio on this clock.
    epoch_qpc_100ns: i64,
}

/// The QPC clock now, in 100 ns units.
pub fn qpc_100ns() -> i64 {
    let (mut counter, mut frequency) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut counter);
        let _ = QueryPerformanceFrequency(&mut frequency);
    }
    if frequency == 0 {
        return 0;
    }
    (counter as i128 * 10_000_000 / frequency as i128) as i64
}

/// The QPC clock now, in seconds: what input and the password guard are stamped with.
pub fn qpc_secs() -> f64 {
    qpc_100ns() as f64 / 1e7
}

impl Clock {
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            epoch_qpc_100ns: qpc_100ns(),
        }
    }

    /// Seconds since the epoch.
    pub fn now_secs(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    /// Seconds from the epoch to `at`; negative before it.
    pub fn secs_at(&self, at: Instant) -> f64 {
        match at.checked_duration_since(self.epoch) {
            Some(after) => after.as_secs_f64(),
            None => -(self.epoch - at).as_secs_f64(),
        }
    }

    /// What to add to a time in QPC seconds to have it in seconds since the epoch.
    pub fn qpc_offset(&self) -> f64 {
        -(self.epoch_qpc_100ns as f64) / 1e7
    }

    /// Converts a WASAPI QPC position (100 ns units) to seconds since the epoch.
    pub fn secs_from_qpc(&self, qpc_100ns: u64) -> f64 {
        (qpc_100ns as i64 - self.epoch_qpc_100ns) as f64 / 1e7
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}
