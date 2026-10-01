//! The `events.revents` writer.
//!
//! One JSON object per line, appended as events arrive and flushed on stop.
//! Writing is deliberately unbuffered-ish (a `BufWriter` with an explicit
//! `flush()` at the end): a recording that is killed mid-take should still
//! leave a file the editor can plan zooms from, and losing the tail of the
//! cursor trail costs nothing.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};

use super::schema::RawEvent;

/// Appends [`RawEvent`]s to a session's `events.revents`.
pub struct EventWriter {
    inner: BufWriter<File>,
    written: u64,
}

impl EventWriter {
    /// Create (or truncate) `path` and get a writer for it.
    pub fn create(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let file = File::create(path)
            .with_context(|| format!("failed to create {}", path.display()))?;
        Ok(Self {
            inner: BufWriter::new(file),
            written: 0,
        })
    }

    /// Append one event. Serialization cannot realistically fail for this
    /// schema, but a write error is reported rather than swallowed — a
    /// half-written trail would silently produce a worse edit.
    pub fn write(&mut self, event: &RawEvent) -> Result<()> {
        let line = serde_json::to_string(event)?;
        self.inner.write_all(line.as_bytes())?;
        self.inner.write_all(b"\n")?;
        self.written += 1;
        Ok(())
    }

    /// Number of events appended so far.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Push buffered bytes to the OS. Called on stop, and every so often during
    /// a take so a crash costs at most a few milliseconds of trail.
    pub fn flush(&mut self) -> Result<()> {
        self.inner.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::schema::EventKind;

    #[test]
    fn writes_one_json_object_per_line() {
        let path = std::env::temp_dir().join(format!("revate-writer-{}.revents", std::process::id()));
        let mut writer = EventWriter::create(&path).unwrap();
        writer.write(&RawEvent::cursor_move(0.0, 1.0, 2.0)).unwrap();
        writer
            .write(&RawEvent::button(
                16.0,
                EventKind::MouseDown,
                super::super::schema::MouseButtonName::Left,
                1.0,
                2.0,
            ))
            .unwrap();
        writer.flush().unwrap();
        assert_eq!(writer.written(), 2);

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains(r#""type":"cursor_move""#));
        assert!(lines[1].contains(r#""type":"mouse_down""#));

        std::fs::remove_file(&path).ok();
    }
}
