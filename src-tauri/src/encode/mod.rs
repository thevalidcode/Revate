//! Video encoding and export.
//! Handles FFmpeg integration for encoding composited frames into MP4.
//! Provides progress reporting and cancel support for long exports.

pub mod ffmpeg;
pub mod progress;
