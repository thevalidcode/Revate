//! Custom error types and error handling utilities.
//! Provides application-specific error types for better error reporting.
//! TODO: Implement error handling utilities.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum RevateError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Recording error: {0}")]
    Recording(String),

    #[error("Encoding error: {0}")]
    Encoding(String),

    #[error("Export error: {0}")]
    Export(String),

    #[error("Permission error: {0}")]
    Permission(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

impl From<anyhow::Error> for RevateError {
    fn from(err: anyhow::Error) -> Self {
        RevateError::Recording(err.to_string())
    }
}
