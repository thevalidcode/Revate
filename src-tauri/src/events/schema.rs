//! Event data schema and serialization.
//! Defines the structure and serialization format for recorded events.
//! TODO: Define comprehensive event schemas for all event types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub timestamp: f64,
    pub event_type: EventType,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventType {
    CursorMove,
    MouseDown,
    MouseUp,
    KeyDown,
    KeyUp,
    WindowFocusChange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum EventPayload {
    CursorMove { x: f64, y: f64 },
    MouseButton { button: String, state: String },
    KeyboardKey { key_code: u32, state: String },
    WindowFocus { title: String, rect: Rect },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
