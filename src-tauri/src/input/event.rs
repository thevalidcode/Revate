//! The parsed, ergonomic form of a recorded input event.
//!
//! [`crate::events::schema::RawEvent`] is the wire format; this is what the
//! rest of the app works with. The difference matters mostly for buttons: the
//! OS reports a button event with no position, so the tracker back-fills the
//! last known cursor location and `InputEvent` can then expose a single
//! `position()` that is meaningful for every variant.

use crate::events::schema::{KeyName, MouseButtonName};

/// A point in screen space (top-left origin), as the OS reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}

impl ScreenPoint {
    /// Straight-line distance to another point.
    pub fn distance_to(self, other: ScreenPoint) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

/// A recorded event, already resolved into something you can branch on.
#[derive(Debug, Clone)]
pub struct InputEvent {
    /// Milliseconds since the recording started — same origin as the video.
    pub t: f64,
    pub kind: InputEventKind,
}

#[derive(Debug, Clone)]
pub enum InputEventKind {
    CursorMove { at: ScreenPoint },
    MouseDown { button: MouseButtonName, at: ScreenPoint },
    MouseUp { button: MouseButtonName, at: ScreenPoint },
    KeyDown { key: KeyName },
    KeyUp { key: KeyName },
}

impl InputEvent {
    pub fn cursor_move(t: f64, at: ScreenPoint) -> Self {
        Self {
            t,
            kind: InputEventKind::CursorMove { at },
        }
    }

    pub fn mouse_down(t: f64, button: MouseButtonName, at: ScreenPoint) -> Self {
        Self {
            t,
            kind: InputEventKind::MouseDown { button, at },
        }
    }

    pub fn mouse_up(t: f64, button: MouseButtonName, at: ScreenPoint) -> Self {
        Self {
            t,
            kind: InputEventKind::MouseUp { button, at },
        }
    }

    pub fn key_down(t: f64, key: KeyName) -> Self {
        Self {
            t,
            kind: InputEventKind::KeyDown { key },
        }
    }

    pub fn key_up(t: f64, key: KeyName) -> Self {
        Self {
            t,
            kind: InputEventKind::KeyUp { key },
        }
    }

    /// Timestamp in milliseconds.
    pub fn t(&self) -> f64 {
        self.t
    }

    /// Timestamp in seconds, the unit zoom segments and FFmpeg use.
    pub fn seconds(&self) -> f64 {
        self.t / 1000.0
    }

    /// Where the pointer was, for any event that carries a position.
    pub fn position(&self) -> Option<ScreenPoint> {
        match &self.kind {
            InputEventKind::CursorMove { at }
            | InputEventKind::MouseDown { at, .. }
            | InputEventKind::MouseUp { at, .. } => Some(*at),
            InputEventKind::KeyDown { .. } | InputEventKind::KeyUp { .. } => None,
        }
    }

    /// True for a button press — the trigger for a click zoom.
    pub fn is_click(&self) -> bool {
        matches!(self.kind, InputEventKind::MouseDown { .. })
    }

    /// True for a button release.
    pub fn is_click_release(&self) -> bool {
        matches!(self.kind, InputEventKind::MouseUp { .. })
    }

    pub fn is_cursor_move(&self) -> bool {
        matches!(self.kind, InputEventKind::CursorMove { .. })
    }

    pub fn is_key(&self) -> bool {
        matches!(
            self.kind,
            InputEventKind::KeyDown { .. } | InputEventKind::KeyUp { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_is_euclidean() {
        let a = ScreenPoint { x: 0.0, y: 0.0 };
        let b = ScreenPoint { x: 3.0, y: 4.0 };
        assert!((a.distance_to(b) - 5.0).abs() < 1e-9);
        assert!((b.distance_to(a) - 5.0).abs() < 1e-9);
        assert!((a.distance_to(a)).abs() < 1e-9);
    }

    #[test]
    fn only_pointer_events_carry_a_position() {
        let at = ScreenPoint { x: 10.0, y: 20.0 };
        assert_eq!(
            InputEvent::cursor_move(0.0, at).position(),
            Some(at)
        );
        assert_eq!(
            InputEvent::mouse_down(0.0, MouseButtonName::Left, at).position(),
            Some(at)
        );
        assert_eq!(InputEvent::key_down(0.0, KeyName::Char("A".into())).position(), None);
    }

    #[test]
    fn classifies_clicks_and_keys() {
        let at = ScreenPoint { x: 1.0, y: 1.0 };
        assert!(InputEvent::mouse_down(0.0, MouseButtonName::Left, at).is_click());
        assert!(!InputEvent::mouse_up(0.0, MouseButtonName::Left, at).is_click());
        assert!(InputEvent::mouse_up(0.0, MouseButtonName::Left, at).is_click_release());
        assert!(InputEvent::key_up(0.0, KeyName::Named("Escape".into())).is_key());
        assert!(!InputEvent::cursor_move(0.0, at).is_key());
    }

    #[test]
    fn converts_milliseconds_to_seconds() {
        let event = InputEvent::cursor_move(1500.0, ScreenPoint { x: 0.0, y: 0.0 });
        assert!((event.seconds() - 1.5).abs() < 1e-9);
    }
}
