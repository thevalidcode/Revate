//! Event logging and storage.
//!
//! One recording session's input trail is a single `events.revents` file —
//! newline-delimited JSON written live by [`crate::input::tracker`] and read
//! back by [`reader`] when the editor or the exporter needs it.

pub mod log;
pub mod reader;
pub mod schema;
