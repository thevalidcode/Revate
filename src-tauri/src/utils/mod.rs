//! Utility helpers.
//! Currently: the shared recording clock, on-disk paths for recording
//! projects, a thin `ffprobe` wrapper used to describe a capture, and the
//! capture-geometry sidecar that makes screen coordinates line up with the
//! video's own pixels.

pub mod capture_meta;
pub mod clock;
pub mod ffprobe;
pub mod paths;
