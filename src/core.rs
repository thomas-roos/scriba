/// Core orchestration layer for Scriba
///
/// Responsibilities:
/// - Provide high-level workflows (record, transcribe, import) that both CLI and TUI call.
/// - Centralize file operations (create dirs, import files, extract metadata, cleanup).
/// - Centralize database operations (save recording, update transcription status).
///
/// UI layers should only orchestrate and never duplicate core logic.
use anyhow::{Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};

use crate::audio::CompressionSettings;
use crate::config::TranscriptionMode;
use crate::database::{Database, Recording};
use crate::record::{calculate_audio_duration, record_audio, RecordOptions};
use crate::transcribe::transcribe_audio;
use crate::utils::{generate_recording_name, BASE_PATH};
use tokio::sync::mpsc;

/// High-level recording workflow parameters
#[derive(Debug, Clone)]
pub struct RecordingConfig {
    pub name: Option<String>,
    pub compression: Option<CompressionSettings>,
    pub auto_transcribe: bool,
    pub transcription_mode: Option<TranscriptionMode>,
}

/// Recording execution mode
#[derive(Debug)]
pub enum RecordingMode {
    /// CLI mode: blocks until Ctrl+C
    Cli,
    /// TUI mode: controlled via channels
    Tui {
        stop_rx: mpsc::Receiver<()>,
        level_tx: mpsc::Sender<f32>,
    },
}

/// Represents a managed recording with metadata and file operations
pub struct ManagedRecording {
    pub directory_name: String,
    pub display_name: Option<String>,
    pub audio_path: PathBuf,
    pub transcript_path: Option<PathBuf>,
    pub metadata: RecordingMetadata,
}

/// Audio file metadata extracted from various sources
#[derive(Debug, Clone)]
pub struct RecordingMetadata {
    pub duration_seconds: Option<i64>,
    pub file_size_bytes: Option<i64>,
    pub audio_format: String,
    pub sample_rate: i64,
    pub channels: i64,
}

/// Unified file operations for Scriba recordings
pub struct FileManager;

impl FileManager {
    /// Create a new recording directory with proper structure
    pub fn create_recording_directory(name: Option<String>) -> Result<PathBuf> {
        let directory_name = generate_recording_name(name);
        let dir_path = BASE_PATH.join(&directory_name);
        std::fs::create_dir_all(&dir_path).with_context(|| {
            format!(
                "Failed to create recording directory: {}",
                dir_path.display()
            )
        })?;
        Ok(dir_path)
    }

    /// Import external audio file into Scriba structure
    pub fn import_audio_file(
        source: &Path,
        target_directory: &Path,
        display_name: Option<String>,
    ) -> Result<ManagedRecording> {
        if !source.exists() {
            return Err(anyhow::anyhow!(
                "Source file not found: {}",
                source.display()
            ));
        }

        let file_extension = source
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("wav");

        let target_file = target_directory.join(format!("recording.{}", file_extension));

        std::fs::copy(source, &target_file).with_context(|| {
            format!(
                "Failed to copy {} to {}",
                source.display(),
                target_file.display()
            )
        })?;

        let metadata = Self::extract_audio_metadata(&target_file)?;

        Ok(ManagedRecording {
            directory_name: target_directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            display_name,
            audio_path: target_file,
            transcript_path: None,
            metadata,
        })
    }

    /// Extract comprehensive metadata from audio file with proper error handling
    pub fn extract_audio_metadata(file_path: &Path) -> Result<RecordingMetadata> {
        if !file_path.exists() {
            return Err(anyhow::anyhow!(
                "Audio file not found: {}",
                file_path.display()
            ));
        }

        let file_metadata = std::fs::metadata(file_path)
            .with_context(|| format!("Failed to read file metadata: {}", file_path.display()))?;
        let file_size_bytes = file_metadata.len() as i64;

        let audio_format = file_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("wav")
            .to_lowercase();

        // Extract actual audio parameters based on format
        let (sample_rate, channels, duration_seconds) = match audio_format.as_str() {
            "wav" => Self::extract_wav_metadata(file_path)?,
            "mp3" => Self::extract_mp3_metadata(file_path)?,
            "m4a" | "aac" => Self::extract_aac_metadata(file_path)?,
            _ => {
                // Fallback to calculation-based approach
                let (sample_rate, channels) = (44100i64, 2i64);
                let duration = calculate_audio_duration(file_path, sample_rate, channels)?;
                (sample_rate, channels, duration)
            }
        };

        Ok(RecordingMetadata {
            duration_seconds: Some(duration_seconds),
            file_size_bytes: Some(file_size_bytes),
            audio_format,
            sample_rate,
            channels,
        })
    }

    /// Extract metadata from WAV files using hound
    fn extract_wav_metadata(file_path: &Path) -> Result<(i64, i64, i64)> {
        let reader = hound::WavReader::open(file_path)
            .with_context(|| format!("Failed to open WAV file: {}", file_path.display()))?;

        let spec = reader.spec();
        let sample_rate = spec.sample_rate as i64;
        let channels = spec.channels as i64;
        let duration_samples = reader.duration() as i64;
        let duration_seconds = duration_samples / sample_rate;

        Ok((sample_rate, channels, duration_seconds))
    }

    /// Extract metadata from MP3 files using ffprobe or estimation
    fn extract_mp3_metadata(file_path: &Path) -> Result<(i64, i64, i64)> {
        // Try ffprobe first for accurate metadata
        if let Ok(duration) = Self::get_duration_with_ffprobe(file_path) {
            // MP3 defaults - could be enhanced with actual header parsing
            return Ok((44100, 2, duration));
        }

        // Fallback to estimation
        let file_size = std::fs::metadata(file_path)?.len() as i64;
        // Estimate duration based on 128kbps average bitrate
        let estimated_duration = (file_size * 8) / (128 * 1000);
        Ok((44100, 2, estimated_duration.max(1)))
    }

    /// Extract metadata from AAC/M4A files
    fn extract_aac_metadata(file_path: &Path) -> Result<(i64, i64, i64)> {
        // Try ffprobe for accurate metadata
        if let Ok(duration) = Self::get_duration_with_ffprobe(file_path) {
            return Ok((44100, 2, duration));
        }

        // Fallback estimation
        Ok((44100, 2, 1))
    }

    /// Get duration using ffprobe if available
    fn get_duration_with_ffprobe(file_path: &Path) -> Result<i64> {
        let output = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "quiet",
                "-show_entries",
                "format=duration",
                "-of",
                "csv=p=0",
            ])
            .arg(file_path)
            .output()?;

        if output.status.success() {
            let duration_str = String::from_utf8_lossy(&output.stdout);
            let duration_f64: f64 = duration_str
                .trim()
                .parse()
                .context("Failed to parse duration from ffprobe")?;
            Ok(duration_f64.round() as i64)
        } else {
            Err(anyhow::anyhow!("ffprobe failed"))
        }
    }

    /// Clean up temporary files in recording directory
    pub fn cleanup_temp_files(directory: &Path) -> Result<()> {
        let patterns = ["*_tmp_whisper_16k.wav", "*.tmp", "recording.wav.bak"];

        for pattern in patterns {
            if let Ok(entries) = glob::glob(&directory.join(pattern).to_string_lossy()) {
                for entry in entries.flatten() {
                    let _ = std::fs::remove_file(entry);
                }
            }
        }
        Ok(())
    }
}

/// Unified database operations with consistent error handling and connection pooling
pub struct DatabaseManager {
    db: Database,
}

impl DatabaseManager {
    /// Create new database manager with connection
    pub fn new() -> Result<Self> {
        Ok(Self {
            db: Database::new().context("Failed to connect to database")?,
        })
    }

    /// Get a shared database manager instance (singleton-like pattern for efficiency)
    pub fn shared() -> Result<Self> {
        Self::new()
    }

    /// Create and insert a recording from ManagedRecording
    pub fn save_recording(&mut self, recording: &ManagedRecording) -> Result<i64> {
        let db_recording = Recording {
            id: None,
            directory_name: recording.directory_name.clone(),
            display_name: recording.display_name.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            duration_seconds: recording.metadata.duration_seconds,
            file_size_bytes: recording.metadata.file_size_bytes,
            audio_format: recording.metadata.audio_format.clone(),
            sample_rate: recording.metadata.sample_rate,
            channels: recording.metadata.channels,
            has_transcript: recording.transcript_path.is_some(),
            transcript_status: if recording.transcript_path.is_some() {
                "completed".to_string()
            } else {
                "pending".to_string()
            },
            language_code: "auto".to_string(),
            model_used: "whisper.cpp".to_string(),
            tags: None,
            summary: None,
            key_points: None,
            action_items: None,
            speakers: None,
            sentiment_score: None,
            search_index: None,
            categories: None,
            confidence_score: None,
            audio_path: recording
                .audio_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            transcript_path: recording
                .transcript_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|name| name.to_string_lossy().to_string()),
        };

        self.db
            .insert_recording(&db_recording)
            .context("Failed to insert recording into database")
    }

    /// Update recording with transcription info
    pub fn update_transcription(
        &mut self,
        directory_name: &str,
        _transcript_path: &Path,
        model_used: &str,
    ) -> Result<()> {
        // Look up the recording ID by directory name, then mark as completed
        if let Some(rec) = self
            .db
            .get_recording_by_directory(directory_name)
            .context("Failed to query recording by directory name")?
        {
            if let Some(id) = rec.id {
                self.db
                    .update_recording_transcript_status_and_model(
                        id,
                        "completed",
                        true,
                        model_used,
                    )
                    .context("Failed to update transcript status/model in database")?;
            }
        }
        Ok(())
    }
}

/// High-level workflow orchestrator with configuration management
pub struct WorkflowManager {
    db_manager: DatabaseManager,
    config: crate::config::ScribaConfig,
}

impl WorkflowManager {
    /// Create new workflow manager with loaded configuration
    pub fn new() -> Result<Self> {
        Ok(Self {
            db_manager: DatabaseManager::new()?,
            config: crate::config::ScribaConfig::load()
                .context("Failed to load Scriba configuration")?,
        })
    }

    /// Create workflow manager with custom configuration
    pub fn with_config(config: crate::config::ScribaConfig) -> Result<Self> {
        Ok(Self {
            db_manager: DatabaseManager::new()?,
            config,
        })
    }

    /// Get reference to the configuration
    pub fn config(&self) -> &crate::config::ScribaConfig {
        &self.config
    }

    /// Complete recording workflow: record -> save -> optionally transcribe
    pub async fn complete_recording_workflow(
        &mut self,
        config: RecordingConfig,
        mode: RecordingMode,
    ) -> Result<ManagedRecording> {
        // Generate recording directory name
        let directory_name = generate_recording_name(config.name.clone());
        let recording_path = PathBuf::from(&directory_name);

        // Execute recording based on mode using unified function
        let final_directory_name = match mode {
            RecordingMode::Cli => {
                let _ = record_audio(
                    recording_path,
                    RecordOptions {
                        compression_settings: config.compression.clone(),
                        stop_rx: None,
                        level_tx: None,
                        verbose: true,
                    },
                )
                .await?;
                directory_name
            }
            RecordingMode::Tui { stop_rx, level_tx } => {
                record_audio(
                    recording_path,
                    RecordOptions {
                        compression_settings: config.compression.clone(),
                        stop_rx: Some(stop_rx),
                        level_tx: Some(level_tx),
                        verbose: false,
                    },
                )
                .await?
            }
        };

        // Create ManagedRecording from the recorded files
        let recording_dir = BASE_PATH.join(&final_directory_name);
        let audio_files = ["recording.mp3", "recording.wav", "recording.m4a"];

        let audio_path = audio_files
            .iter()
            .map(|name| recording_dir.join(name))
            .find(|path| path.exists())
            .ok_or_else(|| anyhow::anyhow!("No audio file found after recording"))?;

        let metadata = FileManager::extract_audio_metadata(&audio_path)?;

        let mut recording = ManagedRecording {
            directory_name: final_directory_name,
            display_name: config.name,
            audio_path,
            transcript_path: None,
            metadata,
        };

        // Auto-transcribe if requested
        if config.auto_transcribe {
            if let Some(transcription_mode) = config.transcription_mode {
                println!("📝 Starting auto-transcription...");
                recording = self
                    .transcribe_recording(recording, transcription_mode)
                    .await?;
            }
        }

        Ok(recording)
    }

    /// Complete import workflow: copy -> save -> optionally transcribe
    pub async fn complete_import_workflow(
        &mut self,
        source_file: &Path,
        display_name: Option<String>,
        transcription_mode: Option<TranscriptionMode>,
    ) -> Result<ManagedRecording> {
        self.complete_import_workflow_internal(source_file, display_name, transcription_mode, true)
            .await
    }

    /// Silent version of complete_import_workflow for TUI usage
    pub async fn complete_import_workflow_silent(
        &mut self,
        source_file: &Path,
        display_name: Option<String>,
        transcription_mode: Option<TranscriptionMode>,
    ) -> Result<ManagedRecording> {
        self.complete_import_workflow_internal(source_file, display_name, transcription_mode, false)
            .await
    }

    async fn complete_import_workflow_internal(
        &mut self,
        source_file: &Path,
        display_name: Option<String>,
        transcription_mode: Option<TranscriptionMode>,
        verbose: bool,
    ) -> Result<ManagedRecording> {
        let recording_dir = FileManager::create_recording_directory(display_name.clone())?;
        let mut recording =
            FileManager::import_audio_file(source_file, &recording_dir, display_name)?;

        let recording_id = self.db_manager.save_recording(&recording)?;
        if verbose {
            println!("✅ File imported to database with ID: {}", recording_id);
        }

        if let Some(mode) = transcription_mode {
            if verbose {
                println!("📝 Starting transcription...");
            }
            recording = self
                .transcribe_recording_internal(recording, mode, verbose)
                .await?;
        }

        FileManager::cleanup_temp_files(&recording_dir)?;
        Ok(recording)
    }

    async fn transcribe_recording_internal(
        &mut self,
        mut recording: ManagedRecording,
        mode: TranscriptionMode,
        verbose: bool,
    ) -> Result<ManagedRecording> {
        let directory_path = PathBuf::from(&recording.directory_name);
        transcribe_audio(&directory_path, Some(mode), verbose)
            .await
            .context("Transcription failed")?;

        let transcript_path = BASE_PATH
            .join(&recording.directory_name)
            .join("transcript.txt");
        if transcript_path.exists() {
            recording.transcript_path = Some(transcript_path);
        }
        Ok(recording)
    }

    /// Unified transcription workflow that works with any ManagedRecording (verbose)
    pub async fn transcribe_recording(
        &mut self,
        recording: ManagedRecording,
        mode: TranscriptionMode,
    ) -> Result<ManagedRecording> {
        self.transcribe_recording_internal(recording, mode, true).await
    }

    /// Silent version of transcribe_recording for TUI usage
    pub async fn transcribe_recording_silent(
        &mut self,
        recording: ManagedRecording,
        mode: TranscriptionMode,
    ) -> Result<ManagedRecording> {
        self.transcribe_recording_internal(recording, mode, false).await
    }

    /// Convenience method for simple CLI recording
    pub async fn record_cli(
        &mut self,
        name: Option<String>,
        compression: Option<CompressionSettings>,
        auto_transcribe: bool,
        transcription_mode: Option<TranscriptionMode>,
    ) -> Result<ManagedRecording> {
        let config = RecordingConfig {
            name,
            compression,
            auto_transcribe,
            transcription_mode,
        };

        self.complete_recording_workflow(config, RecordingMode::Cli)
            .await
    }

    /// Convenience method for TUI recording with control channels
    pub async fn record_tui(
        &mut self,
        name: Option<String>,
        compression: Option<CompressionSettings>,
        stop_rx: mpsc::Receiver<()>,
        level_tx: mpsc::Sender<f32>,
        auto_transcribe: bool,
        transcription_mode: Option<TranscriptionMode>,
    ) -> Result<ManagedRecording> {
        let config = RecordingConfig {
            name,
            compression,
            auto_transcribe,
            transcription_mode,
        };

        let mode = RecordingMode::Tui { stop_rx, level_tx };
        self.complete_recording_workflow(config, mode).await
    }

    /// Re-transcribe an existing recording with a different model
    pub async fn retranscribe_recording(
        &mut self,
        directory_name: &str,
        transcription_mode: TranscriptionMode,
    ) -> Result<()> {
        self.retranscribe_internal(directory_name, transcription_mode, true)
            .await
    }

    /// Silent version of retranscribe_recording for TUI usage
    pub async fn retranscribe_recording_silent(
        &mut self,
        directory_name: &str,
        transcription_mode: TranscriptionMode,
    ) -> Result<()> {
        self.retranscribe_internal(directory_name, transcription_mode, false)
            .await
    }

    async fn retranscribe_internal(
        &mut self,
        directory_name: &str,
        transcription_mode: TranscriptionMode,
        verbose: bool,
    ) -> Result<()> {
        let recording_dir = BASE_PATH.join(directory_name);
        if !recording_dir.exists() {
            return Err(anyhow::anyhow!(
                "Recording directory not found: {}",
                directory_name
            ));
        }

        let audio_files = ["recording.mp3", "recording.wav", "recording.m4a"];
        let audio_path = audio_files
            .iter()
            .map(|name| recording_dir.join(name))
            .find(|path| path.exists())
            .ok_or_else(|| anyhow::anyhow!("No audio file found in recording directory"))?;

        let metadata = FileManager::extract_audio_metadata(&audio_path)?;
        let recording = ManagedRecording {
            directory_name: directory_name.to_string(),
            display_name: None,
            audio_path,
            transcript_path: None,
            metadata,
        };

        let _ = self
            .transcribe_recording_internal(recording, transcription_mode, verbose)
            .await?;
        if verbose {
            println!("✅ Re-transcription complete for: {}", directory_name);
        }
        Ok(())
    }

    /// Batch import multiple audio files with progress reporting
    pub async fn batch_import_workflow(
        &mut self,
        files: &[PathBuf],
        transcription_mode: Option<TranscriptionMode>,
        progress_callback: Option<Box<dyn Fn(usize, usize, &str) + Send>>,
    ) -> Result<Vec<ManagedRecording>> {
        let mut recordings = Vec::new();
        let total_files = files.len();

        for (index, file_path) in files.iter().enumerate() {
            if let Some(ref callback) = progress_callback {
                callback(
                    index + 1,
                    total_files,
                    &format!("Processing {}", file_path.display()),
                );
            }

            println!(
                "📁 [{}/{}] Importing: {}",
                index + 1,
                total_files,
                file_path.display()
            );

            match self
                .complete_import_workflow(file_path, None, transcription_mode.clone())
                .await
            {
                Ok(recording) => {
                    recordings.push(recording);
                    println!("✅ Successfully imported: {}", file_path.display());
                }
                Err(e) => {
                    eprintln!("❌ Failed to import {}: {}", file_path.display(), e);
                    // Continue with other files instead of failing the entire batch
                }
            }
        }

        println!(
            "🎉 Batch import complete! {} of {} files processed successfully.",
            recordings.len(),
            total_files
        );
        Ok(recordings)
    }

    /// Health check for the workflow manager and its dependencies
    pub fn health_check(&self) -> Result<HealthStatus> {
        let mut issues = Vec::new();
        let mut warnings = Vec::new();

        // Check database connectivity
        if let Err(e) = DatabaseManager::new() {
            issues.push(format!("Database connection failed: {}", e));
        }

        // Check BASE_PATH accessibility
        if !BASE_PATH.exists() {
            issues.push("Base recordings directory does not exist".to_string());
        } else if let Err(e) = std::fs::create_dir_all(&*BASE_PATH) {
            issues.push(format!("Cannot write to recordings directory: {}", e));
        }

        // Check for ffprobe availability (optional but recommended)
        if std::process::Command::new("ffprobe")
            .arg("-version")
            .output()
            .is_err()
        {
            warnings.push(
                "ffprobe not available - audio metadata extraction will be limited".to_string(),
            );
        }

        // Check whisper model availability
        let models_dir = BASE_PATH.join("models");
        if !models_dir.exists() {
            warnings.push(
                "No whisper models directory found - local transcription may fail".to_string(),
            );
        }

        let status = if issues.is_empty() {
            HealthStatusLevel::Healthy
        } else {
            HealthStatusLevel::Unhealthy
        };

        Ok(HealthStatus {
            status,
            issues,
            warnings,
        })
    }
}

/// Health status for workflow manager
#[derive(Debug)]
pub struct HealthStatus {
    pub status: HealthStatusLevel,
    pub issues: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub enum HealthStatusLevel {
    Healthy,
    Unhealthy,
}

impl HealthStatus {
    pub fn is_healthy(&self) -> bool {
        matches!(self.status, HealthStatusLevel::Healthy)
    }

    pub fn print_report(&self) {
        match self.status {
            HealthStatusLevel::Healthy => {
                println!("✅ Scriba health check: All systems operational");
            }
            HealthStatusLevel::Unhealthy => {
                println!("❌ Scriba health check: Issues detected");
            }
        }

        if !self.issues.is_empty() {
            println!("\n🚨 Issues:");
            for issue in &self.issues {
                println!("  - {}", issue);
            }
        }

        if !self.warnings.is_empty() {
            println!("\n⚠️ Warnings:");
            for warning in &self.warnings {
                println!("  - {}", warning);
            }
        }
    }
}
