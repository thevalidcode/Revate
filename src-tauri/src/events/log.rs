//! Event logging.
//! Records input and window events during a recording session.
//! TODO: Implement event logging with high-resolution timestamps.

pub fn log_event(event: Event) -> Result<(), anyhow::Error> {
    todo!("Implement event logging")
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Event {
    pub timestamp: f64,
    pub event_type: String,
    pub data: serde_json::Value,
}

use serde::{Deserialize, Serialize};
