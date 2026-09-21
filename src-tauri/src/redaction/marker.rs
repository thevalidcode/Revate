//! Redaction markers and regions.
//! Defines regions to be blurred/redacted during export.
//! Supports static rectangles and window-locked regions.
//! TODO: Implement redaction marker management.

pub struct RedactionMarker {
    pub id: u64,
    pub region: Region,
    pub type_: RedactionType,
    pub start_time: f64,
    pub end_time: f64,
}

pub enum RedactionType {
    Static,
    WindowLock { window_id: u64 },
}

pub struct Region {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub fn add_marker(marker: RedactionMarker) -> Result<(), anyhow::Error> {
    todo!("Implement marker addition")
}
