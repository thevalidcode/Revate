//! Video compositing and frame composition.
//! Combines video frames with overlays (cursor, zoom, redaction).
//! Renders the final composited frames for export.

pub mod frame;
pub mod renderer;
pub mod cursor_overlay;
pub mod spotlight;
