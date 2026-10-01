//! SFX scheduling and timing.
//! Schedules sound effects to play at specific timestamps during export.
//! Aligns SFX with input events (clicks, key presses).

use anyhow::{anyhow, Result};

/// Schedules SFX for the given input events.
///
/// Each event is mapped to a `ScheduledSFX` with a small random pitch
/// variation and volume so that rapid repeats don't sound robotic.
pub fn schedule_sfx(
    events: &[InputEvent],
    samples: &SamplePool,
) -> Result<Vec<ScheduledSFX>> {
    let mut scheduled = Vec::with_capacity(events.len());

    for event in events {
        let sample_type = match event.event_type.as_str() {
            "click" | "mouse_down" | "mouse_up" => SampleType::Click,
            "key" | "key_down" | "key_press" | "typing" => SampleType::Typing,
            other => {
                // Unknown event kind — skip rather than fail the whole export.
                // (Change to `return Err(...)` if you'd rather be strict.)
                continue;
            }
        };

        // Pick the sample so we can sanity-check it exists / isn't empty.
        let sample = match sample_type {
            SampleType::Click => &samples.click,
            SampleType::Typing => &samples.typing,
        };
        if sample.data.is_empty() {
            return Err(anyhow!(
                "sample pool is missing data for {:?}",
                sample_type
            ));
        }

        // Deterministic per-event pitch jitter in [0.98, 1.02].
        // Swap for `rand` if you want true randomness.
        let jitter = pseudo_jitter(event.timestamp);
        let pitch_variation = 1.0 + jitter * 0.02;

        scheduled.push(ScheduledSFX {
            timestamp: event.timestamp,
            sample_type,
            pitch_variation,
            volume_db: 0.0,
        });
    }

    // Events may not arrive in time order (e.g. parsed from a log).
    scheduled.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap_or(std::cmp::Ordering::Equal));

    Ok(scheduled)
}

/// Cheap deterministic hash → value in [-1.0, 1.0].
fn pseudo_jitter(seed: f64) -> f32 {
    let bits = seed.to_bits();
    let mixed = bits.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    // Take low 16 bits, map to [-1, 1].
    let x = (mixed & 0xFFFF) as f32 / 32767.5 - 1.0;
    x
}

#[derive(Debug, Clone)]
pub struct ScheduledSFX {
    pub timestamp: f64,
    pub sample_type: SampleType,
    pub pitch_variation: f32,
    pub volume_db: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleType {
    Click,
    Typing,
}

#[derive(Debug, Clone)]
pub struct InputEvent {
    pub timestamp: f64,
    pub event_type: String,
}

#[derive(Debug, Clone)]
pub struct SamplePool {
    pub click: Sample,
    pub typing: Sample,
}

#[derive(Debug, Clone)]
pub struct Sample {
    pub data: Vec<f32>,
    pub sample_rate: u32,
}