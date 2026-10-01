//! Input event capture.
//!
//! [`tracker`] is the live half: it taps the OS while a take is running and
//! writes `events.revents`. [`event`] and [`cursor`] are the replay half — the
//! parsed trail, and the video-space sampling (spring smoothing, click ripples,
//! dwells) that the zoom planner and the cursor renderer build on.

pub mod cursor;
pub mod event;
pub mod tracker;
