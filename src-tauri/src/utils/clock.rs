//! One monotonic time base for everything a recording produces.
//!
//! The video process, the input tracker and the audio devices all start within
//! a few milliseconds of each other, but none of them can share a clock handle.
//! What they *can* share is this: a single `Instant` taken once, before any
//! thread is spawned, and stamped onto every timestamp we write.
//!
//! Timestamps are milliseconds since that origin (`t = 0` is "the moment the
//! user pressed record"), which is exactly what `.revents` stores and what the
//! zoom planner and cursor sampler consume later.

use std::sync::Arc;
use std::time::Instant;

/// A monotonic origin plus a way to read elapsed time against it.
#[derive(Debug, Clone)]
pub struct Clock {
    origin: Instant,
}

impl Clock {
    /// Start a new time base. Take this *before* spawning any capture thread.
    pub fn start() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    /// Milliseconds since the origin. Monotonic, and unaffected by wall-clock
    /// adjustments — which `SystemTime` would be.
    pub fn now_ms(&self) -> f64 {
        self.origin.elapsed().as_secs_f64() * 1000.0
    }

    /// Seconds since the origin, for the places that talk in seconds (FFmpeg's
    /// `t`, zoom segment bounds, easing curves).
    pub fn now_secs(&self) -> f64 {
        self.now_ms() / 1000.0
    }
}

/// Shared form, so capture threads can hold a clock without owning it.
pub type SharedClock = Arc<Clock>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_near_zero_and_never_goes_backwards() {
        let clock = Clock::start();
        let first = clock.now_ms();
        assert!(first < 50.0, "a fresh clock should read ~0, got {first}");

        std::thread::sleep(std::time::Duration::from_millis(20));
        let second = clock.now_ms();
        assert!(second > first, "time must advance: {first} -> {second}");
    }

    #[test]
    fn seconds_track_milliseconds() {
        let clock = Clock::start();
        std::thread::sleep(std::time::Duration::from_millis(30));
        // Two separate reads, so they can differ by however long the calls take;
        // what matters is that they agree to within the sampling gap.
        let ms = clock.now_ms();
        let secs = clock.now_secs();
        assert!((ms / 1000.0 - secs).abs() < 0.001);
    }
}
