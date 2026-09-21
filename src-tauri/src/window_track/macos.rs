//! macOS-specific window tracking implementation.
//! Uses macOS APIs to enumerate and track windows.
//! TODO: Implement macOS window tracking using Cocoa/AppKit APIs.

#[cfg(target_os = "macos")]
pub fn get_window_list() -> Result<Vec<WindowInfo>, anyhow::Error> {
    todo!("Implement macOS window enumeration")
}

#[cfg(target_os = "macos")]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub rect: Rect,
}

#[cfg(target_os = "macos")]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
