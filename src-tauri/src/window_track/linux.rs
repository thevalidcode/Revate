//! Linux-specific window tracking implementation.
//! Uses X11/Wayland APIs to enumerate and track windows.
//! TODO: Implement Linux window tracking using X11 or Wayland APIs.

#[cfg(target_os = "linux")]
pub fn get_window_list() -> Result<Vec<WindowInfo>, anyhow::Error> {
    todo!("Implement Linux window enumeration")
}

#[cfg(target_os = "linux")]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub rect: Rect,
}

#[cfg(target_os = "linux")]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
