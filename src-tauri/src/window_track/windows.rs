//! Window tracking utilities.
//! Cross-platform abstractions for window information retrieval.
//! TODO: Implement cross-platform window tracking.

pub fn get_active_window() -> Result<ActiveWindow, anyhow::Error> {
    todo!("Implement active window retrieval")
}

pub struct ActiveWindow {
    pub id: u64,
    pub title: String,
    pub rect: Rect,
}

pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
