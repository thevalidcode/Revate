//! Project file format definition.
//! Defines the structure and schema for .revate project files.
//! TODO: Define the complete project file schema.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFile {
    pub version: String,
    pub metadata: ProjectMetadata,
    pub recording: RecordingInfo,
    pub events: EventInfo,
    pub settings: ProjectSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadata {
    pub title: String,
    pub created_at: String,
    pub modified_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingInfo {
    pub source_file: String,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventInfo {
    pub events_file: String,
    pub event_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSettings {
    pub zoom_enabled: bool,
    pub speed_ramping_enabled: bool,
    pub sfx_enabled: bool,
    pub chapters_enabled: bool,
    pub redaction_markers: Vec<RedactionMarker>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionMarker {
    pub id: u64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub start_time: f64,
    pub end_time: f64,
}
