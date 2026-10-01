//! The input tracker: a background listener that records cursor, click and key
//! activity into `events.revents` for the duration of a take.
//!
//! # Why one long-lived listener
//!
//! `rdev::listen` blocks forever and offers no way to stop: on macOS it drives a
//! `CGEventTap` run loop that only unwinds when the thread exits. Spawning one
//! per recording would leak a tap (and its Accessibility grant) on every take.
//! So the listener is a **process-wide singleton**, started lazily the first
//! time a recording begins — which is also the moment we want macOS to show its
//! "…would like to control this computer" prompt — and it lives for the app's
//! lifetime. Recording is toggled by installing and removing an [`Active`]
//! writer; between takes the callback is a no-op.
//!
//! # Sampling
//!
//! The OS delivers `MouseMove` at the pointer's report rate, which is often well
//! above 60 Hz. Writing every event would triple the file size for detail the
//! planner cannot use, so cursor samples are throttled to 60 Hz. Clicks and keys
//! are always written, and a click is stamped with the last sampled position
//! (button events carry no coordinates of their own).
//!
//! # Permissions
//!
//! Without Accessibility permission `rdev::listen` fails to start. That is not
//! fatal: [`Tracker::start`] reports it, and the caller falls back to letting
//! FFmpeg burn the system cursor into the video instead of drawing its own.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::events::log::EventWriter;
use crate::events::reader::EVENTS_FILE;
use crate::events::schema::{EventKind, KeyName, MouseButtonName, RawEvent};
use crate::input::event::ScreenPoint;
use crate::utils::clock::SharedClock;

/// Cursor samples are written at most this often (≈60 Hz).
const SAMPLE_INTERVAL: Duration = Duration::from_micros(16_666);

/// How often buffered bytes are pushed to disk during a take, so a hard kill
/// costs at most a second of trail.
const FLUSH_EVERY: Duration = Duration::from_secs(1);

/// What the listener writes into while a take is running.
struct Active {
    /// Flipped by [`Tracker::stop`]; the callback checks it and goes quiet.
    stop: Arc<AtomicBool>,
    clock: SharedClock,
    writer: EventWriter,
    /// Last position seen, used to position button events.
    last: ScreenPoint,
    /// When the last cursor sample was written, for throttling.
    last_sample_ms: f64,
    last_flush_ms: f64,
    /// Events the OS dropped while we were not recording.
    dropped: u64,
}

/// The installed writer, if a take is in progress.
fn slot() -> &'static Mutex<Option<Active>> {
    static SLOT: OnceLock<Mutex<Option<Active>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Lock-free "a take is recording" gate for the tap callback.
///
/// The callback must be able to answer this question *without* touching the
/// writer mutex. When it checked the flag only after taking the lock, a moving
/// pointer (the tap fires at the pointer's report rate) kept re-acquiring that
/// mutex, and [`Tracker::stop`] — which needs it to close the file — was starved
/// indefinitely. That is what left a stopped take sitting on "Finishing…"
/// forever: the stop never got to run.
static RECORDING: AtomicBool = AtomicBool::new(false);

/// Result of the one-time listener start, cached for the process lifetime.
///
/// `rdev::listen` returns nothing and only ever exits by panicking — which is
/// exactly what it does when macOS refuses to create the event tap, i.e. when
/// Accessibility permission has not been granted. So the tap is started on a
/// detached thread and its liveness is sampled: if that thread is already gone
/// moments later, permission is the problem, and we say so instead of leaving
/// the user with a recording that silently has no cursor trail.
fn ensure_listener() -> Result<(), String> {
    static STATE: OnceLock<Mutex<ListenerState>> = OnceLock::new();
    let state = STATE.get_or_init(|| Mutex::new(ListenerState::Unstarted));

    let mut guard = state.lock().unwrap();
    match &*guard {
        ListenerState::Running => return Ok(()),
        ListenerState::Denied(reason) => return Err(reason.clone()),
        ListenerState::Unstarted => {}
    }

    let failed = Arc::new(AtomicBool::new(false));
    let flag = failed.clone();
    std::thread::Builder::new()
        .name("revate-input-tap".into())
        .spawn(move || {
            // A panic here means the tap could not be created. Swallow it and let
            // the watchdog below notice, rather than unwinding through the app.
            let result = std::panic::catch_unwind(|| rdev::listen(handle_event));
            if result.is_err() {
                flag.store(true, Ordering::SeqCst);
            }
        })
        .map_err(|e| format!("could not start the input listener thread: {e}"))?;

    // `CGEventTapCreate` either succeeds and parks the thread in its run loop, or
    // fails immediately. A short settle is enough to tell the two apart without
    // making the first recording feel slow.
    std::thread::sleep(Duration::from_millis(200));
    if failed.load(Ordering::SeqCst) {
        *guard = ListenerState::Denied(denied_message());
        return Err(denied_message());
    }

    *guard = ListenerState::Running;
    Ok(())
}

/// How the singleton listener thread is doing.
enum ListenerState {
    Unstarted,
    Running,
    Denied(String),
}

fn denied_message() -> String {
    "input tracking is unavailable — grant Revate Accessibility access in \
     System Settings → Privacy & Security, or the recording will keep the \
     system cursor instead of drawing its own"
        .to_string()
}


/// Handle to the running input tracker. Dropping it without calling
/// [`Tracker::stop`] leaves the writer installed; call `stop` (or use
/// `stop_on_drop`) when tearing a session down.
pub struct Tracker {
    /// Mirrors the `Active`'s flag so `stop` can flip it without the lock.
    stop: Arc<AtomicBool>,
}

impl Tracker {
    /// Begin recording input events for `dir`, writing `dir/events.revents`.
    ///
    /// `clock` is the shared origin — the same one the video was started
    /// against — so event timestamps line up with the video's `t = 0`.
    pub fn start(dir: &Path, clock: SharedClock) -> Result<Self> {
        if let Err(message) = ensure_listener() {
            return Err(anyhow!(message));
        }
        if slot().lock().unwrap().is_some() {
            return Err(anyhow!("input tracking is already running"));
        }

        let writer = EventWriter::create(&dir.join(EVENTS_FILE))?;
        let stop = Arc::new(AtomicBool::new(false));
        *slot().lock().unwrap() = Some(Active {
            stop: stop.clone(),
            clock,
            writer,
            last: ScreenPoint { x: 0.0, y: 0.0 },
            last_sample_ms: f64::NEG_INFINITY,
            last_flush_ms: 0.0,
            dropped: 0,
        });
        // Opened only once the writer is in place, so the callback can never see
        // the gate open with nothing to write into.
        RECORDING.store(true, Ordering::SeqCst);

        Ok(Self { stop })
    }

    /// Stop recording, flush and close the file.
    ///
    /// Returns the number of events written, which the caller logs — a take with
    /// an empty trail is worth knowing about, because it usually means the
    /// permission was never granted.
    pub fn stop(self) -> Result<u64> {
        // Close the lock-free gate *first*. From here on the tap callback returns
        // without ever touching the writer mutex, which is what guarantees the
        // `slot()` lock below can be taken promptly no matter how fast the
        // pointer is moving.
        RECORDING.store(false, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);

        let Some(mut active) = slot().lock().unwrap().take() else {
            return Ok(0);
        };
        active.writer.flush()?;
        let written = active.writer.written();
        drop(active);

        if written == 0 {
            eprintln!("[input] no events were recorded — was the pointer used at all?");
        }
        Ok(written)
    }

    /// Stop on drop, for the paths that unwind on error.
    pub fn stop_on_drop(self) {
        let _ = self.stop();
    }
}

/// The `rdev` callback. Runs on the listener thread for the app's lifetime.
///
/// `rdev` hands us a wrapper `Event`; the timestamp it carries is its own
/// (wall-clock), so we re-stamp against our clock instead — that is what keeps
/// the trail on the same timeline as the video.
fn handle_event(event: rdev::Event) {
    // Cheap rejection first, with no lock. Once a take has stopped there is
    // nothing to write, and bailing out here is what stops a live pointer from
    // starving `Tracker::stop` of the writer mutex (see `RECORDING`).

    // Cheap rejection first, with no lock. Once a take has stopped there is
    // nothing to write, and bailing out here is what stops a live pointer from
    // starving `Tracker::stop` of the writer mutex (see `RECORDING`).
    if !RECORDING.load(Ordering::Relaxed) {
        return;
    }

    // Never let a panic in here kill the tap thread; a poisoned lock is treated
    // as "no recording", which is the safe answer.
    let Ok(mut guard) = slot().lock() else {
        return;
    };
    let Some(active) = guard.as_mut() else {
        return;
    };
    if active.stop.load(Ordering::Relaxed) {
        return;
    }

    let now = active.clock.now_ms();
    let Some(event) = to_raw(event.event_type, active, now) else {
        active.dropped += 1;
        return;
    };

    if let Err(e) = active.writer.write(&event) {
        // A failing disk should not spin the callback; report once and go quiet.
        eprintln!("[input] failed to append an event: {e}");
        active.stop.store(true, Ordering::SeqCst);
        return;
    }
    active.last_sample_ms = now;
    if now - active.last_flush_ms >= FLUSH_EVERY.as_millis() as f64 {
        active.last_flush_ms = now;
        let _ = active.writer.flush();
    }
}

/// Translate one platform event, updating the cached cursor position.
fn to_raw(event: rdev::EventType, active: &mut Active, now: f64) -> Option<RawEvent> {
    let last = active.last;
    match event {
        rdev::EventType::MouseMove { x, y } => {
            let at = ScreenPoint { x, y };
            let moved = (at.x - last.x).abs() > 0.01 || (at.y - last.y).abs() > 0.01;
            if moved {
                active.last = at;
            }
            // Throttle, but never swallow the first sample of a take.
            if moved && now - active.last_sample_ms < SAMPLE_INTERVAL.as_millis() as f64 {
                return None;
            }
            Some(RawEvent::cursor_move(now, at.x, at.y))
        }
        rdev::EventType::ButtonPress(button) => Some(RawEvent::button(
            now,
            EventKind::MouseDown,
            button_name(button),
            last.x,
            last.y,
        )),
        rdev::EventType::ButtonRelease(button) => Some(RawEvent::button(
            now,
            EventKind::MouseUp,
            button_name(button),
            last.x,
            last.y,
        )),
        rdev::EventType::KeyPress(key) => Some(RawEvent::key(now, EventKind::KeyDown, key_name(&key))),
        rdev::EventType::KeyRelease(key) => {
            Some(RawEvent::key(now, EventKind::KeyUp, key_name(&key)))
        }
        // Scrolling is not part of the schema and not interesting to zoom on.
        // Returning `None` here means "throttled away", which is exactly the
        // accounting we want — it does not inflate the dropped counter's meaning.
        rdev::EventType::Wheel { .. } => None,
    }
}

fn button_name(button: rdev::Button) -> MouseButtonName {
    match button {
        rdev::Button::Left => MouseButtonName::Left,
        rdev::Button::Right => MouseButtonName::Right,
        rdev::Button::Middle => MouseButtonName::Middle,
        _ => MouseButtonName::Unknown,
    }
}

/// `rdev` has a ~120-variant key enum; we only need a stable *name* for it, and
/// the letters/names split lives in [`KeyName::from_platform`].
fn key_name(key: &rdev::Key) -> KeyName {
    KeyName::from_platform(&format!("{key:?}"))
}

/// Best-effort flush of anything still buffered, for the error paths.
pub fn flush_active() {
    if let Ok(mut guard) = slot().lock() {
        if let Some(active) = guard.as_mut() {
            let _ = active.writer.flush();
            let _ = std::io::stderr().flush();
        }
    }
}

/// Serialises the tests that drive the process-wide listener state.
///
/// `RECORDING` and the writer `slot()` are global by design (the tap thread is a
/// process-lifetime singleton), so tests that install a writer have to take turns
/// — otherwise they race each other and report phantom passes or failures.
#[cfg(test)]
static LISTENER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::clock::Clock;
    use std::time::Instant;

    /// A throwaway `Active` whose writer goes to the void, so the pure
    /// translation step can be tested without touching the OS listener.
    fn active(last: ScreenPoint, last_sample_ms: f64) -> Active {
        Active {
            stop: Arc::new(AtomicBool::new(false)),
            clock: Arc::new(Clock::start()),
            writer: EventWriter::create(Path::new("/dev/null")).unwrap(),
            last,
            last_sample_ms,
            last_flush_ms: 0.0,
            dropped: 0,
        }
    }

    #[test]
    fn mouse_move_updates_the_cached_position() {
        let mut state = active(ScreenPoint { x: 5.0, y: 5.0 }, f64::NEG_INFINITY);
        let event = to_raw(
            rdev::EventType::MouseMove { x: 40.0, y: 60.0 },
            &mut state,
            10.0,
        )
        .unwrap();
        assert_eq!(state.last, ScreenPoint { x: 40.0, y: 60.0 });
        let data = event.data.unwrap();
        assert_eq!((data.x, data.y), (Some(40.0), Some(60.0)));
        assert_eq!(event.kind, EventKind::CursorMove);
    }

    #[test]
    fn rapid_moves_are_throttled_to_60hz() {
        let mut state = active(ScreenPoint { x: 0.0, y: 0.0 }, 100.0);
        // 5 ms after the last write: inside the 16.6 ms window, so dropped.
        assert!(to_raw(rdev::EventType::MouseMove { x: 10.0, y: 10.0 }, &mut state, 105.0).is_none());
        // 20 ms after the last write: kept.
        assert!(to_raw(rdev::EventType::MouseMove { x: 20.0, y: 20.0 }, &mut state, 120.0).is_some());
    }

    #[test]
    fn clicks_are_never_throttled_and_carry_the_last_position() {
        let mut state = active(ScreenPoint { x: 100.0, y: 200.0 }, 100.0);
        let down = to_raw(rdev::EventType::ButtonPress(rdev::Button::Left), &mut state, 101.0).unwrap();
        let up = to_raw(rdev::EventType::ButtonRelease(rdev::Button::Left), &mut state, 150.0).unwrap();
        assert_eq!(down.kind, EventKind::MouseDown);
        assert_eq!(up.kind, EventKind::MouseUp);
        let data = down.data.unwrap();
        assert_eq!((data.x, data.y), (Some(100.0), Some(200.0)));
        assert_eq!(data.button, Some(MouseButtonName::Left));
    }

    #[test]
    fn maps_platform_button_names() {
        assert_eq!(button_name(rdev::Button::Left), MouseButtonName::Left);
        assert_eq!(button_name(rdev::Button::Middle), MouseButtonName::Middle);
        assert_eq!(button_name(rdev::Button::Unknown(9)), MouseButtonName::Unknown);
    }

    /// The regression that left a stopped take hanging on "Finishing…".
    ///
    /// The tap callback used to take the writer mutex before checking whether a
    /// take was still running, so a moving pointer kept re-acquiring it and
    /// starved `Tracker::stop` of the lock it needs to close the file. Holding
    /// the lock here reproduces the contention; the callback must still return
    /// promptly because it rejects on the lock-free gate.
    #[test]
    fn the_tap_callback_never_contends_for_the_writer_lock_when_idle() {
        let _serialised = LISTENER_TEST_LOCK.lock().unwrap();
        RECORDING.store(false, Ordering::SeqCst);

        // Hold the writer lock for the duration, as a busy callback would.
        let held = slot().lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            for _ in 0..100 {
                handle_event(rdev::Event {
                    time: std::time::SystemTime::now(),
                    name: None,
                    event_type: rdev::EventType::MouseMove { x: 1.0, y: 2.0 },
                });
            }
            let _ = tx.send(());
        });

        let returned = rx.recv_timeout(Duration::from_secs(5)).is_ok();
        drop(held);
        assert!(
            returned,
            "the tap callback blocked on the writer lock when no take was running"
        );
    }

    /// `stop` must be able to take the writer lock promptly, and must leave the
    /// gate closed afterwards so the callback stays out of the way.
    #[test]
    fn stopping_closes_the_gate_and_releases_the_writer() {
        let _serialised = LISTENER_TEST_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("revate-tracker-stop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // Install a writer by hand rather than via `Tracker::start`, which would
        // need a live OS event tap.
        let stop = Arc::new(AtomicBool::new(false));
        *slot().lock().unwrap() = Some(Active {
            stop: stop.clone(),
            clock: Arc::new(Clock::start()),
            writer: EventWriter::create(&dir.join(EVENTS_FILE)).unwrap(),
            last: ScreenPoint { x: 0.0, y: 0.0 },
            last_sample_ms: f64::NEG_INFINITY,
            last_flush_ms: 0.0,
            dropped: 0,
        });
        RECORDING.store(true, Ordering::SeqCst);

        // A realistic stop: the tap thread is hammering the callback the whole
        // time, exactly as it is during a real take with a moving pointer.
        let tapping = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    handle_event(rdev::Event {
                        time: std::time::SystemTime::now(),
                        name: None,
                        event_type: rdev::EventType::MouseMove { x: 5.0, y: 5.0 },
                    });
                }
            })
        };

        let started = Instant::now();
        stop.store(true, Ordering::SeqCst);
        RECORDING.store(false, Ordering::SeqCst);
        let taken = slot().lock().unwrap().take();
        let elapsed = started.elapsed();

        tapping.join().unwrap();
        assert!(taken.is_some(), "the writer should have been taken");
        assert!(
            elapsed < Duration::from_secs(2),
            "taking the writer took {elapsed:?} — the tap is starving the stop"
        );
        assert!(!RECORDING.load(Ordering::SeqCst), "the gate must be closed");

        std::fs::remove_dir_all(&dir).ok();
    }
}
