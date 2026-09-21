//! Active window tracking.
//! Monitors window focus changes and tracks active window information.
//! Provides window metadata including title, position, and size.

pub mod windows;
pub mod macos;
pub mod linux;
pub mod focus;
