//! Redaction region tracking.
//! Tracks moving regions across frames for dynamic redaction.
//! Uses template matching or optical flow for region following.
//! TODO: Implement region tracking for moving redaction areas.

pub fn track_region(
    region: &Region,
    frames: &[Frame],
) -> Result<Vec<TrackedRegion>, anyhow::Error> {
    todo!("Implement region tracking")
}

#[derive(Debug, Clone)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone)]
pub struct TrackedRegion {
    pub frame_index: usize,
    pub region: Region,
}

pub struct Frame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
