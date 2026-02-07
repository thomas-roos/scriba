//! High-level workflow orchestration for Scriba.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

use super::audio::CompressionSettings;
use super::config::{ScribaConfig, TranscriptionMode};
use super::files::FileManager;
use super::recording::{record_audio, RecordOptions};
use super::transcription::transcribe_audio;
use super::types::{ManagedRecording, RecordingConfig, RecordingMode};
use crate::database::{Database, Recording};
use crate::enrichment::{EnrichmentService, WorldContext, WorldData};
use crate::entities::{EntityLinker, EntityRegistry};
use crate::utils::{generate_recording_name, BASE_PATH};
use chrono::Utc;

/// Unified database operations with consistent error handling.
pub struct DatabaseManager {
    db: Database,
}

impl DatabaseManager {
    pub fn new() -> Result<Self> {
        Ok(Self {
            db: Database::new().context("Failed to connect to database")?,
        })
    }

    /// Create and insert a recording from ManagedRecording.
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

    /// Update recording with transcription info.
    pub fn update_transcription(
        &mut self,
        directory_name: &str,
        _transcript_path: &Path,
        model_used: &str,
    ) -> Result<()> {
        if let Some(rec) = self
            .db
            .get_recording_by_directory(directory_name)
            .context("Failed to query recording by directory name")?
        {
            if let Some(id) = rec.id {
                self.db
                    .update_recording_transcript_status_and_model(id, "completed", true, model_used)
                    .context("Failed to update transcript status/model in database")?;
            }
        }
        Ok(())
    }
}

/// High-level workflow orchestrator with configuration management.
pub struct WorkflowManager {
    db_manager: DatabaseManager,
    config: ScribaConfig,
}

impl WorkflowManager {
    /// Create new workflow manager with loaded configuration.
    pub fn new() -> Result<Self> {
        Ok(Self {
            db_manager: DatabaseManager::new()?,
            config: ScribaConfig::load().context("Failed to load Scriba configuration")?,
        })
    }

    /// Create workflow manager with custom configuration.
    pub fn with_config(config: ScribaConfig) -> Result<Self> {
        Ok(Self {
            db_manager: DatabaseManager::new()?,
            config,
        })
    }

    /// Get reference to the configuration.
    pub fn config(&self) -> &ScribaConfig {
        &self.config
    }

    /// Complete recording workflow: record -> save -> optionally transcribe.
    pub async fn complete_recording_workflow(
        &mut self,
        config: RecordingConfig,
        mode: RecordingMode,
    ) -> Result<ManagedRecording> {
        let directory_name = generate_recording_name(config.name.clone());
        let recording_path = PathBuf::from(&directory_name);

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

        let recording_dir = BASE_PATH.join(&final_directory_name);
        let audio_path = FileManager::find_audio_file(&recording_dir)
            .ok_or_else(|| anyhow::anyhow!("No audio file found after recording"))?;

        let metadata = FileManager::extract_audio_metadata(&audio_path)?;

        let mut recording = ManagedRecording {
            directory_name: final_directory_name,
            display_name: config.name,
            audio_path,
            transcript_path: None,
            metadata,
        };

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

    /// Complete import workflow: copy -> save -> optionally transcribe.
    pub async fn complete_import_workflow(
        &mut self,
        source_file: &Path,
        display_name: Option<String>,
        transcription_mode: Option<TranscriptionMode>,
    ) -> Result<ManagedRecording> {
        self.complete_import_workflow_internal(source_file, display_name, transcription_mode, true)
            .await
    }

    /// Silent version of complete_import_workflow for TUI usage.
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
            recording.transcript_path = Some(transcript_path.clone());

            // Run enrichment if enabled
            if self.config.enrichment.enabled {
                if let Err(e) = self
                    .enrich_recording(&recording.directory_name, &transcript_path, verbose)
                    .await
                {
                    if verbose {
                        eprintln!("⚠️ Enrichment failed (non-fatal): {}", e);
                    }
                }
            }
        }
        Ok(recording)
    }

    /// Enrich a recording with AI-extracted metadata.
    pub async fn enrich_recording(
        &mut self,
        directory_name: &str,
        transcript_path: &Path,
        verbose: bool,
    ) -> Result<()> {
        let config = &self.config.enrichment;

        if verbose {
            println!("🧠 Starting knowledge extraction...");
        }

        // Read transcript content
        let transcript_content = std::fs::read_to_string(transcript_path)
            .context("Failed to read transcript for enrichment")?;

        if transcript_content.trim().is_empty() {
            if verbose {
                println!("⚠️ Transcript is empty, skipping enrichment");
            }
            return Ok(());
        }

        // Create enrichment service
        let service = EnrichmentService::new(&config.ollama_endpoint, &config.ollama_model);

        // Check if Ollama is available
        if let Err(e) = service.health_check().await {
            return Err(anyhow::anyhow!(
                "Ollama not available for enrichment: {}. Make sure Ollama is running with model '{}'.",
                e,
                config.ollama_model
            ));
        }

        if verbose {
            println!("  📊 Extracting metadata from transcript...");
        }

        // Load world context (Scriba's understanding of the owner)
        let mut world = WorldContext::load().unwrap_or_default();
        let world_content = if world.has_content() {
            if verbose {
                println!("  🌍 Using world context for extraction");
            }
            Some(world.content.clone())
        } else {
            None
        };

        // Extract metadata with world context — the LLM resolves entities inline
        let extraction = service
            .extract_with_full_context(
                &transcript_content,
                world_content.as_deref(),
            )
            .await
            .context("Failed to extract metadata from transcript")?;

        if verbose {
            println!("  ✅ Extracted: title='{}', {} topics, {} people, {} organizations",
                extraction.title,
                extraction.topics.len(),
                extraction.people.len(),
                extraction.organizations.len()
            );
        }

        // Get recording from database
        let recording = self.db_manager.db.get_recording_by_directory(directory_name)?
            .ok_or_else(|| anyhow::anyhow!("Recording not found: {}", directory_name))?;
        let recording_id = recording.id
            .ok_or_else(|| anyhow::anyhow!("Recording has no ID"))?;

        // Update recording with enrichment data
        let key_points_json = if extraction.key_points.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&extraction.key_points)?)
        };

        let action_items_json = if extraction.action_items.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&extraction.action_items)?)
        };

        let display_name = Some(extraction.title.as_str());

        self.db_manager.db.update_recording_enrichment(
            recording_id,
            display_name,
            Some(&extraction.summary),
            key_points_json.as_deref(),
            action_items_json.as_deref(),
        )?;

        // Update transcript with topics and entities
        let topics_json = if extraction.topics.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&extraction.topics)?)
        };

        // Combine people and organizations into entities JSON
        let entities: Vec<serde_json::Value> = extraction
            .people
            .iter()
            .map(|p| serde_json::json!({"type": "person", "name": p.name, "context": p.context}))
            .chain(
                extraction
                    .organizations
                    .iter()
                    .map(|o| serde_json::json!({"type": "organization", "name": o.name, "context": o.context})),
            )
            .collect();

        let entities_json = if entities.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&entities)?)
        };

        self.db_manager.db.update_transcript_enrichment(
            recording_id,
            entities_json.as_deref(),
            topics_json.as_deref(),
        )?;

        // Process entity linking — entities are already resolved by the LLM
        if verbose {
            println!("  🔗 Linking entities...");
        }

        let linker = EntityLinker::new();
        let report = linker
            .process_extraction(&mut self.db_manager.db, recording_id, &extraction)?;

        if verbose {
            println!(
                "  ✅ Entity linking complete: {} linked, {} created",
                report.linked_existing, report.created_new
            );
        }

        // Evolve world description if enabled and world exists
        if config.evolve_world && world.has_content() {
            if verbose {
                println!("  🌍 Evolving world understanding...");
            }

            // Migrate legacy (non-JSON) world to structured format first
            let mut world_data = if let Some(data) = world.parsed() {
                data
            } else {
                if verbose {
                    println!("  🔄 Migrating world to structured format...");
                }
                match service.extract_world_seed(&world.content).await {
                    Ok(data) => data,
                    Err(e) => {
                        if verbose {
                            eprintln!("  ⚠️ Could not migrate world: {}", e);
                        }
                        WorldData::default()
                    }
                }
            };

            match service
                .evolve_world(
                    &world_data.to_json().unwrap_or_default(),
                    &transcript_content,
                    &extraction,
                )
                .await
            {
                Ok(Some(delta)) => {
                    world_data.merge(&delta);
                    if let Err(e) = world.update_data(&world_data) {
                        if verbose {
                            eprintln!("  ⚠️ Failed to save world update: {}", e);
                        }
                    } else if verbose {
                        println!("  ✅ World description updated");
                    }
                }
                Ok(None) => {
                    // Delta parse failed, but if we migrated, still save the migration
                    if let Err(e) = world.update_data(&world_data) {
                        if verbose {
                            eprintln!("  ⚠️ Failed to save world: {}", e);
                        }
                    } else if verbose {
                        println!("  ⚠️ No new changes from this recording");
                    }
                }
                Err(e) => {
                    if verbose {
                        eprintln!("  ⚠️ Failed to evolve world (non-fatal): {}", e);
                    }
                }
            }
        }

        // Sync world entities to database (ensures DB stays in sync with world)
        if world.has_content() {
            if let Some(world_data) = world.parsed() {
                if let Err(e) = sync_world_to_entities(&mut self.db_manager.db, &world_data) {
                    if verbose {
                        eprintln!("  ⚠️ Failed to sync world to entities: {}", e);
                    }
                }
            }
        }

        if verbose {
            println!("✅ Knowledge extraction complete!");
        }

        Ok(())
    }

    /// Enrich an existing recording (for manual enrichment or re-enrichment).
    pub async fn enrich_existing_recording(
        &mut self,
        directory_name: &str,
        verbose: bool,
    ) -> Result<()> {
        let recording_dir = BASE_PATH.join(directory_name);
        let transcript_path = recording_dir.join("transcript.txt");

        if !transcript_path.exists() {
            return Err(anyhow::anyhow!(
                "No transcript found for recording: {}",
                directory_name
            ));
        }

        self.enrich_recording(directory_name, &transcript_path, verbose)
            .await
    }

    /// Unified transcription workflow that works with any ManagedRecording (verbose).
    pub async fn transcribe_recording(
        &mut self,
        recording: ManagedRecording,
        mode: TranscriptionMode,
    ) -> Result<ManagedRecording> {
        self.transcribe_recording_internal(recording, mode, true)
            .await
    }

    /// Silent version of transcribe_recording for TUI usage.
    pub async fn transcribe_recording_silent(
        &mut self,
        recording: ManagedRecording,
        mode: TranscriptionMode,
    ) -> Result<ManagedRecording> {
        self.transcribe_recording_internal(recording, mode, false)
            .await
    }

    /// Convenience method for simple CLI recording.
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

    /// Convenience method for TUI recording with control channels.
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

    /// Re-transcribe an existing recording with a different model.
    pub async fn retranscribe_recording(
        &mut self,
        directory_name: &str,
        transcription_mode: TranscriptionMode,
    ) -> Result<()> {
        self.retranscribe_internal(directory_name, transcription_mode, true)
            .await
    }

    /// Silent version of retranscribe_recording for TUI usage.
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

        let audio_path = FileManager::find_audio_file(&recording_dir)
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

    /// Batch import multiple audio files with progress reporting.
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

    /// Health check for the workflow manager and its dependencies.
    pub fn health_check(&self) -> Result<HealthStatus> {
        let mut issues = Vec::new();
        let mut warnings = Vec::new();

        if let Err(e) = DatabaseManager::new() {
            issues.push(format!("Database connection failed: {}", e));
        }

        if !BASE_PATH.exists() {
            issues.push("Base recordings directory does not exist".to_string());
        } else if let Err(e) = std::fs::create_dir_all(&*BASE_PATH) {
            issues.push(format!("Cannot write to recordings directory: {}", e));
        }

        if std::process::Command::new("ffprobe")
            .arg("-version")
            .output()
            .is_err()
        {
            warnings.push(
                "ffprobe not available - audio metadata extraction will be limited".to_string(),
            );
        }

        let models_dir = BASE_PATH.join("models");
        if !models_dir.exists() {
            warnings.push(
                "No whisper models directory found - local transcription may fail".to_string(),
            );
        }

        // Check enrichment/Ollama availability
        if self.config.enrichment.enabled {
            // We can't await here since health_check is not async, so we just note the config
            warnings.push(format!(
                "Enrichment enabled - requires Ollama at {} with model '{}'",
                self.config.enrichment.ollama_endpoint,
                self.config.enrichment.ollama_model
            ));
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

/// Sync world entities to the database.
///
/// For each person and organization in the world, ensure a corresponding
/// entity exists in the database. Updates context if the world has richer info.
fn sync_world_to_entities(db: &mut Database, world: &WorldData) -> Result<()> {
    let mut registry = EntityRegistry::new(db);

    // Sync owner as a person entity
    if !world.owner.name.is_empty() {
        let owner_context = format!(
            "Owner of this Scriba instance. {} at {}",
            world.owner.role, world.owner.organization
        );
        match registry.get_entity_by_name(&world.owner.name)? {
            Some(entity) => {
                let entity_id = entity.id.unwrap();
                // Update context if world has richer info
                if owner_context.len() > entity.context.as_ref().map(|c| c.len()).unwrap_or(0) {
                    registry.update_entity_context(entity_id, &owner_context)?;
                }
                // Sync aliases from world
                for alias in &world.owner.aliases {
                    registry.add_entity_alias(entity_id, alias)?;
                }
            }
            None => {
                let entity = registry.create_entity("person", &world.owner.name, Some(&owner_context))?;
                if let Some(entity_id) = entity.id {
                    for alias in &world.owner.aliases {
                        registry.add_entity_alias(entity_id, alias)?;
                    }
                }
            }
        }
    }

    // Sync people
    for person in &world.people {
        if person.name.is_empty() {
            continue;
        }
        match registry.get_entity_by_name(&person.name)? {
            Some(entity) => {
                let entity_id = entity.id.unwrap();
                if !person.relationship.is_empty()
                    && person.relationship.len() > entity.context.as_ref().map(|c| c.len()).unwrap_or(0)
                {
                    registry.update_entity_context(entity_id, &person.relationship)?;
                }
            }
            None => {
                let ctx = if person.relationship.is_empty() {
                    None
                } else {
                    Some(person.relationship.as_str())
                };
                registry.create_entity("person", &person.name, ctx)?;
            }
        }
    }

    // Sync organizations
    for org in &world.organizations {
        if org.name.is_empty() {
            continue;
        }
        match registry.get_entity_by_name(&org.name)? {
            Some(entity) => {
                let entity_id = entity.id.unwrap();
                if !org.description.is_empty()
                    && org.description.len() > entity.context.as_ref().map(|c| c.len()).unwrap_or(0)
                {
                    registry.update_entity_context(entity_id, &org.description)?;
                }
                // Sync aliases from world
                for alias in &org.aliases {
                    registry.add_entity_alias(entity_id, alias)?;
                }
            }
            None => {
                let ctx = if org.description.is_empty() {
                    None
                } else {
                    Some(org.description.as_str())
                };
                let entity = registry.create_entity("organization", &org.name, ctx)?;
                if let Some(entity_id) = entity.id {
                    for alias in &org.aliases {
                        registry.add_entity_alias(entity_id, alias)?;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Health status for workflow manager.
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
