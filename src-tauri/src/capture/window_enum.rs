//! Window enumeration and selection.
//! Provides functionality to list available windows and select specific windows for capture.
//! TODO: Implement window enumeration using platform-specific APIs.

pub fn enumerate_windows() -> Result<Vec<WindowInfo>, anyhow::Error> {
    todo!("Implement window enumeration")
}

pub struct WindowInfo {
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
