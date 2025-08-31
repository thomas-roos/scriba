/// Comprehensive error types for Scriba operations
/// Provides better error context and user-friendly messages
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ScribaError {
    #[error("Database operation failed: {message}")]
    Database { message: String },

    #[error("Audio file not found: {path}")]
    AudioFileNotFound { path: PathBuf },

    #[error("Recording directory not found: {path}")]
    RecordingDirNotFound { path: PathBuf },

    #[error("Unsupported audio format: {format}")]
    UnsupportedAudioFormat { format: String },

    #[error("Transcription failed: {reason}")]
    TranscriptionFailed { reason: String },

    #[error("Recording failed: {reason}")]
    RecordingFailed { reason: String },

    #[error("Configuration error: {message}")]
    Config { message: String },

    #[error("File operation failed: {operation} on {path}: {reason}")]
    FileOperation {
        operation: String,
        path: PathBuf,
        reason: String,
    },

    #[error("Invalid model configuration: {model}")]
    InvalidModel { model: String },

    #[error("Missing required dependency: {dependency}")]
    MissingDependency { dependency: String },

    #[error("Audio device error: {message}")]
    AudioDevice { message: String },

    #[error("Network error during API call: {message}")]
    Network { message: String },

    #[error("Validation error: {field} - {message}")]
    Validation { field: String, message: String },
}

impl ScribaError {
    /// Create a database error
    pub fn database(message: impl Into<String>) -> Self {
        Self::Database {
            message: message.into(),
        }
    }

    /// Create a file not found error
    pub fn audio_file_not_found(path: impl Into<PathBuf>) -> Self {
        Self::AudioFileNotFound { path: path.into() }
    }

    /// Create a recording directory not found error
    pub fn recording_dir_not_found(path: impl Into<PathBuf>) -> Self {
        Self::RecordingDirNotFound { path: path.into() }
    }

    /// Create a transcription error
    pub fn transcription_failed(reason: impl Into<String>) -> Self {
        Self::TranscriptionFailed {
            reason: reason.into(),
        }
    }

    /// Create a recording error
    pub fn recording_failed(reason: impl Into<String>) -> Self {
        Self::RecordingFailed {
            reason: reason.into(),
        }
    }

    /// Create a file operation error
    pub fn file_operation(
        operation: impl Into<String>,
        path: impl Into<PathBuf>,
        reason: impl Into<String>,
    ) -> Self {
        Self::FileOperation {
            operation: operation.into(),
            path: path.into(),
            reason: reason.into(),
        }
    }
}

/// Result type for Scriba operations
pub type ScribaResult<T> = std::result::Result<T, ScribaError>;

/// Convert anyhow::Error to ScribaError for better error handling
impl From<anyhow::Error> for ScribaError {
    fn from(err: anyhow::Error) -> Self {
        ScribaError::database(err.to_string())
    }
}

/// Convert std::io::Error to ScribaError
impl From<std::io::Error> for ScribaError {
    fn from(err: std::io::Error) -> Self {
        ScribaError::FileOperation {
            operation: "I/O operation".to_string(),
            path: PathBuf::new(),
            reason: err.to_string(),
        }
    }
}
