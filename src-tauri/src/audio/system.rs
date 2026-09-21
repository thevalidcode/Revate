//! System audio capture.
//! Captures system audio output (what the user hears).
//! Platform-dependent implementation (may not be available on all platforms).
//! TODO: Implement system audio capture where supported.

pub fn start_system_audio_capture() -> Result<(), anyhow::Error> {
    todo!("Implement system audio capture start")
}

pub fn stop_system_audio_capture() -> Result<(), anyhow::Error> {
    todo!("Implement system audio capture stop")
}
