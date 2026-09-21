//! Keyboard event tracking.
//! Captures key down/up events with key codes only (never characters).
//! Ignores key repeats during recording.
//! TODO: Implement keyboard event capture.

pub struct KeyboardEvent {
    pub key_code: u32,
    pub state: KeyState,
    pub timestamp: f64,
}

pub enum KeyState {
    Down,
    Up,
}

pub fn capture_keyboard_event() -> Result<KeyboardEvent, anyhow::Error> {
    todo!("Implement keyboard event capture")
}
