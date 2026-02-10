//! Core module for Scriba - business logic without UI dependencies.
//!
//! This module contains:
//! - Audio recording and encoding
//! - Transcription (local Whisper and OpenAI API)
//! - Configuration management
//! - File operations
//! - Workflow orchestration

pub mod audio;
pub mod config;
pub mod files;
pub mod recording;
pub mod transcription;
pub mod types;
pub mod workflow;

// Re-export commonly used types for convenience
pub use audio::{AudioEncoder, AudioFormat, CompressionSettings};
pub use config::{resolve_transcription_mode, EnrichmentConfig, LocalModelSize, ScribaConfig, SilenceAutoStopConfig, TranscriptionMode};
pub use files::FileManager;
pub use recording::{record_audio, AudioLevelMonitor, RecordOptions, RecordingResult};
pub use transcription::{transcribe_audio, TranscriptionProgress};
pub use types::{ManagedRecording, RecordingConfig, RecordingMetadata, RecordingMode};
pub use workflow::{DatabaseManager, HealthStatus, HealthStatusLevel, WorkflowManager, rebuild_world_from_entities, initialize_world_from_seed};
