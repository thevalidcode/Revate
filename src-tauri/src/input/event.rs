//! Input event types and structures.
//! Defines common event structures for all input events (keyboard, mouse, cursor).
//! TODO: Define comprehensive event types for the recording system.

#[derive(Debug, Clone)]
pub struct InputEvent {
    pub timestamp: f64,
    pub event_type: InputEventType,
}

#[derive(Debug, Clone)]
pub enum InputEventType {
    CursorMove { x: f64, y: f64 },
    MouseDown { button: MouseButton },
    MouseUp { button: MouseButton },
    KeyDown { key_code: u32 },
    KeyUp { key_code: u32 },
}

#[derive(Debug, Clone, Copy)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u8),
}
