//! Reading `events.revents` back into [`InputEvent`]s.
//!
//! Two entry points, because the caller usually has one of two things: a
//! session directory (the editor) or bytes it has already slurped (tests, and
//! the export path which reads the file once and keeps it).
//!
//! Parsing is deliberately forgiving. A recording that was killed mid-write can
//! end with a half-written line, and a take from an older build can contain a
//! record this version does not understand. Neither is a reason to refuse to
//! edit the video, so unparseable lines are counted and skipped.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::events::schema::{EventKind, MouseButtonName, RawEvent};
use crate::input::event::{InputEvent, InputEventKind, ScreenPoint};

/// The file a session's input trail lives in.
pub const EVENTS_FILE: &str = "events.revents";

/// What came back from a `.revents` file, including anything we chose to skip.
#[derive(Debug, Default, Clone)]
pub struct EventLog {
    /// Sorted by timestamp, ascending.
    pub events: Vec<InputEvent>,
    /// Lines that were blank, truncated, or from a newer schema.
    pub skipped: usize,
}

impl EventLog {
    /// True when there is nothing to plan with — the editor and the export both
    /// skip cursor overlay and auto-zoom silently in this case.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Just the pointer trail, in time order.
    pub fn pointer_events(&self) -> impl Iterator<Item = &InputEvent> {
        self.events.iter().filter(|e| e.position().is_some())
    }
}

/// Read `<dir>/events.revents`.
///
/// A missing file is **not** an error: it just means the take was recorded
/// without the input tracker (or predates it), and the caller carries on with
/// the video alone.
pub fn read_session_events(dir: &Path) -> EventLog {
    let path = dir.join(EVENTS_FILE);
    match fs::read(&path) {
        Ok(bytes) => parse_event_bytes(&bytes),
        Err(_) => EventLog::default(),
    }
}

/// Parse a whole `.revents` payload.
pub fn parse_event_bytes(bytes: &[u8]) -> EventLog {
    // `from_slice` on a &str needs valid UTF-8; a truncated multi-byte sequence
    // at the tail is exactly the kind of damage we expect from a hard kill.
    let text = String::from_utf8_lossy(bytes);
    parse_lines(&text)
}

/// Parse newline-delimited JSON, dropping what we cannot use.
pub fn parse_lines(text: &str) -> EventLog {
    let mut skipped = 0usize;
    // A BTree keyed by (t, arrival order) gives us a stable sort: two events
    // stamped in the same millisecond keep the order they were written in.
    let mut ordered: BTreeMap<(u64, usize), InputEvent> = BTreeMap::new();
    let mut arrival = 0usize;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        arrival += 1;

        let raw: RawEvent = match serde_json::from_str(trimmed) {
            Ok(raw) => raw,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if !raw.t.is_finite() || raw.t < 0.0 {
            skipped += 1;
            continue;
        }

        let Some(event) = into_input_event(&raw) else {
            skipped += 1;
            continue;
        };
        ordered.insert(((raw.t * 1000.0) as u64, arrival), event);
    }

    EventLog {
        events: ordered.into_values().collect(),
        skipped,
    }
}

/// Convert one wire event, dropping the ones that carry nothing usable.
///
/// A pointer event with a missing coordinate is unusable (we cannot draw or aim
/// at it), and a key event with no key name is equally useless, so both are
/// skipped rather than defaulted to `0,0` — a phantom cursor at the origin
/// would show up as a click in the corner of the video.
fn into_input_event(raw: &RawEvent) -> Option<InputEvent> {
    let data = raw.data.as_ref();
    let point = || -> Option<ScreenPoint> {
        Some(ScreenPoint {
            x: data?.x?,
            y: data?.y?,
        })
    };

    let kind = match raw.kind {
        EventKind::CursorMove => InputEventKind::CursorMove { at: point()? },
        EventKind::MouseDown => InputEventKind::MouseDown {
            button: data.and_then(|d| d.button).unwrap_or(MouseButtonName::Unknown),
            at: point()?,
        },
        EventKind::MouseUp => InputEventKind::MouseUp {
            button: data.and_then(|d| d.button).unwrap_or(MouseButtonName::Unknown),
            at: point()?,
        },
        EventKind::KeyDown => InputEventKind::KeyDown {
            key: data?.key.clone()?,
        },
        EventKind::KeyUp => InputEventKind::KeyUp {
            key: data?.key.clone()?,
        },
    };

    Some(InputEvent { t: raw.t, kind })
}

/// Resolve a session directory's event log, reporting a real error only when
/// the file exists but cannot be read at all.
pub fn read_session_events_strict(dir: &Path) -> Result<EventLog> {
    let path = dir.join(EVENTS_FILE);
    if !path.exists() {
        return Ok(EventLog::default());
    }
    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(parse_event_bytes(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::log::EventWriter;
    use crate::events::schema::KeyName;

    /// A scratch directory owned by one test.
    ///
    /// The name carries the process id *and* the calling test's own name: the
    /// harness runs tests in parallel threads inside a single process, so a
    /// directory keyed on the pid alone is shared between them, and one test's
    /// `remove_dir_all` deletes the file another test is mid-read on.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("revate-reader-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_log(name: &str) -> EventLog {
        let dir = scratch(name);
        let mut writer = EventWriter::create(&dir.join(EVENTS_FILE)).unwrap();
        writer.write(&RawEvent::cursor_move(100.0, 10.0, 10.0)).unwrap();
        writer
            .write(&RawEvent::button(
                50.0,
                EventKind::MouseDown,
                MouseButtonName::Left,
                20.0,
                20.0,
            ))
            .unwrap();
        writer
            .write(&RawEvent::key(75.0, EventKind::KeyDown, KeyName::Char("K".into())))
            .unwrap();
        writer.flush().unwrap();
        read_session_events(&dir)
    }

    #[test]
    fn parses_and_sorts_by_timestamp() {
        let log = sample_log("sorts");
        assert_eq!(log.events.len(), 3);
        assert_eq!(log.events[0].t, 50.0);
        assert_eq!(log.events[1].t, 75.0);
        assert_eq!(log.events[2].t, 100.0);
        assert_eq!(log.skipped, 0);
        std::fs::remove_dir_all(scratch("sorts")).ok();
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let log = read_session_events(Path::new("/definitely/not/here"));
        assert!(log.is_empty());
        assert_eq!(log.skipped, 0);
    }

    #[test]
    fn skips_a_truncated_tail_and_keeps_the_rest() {
        let text = concat!(
            r#"{"t":0.0,"type":"cursor_move","data":{"x":1.0,"y":2.0}}"#,
            "\n",
            r#"{"t":16.0,"type":"cursor_move","data":{"x":3.0,"y":4.0}}"#,
            "\n",
            r#"{"t":32.0,"type":"mouse_do"#,
            "\n",
        );
        let log = parse_lines(text);
        assert_eq!(log.events.len(), 2);
        assert_eq!(log.skipped, 1);
    }

    #[test]
    fn drops_pointer_events_without_coordinates() {
        let text = r#"{"t":0.0,"type":"mouse_down","data":{"button":"left"}}"#;
        let log = parse_lines(text);
        assert!(log.is_empty());
        assert_eq!(log.skipped, 1);
    }

    #[test]
    fn keeps_pointer_events_only_for_the_pointer_filter() {
        let log = sample_log("pointer");
        let pointer: Vec<_> = log.pointer_events().collect();
        assert_eq!(pointer.len(), 2);
        assert!(pointer.iter().all(|e| e.position().is_some()));
        std::fs::remove_dir_all(scratch("pointer")).ok();
    }

    #[test]
    fn same_millisecond_events_keep_write_order() {
        let text = concat!(
            r#"{"t":5.0,"type":"cursor_move","data":{"x":1.0,"y":1.0}}"#,
            "\n",
            r#"{"t":5.0,"type":"cursor_move","data":{"x":2.0,"y":2.0}}"#,
            "\n",
        );
        let log = parse_lines(text);
        assert_eq!(log.events[0].position().unwrap().x, 1.0);
        assert_eq!(log.events[1].position().unwrap().x, 2.0);
    }
}

