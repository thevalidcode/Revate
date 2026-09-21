//! Auto-zoom functionality.
//! Implements cursor-following auto-zoom on clicks and dwell.
//! Uses critically-damped springs for smooth motion and configurable zoom levels.

pub mod planner;
pub mod spring;
pub mod easing;
pub mod viewport;
