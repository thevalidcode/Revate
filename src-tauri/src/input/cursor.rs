//! Cursor tracking.
//! Handles cursor position sampling at high frequency (≥60 Hz).
//! TODO: Implement cursor position tracking.

pub struct CursorPosition {
    pub x: f64,
    pub y: f64,
}

pub fn get_cursor_position() -> Result<CursorPosition, anyhow::Error> {
    todo!("Implement cursor position retrieval")
}
