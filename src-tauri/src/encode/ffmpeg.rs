//! FFmpeg integration and video encoding.
//! Manages FFmpeg sidecar binary and encoding pipeline.
//! Supports stream copy when no video transforms apply.
//! TODO: Implement FFmpeg encoding with sidecar management.

pub fn encode_video(
    input_path: &str,
    output_path: &str,
    options: EncodeOptions,
) -> Result<(), anyhow::Error> {
    todo!("Implement video encoding")
}

pub struct EncodeOptions {
    pub codec: String,
    pub bitrate: u32,
    pub preset: String,
    pub crf: Option<u32>,
}

pub fn find_ffmpeg_binary() -> Result<String, anyhow::Error> {
    todo!("Implement FFmpeg binary location")
}
