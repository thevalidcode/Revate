//! The on-disk event schema — one JSON object per line in `events.revents`.
//!
//! The format is deliberately flat and hand-readable, because a `.revents` file
//! is the kind of thing you want to be able to `head` when a take looks wrong:
//!
//! ```json
//! {"t":0.0,"type":"cursor_move","data":{"x":812.5,"y":433.25}}
//! {"t":3417.0,"type":"mouse_down","data":{"x":812.5,"y":433.25,"button":"left"}}
//! {"t":3417.0,"type":"key_down","data":{"key":"char:R"}}
//! ```
//!
//! `t` is milliseconds since the shared [`crate::utils::clock::Clock`] origin,
//! i.e. since the user pressed record. Coordinates are **screen** points in
//! top-left-origin space (what the OS reports), not video pixels — converting
//! into the recording's own geometry needs `capture.json`, which is written
//! alongside the video at record time.

use serde::{Deserialize, Serialize};

/// One recorded input event, as stored in `events.revents`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvent {
    /// Milliseconds since the recording started.
    pub t: f64,
    /// `cursor_move`, `mouse_down`, … Serialized as `type` (a Rust keyword).
    #[serde(rename = "type")]
    pub kind: EventKind,
    /// Type-specific fields. Omitted entirely when there is nothing to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<EventData>,
}

impl RawEvent {
    /// A cursor sample in screen space.
    pub fn cursor_move(t: f64, x: f64, y: f64) -> Self {
        Self {
            t,
            kind: EventKind::CursorMove,
            data: Some(EventData {
                x: Some(x),
                y: Some(y),
                ..EventData::default()
            }),
        }
    }

    /// A button press/release, positioned at the last known cursor location —
    /// the OS does not include a position on button events.
    pub fn button(t: f64, kind: EventKind, button: MouseButtonName, x: f64, y: f64) -> Self {
        Self {
            t,
            kind,
            data: Some(EventData {
                x: Some(x),
                y: Some(y),
                button: Some(button),
                ..EventData::default()
            }),
        }
    }

    /// A key press/release. Only the physical key is stored, never the
    /// character it produced, so typed text stays out of the file.
    pub fn key(t: f64, kind: EventKind, key: KeyName) -> Self {
        Self {
            t,
            kind,
            data: Some(EventData {
                key: Some(key),
                ..EventData::default()
            }),
        }
    }
}

/// Which kind of input produced an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    CursorMove,
    MouseDown,
    MouseUp,
    KeyDown,
    KeyUp,
}

impl EventKind {
    /// True for the two button events, which are what triggers a zoom.
    pub fn is_click(self) -> bool {
        matches!(self, Self::MouseDown | Self::MouseUp)
    }

    /// True for the two key events.
    pub fn is_key(self) -> bool {
        matches!(self, Self::KeyDown | Self::KeyUp)
    }
}

/// Pointer buttons, as reported by the platform layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButtonName {
    Left,
    Right,
    Middle,
    Unknown,
}

/// A physical key, named by its position rather than by layout or character.
///
/// Serialized as a tagged *string* — `"char:R"`, `"named:Escape"`,
/// `"other:65"` — rather than serde's default externally-tagged object form.
/// That keeps a `.revents` file greppable and keeps the `data` object flat, which
/// is what the format at the top of this file promises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyName {
    Char(String),
    /// Escape, Enter, Tab, arrows, …
    Named(String),
    Other(u32),
}

impl KeyName {
    /// Build from a platform key name (`"KeyA"`, `"Escape"`, …).
    pub fn from_platform(name: &str) -> Self {
        let letter = match name.strip_prefix("Key") {
            Some(rest) if rest.len() == 1 => rest.to_string(),
            _ => match name.strip_prefix("Digit") {
                Some(rest) if rest.len() == 1 => rest.to_string(),
                _ => return Self::Named(name.to_string()),
            },
        };
        Self::Char(letter)
    }

    /// The tagged string written to disk.
    fn to_wire(&self) -> String {
        match self {
            Self::Char(name) => format!("char:{name}"),
            Self::Named(name) => format!("named:{name}"),
            Self::Other(code) => format!("other:{code}"),
        }
    }

    /// Read back [`Self::to_wire`].
    ///
    /// An unrecognised or malformed tag is kept as a [`Self::Named`] rather than
    /// rejected, so a file written by a newer build still loads — the key is
    /// simply described less precisely.
    fn from_wire(text: &str) -> Self {
        match text.split_once(':') {
            Some(("char", name)) => Self::Char(name.to_string()),
            Some(("named", name)) => Self::Named(name.to_string()),
            Some(("other", code)) => match code.parse() {
                Ok(code) => Self::Other(code),
                Err(_) => Self::Named(text.to_string()),
            },
            _ => Self::Named(text.to_string()),
        }
    }
}

impl Serialize for KeyName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_wire())
    }
}

impl<'de> Deserialize<'de> for KeyName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(Self::from_wire(&text))
    }
}

/// The `data` object. Every field is optional because a given event kind only
/// fills in what applies to it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<MouseButtonName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<KeyName>,
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_flat_with_a_type_tag() {
        let event = RawEvent::cursor_move(12.5, 800.0, 600.0);
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(
            json,
            r#"{"t":12.5,"type":"cursor_move","data":{"x":800.0,"y":600.0}}"#
        );
    }

    #[test]
    fn round_trips_a_click() {
        let event = RawEvent::button(
            3417.0,
            EventKind::MouseDown,
            MouseButtonName::Left,
            812.5,
            433.25,
        );
        let line = serde_json::to_string(&event).unwrap();
        let back: RawEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(back.t, 3417.0);
        assert_eq!(back.kind, EventKind::MouseDown);
        let data = back.data.unwrap();
        assert_eq!(data.x, Some(812.5));
        assert_eq!(data.y, Some(433.25));
        assert_eq!(data.button, Some(MouseButtonName::Left));
    }

    #[test]
    fn key_events_carry_a_position_free_payload() {
        let event = RawEvent::key(5.0, EventKind::KeyDown, KeyName::from_platform("KeyR"));
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, r#"{"t":5.0,"type":"key_down","data":{"key":"char:R"}}"#);
    }

    #[test]
    fn named_keys_stay_named() {
        assert_eq!(KeyName::from_platform("Escape"), KeyName::Named("Escape".into()));
        assert_eq!(KeyName::from_platform("Digit7"), KeyName::Char("7".into()));
    }

    #[test]
    fn every_key_variant_round_trips_through_its_tagged_string() {
        for key in [
            KeyName::Char("R".into()),
            KeyName::Named("Escape".into()),
            KeyName::Other(65),
        ] {
            let line = serde_json::to_string(&key).unwrap();
            let back: KeyName = serde_json::from_str(&line).unwrap();
            assert_eq!(back, key, "round trip failed for {line}");
        }
    }

    #[test]
    fn an_unrecognised_key_tag_still_loads() {
        // A newer build writing `"gesture:Fn"` must not make an old file
        // unreadable — the key survives, just described less precisely.
        let back: KeyName = serde_json::from_str(r#""gesture:Fn""#).unwrap();
        assert_eq!(back, KeyName::Named("gesture:Fn".into()));
    }
}
