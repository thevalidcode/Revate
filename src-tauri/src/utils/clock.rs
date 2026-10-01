//! High-resolution clock and timing utilities.
//! Provides monotonic time measurement for precise event timing.
use std::time::Instant;

#[derive(Clone)]
pub struct Clock {
    origin: Instant,
}

impl Clock {
    pub fn new() -> Self {
        Self { origin: Instant::now() }
    }

    pub fn now_secs(&self) -> f64 {
        self.origin.elapsed().as_secs_f64()
    }
}

impl Default for Clock {
    fn default() -> Self { Self::new() }
}