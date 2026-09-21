//! Zoom planning and segment management.
//! Plans zoom segments based on input events (clicks, dwell, window changes).
//! Merges overlapping or nearby zoom segments for smooth playback.
//! TODO: Implement zoom segment planning logic.

pub fn plan_zoom_segments(events: &[Event]) -> Result<Vec<ZoomSegment>, anyhow::Error> {
    todo!("Implement zoom planning")
}

#[derive(Debug, Clone)]
pub struct ZoomSegment {
    pub start_time: f64,
    pub end_time: f64,
    pub target_x: f64,
    pub target_y: f64,
    pub zoom_level: f64,
}

use super::super::events::schema::Event;
