//! Cursor rendering.
//!
//! Turns a [`crate::input::cursor::CursorTrack`] into pixels: an arrow that
//! tracks the pointer, a ripple on each click, and per-segment style overrides
//! so a zoomed-in passage can carry a larger, higher-contrast cursor.
//!
//! # Two output paths, one rasterizer
//!
//! The editor's preview draws the cursor into a small canvas and the exporter
//! feeds a raw RGBA stream to FFmpeg's `overlay`. Both go through
//! [`Frame::blend`] and [`Frame::draw_cursor`], so what the user approves in the
//! preview is pixel-for-pixel what gets encoded — the alternative, a separate
//! `drawtext`/`drawbox` filter chain for the export, drifts from the preview
//! every time either side is tweaked.

pub mod asset;
pub mod renderer;
