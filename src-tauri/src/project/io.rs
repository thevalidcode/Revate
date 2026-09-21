//! Project file I/O operations.
//! Handles reading and writing project files to disk.
//! TODO: Implement project file read/write operations.

pub fn save_project(project: &ProjectFile, path: &str) -> Result<(), anyhow::Error> {
    todo!("Implement project save")
}

pub fn load_project(path: &str) -> Result<ProjectFile, anyhow::Error> {
    todo!("Implement project load")
}

pub fn validate_project(project: &ProjectFile) -> Result<(), ValidationError> {
    todo!("Implement project validation")
}

#[derive(Debug, Clone)]
pub enum ValidationError {
    MissingFile(String),
    InvalidFormat(String),
    VersionMismatch { expected: String, found: String },
}

use super::format::ProjectFile;
