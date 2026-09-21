//! Idle detection.
//! Detects periods of inactivity (no input + no screen change).
//! Uses frame difference analysis and input monitoring to identify idle segments.
//! TODO: Implement idle detection algorithms.

pub fn detect_idle(
    frames: &[Frame],
    events: &[Event],
    threshold: f64,
    duration: f64,
) -> Result<Vec<IdleSegment>, anyhow::Error> {
    todo!("Implement idle detection")
}

#[derive(Debug, Clone)]
pub struct IdleSegment {
    pub start_time: f64,
    pub end_time: f64,
}

pub struct Frame {
    pub data: Vec<u8>,
    pub timestamp: f64,
}

use super::super::events::schema::Event;
