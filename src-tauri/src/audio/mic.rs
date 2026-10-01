//! Microphone audio capture.
//!
//! Uses `cpal` to open the selected input device and streams samples straight
//! into a 32-bit float WAV (`hound`). Capture runs on its own thread — cpal's
//! `Stream` is `!Send`, so the device and stream are both created *inside* that
//! thread — and stops when an atomic flag is set.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, StreamConfig};
use hound::{SampleFormat as HoundSampleFormat, WavSpec, WavWriter};

type Writer = WavWriter<std::io::BufWriter<std::fs::File>>;

/// Handle to a running microphone capture.
pub struct MicRecording {
    stop_flag: Arc<AtomicBool>,
    handle: JoinHandle<Result<PathBuf>>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl MicRecording {
    /// Signal the capture loop to stop and wait for the WAV to be finalized.
    pub fn stop(self) -> Result<PathBuf> {
        self.stop_flag.store(true, Ordering::SeqCst);
        self.handle
            .join()
            .map_err(|e| anyhow!("microphone thread panicked: {e:?}"))?
    }
}

/// Names of every available capture (input) device, e.g. for the mic dropdown.
pub fn list_input_devices() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let mut names = Vec::new();
    for device in host
        .input_devices()
        .context("failed to enumerate input devices")?
    {
        if let Ok(name) = device.name() {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    Ok(names)
}

/// Open `device_name` (or the system default) and start writing float samples
/// to `output`. Returns once the stream is actually running so the caller can
/// surface a permission/device error immediately.
pub fn start_microphone_capture(
    device_name: Option<String>,
    output: PathBuf,
) -> Result<MicRecording> {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (init_tx, init_rx) = std::sync::mpsc::channel::<Result<(u32, u16), String>>();

    let flag = stop_flag.clone();
    let out = output.clone();
    let handle = std::thread::spawn(move || capture_thread(device_name, &out, flag, &init_tx));

    let (sample_rate, channels) = match init_rx.recv() {
        Ok(Ok(v)) => v,
        Ok(Err(msg)) => {
            let _ = handle.join();
            return Err(anyhow!(msg));
        }
        Err(_) => return Err(anyhow!("microphone thread exited before the stream started")),
    };

    Ok(MicRecording {
        stop_flag,
        handle,
        sample_rate,
        channels,
    })
}

fn capture_thread(
    device_name: Option<String>,
    output: &Path,
    stop: Arc<AtomicBool>,
    init: &Sender<Result<(u32, u16), String>>,
) -> Result<PathBuf> {
    let (stream, writer, sample_rate, channels) =
        match open_stream(device_name, output) {
            Ok(v) => v,
            Err(e) => {
                let _ = init.send(Err(e.to_string()));
                return Err(e);
            }
        };

    let _ = init.send(Ok((sample_rate, channels)));

    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(50));
    }

    // Dropping the stream stops the callbacks, after which the writer is safe
    // to finalize (hound has to patch the RIFF header).
    drop(stream);
    if let Some(w) = writer.lock().unwrap().take() {
        w.finalize().context("failed to finalize microphone WAV")?;
    }
    Ok(output.to_path_buf())
}


fn open_stream(
    device_name: Option<String>,
    output: &Path,
) -> Result<(cpal::Stream, Arc<Mutex<Option<Writer>>>, u32, u16)> {
    let host = cpal::default_host();
    let device = match device_name.as_deref() {
        Some(name) => host
            .input_devices()
            .context("failed to enumerate input devices")?
            .find(|d| d.name().map(|n| n == name).unwrap_or(false))
            .ok_or_else(|| anyhow!("input device `{name}` was not found"))?,
        None => host
            .default_input_device()
            .ok_or_else(|| anyhow!("no default input device available"))?,
    };

    let supported = device
        .default_input_config()
        .context("device has no default input configuration")?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.config();

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let spec = WavSpec {
        channels: config.channels,
        sample_rate: config.sample_rate.0,
        bits_per_sample: 32,
        sample_format: HoundSampleFormat::Float,
    };
    let writer = Arc::new(Mutex::new(Some(
        WavWriter::create(output, spec).context("failed to create microphone WAV")?,
    )));

    let stream = match sample_format {
        SampleFormat::F32 => build_stream::<f32>(&device, &config, writer.clone())?,
        SampleFormat::I16 => build_stream::<i16>(&device, &config, writer.clone())?,
        SampleFormat::U16 => build_stream::<u16>(&device, &config, writer.clone())?,
        other => return Err(anyhow!("unsupported microphone sample format: {other:?}")),
    };

    stream.play().context("failed to start microphone stream")?;

    Ok((stream, writer, config.sample_rate.0, config.channels))
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    writer: Arc<Mutex<Option<Writer>>>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: cpal::FromSample<T>,
{
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let mut guard = writer.lock().unwrap();
            if let Some(w) = guard.as_mut() {
                for &sample in data {
                    // Normalize every input format to f32; the WAV spec above is
                    // float-32 so this is lossless for F32 and precise for the
                    // integer formats.
                    let _ = w.write_sample(sample.to_sample::<f32>());
                }
            }
        },
        |err| eprintln!("[mic] stream error: {err}"),
        None,
    )?;
    Ok(stream)
}

