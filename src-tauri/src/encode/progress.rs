//! Export progress tracking and reporting.
//! Parses FFmpeg stderr to extract progress information.
//! Provides progress updates for UI display.
//! TODO: Implement progress parsing and reporting.

pub fn parse_progress(stderr_line: &str) -> Option<ProgressUpdate> {
    todo!("Implement progress parsing")
}

pub fn calculate_progress(
    current_time: f64,
    total_duration: f64,
) -> f32 {
    todo!("Implement progress calculation")
}

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub percentage: f32,
    pub current_time: f64,
    pub total_duration: f64,
    pub fps: f32,
    pub speed: f32,
}
