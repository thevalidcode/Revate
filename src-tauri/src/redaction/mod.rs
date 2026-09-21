//! Privacy redaction and auto-blur.
//! Handles marking and blurring regions for privacy (e.g., passwords, sensitive info).
//! Supports static regions and window-locked regions.

pub mod marker;
pub mod tracker;
pub mod blur;
