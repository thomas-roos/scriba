use crate::core::{
    CloudProvider, CompressionSettings, EnrichmentMode, LocalModelSize, RecordOptions, RecordingResult, ScribaConfig,
    TranscriptionMode, VoiceCommand, VoiceDetectorHandle, VoiceListeningState,
    WorkflowManager, record_audio, rebuild_world_from_entities, initialize_world_from_seed,
    start_voice_detector,
};
use crate::database::{Database, Entity, Recording, RecordingStats};
use crate::enrichment::{OllamaClient, WorldContext, WorldData, WorldEntityExtractionResult};
use crate::enrichment::chat_prompts;
use crate::entities::EntityRegistry;
use crate::utils::generate_recording_name;
use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table, TableState, Wrap},
    Frame, Terminal,
};
use std::io;
use tokio::sync::mpsc;

use anyhow::Context;
use dirs::home_dir;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command as TokioCommand;

// ─────────────────────────────────────────────────────────────────────────────
// Chat types for "Ask Scriba"
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum ChatStreamEvent {
    Status(String),
    Chunk(String),
    Done,
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
enum ChatRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, PartialEq)]
enum ChatContext {
    Global,
    Recording { recording_id: i64, recording_name: String },
}

#[derive(Debug, Clone)]
struct ChatMessage {
    role: ChatRole,
    content: String,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum ChatFocus {
    Table,
    ChatInput,
}

struct ChatState {
    context: ChatContext,
    messages: Vec<ChatMessage>,
    input_buffer: String,
    scroll_offset: usize,

    // Streaming state
    stream_rx: Option<mpsc::Receiver<ChatStreamEvent>>,
    current_status: Option<String>,
    streaming_content: String,
    is_generating: bool,

    // Generation task handle
    generation_task: Option<tokio::task::JoinHandle<()>>,

    // Suggestions
    suggestions: Vec<String>,
    show_suggestions: bool,
    selected_suggestion: usize,

    // Pre-built system prompt
    system_prompt: String,

    // Focus
    focus: ChatFocus,

    // Spinner frame
    spinner_frame: usize,

    // Auto-scroll: stays true until user manually scrolls up
    auto_scroll: bool,

    // Queued message: submitted while generating, will auto-send when done
    pending_message: Option<String>,

    // Last rendered chat panel area (for mouse hit-testing)
    panel_rect: ratatui::layout::Rect,

    // Total content lines (for scroll clamping in mouse handler)
    total_content_lines: usize,

    // Text selection state (click-drag to select, auto-copy on release)
    selection_anchor: Option<(usize, usize)>, // (content_line, char_col) where drag started
    selection_end: Option<(usize, usize)>,    // (content_line, char_col) current drag position
    content_texts: Vec<String>,               // plain text of each content line for extraction

    // Rendering cache for completed messages
    cached_msg_lines: Vec<Line<'static>>,
    cached_msg_texts: Vec<String>,
    cached_msg_count: usize,
    cached_width: usize,
}

impl ChatState {
    fn new() -> Self {
        Self {
            context: ChatContext::Global,
            messages: Vec::new(),
            input_buffer: String::new(),
            scroll_offset: 0,
            stream_rx: None,
            current_status: None,
            streaming_content: String::new(),
            is_generating: false,
            generation_task: None,
            suggestions: Vec::new(),
            show_suggestions: true,
            selected_suggestion: 0,
            system_prompt: String::new(),
            focus: ChatFocus::Table,
            spinner_frame: 0,
            auto_scroll: true,
            pending_message: None,
            panel_rect: ratatui::layout::Rect::default(),
            total_content_lines: 0,
            selection_anchor: None,
            selection_end: None,
            content_texts: Vec::new(),
            cached_msg_lines: Vec::new(),
            cached_msg_texts: Vec::new(),
            cached_msg_count: 0,
            cached_width: 0,
        }
    }
}

pub struct Dashboard {
    db: Database,
    recordings: Vec<Recording>,
    table_state: TableState,
    current_page: usize,
    page_size: usize,
    stats: Option<RecordingStats>,
    show_help: bool,
    current_view: DashboardView,
    search_mode: bool,
    search_query: String,
    show_message: bool,
    message: String,
    show_transcript: bool,
    transcript_content: String,
    transcript_scroll_offset: usize,
    show_delete_confirm: bool,
    delete_candidate: Option<Recording>,
    current_playback_pid: Option<u32>,
    playback_finished_rx: Option<mpsc::Receiver<()>>, // Channel to receive playback completion
    last_transcribe_warning: Option<usize>, // Track which recording showed overwrite warning
    progress_animation: Option<String>,     // Base message for progress animation
    progress_frame: usize,                  // Animation frame counter
    transcription_task: Option<tokio::task::JoinHandle<Result<(), anyhow::Error>>>, // Background transcription task
    transcribing_recording_name: Option<String>, // directory_name of recording being transcribed
    notification_message: Option<(String, usize)>, // (message, frames_remaining) — auto-dismiss
    header_anim_frame: usize, // Main header owl animation counter
    recording_task: Option<tokio::task::JoinHandle<Result<RecordingResult, anyhow::Error>>>,
    recording_mode: Option<RecordingMode>, // Track if we should transcribe after recording
    recording_stop_tx: Option<mpsc::Sender<()>>, // Channel to stop recording
    recording_level_rx: Option<mpsc::Receiver<f32>>, // Channel to receive volume levels
    current_volume_level: f32,             // Current recording volume for display
    config: ScribaConfig,                  // App configuration
    settings_selection: usize,             // Current setting selection
    editing_api_key: bool,                 // Whether we're editing API key
    api_key_input: String,                 // API key input buffer
    editing_enrichment_model: bool,         // Whether we're editing enrichment model
    enrichment_model_input: String,        // Enrichment model input buffer
    editing_enrichment_endpoint: bool,     // Whether we're editing Ollama endpoint (local mode)
    enrichment_endpoint_input: String,     // Ollama endpoint input buffer (local mode)
    editing_enrichment_api_key: bool,      // Whether we're editing enrichment API key
    enrichment_api_key_input: String,      // Enrichment API key input buffer
    return_to_view: Option<DashboardView>, // View to return to after message dismissal
    // File import dialog state
    show_file_dialog: bool,
    file_path_input: String,
    file_name_input: String,
    file_dialog_stage: FileDialogStage, // Current stage of file import process

    // Entity view state
    entities: Vec<Entity>,
    entity_table_state: TableState,
    selected_entity: Option<Entity>,
    show_entity_detail: bool,
    entity_mode: EntityMode,
    entity_edit_field: EntityEditField,
    entity_edit_name: String,
    entity_edit_type: String,
    entity_edit_context: String,
    merge_source_entity: Option<Entity>,
    // Add entity state
    entity_add_name: String,
    entity_add_type: String,
    entity_add_context: String,
    entity_add_aliases: String,
    entity_add_field: EntityEditField,
    // Owl world view state
    owl_quip: String,
    owl_quip_timer: usize,
    owl_world_anim_frame: usize,
    owl_world_mood: OwlWorldMood,
    // Transcript enrichment data
    transcript_summary: Option<String>,
    transcript_key_points: Option<Vec<String>>,
    transcript_topics: Option<Vec<String>>,
    transcript_entities: Option<Vec<(String, String)>>, // (name, type)

    // Onboarding state
    onboarding: Option<OnboardingState>,

    // Voice mode ("Scriba Forever") state
    voice_command_rx: Option<mpsc::Receiver<VoiceCommand>>,
    voice_detector_handle: Option<VoiceDetectorHandle>,
    voice_mode_active: bool,

    // Chat state ("Ask Scriba")
    chat: ChatState,
    global_chat_messages: Vec<ChatMessage>,
    // Track the currently-viewed recording for chat context
    current_transcript_recording: Option<Recording>,
}

#[derive(Debug, PartialEq)]
enum DashboardView {
    Main,
    Help,
    Settings,
    Entities,
    Onboarding,
}

// ─────────────────────────────────────────────────────────────────────────────
// Onboarding: Scriba the Owl
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum OnboardingStep {
    Entrance,
    Intro,
    ModeSelection,
    ProviderSelection,
    ApiKeyEntry,
    ApiKeyValidation,
    AskName,
    AskRole,
    AskAliases,
    Processing,
    Confirmation,
    Done,
}

struct OnboardingState {
    step: OnboardingStep,
    // Typewriter
    full_text: String,
    visible_chars: usize,
    text_complete: bool,
    // Animation
    anim_frame: usize,
    entrance_complete: bool,
    // Mode selection
    selected_mode: usize,         // 0 = Cloud, 1 = Privacy (local)
    selected_provider: usize,     // 0 = Anthropic, 1 = OpenAI, 2 = Google
    api_key_input: String,
    api_key_valid: Option<bool>,  // None = not checked, Some(true/false) = result
    validation_task: Option<tokio::task::JoinHandle<Result<bool, anyhow::Error>>>,
    // User inputs
    user_name: String,
    user_role: String,
    user_aliases: String,
    // Processing
    /// Returns (world_result, provider_hint). Hint is set when provider is unavailable.
    processing_task: Option<tokio::task::JoinHandle<Result<(Option<(WorldData, WorldEntityExtractionResult)>, Option<String>), anyhow::Error>>>,
    processed_world: Option<WorldData>,
    processed_entities: Option<WorldEntityExtractionResult>,
    ollama_available: bool,
    // Transition
    transition_frame: usize,
    transitioning: bool,
    // Confirmation data (parsed from world)
    confirm_owner: String,
    confirm_role: String,
    confirm_org: String,
    confirm_people: String,
}

impl OnboardingState {
    fn new() -> Self {
        Self {
            step: OnboardingStep::Entrance,
            full_text: String::new(),
            visible_chars: 0,
            text_complete: false,
            anim_frame: 0,
            entrance_complete: false,
            selected_mode: 0,
            selected_provider: 0,
            api_key_input: String::new(),
            api_key_valid: None,
            validation_task: None,
            user_name: String::new(),
            user_role: String::new(),
            user_aliases: String::new(),
            processing_task: None,
            processed_world: None,
            processed_entities: None,
            ollama_available: true,
            transition_frame: 0,
            transitioning: false,
            confirm_owner: String::new(),
            confirm_role: String::new(),
            confirm_org: String::new(),
            confirm_people: String::new(),
        }
    }

    fn set_step_text(&mut self, text: &str) {
        self.full_text = text.to_string();
        self.visible_chars = 0;
        self.text_complete = false;
    }

    fn visible_text(&self) -> &str {
        let end = self.visible_chars.min(self.full_text.len());
        // Make sure we don't split a multi-byte char
        let mut byte_end = 0;
        for (i, (idx, _)) in self.full_text.char_indices().enumerate() {
            if i >= end {
                break;
            }
            byte_end = idx;
        }
        if end > 0 {
            // Include the last char
            if let Some((_, ch)) = self.full_text.char_indices().nth(end - 1) {
                byte_end += ch.len_utf8();
            }
        }
        &self.full_text[..byte_end]
    }

    fn tick_typewriter(&mut self) {
        if !self.text_complete {
            let char_count = self.full_text.chars().count();
            self.visible_chars = (self.visible_chars + 4).min(char_count);
            if self.visible_chars >= char_count {
                self.text_complete = true;
            }
        }
    }

    fn complete_text(&mut self) {
        self.visible_chars = self.full_text.chars().count();
        self.text_complete = true;
    }
}

// Owl animation frames
const OWL_IDLE: [&str; 2] = [
    // Normal
    "   (o,o)\n   {`\"'}\n   -\"-\"-",
    // Blink
    "   (-,-)\n   {`\"'}\n   -\"-\"-",
];

// Entrance frames: owl flies in from right (position offset decreases)
const OWL_FLYING: &str = "~(o,o)~";
const OWL_APPROACH: &str = "(^,^)";
const OWL_LANDED: &str = "   (o,o)\n   {`\"'}\n   -\"-\"-";

// Thinking animation: 8 frames (played at 1/3 speed — one change every ~300ms)
const OWL_THINKING: [&str; 8] = [
    "   (o,o)  ?\n   {`\"'}\n   -\"-\"-",
    "   (o,o)\n   {`~'}  ~\n   -\"-\"-",
    "   (o,o)  !\n   {`\"'}\n   -\"-\"-",
    "   (o,o)\n   {`~'}  ~~\n   -\"-\"-",
    "   (-,-)  ?\n   {`\"'}\n   -\"-\"-",
    "   (o,o)\n   {`~'}  ~~~\n   -\"-\"-",
    "   (o,o)  !\n   {`\"'}\n   -\"-\"-",
    "   (o,o)  ...\n   {`\"'}\n   -\"-\"-",
];

// Celebration: 4 frames
const OWL_CELEBRATE: [&str; 4] = [
    "  \\(^,^)/\n   {`\"'}\n   -\"-\"-",
    "  /(^,^)\\\n   {`\"'}\n   -\"-\"-",
    "  \\(^,^)/\n   {`\"'}\n   -\"-\"-",
    "   (^,^)\n   {`\"'}\n   -\"-\"-",
];

// Sparkle characters for magic transition
const SPARKLE_CHARS: [char; 5] = ['*', '+', '.', '~', ' '];

#[derive(Debug, PartialEq)]
enum FileDialogStage {
    FilePath, // Asking for file path
    FileName, // Asking for display name (optional)
}

#[derive(Debug, Clone)]
enum RecordingMode {
    RecordAndTranscribe,
}

#[derive(Debug, PartialEq)]
enum EntityMode {
    Browse,
    Editing,
    Adding,
    DeleteConfirm,
    MergeSelectTarget,
    MergeConfirm,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum OwlWorldMood {
    Idle,
    Thinking,
    Celebrating,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum EntityEditField {
    Name,
    Type,
    Context,
}

const ENTITY_TYPES: &[&str] = &["person", "organization", "project", "other"];

#[derive(Debug)]
enum DashboardAction {
    Continue,
    Quit,
    RecordAndTranscribe,
    AddExternalFile,
    TranscribeSelected,
}

impl Dashboard {
    pub fn new() -> Result<Self> {
        let db = Database::new()?;
        let mut table_state = TableState::default();
        table_state.select(Some(0));
        let config = ScribaConfig::load()?;

        Ok(Self {
            db,
            recordings: Vec::new(),
            table_state,
            current_page: 0,
            page_size: 50, // Show more recordings per page
            stats: None,
            show_help: false,
            current_view: DashboardView::Main,
            search_mode: false,
            search_query: String::new(),
            show_message: false,
            message: String::new(),
            show_transcript: false,
            transcript_content: String::new(),
            transcript_scroll_offset: 0,
            show_delete_confirm: false,
            delete_candidate: None,
            current_playback_pid: None,
            playback_finished_rx: None,
            last_transcribe_warning: None,
            progress_animation: None,
            progress_frame: 0,
            transcription_task: None,
            transcribing_recording_name: None,
            notification_message: None,
            header_anim_frame: 0,
            recording_task: None,
            recording_mode: None,
            recording_stop_tx: None,
            recording_level_rx: None,
            current_volume_level: 0.0,
            config,
            settings_selection: 0,
            editing_api_key: false,
            api_key_input: String::new(),
            editing_enrichment_model: false,
            enrichment_model_input: String::new(),
            editing_enrichment_endpoint: false,
            enrichment_endpoint_input: String::new(),
            editing_enrichment_api_key: false,
            enrichment_api_key_input: String::new(),
            return_to_view: None,
            // File import dialog state
            show_file_dialog: false,
            file_path_input: String::new(),
            file_name_input: String::new(),
            file_dialog_stage: FileDialogStage::FilePath,

            // Entity view state
            entities: Vec::new(),
            entity_table_state: TableState::default(),
            selected_entity: None,
            show_entity_detail: false,
            entity_mode: EntityMode::Browse,
            entity_edit_field: EntityEditField::Name,
            entity_edit_name: String::new(),
            entity_edit_type: String::new(),
            entity_edit_context: String::new(),
            merge_source_entity: None,
            // Add entity state
            entity_add_name: String::new(),
            entity_add_type: "person".to_string(),
            entity_add_context: String::new(),
            entity_add_aliases: String::new(),
            entity_add_field: EntityEditField::Name,
            // Owl world view state
            owl_quip: String::new(),
            owl_quip_timer: 0,
            owl_world_anim_frame: 0,
            owl_world_mood: OwlWorldMood::Idle,
            // Transcript enrichment data
            transcript_summary: None,
            transcript_key_points: None,
            transcript_topics: None,
            transcript_entities: None,

            // Onboarding
            onboarding: None,

            // Voice mode
            voice_command_rx: None,
            voice_detector_handle: None,
            voice_mode_active: false,

            // Chat
            chat: ChatState::new(),
            global_chat_messages: Vec::new(),
            current_transcript_recording: None,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Load initial data
        self.load_recordings()?;
        self.load_stats()?;

        // Check if onboarding is needed (no world.md exists)
        if !WorldContext::exists() {
            self.current_view = DashboardView::Onboarding;
            self.onboarding = Some(OnboardingState::new());
        } else {
            // Initialize chat context for global view
            self.load_entities().ok();
            self.init_global_chat();
        }

        let result = self.run_app(&mut terminal).await;

        // Shut down voice detector if active
        if let Some(handle) = self.voice_detector_handle.take() {
            handle.shutdown();
        }

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    async fn run_app<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        let mut last_anim_tick = std::time::Instant::now();
        let anim_interval = Duration::from_millis(100); // animations stay at ~10fps

        'main: loop {
            // ── Animation tick (throttled to ~100ms) ──────────────────────
            let now = std::time::Instant::now();
            let anim_tick = now.duration_since(last_anim_tick) >= anim_interval;
            if anim_tick {
                last_anim_tick = now;
            }

            // Check if recording task completed
            if let Some(task) = &mut self.recording_task {
                if task.is_finished() {
                    let completed_task = self.recording_task.take().unwrap();
                    let recording_mode = self.recording_mode.take();

                    // Clean up channels
                    self.recording_stop_tx = None;
                    self.recording_level_rx = None;
                    self.current_volume_level = 0.0;

                    match completed_task.await {
                        Ok(Ok(result)) => {
                            let recording_name = result.recording_name;
                            let auto_stopped = result.auto_stopped;

                            // Show owl quip if silence auto-stopped
                            if auto_stopped {
                                self.notification_message = Some((
                                    "Hoo left the mic on? I stopped it for you.".to_string(),
                                    120,
                                ));
                            }

                            // Recording completed successfully
                            if let Some(RecordingMode::RecordAndTranscribe) = recording_mode {
                                // Dismiss recording modal — transcription runs non-blocking
                                self.stop_progress_animation();
                                self.show_message = false;
                                self.message.clear();
                                self.transcribing_recording_name = Some(recording_name.clone());
                                let _ = self.load_recordings();
                                let _ = self.load_stats();

                                // Start auto-transcription in background
                                let transcription_mode = self.config.transcription.clone();
                                let recording_name_clone = recording_name.clone();

                                self.transcription_task = Some(tokio::spawn(async move {
                                    let mut workflow = WorkflowManager::new().unwrap();
                                    workflow
                                        .retranscribe_recording_silent(
                                            &recording_name_clone,
                                            transcription_mode,
                                        )
                                        .await
                                }));
                            } else {
                                // Recording only mode - complete
                                self.stop_progress_animation();
                                self.message = "Recording complete.".to_string();
                                self.show_message = true;
                                // Reload data to show new recording
                                let _ = self.load_recordings();
                                let _ = self.load_stats();
                            }
                        }
                        Ok(Err(err)) => {
                            self.stop_progress_animation();
                            self.message = format!("Recording failed: {}", err);
                            self.show_message = true;
                        }
                        Err(_) => {
                            self.stop_progress_animation();
                            self.message = "Recording task failed.".to_string();
                            self.show_message = true;
                        }
                    }
                }
            }

            // Check if transcription task completed
            if let Some(task) = &mut self.transcription_task {
                if task.is_finished() {
                    let completed_task = self.transcription_task.take().unwrap();
                    match completed_task.await {
                        Ok(Ok(())) => {
                            self.transcribing_recording_name = None;
                            self.notification_message = Some(("Transcription complete!".to_string(), 30));
                            let _ = self.load_recordings();
                            let _ = self.load_stats();
                        }
                        Ok(Err(err)) => {
                            self.transcribing_recording_name = None;
                            self.notification_message = Some((format!("Transcription failed: {}", err), 50));
                        }
                        Err(_) => {
                            self.transcribing_recording_name = None;
                            self.notification_message = Some(("Transcription task failed".to_string(), 50));
                        }
                    }
                }
            }

            // Receive volume levels from recording
            if let Some(level_rx) = &mut self.recording_level_rx {
                if let Ok(level) = level_rx.try_recv() {
                    self.current_volume_level = level;
                }
            }

            // Check for playback completion
            if let Some(finished_rx) = &mut self.playback_finished_rx {
                if finished_rx.try_recv().is_ok() {
                    self.current_playback_pid = None;
                    self.playback_finished_rx = None;
                }
            }

            // Check for voice commands
            if let Some(ref mut rx) = self.voice_command_rx {
                if let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        VoiceCommand::Record => {
                            self.handle_voice_record_command().await;
                        }
                        VoiceCommand::Stop => {
                            self.handle_voice_stop_command().await;
                        }
                    }
                }
            }

            // Update progress animation if active (throttled)
            if anim_tick && self.progress_animation.is_some() {
                self.update_progress_message();
            }

            // Tick progress frame for inline transcription animation (throttled)
            if anim_tick && self.transcribing_recording_name.is_some() {
                self.progress_frame = self.progress_frame.wrapping_add(1);
            }

            // Auto-dismiss notification countdown (throttled)
            if anim_tick {
                if let Some((_, ref mut frames)) = self.notification_message {
                    if *frames == 0 {
                        self.notification_message = None;
                    } else {
                        *frames -= 1;
                    }
                }
            }

            // Onboarding tick logic (runs every frame for typewriter + async polling)
            if let Some(ref mut ob) = self.onboarding {
                if anim_tick {
                    ob.anim_frame = ob.anim_frame.wrapping_add(1);
                }

                match ob.step {
                    OnboardingStep::Entrance => {
                        // Auto-advance after 40 frames (~4s)
                        if ob.anim_frame >= 40 {
                            ob.entrance_complete = true;
                            ob.step = OnboardingStep::Intro;
                            ob.set_step_text(
                                "Hoo hoo! I'm Scriba, your personal audio owl.\n\n\
                                 I listen to your recordings, transcribe them,\n\
                                 and remember everything -- names, places, topics.\n\
                                 Think of me as your wise little note-taker\n\
                                 with very large ears.\n\n\
                                 Let me get to know you first!"
                            );
                        }
                    }
                    OnboardingStep::Intro | OnboardingStep::ModeSelection | OnboardingStep::ProviderSelection
                    | OnboardingStep::ApiKeyEntry
                    | OnboardingStep::AskName | OnboardingStep::AskRole | OnboardingStep::AskAliases => {
                        ob.tick_typewriter();
                    }
                    OnboardingStep::ApiKeyValidation => {
                        ob.tick_typewriter();
                        // Poll the async validation task
                        if let Some(ref task) = ob.validation_task {
                            if task.is_finished() {
                                let completed = ob.validation_task.take().unwrap();
                                match completed.await {
                                    Ok(Ok(valid)) => {
                                        ob.api_key_valid = Some(valid);
                                        if valid {
                                            ob.step = OnboardingStep::AskName;
                                            ob.anim_frame = 0;
                                            ob.set_step_text(
                                                "API key works! We're in business.\n\n\
                                                 So, who am I working for?\n\nWhat's your name?"
                                            );
                                        } else {
                                            ob.set_step_text(
                                                "Hmm, that key didn't work.\n\n\
                                                 [1] Try a different key\n\
                                                 [2] Skip for now (you can set it later)"
                                            );
                                        }
                                    }
                                    _ => {
                                        ob.api_key_valid = Some(false);
                                        ob.set_step_text(
                                            "Hmm, that key didn't work.\n\n\
                                             [1] Try a different key\n\
                                             [2] Skip for now (you can set it later)"
                                        );
                                    }
                                }
                            }
                        }
                    }
                    OnboardingStep::Processing => {
                        ob.tick_typewriter();
                        // Check if processing task completed
                        if let Some(ref task) = ob.processing_task {
                            if task.is_finished() {
                                let completed = ob.processing_task.take().unwrap();
                                match completed.await {
                                    Ok(Ok((Some((world_data, entities)), _))) => {
                                        // Fill confirmation data
                                        ob.confirm_owner = world_data.owner.name.clone();
                                        ob.confirm_role = world_data.owner.role.clone();
                                        ob.confirm_org = world_data.owner.organization.clone();
                                        let people_names: Vec<String> = world_data.people.iter()
                                            .map(|p| p.name.clone()).collect();
                                        ob.confirm_people = if people_names.is_empty() {
                                            "(none detected)".to_string()
                                        } else {
                                            people_names.join(", ")
                                        };
                                        ob.processed_world = Some(world_data);
                                        ob.processed_entities = Some(entities);
                                        ob.ollama_available = true;
                                        // Advance to confirmation
                                        ob.step = OnboardingStep::Confirmation;
                                        let text = format!(
                                            "Here's what I've got:\n\n\
                                             Owner: {}\n\
                                             Role:  {}\n\
                                             Org:   {}\n\
                                             Known: {}\n\n\
                                             Does that look right?",
                                            ob.confirm_owner, ob.confirm_role, ob.confirm_org, ob.confirm_people
                                        );
                                        ob.set_step_text(&text);
                                    }
                                    Ok(Ok((None, hint))) => {
                                        // Ollama not available — show actionable hint
                                        ob.ollama_available = false;
                                        let msg = if let Some(hint) = hint {
                                            format!(
                                                "Hm, I can't think right now.\n\n\
                                                 {}\n\n\
                                                 No worries -- I saved your info as-is.\n\
                                                 Fix that, restart scriba, and I'll be much smarter!",
                                                hint
                                            )
                                        } else {
                                            "Hm, my brain (Ollama) seems to be sleeping.\n\n\
                                             No worries -- I saved your info as-is.\n\
                                             Set up Ollama later and I'll get much smarter!".to_string()
                                        };
                                        ob.set_step_text(&msg);
                                    }
                                    Ok(Err(_)) | Err(_) => {
                                        ob.ollama_available = false;
                                        ob.set_step_text(
                                            "Hm, something went wrong with my brain.\n\n\
                                             No worries -- I saved your info as-is.\n\
                                             Set up Ollama later and I'll get much smarter!"
                                        );
                                    }
                                }
                            }
                        }
                    }
                    OnboardingStep::Confirmation => {
                        ob.tick_typewriter();
                    }
                    OnboardingStep::Done => {
                        ob.tick_typewriter();
                        // After text is complete and celebration plays (~3s), start transition
                        if ob.text_complete && ob.anim_frame > 40 && !ob.transitioning {
                            ob.transitioning = true;
                            ob.transition_frame = 0;
                        }
                        if ob.transitioning {
                            ob.transition_frame += 1;
                            if ob.transition_frame > 30 {
                                // Transition complete — go to main dashboard
                                self.onboarding = None;
                                self.current_view = DashboardView::Main;
                                self.load_entities().ok();
                                self.init_global_chat();
                            }
                        }
                    }
                }
            }

            // Poll chat stream events (non-blocking — every frame for smooth streaming)
            self.poll_chat_stream();

            // Animation counters (throttled to ~10fps)
            if anim_tick {
                self.chat.spinner_frame = self.chat.spinner_frame.wrapping_add(1);

                if self.current_view == DashboardView::Main {
                    self.header_anim_frame = self.header_anim_frame.wrapping_add(1);
                }

                if self.current_view == DashboardView::Entities {
                    self.owl_world_anim_frame = self.owl_world_anim_frame.wrapping_add(1);
                    if self.owl_quip_timer > 0 {
                        self.owl_quip_timer -= 1;
                        if self.owl_quip_timer == 0 {
                            self.owl_world_mood = OwlWorldMood::Idle;
                            self.set_owl_quip_for_browse();
                        }
                    }
                    if self.owl_world_mood == OwlWorldMood::Celebrating && self.owl_quip_timer == 0 {
                        self.owl_world_mood = OwlWorldMood::Idle;
                    }
                }
            }

            terminal.draw(|f| self.ui(f))?;

            // Process all pending events (drain queue for smooth scrolling)
            while event::poll(Duration::from_millis(0))? {
                match event::read()? {
                    Event::Key(key) => {
                        if key.kind == KeyEventKind::Press {
                            match self.handle_key_event(key.code).await {
                                Ok(DashboardAction::Continue) => {}
                                Ok(DashboardAction::Quit) => break 'main,
                                Ok(action) => {
                                    self.handle_dashboard_action(action).await?;
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    }
                    Event::Mouse(mouse) => {
                        self.handle_mouse_event(mouse);
                    }
                    _ => {}
                }
            }

            // Sleep to target ~30fps when no events pending
            tokio::time::sleep(Duration::from_millis(16)).await;
        }
        Ok(())
    }

    async fn handle_key_event(&mut self, key_code: KeyCode) -> Result<DashboardAction> {
        // If audio is playing and ESC is pressed, stop it immediately
        if matches!(key_code, KeyCode::Esc) {
            // Check if we're in audio playback mode (either with PID or with playback message)
            let is_playing_audio = self.current_playback_pid.is_some()
                || (self.show_message && self.message.contains("Playing:"));

            if is_playing_audio {
                // Try both methods to ensure reliable stopping
                if let Some(pid) = self.current_playback_pid {
                    self.stop_audio_playback(pid)?;
                }
                // Also use emergency stop as a fallback (in case PID method fails)
                self.emergency_stop_all_audio_players()?;

                // Clear playback state
                self.current_playback_pid = None;
                self.playback_finished_rx = None;
                self.show_message = false;
                self.message.clear();

                // Audio playback stops immediately - return to dashboard
                return Ok(DashboardAction::Continue);
            }
        }

        // If recording is active and Escape is pressed, stop recording
        if self.recording_task.is_some() && matches!(key_code, KeyCode::Esc) {
            // Send stop signal to recording task
            if let Some(stop_tx) = self.recording_stop_tx.take() {
                let _ = stop_tx.send(()).await;
            }
            // The recording task will handle cleanup and completion
            return Ok(DashboardAction::Continue);
        }

        // Onboarding key handling
        if self.current_view == DashboardView::Onboarding {
            return self.handle_onboarding_keys(key_code).await;
        }

        if self.show_file_dialog {
            return self.handle_file_dialog_keys(key_code).await;
        }

        // Dismiss notification on any keypress (without consuming the key)
        if self.notification_message.is_some() {
            self.notification_message = None;
        }

        if self.show_message {
            // Special-case: allow confirming re-transcribe overwrite with T while message is visible
            if matches!(key_code, KeyCode::Char('t') | KeyCode::Char('T'))
                && self.last_transcribe_warning.is_some()
                && self.progress_animation.is_none()
            {
                // Dismiss the warning and trigger the action
                self.show_message = false;
                self.message.clear();
                return Ok(DashboardAction::TranscribeSelected);
            }

            // Don't close message if progress animation is active
            if self.progress_animation.is_none() && matches!(key_code, KeyCode::Esc) {
                // Only Esc key closes the message popup (consistent behavior)
                self.show_message = false;
                self.message.clear();

                // Return to the previous view if one was set
                if let Some(return_view) = self.return_to_view.take() {
                    self.current_view = return_view;
                }
            }
            return Ok(DashboardAction::Continue);
        }

        if self.search_mode {
            return self.handle_search_input(key_code).await;
        }

        if self.show_help {
            self.show_help = false;
            self.current_view = DashboardView::Main;
            return Ok(DashboardAction::Continue);
        }

        if self.current_view == DashboardView::Settings {
            return self.handle_settings_keys(key_code).await;
        }

        if self.current_view == DashboardView::Entities {
            return self.handle_entities_keys(key_code).await;
        }

        if self.show_transcript {
            // Chat keys in transcript view take priority when chat is focused
            if self.handle_transcript_chat_key(key_code) {
                return Ok(DashboardAction::Continue);
            }
            return self.handle_transcript_keys(key_code).await;
        }

        if self.show_delete_confirm {
            return self.handle_delete_confirmation(key_code).await;
        }

        // Chat key handling (Tab focus toggle, chat input when focused)
        if self.handle_chat_key(key_code) {
            return Ok(DashboardAction::Continue);
        }

        match key_code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(DashboardAction::Quit),
            KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::F(1) => {
                self.show_help = true;
                self.current_view = DashboardView::Help;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.current_view = DashboardView::Settings;
                self.settings_selection = 0;
            }
            KeyCode::Char('w') | KeyCode::Char('W') => {
                self.load_entities()?;
                self.current_view = DashboardView::Entities;
                self.entity_table_state.select(Some(0));
                self.owl_world_anim_frame = 0;
                self.owl_world_mood = OwlWorldMood::Idle;
                self.set_owl_quip_for_browse();
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                return Ok(DashboardAction::RecordAndTranscribe);
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                return Ok(DashboardAction::AddExternalFile);
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                return Ok(DashboardAction::TranscribeSelected);
            }
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_query.clear();
            }
            KeyCode::Up => {
                self.previous_recording();
            }
            KeyCode::Down => {
                self.next_recording();
            }
            KeyCode::PageUp | KeyCode::Char('[') => {
                self.previous_page().await?;
            }
            KeyCode::PageDown | KeyCode::Char(']') => {
                self.next_page().await?;
            }
            KeyCode::Enter => match self.show_selected_transcript().await {
                Ok(()) => {}
                Err(e) => {
                    self.message = format!("Failed to load transcript: {}", e);
                    self.show_message = true;
                }
            },
            KeyCode::Char('d') => {
                self.show_delete_confirmation();
            }
            KeyCode::Delete => {
                self.show_delete_confirmation();
            }
            KeyCode::Char('p') | KeyCode::Char('P') => match self.play_selected_recording().await {
                Ok(()) => {}
                Err(e) => {
                    self.message = format!("Failed to play recording: {}", e);
                    self.show_message = true;
                }
            },
            _ => {}
        }

        Ok(DashboardAction::Continue)
    }

    async fn handle_dashboard_action(&mut self, action: DashboardAction) -> Result<()> {
        match action {
            DashboardAction::RecordAndTranscribe => {
                self.execute_record_and_transcribe().await?;
            }
            DashboardAction::AddExternalFile => {
                self.execute_add_external_file().await?;
            }
            DashboardAction::TranscribeSelected => {
                self.execute_transcribe_selected().await?;
            }
            _ => {}
        }
        Ok(())
    }

    fn load_recordings(&mut self) -> Result<()> {
        let offset = (self.current_page * self.page_size) as i64;

        self.recordings = if self.search_query.is_empty() {
            self.db
                .list_recordings(Some(self.page_size as i64), Some(offset))?
        } else {
            let search_results = self.db.search_transcripts(&self.search_query, None)?;
            search_results
                .into_iter()
                .map(|(recording, _)| recording)
                .collect()
        };

        if !self.recordings.is_empty() {
            self.table_state.select(Some(0));
        } else {
            self.table_state.select(None);
        }

        Ok(())
    }

    fn load_stats(&mut self) -> Result<()> {
        self.stats = Some(self.db.get_stats()?);
        Ok(())
    }

    fn load_entities(&mut self) -> Result<()> {
        self.entities = self.db.list_entities(None, None)?;
        if !self.entities.is_empty() && self.entity_table_state.selected().is_none() {
            self.entity_table_state.select(Some(0));
        }
        Ok(())
    }

    async fn handle_entities_keys(&mut self, key_code: KeyCode) -> Result<DashboardAction> {
        match self.entity_mode {
            EntityMode::Browse => {
                if self.show_entity_detail {
                    match key_code {
                        KeyCode::Esc => {
                            self.show_entity_detail = false;
                            self.selected_entity = None;
                        }
                        _ => {}
                    }
                    return Ok(DashboardAction::Continue);
                }

                match key_code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        self.current_view = DashboardView::Main;
                        self.show_entity_detail = false;
                        self.selected_entity = None;
                    }
                    KeyCode::Up => {
                        self.entity_navigate_up();
                    }
                    KeyCode::Down => {
                        self.entity_navigate_down();
                    }
                    KeyCode::Enter => {
                        if let Some(idx) = self.entity_table_state.selected() {
                            if let Some(entity) = self.entities.get(idx) {
                                self.selected_entity = Some(entity.clone());
                                self.show_entity_detail = true;
                            }
                        }
                    }
                    KeyCode::Char('e') | KeyCode::Char('E') => {
                        self.start_entity_edit();
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
                        if let Some(idx) = self.entity_table_state.selected() {
                            if let Some(entity) = self.entities.get(idx) {
                                self.selected_entity = Some(entity.clone());
                                self.entity_mode = EntityMode::DeleteConfirm;
                            }
                        }
                    }
                    KeyCode::Char('m') | KeyCode::Char('M') => {
                        if let Some(idx) = self.entity_table_state.selected() {
                            if let Some(entity) = self.entities.get(idx) {
                                self.merge_source_entity = Some(entity.clone());
                                self.entity_mode = EntityMode::MergeSelectTarget;
                            }
                        }
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        self.start_entity_add();
                    }
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        self.load_entities()?;
                    }
                    _ => {}
                }
            }
            EntityMode::Adding => {
                match key_code {
                    KeyCode::Esc => {
                        self.entity_mode = EntityMode::Browse;
                        self.owl_world_mood = OwlWorldMood::Idle;
                        self.set_owl_quip_for_browse();
                    }
                    KeyCode::Tab | KeyCode::Down => {
                        self.entity_add_field = match self.entity_add_field {
                            EntityEditField::Name => EntityEditField::Type,
                            EntityEditField::Type => EntityEditField::Context,
                            EntityEditField::Context => EntityEditField::Name,
                        };
                    }
                    KeyCode::BackTab | KeyCode::Up => {
                        self.entity_add_field = match self.entity_add_field {
                            EntityEditField::Name => EntityEditField::Context,
                            EntityEditField::Type => EntityEditField::Name,
                            EntityEditField::Context => EntityEditField::Type,
                        };
                    }
                    KeyCode::Enter => {
                        if self.entity_add_field == EntityEditField::Type {
                            // Cycle type on Enter
                            let current_idx = ENTITY_TYPES.iter().position(|t| *t == self.entity_add_type).unwrap_or(0);
                            self.entity_add_type = ENTITY_TYPES[(current_idx + 1) % ENTITY_TYPES.len()].to_string();
                        } else if !self.entity_add_name.trim().is_empty() {
                            // Save if name is not empty
                            self.save_entity_add()?;
                            self.entity_mode = EntityMode::Browse;
                        }
                    }
                    KeyCode::Char(' ') if self.entity_add_field == EntityEditField::Type => {
                        let current_idx = ENTITY_TYPES.iter().position(|t| *t == self.entity_add_type).unwrap_or(0);
                        self.entity_add_type = ENTITY_TYPES[(current_idx + 1) % ENTITY_TYPES.len()].to_string();
                    }
                    KeyCode::Backspace => {
                        match self.entity_add_field {
                            EntityEditField::Name => { self.entity_add_name.pop(); }
                            EntityEditField::Type => {}
                            EntityEditField::Context => { self.entity_add_context.pop(); }
                        }
                    }
                    KeyCode::Char(c) => {
                        match self.entity_add_field {
                            EntityEditField::Name => self.entity_add_name.push(c),
                            EntityEditField::Type => {}
                            EntityEditField::Context => self.entity_add_context.push(c),
                        }
                    }
                    _ => {}
                }
            }
            EntityMode::Editing => {
                match key_code {
                    KeyCode::Esc => {
                        self.save_entity_edit()?;
                        self.entity_mode = EntityMode::Browse;
                    }
                    KeyCode::Tab | KeyCode::Down => {
                        self.entity_edit_field = match self.entity_edit_field {
                            EntityEditField::Name => EntityEditField::Type,
                            EntityEditField::Type => EntityEditField::Context,
                            EntityEditField::Context => EntityEditField::Name,
                        };
                    }
                    KeyCode::BackTab | KeyCode::Up => {
                        self.entity_edit_field = match self.entity_edit_field {
                            EntityEditField::Name => EntityEditField::Context,
                            EntityEditField::Type => EntityEditField::Name,
                            EntityEditField::Context => EntityEditField::Type,
                        };
                    }
                    KeyCode::Enter => {
                        if self.entity_edit_field == EntityEditField::Type {
                            self.cycle_entity_type();
                        } else {
                            // Move to next field
                            self.entity_edit_field = match self.entity_edit_field {
                                EntityEditField::Name => EntityEditField::Type,
                                EntityEditField::Type => EntityEditField::Context,
                                EntityEditField::Context => EntityEditField::Name,
                            };
                        }
                    }
                    KeyCode::Char(' ') if self.entity_edit_field == EntityEditField::Type => {
                        self.cycle_entity_type();
                    }
                    KeyCode::Char(c) => {
                        match self.entity_edit_field {
                            EntityEditField::Name => self.entity_edit_name.push(c),
                            EntityEditField::Context => self.entity_edit_context.push(c),
                            EntityEditField::Type => {} // Type is cycled, not typed
                        }
                    }
                    KeyCode::Backspace => {
                        match self.entity_edit_field {
                            EntityEditField::Name => { self.entity_edit_name.pop(); }
                            EntityEditField::Context => { self.entity_edit_context.pop(); }
                            EntityEditField::Type => {}
                        }
                    }
                    _ => {}
                }
            }
            EntityMode::DeleteConfirm => {
                match key_code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        self.perform_entity_delete()?;
                        self.entity_mode = EntityMode::Browse;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.selected_entity = None;
                        self.entity_mode = EntityMode::Browse;
                    }
                    _ => {}
                }
            }
            EntityMode::MergeSelectTarget => {
                match key_code {
                    KeyCode::Up => {
                        self.entity_navigate_up();
                    }
                    KeyCode::Down => {
                        self.entity_navigate_down();
                    }
                    KeyCode::Enter => {
                        if let Some(idx) = self.entity_table_state.selected() {
                            if let Some(target) = self.entities.get(idx) {
                                let source_id = self.merge_source_entity.as_ref()
                                    .and_then(|e| e.id);
                                if source_id != target.id {
                                    self.selected_entity = Some(target.clone());
                                    self.entity_mode = EntityMode::MergeConfirm;
                                }
                            }
                        }
                    }
                    KeyCode::Esc => {
                        self.merge_source_entity = None;
                        self.entity_mode = EntityMode::Browse;
                    }
                    _ => {}
                }
            }
            EntityMode::MergeConfirm => {
                match key_code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        self.perform_entity_merge()?;
                        self.entity_mode = EntityMode::Browse;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.selected_entity = None;
                        self.merge_source_entity = None;
                        self.entity_mode = EntityMode::Browse;
                    }
                    _ => {}
                }
            }
        }
        Ok(DashboardAction::Continue)
    }

    fn entity_navigate_up(&mut self) {
        let i = match self.entity_table_state.selected() {
            Some(i) => if i == 0 { self.entities.len().saturating_sub(1) } else { i - 1 },
            None => 0,
        };
        self.entity_table_state.select(Some(i));
    }

    fn entity_navigate_down(&mut self) {
        let i = match self.entity_table_state.selected() {
            Some(i) => if i >= self.entities.len().saturating_sub(1) { 0 } else { i + 1 },
            None => 0,
        };
        self.entity_table_state.select(Some(i));
    }

    fn start_entity_edit(&mut self) {
        if let Some(idx) = self.entity_table_state.selected() {
            if let Some(entity) = self.entities.get(idx) {
                self.selected_entity = Some(entity.clone());
                self.entity_edit_name = entity.canonical_name.clone();
                self.entity_edit_type = entity.entity_type.clone();
                self.entity_edit_context = entity.context.clone().unwrap_or_default();
                self.entity_edit_field = EntityEditField::Name;
                self.entity_mode = EntityMode::Editing;
                self.owl_world_mood = OwlWorldMood::Thinking;
                self.owl_quip = "Editing... I'm watching closely.".to_string();
                self.owl_quip_timer = 0;
            }
        }
    }

    fn cycle_entity_type(&mut self) {
        let current_idx = ENTITY_TYPES.iter()
            .position(|t| *t == self.entity_edit_type)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % ENTITY_TYPES.len();
        self.entity_edit_type = ENTITY_TYPES[next_idx].to_string();
    }

    fn save_entity_edit(&mut self) -> Result<()> {
        if let Some(entity) = &self.selected_entity {
            let entity_id = match entity.id {
                Some(id) => id,
                None => return Ok(()),
            };

            let mut registry = EntityRegistry::new(&mut self.db);

            // Update name if changed
            if self.entity_edit_name != entity.canonical_name && !self.entity_edit_name.is_empty() {
                registry.rename_entity(entity_id, &self.entity_edit_name)?;
            }

            // Update type if changed
            if self.entity_edit_type != entity.entity_type {
                registry.update_entity_type(entity_id, &self.entity_edit_type)?;
            }

            // Update context if changed
            let old_context = entity.context.as_deref().unwrap_or("");
            if self.entity_edit_context != old_context {
                let ctx = if self.entity_edit_context.is_empty() { "" } else { &self.entity_edit_context };
                registry.update_entity_context(entity_id, ctx)?;
            }

            drop(registry);
            let _ = rebuild_world_from_entities(&self.db);
            self.load_entities()?;
            self.selected_entity = None;
            self.set_owl_quip_action("Noted! Knowledge updated.");
        }
        Ok(())
    }

    fn perform_entity_delete(&mut self) -> Result<()> {
        if let Some(entity) = &self.selected_entity {
            if let Some(id) = entity.id {
                self.db.delete_entity(id)?;
                let _ = rebuild_world_from_entities(&self.db);
                self.load_entities()?;
                // Adjust selection if needed
                if let Some(selected) = self.entity_table_state.selected() {
                    if selected >= self.entities.len() && !self.entities.is_empty() {
                        self.entity_table_state.select(Some(self.entities.len() - 1));
                    }
                }
            }
        }
        self.selected_entity = None;
        self.owl_quip = "Gone but not forgotten... well, actually forgotten.".to_string();
        self.owl_quip_timer = 30;
        self.owl_world_mood = OwlWorldMood::Idle;
        Ok(())
    }

    fn perform_entity_merge(&mut self) -> Result<()> {
        let source_id = self.merge_source_entity.as_ref().and_then(|e| e.id);
        let target_id = self.selected_entity.as_ref().and_then(|e| e.id);

        if let (Some(src), Some(tgt)) = (source_id, target_id) {
            let mut registry = EntityRegistry::new(&mut self.db);
            registry.merge_entities(src, tgt)?;
            drop(registry);
            let _ = rebuild_world_from_entities(&self.db);
            self.load_entities()?;
        }

        self.merge_source_entity = None;
        self.selected_entity = None;
        self.set_owl_quip_action("Two become one. Efficient!");
        Ok(())
    }

    fn set_owl_quip_for_browse(&mut self) {
        let count = self.entities.len();
        self.owl_quip = match count {
            0 => "Nothing here yet... record something and I'll fill this up!".to_string(),
            1..=3 => "Just getting started... record more!".to_string(),
            4..=10 => "A growing world... I like it.".to_string(),
            _ => "Hoo knows everyone around here!".to_string(),
        };
        self.owl_quip_timer = 0;
    }

    fn set_owl_quip_action(&mut self, quip: &str) {
        self.owl_quip = quip.to_string();
        self.owl_quip_timer = 30;
        self.owl_world_mood = OwlWorldMood::Celebrating;
    }

    fn start_entity_add(&mut self) {
        self.entity_add_name.clear();
        self.entity_add_type = "person".to_string();
        self.entity_add_context.clear();
        self.entity_add_aliases.clear();
        self.entity_add_field = EntityEditField::Name;
        self.entity_mode = EntityMode::Adding;
        self.owl_world_mood = OwlWorldMood::Thinking;
        self.owl_quip = "Tell me about this new entry...".to_string();
        self.owl_quip_timer = 0;
    }

    fn save_entity_add(&mut self) -> Result<()> {
        if self.entity_add_name.trim().is_empty() {
            return Ok(());
        }
        let mut registry = EntityRegistry::new(&mut self.db);
        let entity = registry.create_entity(
            self.entity_add_type.trim(),
            self.entity_add_name.trim(),
            if self.entity_add_context.trim().is_empty() {
                None
            } else {
                Some(self.entity_add_context.trim())
            },
        )?;
        // Add aliases if provided
        if let Some(id) = entity.id {
            for alias in self.entity_add_aliases.split(',') {
                let alias = alias.trim();
                if !alias.is_empty() {
                    registry.add_entity_alias(id, alias)?;
                }
            }
        }
        drop(registry);
        let _ = rebuild_world_from_entities(&self.db);
        self.load_entities()?;
        // Select the new entity (should be last or find by name)
        if let Some(pos) = self.entities.iter().position(|e| e.canonical_name == self.entity_add_name.trim()) {
            self.entity_table_state.select(Some(pos));
        }
        self.set_owl_quip_action("Hoo hoo! Welcome to the world!");
        Ok(())
    }

    async fn handle_search_input(&mut self, key_code: KeyCode) -> Result<DashboardAction> {
        match key_code {
            KeyCode::Esc => {
                self.search_mode = false;
                self.search_query.clear();
                self.load_recordings()?;
            }
            KeyCode::Enter => {
                self.search_mode = false;
                self.load_recordings()?;
            }
            KeyCode::Backspace => {
                self.search_query.pop();
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
            }
            _ => {}
        }
        Ok(DashboardAction::Continue)
    }

    fn next_recording(&mut self) {
        let i = match self.table_state.selected() {
            Some(i) => {
                if i >= self.recordings.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    fn previous_recording(&mut self) {
        let i = match self.table_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.recordings.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    async fn next_page(&mut self) -> Result<()> {
        // Try to load next page - if it has recordings, advance
        let old_page = self.current_page;
        self.current_page += 1;
        self.load_recordings()?;

        // If no recordings found on next page, go back to previous page
        if self.recordings.is_empty() {
            self.current_page = old_page;
            self.load_recordings()?;
        }

        Ok(())
    }

    async fn previous_page(&mut self) -> Result<()> {
        if self.current_page > 0 {
            self.current_page -= 1;
            self.load_recordings()?;
        }
        Ok(())
    }

    async fn show_selected_transcript(&mut self) -> Result<()> {
        if let Some(selected) = self.table_state.selected() {
            if let Some(recording) = self.recordings.get(selected).cloned() {
                if recording.has_transcript {
                    match self.load_transcript_content(&recording) {
                        Ok(content) => {
                            self.transcript_content = content;
                            self.show_transcript = true;
                            // Load enrichment data
                            self.load_enrichment_data(&recording);
                            // Initialize recording chat context
                            self.init_recording_chat(&recording);
                        }
                        Err(e) => {
                            self.message = format!("Failed to load transcript: {}", e);
                            self.show_message = true;
                        }
                    }
                } else {
                    self.message =
                        "No transcript available for this recording. Use P to play instead."
                            .to_string();
                    self.show_message = true;
                }
            }
        }
        Ok(())
    }

    fn load_enrichment_data(&mut self, recording: &Recording) {
        // Load summary and key points from recording
        self.transcript_summary = recording.summary.clone();
        self.transcript_key_points = recording
            .key_points
            .as_ref()
            .and_then(|s| serde_json::from_str(s).ok());

        // Load topics and entities from transcript
        if let Some(id) = recording.id {
            if let Ok(Some(transcript)) = self.db.get_transcript_by_recording_id(id) {
                self.transcript_topics = transcript
                    .topics
                    .as_ref()
                    .and_then(|s| serde_json::from_str(s).ok());

                // Parse entities JSON: [{"name": "...", "type": "..."}]
                self.transcript_entities = transcript.entities.as_ref().and_then(|s| {
                    let parsed: Result<Vec<serde_json::Value>, _> = serde_json::from_str(s);
                    parsed.ok().map(|entities| {
                        entities
                            .iter()
                            .filter_map(|e| {
                                let name = e.get("name")?.as_str()?.to_string();
                                let entity_type = e.get("type")?.as_str()?.to_string();
                                Some((name, entity_type))
                            })
                            .collect()
                    })
                });
            }
        }
    }

    fn clear_enrichment_data(&mut self) {
        self.transcript_summary = None;
        self.transcript_key_points = None;
        self.transcript_topics = None;
        self.transcript_entities = None;
    }

    async fn play_selected_recording(&mut self) -> Result<()> {
        use anyhow::anyhow;
        if let Some(selected) = self.table_state.selected() {
            if let Some(recording) = self.recordings.get(selected) {
                // Locate the audio file in ~/scriba_recordings/<directory_name>/
                let audio_path = self
                    .find_audio_file(recording)
                    .ok_or_else(|| anyhow!("Could not find an audio file for this recording"))?;

                // Determine file extension to choose optimal players
                let is_mp3 = audio_path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.to_lowercase() == "mp3")
                    .unwrap_or(false);

                // Candidate players differ by platform. For MP3 files, prioritize mpv/ffplay over afplay
                #[cfg(target_os = "macos")]
                let candidates: Vec<(&str, &[&str])> = if is_mp3 {
                    vec![
                        ("mpv", &["--really-quiet", "--audio-channels=stereo"]),
                        (
                            "ffplay",
                            &["-nodisp", "-autoexit", "-loglevel", "quiet", "-ac", "2"],
                        ),
                        ("afplay", &[]), // Last resort for MP3
                    ]
                } else {
                    vec![
                        ("mpv", &["--really-quiet", "--audio-channels=stereo"]),
                        (
                            "ffplay",
                            &["-nodisp", "-autoexit", "-loglevel", "quiet", "-ac", "2"],
                        ),
                        ("afplay", &[]), // Works well with WAV
                    ]
                };

                #[cfg(all(unix, not(target_os = "macos")))]
                let candidates: Vec<(&str, &[&str])> = vec![
                    ("mpv", &["--really-quiet", "--audio-channels=stereo"]),
                    (
                        "ffplay",
                        &["-nodisp", "-autoexit", "-loglevel", "quiet", "-ac", "2"],
                    ),
                    ("aplay", &["-c", "2"]), // Force stereo output
                ];

                #[cfg(target_os = "windows")]
                let candidates: Vec<(&str, &[&str])> = vec![
                    ("mpv", &["--really-quiet", "--audio-channels=stereo"]),
                    (
                        "ffplay",
                        &["-nodisp", "-autoexit", "-loglevel", "quiet", "-ac", "2"],
                    ),
                    (
                        "powershell",
                        &["-NoProfile", "-Command", "(New-Object Media.SoundPlayer '"],
                    ), // will be handled specially
                ];

                // Try each candidate until one spawns successfully
                let mut launched_with: Option<String> = None;

                #[cfg(not(target_os = "windows"))]
                for (prog, base_args) in candidates {
                    let mut cmd = TokioCommand::new(prog);
                    // Detach from TTY so player doesn't consume keyboard (Esc) input
                    cmd.stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null());

                    // For afplay on macOS, check if this is a mono WAV file and needs special handling
                    if prog == "afplay" && recording.channels == 1 && !is_mp3 {
                        // Create a temporary stereo version of the mono WAV file
                        if let Ok(stereo_path) = self.create_stereo_temp_file(&audio_path).await {
                            cmd.arg(stereo_path);
                        } else {
                            // Fallback to original mono file
                            cmd.arg(&audio_path);
                        }
                    } else {
                        for a in base_args {
                            cmd.arg(a);
                        }
                        cmd.arg(&audio_path);
                    }

                    match cmd.spawn() {
                        Ok(mut child) => {
                            launched_with = Some(prog.to_string());

                            // Store child process for potential termination - ensure we have a valid PID
                            if let Some(child_id) = child.id() {
                                // Store the process ID for killing on key press immediately
                                self.current_playback_pid = Some(child_id);

                                // Create channel for playback completion notification
                                let (finished_tx, finished_rx) = mpsc::channel(1);
                                self.playback_finished_rx = Some(finished_rx);

                                tokio::spawn(async move {
                                    let _ = child.wait().await;
                                    let _ = finished_tx.send(()).await;
                                });
                                break;
                            } else {
                                // If we can't get PID, we can't control the process
                                launched_with = None;
                                continue;
                            }
                        }
                        Err(e) => {
                            // Store error for debugging if no player works
                            if prog == "mpv" && is_mp3 {
                                self.message = format!("mpv failed to play MP3: {}", e);
                            }
                        }
                    }
                }

                #[cfg(target_os = "windows")]
                {
                    // Try standard players first (mpv, ffplay), then fallback to PowerShell
                    for (prog, base_args) in &candidates[..candidates.len() - 1] {
                        // All except powershell
                        let mut cmd = TokioCommand::new(prog);
                        // Detach from TTY so player doesn't consume keyboard (Esc) input
                        cmd.stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null());
                        for a in base_args {
                            cmd.arg(a);
                        }
                        cmd.arg(&audio_path);
                        match cmd.spawn() {
                            Ok(mut child) => {
                                if let Some(child_id) = child.id() {
                                    launched_with = Some(prog.to_string());
                                    self.current_playback_pid = Some(child_id);

                                    // Create channel for playback completion notification
                                    let (finished_tx, finished_rx) = mpsc::channel(1);
                                    self.playback_finished_rx = Some(finished_rx);

                                    tokio::spawn(async move {
                                        let _ = child.wait().await;
                                        let _ = finished_tx.send(()).await;
                                    });
                                    break;
                                }
                            }
                            Err(_e) => continue,
                        }
                    }

                    // PowerShell SoundPlayer fallback if no other player worked
                    if launched_with.is_none() {
                        let escaped = audio_path.to_string_lossy().replace("'", "''");
                        let ps =
                            format!("$p=New-Object Media.SoundPlayer '{}';$p.Play();", escaped);
                        let mut pscmd = TokioCommand::new("powershell");
                        // Detach from TTY so player doesn't consume keyboard (Esc) input
                        pscmd
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null());
                        match pscmd.arg("-NoProfile").arg("-Command").arg(ps).spawn() {
                            Ok(mut child) => {
                                if let Some(child_id) = child.id() {
                                    launched_with = Some("powershell".to_string());
                                    self.current_playback_pid = Some(child_id);

                                    // Create channel for playback completion notification
                                    let (finished_tx, finished_rx) = mpsc::channel(1);
                                    self.playback_finished_rx = Some(finished_rx);

                                    tokio::spawn(async move {
                                        let _ = child.wait().await;
                                        let _ = finished_tx.send(()).await;
                                    });
                                }
                            }
                            Err(_e) => {}
                        }
                    }
                }

                if let Some(player) = launched_with {
                    let name = recording
                        .display_name
                        .as_ref()
                        .unwrap_or(&recording.directory_name);
                    self.message = format!(
                        "▶ Playing: {}\nUsing player: {}\n\nPress ESC to stop playback",
                        name, player
                    );
                    self.show_message = true;
                    return Ok(());
                }

                // If we reach here, no player succeeded
                #[cfg(target_os = "macos")]
                let hint = "Install `mpv` (brew install mpv) or ensure `afplay` is available.";
                #[cfg(all(unix, not(target_os = "macos")))]
                let hint = "Install `mpv` or `ffmpeg` (ffplay).";
                #[cfg(target_os = "windows")]
                let hint = "Ensure PowerShell is available or install a player like mpv.";

                Err(anyhow!("No audio player found on PATH. {}", hint))
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    fn find_audio_file(&self, recording: &Recording) -> Option<PathBuf> {
        let base_path = home_dir()?
            .join("scriba_recordings")
            .join(&recording.directory_name);
        if !base_path.exists() {
            return None;
        }
        let exts = [
            "wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "aiff", "aif", "caf",
        ];
        if let Ok(read_dir) = std::fs::read_dir(base_path) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if exts.iter().any(|x| x.eq_ignore_ascii_case(ext)) {
                        return Some(path);
                    }
                }
            }
        }
        None
    }

    fn get_current_recording(&self) -> Option<Recording> {
        let selected_index = self.table_state.selected()?;
        self.recordings.get(selected_index).cloned()
    }

    fn show_delete_confirmation(&mut self) {
        if let Some(selected) = self.table_state.selected() {
            if let Some(recording) = self.recordings.get(selected).cloned() {
                self.delete_candidate = Some(recording);
                self.show_delete_confirm = true;
            }
        }
    }

    async fn handle_transcript_keys(&mut self, key_code: KeyCode) -> Result<DashboardAction> {
        match key_code {
            KeyCode::Up => {
                // Scroll up (decrease offset)
                if self.transcript_scroll_offset > 0 {
                    self.transcript_scroll_offset = self.transcript_scroll_offset.saturating_sub(1);
                }
                Ok(DashboardAction::Continue)
            }
            KeyCode::Down => {
                // Scroll down (increase offset)
                let content_width = 120; // Conservative estimate for terminal width
                let content_height = 25; // Conservative estimate for terminal height
                let wrapped_lines =
                    self.wrap_text_to_lines(&self.transcript_content, content_width);
                let max_scroll = wrapped_lines.len().saturating_sub(content_height);
                if self.transcript_scroll_offset < max_scroll {
                    self.transcript_scroll_offset += 1;
                }
                Ok(DashboardAction::Continue)
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                // Copy transcript to clipboard
                match self.copy_transcript_to_clipboard() {
                    Ok(()) => {
                        self.message = "Transcript copied to clipboard.".to_string();
                        self.show_message = true;
                    }
                    Err(e) => {
                        self.message = format!("Failed to copy to clipboard: {}", e);
                        self.show_message = true;
                    }
                }
                Ok(DashboardAction::Continue)
            }
            KeyCode::PageUp => {
                // Page up (scroll up by larger amount)
                self.transcript_scroll_offset = self.transcript_scroll_offset.saturating_sub(10);
                Ok(DashboardAction::Continue)
            }
            KeyCode::PageDown => {
                // Page down (scroll down by larger amount)
                let content_width = 120; // Conservative estimate for terminal width
                let content_height = 25; // Conservative estimate for terminal height
                let wrapped_lines =
                    self.wrap_text_to_lines(&self.transcript_content, content_width);
                let max_offset = wrapped_lines.len().saturating_sub(content_height);
                self.transcript_scroll_offset =
                    std::cmp::min(self.transcript_scroll_offset + 10, max_offset);
                Ok(DashboardAction::Continue)
            }
            KeyCode::Home => {
                // Jump to top of transcript
                self.transcript_scroll_offset = 0;
                Ok(DashboardAction::Continue)
            }
            KeyCode::End => {
                // Jump to bottom of transcript
                let content_width = 120; // Conservative estimate for terminal width
                let content_height = 25; // Conservative estimate for terminal height
                let wrapped_lines =
                    self.wrap_text_to_lines(&self.transcript_content, content_width);
                if wrapped_lines.len() > content_height {
                    self.transcript_scroll_offset =
                        wrapped_lines.len().saturating_sub(content_height);
                } else {
                    self.transcript_scroll_offset = 0;
                }
                Ok(DashboardAction::Continue)
            }
            KeyCode::Char('g') => {
                // Jump to top of transcript (vim-style, alternative to Home)
                self.transcript_scroll_offset = 0;
                Ok(DashboardAction::Continue)
            }
            KeyCode::Char('G') => {
                // Jump to bottom of transcript (vim-style, alternative to End)
                let content_width = 120; // Conservative estimate for terminal width
                let content_height = 25; // Conservative estimate for terminal height
                let wrapped_lines =
                    self.wrap_text_to_lines(&self.transcript_content, content_width);
                if wrapped_lines.len() > content_height {
                    self.transcript_scroll_offset =
                        wrapped_lines.len().saturating_sub(content_height);
                } else {
                    self.transcript_scroll_offset = 0;
                }
                Ok(DashboardAction::Continue)
            }
            KeyCode::Char('b') => {
                // Page up (vim-style, alternative to PageUp)
                self.transcript_scroll_offset = self.transcript_scroll_offset.saturating_sub(10);
                Ok(DashboardAction::Continue)
            }
            KeyCode::Char('f') => {
                // Page down (vim-style, alternative to PageDown)
                let content_width = 120; // Conservative estimate for terminal width
                let content_height = 25; // Conservative estimate for terminal height
                let wrapped_lines =
                    self.wrap_text_to_lines(&self.transcript_content, content_width);
                let max_offset = wrapped_lines.len().saturating_sub(content_height);
                self.transcript_scroll_offset =
                    std::cmp::min(self.transcript_scroll_offset + 10, max_offset);
                Ok(DashboardAction::Continue)
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                // Re-transcribe the current recording
                if let Some(recording) = self.get_current_recording() {
                    if self.transcription_task.is_some() {
                        self.message = "Transcription already in progress".to_string();
                        self.show_message = true;
                        return Ok(DashboardAction::Continue);
                    }

                    // Close transcript view
                    self.show_transcript = false;
                    self.transcript_content.clear();
                    self.transcript_scroll_offset = 0;

                    // Start re-transcription using unified workflow
                    let transcription_mode = self.config.transcription.clone();
                    let directory_name = recording.directory_name.clone();
                    // Track transcription inline — no blocking popup
                    self.transcribing_recording_name = Some(directory_name.clone());
                    self.progress_frame = 0;

                    // Start re-transcription in background
                    self.transcription_task = Some(tokio::spawn(async move {
                        let mut workflow = WorkflowManager::new().unwrap();
                        workflow
                            .retranscribe_recording_silent(&directory_name, transcription_mode)
                            .await
                    }));
                }
                Ok(DashboardAction::Continue)
            }
            KeyCode::Esc => {
                // Only Esc key closes the transcript (consistent behavior)
                self.show_transcript = false;
                self.transcript_content.clear();
                self.transcript_scroll_offset = 0;
                self.clear_enrichment_data();
                self.restore_global_chat();
                Ok(DashboardAction::Continue)
            }
            _ => {
                // Other keys are ignored (consistent behavior)
                Ok(DashboardAction::Continue)
            }
        }
    }

    fn is_editing_settings_field(&self) -> bool {
        self.editing_api_key || self.editing_enrichment_model || self.editing_enrichment_endpoint || self.editing_enrichment_api_key
    }

    fn save_enrichment_config(&mut self) -> Result<()> {
        self.config.save()?;
        self.config = ScribaConfig::load()?;
        Ok(())
    }

    async fn handle_settings_keys(&mut self, key_code: KeyCode) -> Result<DashboardAction> {
        // Max settings index: 0=Mode, 1=ModeSpecific,
        //                    2=EnrichProvider, 3=EnrichModel, 4=EnrichKeyOrEndpoint,
        //                    5=SilenceAutoStop, 6=SilenceTimeout,
        //                    7=Diarization, 8=MaxSpeakers,
        //                    9=VoiceMode, 10=VoiceSensitivity
        let max_index = 10;

        match key_code {
            KeyCode::Esc => {
                if self.is_editing_settings_field() {
                    self.editing_api_key = false;
                    self.editing_enrichment_model = false;
                    self.editing_enrichment_endpoint = false;
                    self.editing_enrichment_api_key = false;
                } else {
                    self.current_view = DashboardView::Main;
                }
                Ok(DashboardAction::Continue)
            }
            KeyCode::Up => {
                if !self.is_editing_settings_field() {
                    self.settings_selection = self.settings_selection.saturating_sub(1);
                }
                Ok(DashboardAction::Continue)
            }
            KeyCode::Down => {
                if !self.is_editing_settings_field() {
                    self.settings_selection = std::cmp::min(self.settings_selection + 1, max_index);
                }
                Ok(DashboardAction::Continue)
            }
            KeyCode::Enter => {
                if self.editing_api_key {
                    // Save API key
                    let new_mode = TranscriptionMode::Api {
                        api_key: self.api_key_input.clone(),
                    };
                    match self.config.set_transcription_mode(new_mode) {
                        Ok(()) => {
                            self.config = ScribaConfig::load()?;
                        }
                        Err(e) => {
                            self.message = format!("Failed to save API key: {}", e);
                            self.show_message = true;
                            self.return_to_view = Some(DashboardView::Settings);
                        }
                    }
                    self.editing_api_key = false;
                    self.api_key_input.clear();
                } else if self.editing_enrichment_model {
                    let new_model = self.enrichment_model_input.trim().to_string();
                    if !new_model.is_empty() {
                        match &mut self.config.enrichment.mode {
                            EnrichmentMode::Cloud { model, .. } => {
                                *model = Some(new_model);
                            }
                            EnrichmentMode::Local { ollama_model, .. } => {
                                *ollama_model = new_model;
                            }
                        }
                        if let Err(e) = self.save_enrichment_config() {
                            self.message = format!("Failed to save enrichment model: {}", e);
                            self.show_message = true;
                            self.return_to_view = Some(DashboardView::Settings);
                        }
                    }
                    self.editing_enrichment_model = false;
                    self.enrichment_model_input.clear();
                } else if self.editing_enrichment_endpoint {
                    let new_endpoint = self.enrichment_endpoint_input.trim().to_string();
                    if !new_endpoint.is_empty() {
                        self.config.enrichment.set_ollama_endpoint(new_endpoint);
                        if let Err(e) = self.save_enrichment_config() {
                            self.message = format!("Failed to save Ollama endpoint: {}", e);
                            self.show_message = true;
                            self.return_to_view = Some(DashboardView::Settings);
                        }
                    }
                    self.editing_enrichment_endpoint = false;
                    self.enrichment_endpoint_input.clear();
                } else if self.editing_enrichment_api_key {
                    let new_key = self.enrichment_api_key_input.trim().to_string();
                    if let EnrichmentMode::Cloud { api_key, .. } = &mut self.config.enrichment.mode {
                        *api_key = new_key;
                    }
                    if let Err(e) = self.save_enrichment_config() {
                        self.message = format!("Failed to save API key: {}", e);
                        self.show_message = true;
                        self.return_to_view = Some(DashboardView::Settings);
                    }
                    self.editing_enrichment_api_key = false;
                    self.enrichment_api_key_input.clear();
                } else {
                    match self.settings_selection {
                        0 => {
                            // Toggle transcription mode
                            let new_mode = match &self.config.transcription {
                                TranscriptionMode::Local { .. } => {
                                    let api_key = self
                                        .config
                                        .last_api_key
                                        .as_ref()
                                        .map(|key| key.clone())
                                        .unwrap_or_else(String::new);
                                    TranscriptionMode::Api { api_key }
                                }
                                TranscriptionMode::Api { .. } => TranscriptionMode::Local {
                                    model_size: LocalModelSize::Medium,
                                },
                            };
                            match self.config.set_transcription_mode(new_mode) {
                                Ok(()) => {
                                    self.config = ScribaConfig::load()?;
                                    self.settings_selection = 0;
                                }
                                Err(e) => {
                                    self.message = format!("Failed to change mode: {}", e);
                                    self.show_message = true;
                                    self.return_to_view = Some(DashboardView::Settings);
                                }
                            }
                        }
                        1 => {
                            match &self.config.transcription {
                                TranscriptionMode::Local { model_size } => {
                                    let new_model = match model_size {
                                        LocalModelSize::Tiny => LocalModelSize::Base,
                                        LocalModelSize::Base => LocalModelSize::Small,
                                        LocalModelSize::Small => LocalModelSize::Medium,
                                        LocalModelSize::Medium => LocalModelSize::Large,
                                        LocalModelSize::Large => LocalModelSize::Turbo,
                                        LocalModelSize::Turbo => LocalModelSize::Tiny,
                                    };
                                    let new_mode = TranscriptionMode::Local {
                                        model_size: new_model,
                                    };
                                    match self.config.set_transcription_mode(new_mode) {
                                        Ok(()) => {
                                            self.config = ScribaConfig::load()?;
                                        }
                                        Err(e) => {
                                            self.message =
                                                format!("Failed to change model: {}", e);
                                            self.show_message = true;
                                            self.return_to_view = Some(DashboardView::Settings);
                                        }
                                    }
                                }
                                TranscriptionMode::Api { .. } => {
                                    self.editing_api_key = true;
                                    self.api_key_input = match &self.config.transcription {
                                        TranscriptionMode::Api { api_key } => api_key.clone(),
                                        _ => String::new(),
                                    };
                                }
                            }
                        }
                        2 => {
                            // Enrichment Provider — cycle through providers
                            let new_mode = match &self.config.enrichment.mode {
                                EnrichmentMode::Cloud { provider, api_key, .. } => {
                                    let next = match provider {
                                        CloudProvider::Anthropic => CloudProvider::OpenAI,
                                        CloudProvider::OpenAI => CloudProvider::Google,
                                        CloudProvider::Google => CloudProvider::Anthropic,
                                    };
                                    EnrichmentMode::Cloud {
                                        provider: next,
                                        api_key: api_key.clone(),
                                        model: None,
                                    }
                                }
                                EnrichmentMode::Local { .. } => {
                                    EnrichmentMode::Cloud {
                                        provider: CloudProvider::Anthropic,
                                        api_key: String::new(),
                                        model: None,
                                    }
                                }
                            };
                            self.config.enrichment.mode = new_mode;
                            if let Err(e) = self.save_enrichment_config() {
                                self.message = format!("Failed to save provider: {}", e);
                                self.show_message = true;
                                self.return_to_view = Some(DashboardView::Settings);
                            }
                        }
                        3 => {
                            // Enrichment Model
                            self.editing_enrichment_model = true;
                            self.enrichment_model_input = self.config.enrichment.model_name().to_string();
                        }
                        4 => {
                            // Enrichment API Key (cloud) or Ollama Endpoint (local)
                            if self.config.enrichment.is_local() {
                                self.editing_enrichment_endpoint = true;
                                self.enrichment_endpoint_input = self.config.enrichment.ollama_endpoint();
                            } else {
                                self.editing_enrichment_api_key = true;
                                self.enrichment_api_key_input = self.config.enrichment.api_key().unwrap_or("").to_string();
                            }
                        }
                        5 => {
                            // Toggle silence auto-stop enabled/disabled
                            self.config.silence_auto_stop.enabled = !self.config.silence_auto_stop.enabled;
                            if let Err(e) = self.config.save() {
                                self.message = format!("Failed to save setting: {}", e);
                                self.show_message = true;
                                self.return_to_view = Some(DashboardView::Settings);
                            } else {
                                self.config = ScribaConfig::load()?;
                            }
                        }
                        6 => {
                            // Cycle silence timeout: 30s → 60s → 120s → 300s
                            if self.config.silence_auto_stop.enabled {
                                self.config.silence_auto_stop.timeout_seconds = match self.config.silence_auto_stop.timeout_seconds {
                                    30 => 60,
                                    60 => 120,
                                    120 => 300,
                                    _ => 30,
                                };
                                if let Err(e) = self.config.save() {
                                    self.message = format!("Failed to save setting: {}", e);
                                    self.show_message = true;
                                    self.return_to_view = Some(DashboardView::Settings);
                                } else {
                                    self.config = ScribaConfig::load()?;
                                }
                            }
                        }
                        7 => {
                            // Toggle speaker diarization enabled/disabled
                            self.config.diarization.enabled = !self.config.diarization.enabled;
                            if let Err(e) = self.config.save() {
                                self.message = format!("Failed to save setting: {}", e);
                                self.show_message = true;
                                self.return_to_view = Some(DashboardView::Settings);
                            } else {
                                self.config = ScribaConfig::load()?;
                            }
                        }
                        8 => {
                            // Cycle max speakers: 2 → 4 → 6 → 8
                            if self.config.diarization.enabled {
                                self.config.diarization.max_speakers = match self.config.diarization.max_speakers {
                                    2 => 4,
                                    4 => 6,
                                    6 => 8,
                                    _ => 2,
                                };
                                if let Err(e) = self.config.save() {
                                    self.message = format!("Failed to save setting: {}", e);
                                    self.show_message = true;
                                    self.return_to_view = Some(DashboardView::Settings);
                                } else {
                                    self.config = ScribaConfig::load()?;
                                }
                            }
                        }
                        9 => {
                            // Toggle voice mode on/off
                            self.toggle_voice_mode().await;
                            self.config.voice.enabled = self.voice_mode_active;
                            if let Err(e) = self.config.save() {
                                self.message = format!("Failed to save setting: {}", e);
                                self.show_message = true;
                                self.return_to_view = Some(DashboardView::Settings);
                            }
                        }
                        10 => {
                            // Cycle voice sensitivity: 0.005 → 0.01 → 0.02 → 0.05
                            if self.voice_mode_active {
                                self.config.voice.vad_threshold = match self.config.voice.vad_threshold {
                                    t if t <= 0.005 => 0.01,
                                    t if t <= 0.01 => 0.02,
                                    t if t <= 0.02 => 0.05,
                                    _ => 0.005,
                                };
                                if let Err(e) = self.config.save() {
                                    self.message = format!("Failed to save setting: {}", e);
                                    self.show_message = true;
                                    self.return_to_view = Some(DashboardView::Settings);
                                } else {
                                    self.config = ScribaConfig::load()?;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ok(DashboardAction::Continue)
            }
            KeyCode::Char(c) => {
                if self.editing_api_key {
                    self.api_key_input.push(c);
                } else if self.editing_enrichment_model {
                    self.enrichment_model_input.push(c);
                } else if self.editing_enrichment_endpoint {
                    self.enrichment_endpoint_input.push(c);
                } else if self.editing_enrichment_api_key {
                    self.enrichment_api_key_input.push(c);
                }
                Ok(DashboardAction::Continue)
            }
            KeyCode::Backspace => {
                if self.editing_api_key {
                    self.api_key_input.pop();
                } else if self.editing_enrichment_model {
                    self.enrichment_model_input.pop();
                } else if self.editing_enrichment_endpoint {
                    self.enrichment_endpoint_input.pop();
                } else if self.editing_enrichment_api_key {
                    self.enrichment_api_key_input.pop();
                }
                Ok(DashboardAction::Continue)
            }
            _ => Ok(DashboardAction::Continue),
        }
    }

    async fn handle_delete_confirmation(&mut self, key_code: KeyCode) -> Result<DashboardAction> {
        match key_code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // Confirm deletion
                if let Some(recording) = self.delete_candidate.take() {
                    match self.perform_delete_recording(recording).await {
                        Ok(()) => {}
                        Err(e) => {
                            self.message = format!("Failed to delete recording: {}", e);
                            self.show_message = true;
                        }
                    }
                }
                self.show_delete_confirm = false;
                self.delete_candidate = None;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                // Cancel deletion
                self.show_delete_confirm = false;
                self.delete_candidate = None;
            }
            _ => {
                // Any other key, just ignore
            }
        }
        Ok(DashboardAction::Continue)
    }

    async fn handle_file_dialog_keys(&mut self, key_code: KeyCode) -> Result<DashboardAction> {
        match key_code {
            KeyCode::Esc => {
                // Cancel file import
                self.show_file_dialog = false;
                self.file_path_input.clear();
                self.file_name_input.clear();
                self.file_dialog_stage = FileDialogStage::FilePath;
                Ok(DashboardAction::Continue)
            }
            KeyCode::Enter => {
                match self.file_dialog_stage {
                    FileDialogStage::FilePath => {
                        // Validate file path
                        if self.file_path_input.trim().is_empty() {
                            self.message = "Please enter a file path".to_string();
                            self.show_message = true;
                            self.return_to_view = Some(DashboardView::Main);
                            self.show_file_dialog = false;
                            return Ok(DashboardAction::Continue);
                        }

                        // Check if file exists
                        let file_path = PathBuf::from(self.file_path_input.trim());
                        if !file_path.exists() {
                            self.message = "File not found. Please check the path.".to_string();
                            self.show_message = true;
                            self.return_to_view = Some(DashboardView::Main);
                            self.show_file_dialog = false;
                            return Ok(DashboardAction::Continue);
                        }

                        // Move to name input stage
                        self.file_dialog_stage = FileDialogStage::FileName;
                        // Pre-fill with file stem as default name
                        if let Some(stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                            self.file_name_input = stem.to_string();
                        }
                    }
                    FileDialogStage::FileName => {
                        // Use file name or default to file stem
                        let display_name = if self.file_name_input.trim().is_empty() {
                            let file_path = PathBuf::from(self.file_path_input.trim());
                            file_path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("imported_audio")
                                .to_string()
                        } else {
                            self.file_name_input.trim().to_string()
                        };

                        // Start import process
                        self.show_file_dialog = false;
                        self.start_file_import(self.file_path_input.clone(), display_name)
                            .await?;
                    }
                }
                Ok(DashboardAction::Continue)
            }
            KeyCode::Char(c) => {
                // Add character to current input
                match self.file_dialog_stage {
                    FileDialogStage::FilePath => {
                        self.file_path_input.push(c);
                    }
                    FileDialogStage::FileName => {
                        self.file_name_input.push(c);
                    }
                }
                Ok(DashboardAction::Continue)
            }
            KeyCode::Backspace => {
                // Remove character from current input
                match self.file_dialog_stage {
                    FileDialogStage::FilePath => {
                        self.file_path_input.pop();
                    }
                    FileDialogStage::FileName => {
                        self.file_name_input.pop();
                    }
                }
                Ok(DashboardAction::Continue)
            }
            _ => Ok(DashboardAction::Continue),
        }
    }

    async fn perform_delete_recording(&mut self, recording: Recording) -> Result<()> {
        if let Some(id) = recording.id {
            match self.db.delete_recording(id) {
                Ok(()) => {
                    let base_path = home_dir()
                        .context("Could not find home directory")?
                        .join("scriba_recordings");
                    let recording_dir = base_path.join(&recording.directory_name);

                    if recording_dir.exists() {
                        std::fs::remove_dir_all(&recording_dir).ok();
                    }

                    self.load_recordings()?;
                    self.load_stats()?;
                    self.message = "Recording deleted.".to_string();
                    self.show_message = true;
                }
                Err(_e) => {
                    return Err(anyhow::anyhow!(
                        "Could not delete recording (ID: {}).\nHint: This often happens when there are related rows (e.g., transcripts) without ON DELETE CASCADE. Delete dependents first or enable cascading, then retry.",
                        id
                    ));
                }
            }
        } else {
            return Err(anyhow::anyhow!(
                "Selected recording has no database ID; cannot delete."
            ));
        }
        Ok(())
    }

    fn load_transcript_content(&self, recording: &Recording) -> Result<String> {
        // First try to load from database
        if let Some(id) = recording.id {
            if let Ok(Some(transcript)) = self.db.get_transcript_by_recording_id(id) {
                return Ok(transcript.content);
            }
        }

        // Fallback: try to load from file (standard transcript.txt)
        let base_path = home_dir()
            .context("Could not find home directory")?
            .join("scriba_recordings");
        let recording_dir = base_path.join(&recording.directory_name);

        // Try transcript.txt
        let transcript_path = recording_dir.join("transcript.txt");
        if transcript_path.exists() {
            return std::fs::read_to_string(&transcript_path)
                .context("Failed to read transcript.txt file");
        }

        Err(anyhow::anyhow!("No transcript file found (expected transcript.txt)"))
    }

    fn wrap_text_to_lines(&self, text: &str, max_width: usize) -> Vec<String> {
        let mut result = Vec::new();

        for line in text.lines() {
            if line.len() <= max_width {
                result.push(line.to_string());
            } else {
                // Split long lines into multiple wrapped lines
                let words: Vec<&str> = line.split_whitespace().collect();
                let mut current_line = String::new();

                for word in words {
                    if word.len() > max_width {
                        // Handle extremely long words by character breaking
                        if !current_line.is_empty() {
                            result.push(current_line);
                            current_line = String::new();
                        }

                        let chars: Vec<char> = word.chars().collect();
                        for chunk in chars.chunks(max_width) {
                            result.push(chunk.iter().collect());
                        }
                    } else {
                        let test_line = if current_line.is_empty() {
                            word.to_string()
                        } else {
                            format!("{} {}", current_line, word)
                        };

                        if test_line.len() <= max_width {
                            current_line = test_line;
                        } else {
                            result.push(current_line);
                            current_line = word.to_string();
                        }
                    }
                }

                if !current_line.is_empty() {
                    result.push(current_line);
                }
            }
        }

        // Handle edge case where text has no newlines at all
        if result.is_empty() && !text.is_empty() {
            let words: Vec<&str> = text.split_whitespace().collect();
            let mut current_line = String::new();

            for word in words {
                if word.len() > max_width {
                    if !current_line.is_empty() {
                        result.push(current_line);
                        current_line = String::new();
                    }

                    let chars: Vec<char> = word.chars().collect();
                    for chunk in chars.chunks(max_width) {
                        result.push(chunk.iter().collect());
                    }
                } else {
                    let test_line = if current_line.is_empty() {
                        word.to_string()
                    } else {
                        format!("{} {}", current_line, word)
                    };

                    if test_line.len() <= max_width {
                        current_line = test_line;
                    } else {
                        result.push(current_line);
                        current_line = word.to_string();
                    }
                }
            }

            if !current_line.is_empty() {
                result.push(current_line);
            }
        }

        result
    }

    fn copy_transcript_to_clipboard(&self) -> Result<()> {
        use arboard::Clipboard;

        let mut clipboard = Clipboard::new().context("Failed to access clipboard")?;

        clipboard
            .set_text(&self.transcript_content)
            .context("Failed to copy text to clipboard")?;

        Ok(())
    }

    fn ui(&mut self, f: &mut Frame) {
        match self.current_view {
            DashboardView::Main => self.render_main_dashboard(f),
            DashboardView::Help => self.render_help(f, f.size()),
            DashboardView::Settings => self.render_settings(f, f.size()),
            DashboardView::Entities => self.render_entities_view(f, f.size()),
            DashboardView::Onboarding => self.render_onboarding(f, f.size()),
        }
    }

    fn render_main_dashboard(&mut self, f: &mut Frame) {
        let size = f.size();

        if self.show_file_dialog {
            self.render_file_dialog_popup(f, size);
            return;
        }

        if self.show_message {
            self.render_message_popup(f, size);
            return;
        }

        if self.show_transcript {
            self.render_transcript_popup(f, size);
            return;
        }

        if self.show_delete_confirm {
            self.render_delete_confirmation_popup(f, size);
            return;
        }

        // Dynamic chat panel height (unified: suggestions/messages + input in one block)
        let has_chat_content = !self.chat.messages.is_empty()
            || !self.chat.streaming_content.is_empty()
            || self.chat.is_generating;

        let chat_panel_height: u16 = if has_chat_content {
            let available = size.height.saturating_sub(5 + 1); // header + footer
            (available * 65 / 100).max(10)
        } else {
            // Suggestions + input + borders: enough for suggestions + input line
            (self.chat.suggestions.len() as u16 + 4).max(5).min(8)
        };

        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),                // Header (owl + text)
                Constraint::Min(6),                   // Recordings Table (shrinks when chat grows)
                Constraint::Length(chat_panel_height), // Unified chat panel
                Constraint::Length(1),                // Footer (compressed)
            ])
            .split(size);

        // Header
        self.render_header(f, main_chunks[0]);

        // Table (with stats in title)
        self.render_recordings_table(f, main_chunks[1]);

        // Unified chat panel
        self.render_chat_panel(f, main_chunks[2]);

        // Compressed footer
        self.render_compressed_footer(f, main_chunks[3]);

        // Search input overlay
        if self.search_mode {
            self.render_search_input(f, size);
        }
    }

    fn render_header(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        use ratatui::text::{Line, Span};

        let cycle = self.header_anim_frame % 250;
        let is_flying = cycle >= 150 && cycle < 190;
        let is_blink = !is_flying && (cycle % 40) < 3;

        let yellow_bold = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        let gray = Style::default().fg(Color::DarkGray);

        let lines = if is_flying {
            // Flight phase: owl hops right of logo, flies across, bobs up and down
            let fly_frame = cycle - 150; // 0..39
            let inner_width = (area.width as usize).saturating_sub(2);
            let logo_end = 28;
            let flyable = inner_width.saturating_sub(logo_end + 7);

            // Horizontal: out and back over 40 frames
            let progress = if fly_frame < 20 {
                fly_frame as f64 / 19.0
            } else {
                (40 - fly_frame) as f64 / 20.0
            };
            let eased = if progress < 0.5 {
                2.0 * progress * progress
            } else {
                1.0 - (-2.0 * progress + 2.0).powi(2) / 2.0
            };
            let offset = (eased * flyable as f64) as usize;

            // Vertical: bob between line 0 and 1 only
            let vert = ((fly_frame as f64 / 8.0) * std::f64::consts::PI).sin();
            let row = if vert > 0.0 { 0 } else { 1 };

            let face = if fly_frame < 3 || fly_frame > 37 { "(o,o)" } else { "(^,^)" };
            let owl_str = if fly_frame >= 3 && fly_frame <= 37 {
                format!("~{}~", face)
            } else {
                format!(" {} ", face)
            };
            let gap = " ".repeat(offset);
            let owl_span = Span::styled(format!("{}{}", gap, owl_str), yellow_bold);
            let empty_span = Span::raw("");

            // Build 3 lines: logo is always fixed, owl appears on the row it's bobbing to
            let logo_lines = [
                "         ╔═╗╔═╗╦═╗╦╔╗ ╔═╗",
                "         ╚═╗║  ╠╦╝║╠╩╗╠═╣",
                "         ╚═╝╚═╝╩╚═╩╚═╝╩ ╩",
            ];

            vec![
                Line::from(vec![
                    Span::styled(logo_lines[0], yellow_bold),
                    if row == 0 { owl_span.clone() } else { empty_span.clone() },
                ]),
                Line::from(vec![
                    Span::styled(logo_lines[1], yellow_bold),
                    if row == 1 { owl_span.clone() } else { empty_span.clone() },
                ]),
                Line::from(vec![
                    Span::styled(logo_lines[2], yellow_bold),
                    Span::styled(
                        " — hoo remembers everything",
                        gray,
                    ),
                ]),
            ]
        } else {
            // Idle: owl sits next to SCRIBA, blinks occasionally
            let face = if is_blink { "(-,-)" } else { "(o,o)" };
            vec![
                Line::from(Span::styled(
                    format!("  {}  ╔═╗╔═╗╦═╗╦╔╗ ╔═╗", face),
                    yellow_bold,
                )),
                Line::from(Span::styled(
                    "  {`\"'}  ╚═╗║  ╠╦╝║╠╩╗╠═╣",
                    yellow_bold,
                )),
                Line::from(vec![
                    Span::styled(
                        "  -\"-\"-  ╚═╝╚═╝╩╚═╩╚═╝╩ ╩",
                        yellow_bold,
                    ),
                    Span::styled(
                        " — hoo remembers everything",
                        gray,
                    ),
                ]),
            ]
        };

        let header = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Cyan))
                    .border_type(ratatui::widgets::BorderType::Double),
            );
        f.render_widget(header, area);
    }

    fn render_recordings_table(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        let header_cells = ["#", "Status", "Name", "Duration", "Model", "Created"]
            .iter()
            .map(|h| {
                Cell::from(*h).style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            });

        let header = Row::new(header_cells)
            .style(Style::default())
            .height(1)
            .bottom_margin(1);

        let rows: Vec<Row> = self
            .recordings
            .iter()
            .enumerate()
            .map(|(i, recording)| {
                let display_name = recording
                    .display_name
                    .as_ref()
                    .unwrap_or(&recording.directory_name);

                let duration = recording
                    .duration_seconds
                    .map(|d| self.format_duration(d))
                    .unwrap_or_else(|| "Unknown".to_string());

                let is_transcribing = self.transcribing_recording_name.as_deref()
                    == Some(&recording.directory_name);
                let status = if is_transcribing {
                    match self.progress_frame % 4 {
                        0 => "[|]",
                        1 => "[/]",
                        2 => "[-]",
                        _ => "[\\]",
                    }
                } else if recording.has_transcript {
                    "[T]"
                } else {
                    "[A]"
                };
                let created = recording.created_at.format("%m/%d %H:%M").to_string();

                // Calculate global index across all pages
                let global_index = (self.current_page * self.page_size) + i + 1;

                // Format model used for display
                let model_display = if recording.has_transcript {
                    // Parse the model_used field to show a user-friendly format
                    match recording.model_used.as_str() {
                        // API models
                        "whisper-1" => "API",
                        s if s.starts_with("gpt") || s.contains("openai") => "API",
                        // Local models - extract size from format like "whisper-tiny", "whisper-turbo"
                        s if s.starts_with("whisper-") => {
                            let size = s.strip_prefix("whisper-").unwrap_or("");
                            match size {
                                "tiny" => "Tiny",
                                "base" => "Base",
                                "small" => "Small",
                                "medium" => "Medium",
                                "large" => "Large",
                                "large-v2" => "Large-v2",
                                "large-v3" => "Large-v3",
                                "turbo" => "Turbo",
                                _ => "Local",
                            }
                        }
                        // Legacy formats
                        "whisper" | "whisper.cpp" | "whisper-rs" => "Local",
                        // Unknown/empty
                        s if s.is_empty() => "-",
                        _ => &recording.model_used,
                    }
                } else {
                    "-"
                };

                let cells = vec![
                    Cell::from(global_index.to_string()),
                    Cell::from(status).style(if is_transcribing {
                        Style::default().fg(Color::Yellow)
                    } else if recording.has_transcript {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::Blue)
                    }),
                    Cell::from(display_name.clone()),
                    Cell::from(duration),
                    Cell::from(model_display).style(if model_display == "API" {
                        Style::default().fg(Color::Cyan)
                    } else if model_display == "-" {
                        Style::default().fg(Color::Gray)
                    } else {
                        Style::default().fg(Color::Yellow)
                    }),
                    Cell::from(created),
                ];

                Row::new(cells).height(1).bottom_margin(0)
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(4),  // #
                Constraint::Length(8),  // Status
                Constraint::Min(15),    // Name (flexible)
                Constraint::Length(10), // Duration
                Constraint::Length(8),  // Model
                Constraint::Length(12), // Created
            ],
        )
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Cyan))
                .title({
                    let start_index = (self.current_page * self.page_size) + 1;
                    let end_index = start_index + self.recordings.len() - 1;
                    let stats_suffix = if let Some(stats) = &self.stats {
                        format!(" — {} total, {}", stats.total_recordings, stats.format_duration())
                    } else {
                        String::new()
                    };
                    format!(
                        "Recordings (Page {} - #{}-#{}){}",
                        self.current_page + 1,
                        start_index,
                        end_index,
                        stats_suffix
                    )
                }),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

        f.render_stateful_widget(table, area, &mut self.table_state);
    }

    fn render_help(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(15),
                Constraint::Percentage(70),
                Constraint::Percentage(15),
            ])
            .split(area)[1];

        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(15),
                Constraint::Percentage(70),
                Constraint::Percentage(15),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        let help_text = vec![
            Line::from("SCRIBA HELP"),
            Line::from(""),
            Line::from("Quick Actions:"),
            Line::from("  R          - Record Audio + Auto-Transcribe (Esc to stop)"),
            Line::from("  A          - Add External Audio File & Transcribe"),
            Line::from("  T          - Transcribe Selected Recording (also in transcript view)"),
            Line::from(""),
            Line::from("Navigation:"),
            Line::from("  ↑/↓        - Navigate recordings"),
            Line::from("  PgUp/PgDn  - Change pages (or '['/']')"),
            Line::from("  Enter      - View transcript"),
            Line::from("  P          - Play recording"),
            Line::from(""),
            Line::from("Actions:"),
            Line::from("  D          - Delete recording (with confirmation)"),
            Line::from("  /          - Search recordings"),
            Line::from("  S          - Settings (transcription mode, models)"),
            Line::from("  H/F1       - Show this help"),
            Line::from("  Q/Esc      - Quit"),
            Line::from(""),
            Line::from("Transcript Viewer:"),
            Line::from("  ↑/↓        - Scroll up/down"),
            Line::from("  PgUp/PgDn  - Page up/down (or 'b'/'f')"),
            Line::from("  Home/End   - Jump to top/bottom (or 'g'/'G')"),
            Line::from("  C          - Copy transcript to clipboard"),
            Line::from("  T          - Re-transcribe recording"),
            Line::from("  ESC        - Close transcript"),
            Line::from(""),
            Line::from("Recording Control:"),
            Line::from("  • Press R/A to start recording immediately"),
            Line::from("  • Press Esc during recording to stop and save"),
            Line::from("  • Real-time progress indicators for all operations"),
            Line::from(""),
            Line::from("Features:"),
            Line::from("  • Statistics always visible at bottom"),
            Line::from("  • Full-text search through transcripts"),
            Line::from("  • Integrated playback support"),
            Line::from("  • Direct recording from dashboard"),
            Line::from(""),
            Line::from("Press Esc to continue..."),
        ];

        let help_paragraph = Paragraph::new(help_text)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Yellow))
                    .title("Help"),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(help_paragraph, popup_area);
    }

    fn render_settings(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(15),
                Constraint::Percentage(70),
                Constraint::Percentage(15),
            ])
            .split(area)[1];

        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(15),
                Constraint::Percentage(70),
                Constraint::Percentage(15),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        let mut settings_text = vec![
            Line::from(vec![Span::styled(
                "SETTINGS",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
        ];

        // Current transcription mode
        let mode_text = match &self.config.transcription {
            TranscriptionMode::Local { model_size } => {
                format!("Local (Whisper {})  <- Press Enter to change", model_size)
            }
            TranscriptionMode::Api { .. } => "OpenAI API  <- Press Enter to change".to_string(),
        };

        let mode_style = if self.settings_selection == 0 {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        settings_text.push(Line::from(vec![
            Span::styled("Transcription Mode: ", Style::default().fg(Color::Green)),
            Span::styled(mode_text, mode_style),
        ]));

        // Model size (only for local mode)
        if let TranscriptionMode::Local { model_size } = &self.config.transcription {
            let model_style = if self.settings_selection == 1 {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            settings_text.push(Line::from(vec![
                Span::styled("Model Size: ", Style::default().fg(Color::Green)),
                Span::styled(
                    format!("{} <- Press Enter to cycle", model_size),
                    model_style,
                ),
            ]));
        }

        // Add mode-specific settings
        match &self.config.transcription {
            TranscriptionMode::Api { api_key } => {
                // API Mode: Show API Key configuration at index 1
                let api_key_display = if self.editing_api_key {
                    format!("{}_", self.api_key_input) // Show cursor
                } else {
                    if api_key.is_empty() {
                        "[Not Set] <- Press Enter to edit".to_string()
                    } else {
                        format!(
                            "{}****** <- Press Enter to edit",
                            &api_key[..api_key.len().min(4)]
                        )
                    }
                };

                let api_key_style = if self.settings_selection == 1 {
                    if self.editing_api_key {
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    }
                } else {
                    Style::default().fg(Color::White)
                };

                settings_text.push(Line::from(vec![
                    Span::styled("OpenAI API Key: ", Style::default().fg(Color::Green)),
                    Span::styled(api_key_display, api_key_style),
                ]));
            }
            TranscriptionMode::Local { .. } => {
                // Local Mode: No API Key shown, Model Size is already shown above
            }
        }

        // Enrichment settings section
        settings_text.push(Line::from(""));
        let enrichment_header = if self.config.enrichment.is_local() {
            "ENRICHMENT (Privacy Mode)"
        } else {
            "ENRICHMENT"
        };
        settings_text.push(Line::from(vec![Span::styled(
            enrichment_header,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]));

        // Provider (index 2) — cycle through providers
        let provider_display = format!(
            "{} <- Press Enter to cycle",
            self.config.enrichment.provider_display_name()
        );
        let provider_style = if self.settings_selection == 2 {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        settings_text.push(Line::from(vec![
            Span::styled("Provider: ", Style::default().fg(Color::Green)),
            Span::styled(provider_display, provider_style),
        ]));

        // Model (index 3)
        let model_display = if self.editing_enrichment_model {
            format!("{}_", self.enrichment_model_input)
        } else {
            format!(
                "{} <- Press Enter to edit",
                self.config.enrichment.model_name()
            )
        };
        let model_style = if self.settings_selection == 3 {
            if self.editing_enrichment_model {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            }
        } else {
            Style::default().fg(Color::White)
        };
        settings_text.push(Line::from(vec![
            Span::styled("Model: ", Style::default().fg(Color::Green)),
            Span::styled(model_display, model_style),
        ]));

        // API Key or Ollama Endpoint (index 4)
        if self.config.enrichment.is_local() {
            let endpoint_display = if self.editing_enrichment_endpoint {
                format!("{}_", self.enrichment_endpoint_input)
            } else {
                format!(
                    "{} <- Press Enter to edit",
                    self.config.enrichment.ollama_endpoint()
                )
            };
            let endpoint_style = if self.settings_selection == 4 {
                if self.editing_enrichment_endpoint {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                }
            } else {
                Style::default().fg(Color::White)
            };
            settings_text.push(Line::from(vec![
                Span::styled("Ollama Server: ", Style::default().fg(Color::Green)),
                Span::styled(endpoint_display, endpoint_style),
            ]));
        } else {
            let key_display = if self.editing_enrichment_api_key {
                format!("{}_", self.enrichment_api_key_input)
            } else {
                match self.config.enrichment.api_key() {
                    Some(key) if key.len() >= 4 => {
                        format!("{}****** <- Press Enter to edit", &key[..4])
                    }
                    Some(_) => "****** <- Press Enter to edit".to_string(),
                    None => "[Not Set] <- Press Enter to edit".to_string(),
                }
            };
            let key_style = if self.settings_selection == 4 {
                if self.editing_enrichment_api_key {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                }
            } else {
                Style::default().fg(Color::White)
            };
            settings_text.push(Line::from(vec![
                Span::styled("API Key: ", Style::default().fg(Color::Green)),
                Span::styled(key_display, key_style),
            ]));
        }

        // Recording section — silence auto-stop
        settings_text.push(Line::from(""));
        settings_text.push(Line::from(vec![Span::styled(
            "RECORDING",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]));

        // Silence Auto-Stop toggle (index 4)
        let silence_enabled = self.config.silence_auto_stop.enabled;
        let silence_toggle_text = if silence_enabled {
            "Enabled <- Press Enter to toggle"
        } else {
            "Disabled <- Press Enter to toggle"
        };
        let silence_toggle_style = if self.settings_selection == 5 {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        settings_text.push(Line::from(vec![
            Span::styled("Silence Auto-Stop: ", Style::default().fg(Color::Green)),
            Span::styled(silence_toggle_text, silence_toggle_style),
        ]));

        // Silence Timeout (index 5) — only interactive when enabled
        let timeout_secs = self.config.silence_auto_stop.timeout_seconds;
        let timeout_display = match timeout_secs {
            s if s < 60 => format!("{}s", s),
            s if s % 60 == 0 => format!("{}m", s / 60),
            s => format!("{}m {}s", s / 60, s % 60),
        };
        let timeout_text = if silence_enabled {
            format!("{} <- Press Enter to cycle", timeout_display)
        } else {
            format!("{} (enable auto-stop first)", timeout_display)
        };
        let timeout_style = if self.settings_selection == 6 {
            if silence_enabled {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            }
        } else {
            if silence_enabled {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            }
        };
        settings_text.push(Line::from(vec![
            Span::styled("Silence Timeout: ", Style::default().fg(Color::Green)),
            Span::styled(timeout_text, timeout_style),
        ]));

        // Diarization section
        settings_text.push(Line::from(""));
        settings_text.push(Line::from(vec![Span::styled(
            "DIARIZATION",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]));

        // Speaker Diarization toggle (index 6)
        let diarization_enabled = self.config.diarization.enabled;
        let diarization_toggle_text = if diarization_enabled {
            "Enabled <- Press Enter to toggle"
        } else {
            "Disabled <- Press Enter to toggle"
        };
        let diarization_toggle_style = if self.settings_selection == 7 {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        settings_text.push(Line::from(vec![
            Span::styled("Speaker Diarization: ", Style::default().fg(Color::Green)),
            Span::styled(diarization_toggle_text, diarization_toggle_style),
        ]));

        // Max Speakers (index 7) — only interactive when enabled
        let max_speakers = self.config.diarization.max_speakers;
        let max_speakers_text = if diarization_enabled {
            format!("{} <- Press Enter to cycle", max_speakers)
        } else {
            format!("{} (enable diarization first)", max_speakers)
        };
        let max_speakers_style = if self.settings_selection == 8 {
            if diarization_enabled {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            }
        } else if diarization_enabled {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        settings_text.push(Line::from(vec![
            Span::styled("Max Speakers: ", Style::default().fg(Color::Green)),
            Span::styled(max_speakers_text, max_speakers_style),
        ]));

        // Voice mode section
        settings_text.push(Line::from(""));
        settings_text.push(Line::from(vec![Span::styled(
            "VOICE MODE (Scriba Forever)",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]));

        // Voice Mode toggle (index 9)
        let voice_enabled = self.voice_mode_active;
        let voice_toggle_text = if voice_enabled {
            "Active <- Press Enter to toggle"
        } else {
            "Off <- Press Enter to toggle"
        };
        let voice_toggle_style = if self.settings_selection == 9 {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        settings_text.push(Line::from(vec![
            Span::styled("Voice Activation: ", Style::default().fg(Color::Green)),
            Span::styled(voice_toggle_text, voice_toggle_style),
        ]));

        // Voice Sensitivity (index 10)
        let sensitivity_label = match self.config.voice.vad_threshold {
            t if t <= 0.005 => "Very High (0.005)",
            t if t <= 0.01 => "High (0.01)",
            t if t <= 0.02 => "Medium (0.02)",
            _ => "Low (0.05)",
        };
        let sensitivity_text = if voice_enabled {
            format!("{} <- Press Enter to cycle", sensitivity_label)
        } else {
            format!("{} (enable voice mode first)", sensitivity_label)
        };
        let sensitivity_style = if self.settings_selection == 10 {
            if voice_enabled {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            }
        } else if voice_enabled {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        settings_text.push(Line::from(vec![
            Span::styled("Voice Sensitivity: ", Style::default().fg(Color::Green)),
            Span::styled(sensitivity_text, sensitivity_style),
        ]));

        settings_text.push(Line::from(""));
        settings_text.push(Line::from(vec![Span::styled(
            "↑↓ Navigate  ⏎ Enter  ⎋ Esc",
            Style::default().fg(Color::Gray),
        )]));

        let settings_paragraph = Paragraph::new(settings_text)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Settings")
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(settings_paragraph, popup_area);
    }

    fn render_entities_view(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        use ratatui::text::{Line, Span};

        // Main layout: Header + Content + Footer
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),  // Header (owl + summary + quip)
                Constraint::Min(8),     // Entity table
                Constraint::Length(3),  // Footer
            ])
            .split(area);

        // Animated owl header — fixed-width face + separate thought bubble
        let (owl_face, owl_body, owl_feet, thought) = match self.owl_world_mood {
            OwlWorldMood::Idle => {
                let face = if self.owl_world_anim_frame % 30 < 3 { "(-,-)" } else { "(o,o)" };
                (face, "{`\"'}", "-\"-\"-", "")
            }
            OwlWorldMood::Thinking => {
                let frame = (self.owl_world_anim_frame / 3) % 8;
                let face = if frame == 4 { "(-,-)" } else { "(o,o)" };
                let bubble = match frame {
                    0 => "  ?",
                    1 => "  ~",
                    2 => "  !",
                    3 => " ~~",
                    4 => "  ?",
                    5 => "~~~",
                    6 => "  !",
                    _ => "...",
                };
                let beak = if frame % 2 == 1 { "{`~'}" } else { "{`\"'}" };
                (face, beak, "-\"-\"-", bubble)
            }
            OwlWorldMood::Celebrating => {
                let frame = (self.owl_world_anim_frame / 2) % 4;
                let face = match frame {
                    0 | 2 => "(^,^)",
                    _ => "(^,^)",
                };
                (face, "{`\"'}", "-\"-\"-", "")
            }
        };

        // Build owner summary from entities
        let owner_entity = self.entities.iter().find(|e| {
            e.context.as_deref().unwrap_or("").contains("Owner of this Scriba instance")
        });
        let owner_summary = if let Some(owner) = owner_entity {
            let ctx = owner.context.as_deref().unwrap_or("");
            let role_part = ctx.splitn(2, '.').nth(1).unwrap_or("").trim();
            if role_part.is_empty() {
                owner.canonical_name.clone()
            } else {
                format!("{} | {}", owner.canonical_name, role_part)
            }
        } else {
            "No owner set".to_string()
        };

        let people_count = self.entities.iter().filter(|e| {
            e.entity_type == "person" && !e.context.as_deref().unwrap_or("").contains("Owner of this Scriba instance")
        }).count();
        let org_count = self.entities.iter().filter(|e| e.entity_type == "organization").count();
        let project_count = self.entities.iter().filter(|e| e.entity_type == "project").count();

        let mut counts = Vec::new();
        if people_count > 0 { counts.push(format!("{} people", people_count)); }
        if org_count > 0 { counts.push(format!("{} orgs", org_count)); }
        if project_count > 0 { counts.push(format!("{} projects", project_count)); }
        let counts_str = if counts.is_empty() { String::new() } else { format!(" | {}", counts.join(", ")) };

        let header_lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("  {}  ", owl_face),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "SCRIBA'S WORLD",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", thought),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("  {}  ", owl_body),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    format!("{}{}", owner_summary, counts_str),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("  {}  ", owl_feet),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    format!("\"{}\"", self.owl_quip),
                    Style::default().fg(Color::Green),
                ),
            ]),
        ];

        let header = Paragraph::new(header_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Cyan))
                    .border_type(ratatui::widgets::BorderType::Double),
            );
        f.render_widget(header, main_chunks[0]);

        // Entity table
        let header_cells = ["ID", "Type", "Name", "Aliases", "Context", "Mentions"]
            .iter()
            .map(|h| {
                Cell::from(*h).style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            });

        let header_row = Row::new(header_cells)
            .style(Style::default())
            .height(1)
            .bottom_margin(1);

        let merge_source_id = self.merge_source_entity.as_ref().and_then(|e| e.id);
        let rows: Vec<Row> = self
            .entities
            .iter()
            .map(|entity| {
                let is_merge_source = self.entity_mode == EntityMode::MergeSelectTarget
                    && merge_source_id == entity.id;

                let aliases = entity.aliases_list().join(", ");
                let aliases_display = if aliases.is_empty() {
                    "-".to_string()
                } else if aliases.len() > 20 {
                    format!("{}...", &aliases[..17])
                } else {
                    aliases
                };

                let context_display = entity
                    .context
                    .as_ref()
                    .map(|c| {
                        if c.len() > 30 {
                            format!("{}...", &c[..27])
                        } else {
                            c.clone()
                        }
                    })
                    .unwrap_or_else(|| "-".to_string());

                let type_color = if is_merge_source {
                    Color::DarkGray
                } else {
                    match entity.entity_type.as_str() {
                        "person" => Color::Green,
                        "organization" => Color::Blue,
                        _ => Color::Gray,
                    }
                };

                let name_style = if is_merge_source {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().add_modifier(Modifier::BOLD)
                };

                let dim = if is_merge_source { Style::default().fg(Color::DarkGray) } else { Style::default().fg(Color::Gray) };

                let cells = vec![
                    Cell::from(entity.id.unwrap_or(0).to_string()),
                    Cell::from(entity.entity_type.clone()).style(Style::default().fg(type_color)),
                    Cell::from(entity.canonical_name.clone()).style(name_style),
                    Cell::from(aliases_display).style(dim),
                    Cell::from(context_display).style(dim),
                    Cell::from(entity.mention_count.to_string())
                        .style(if is_merge_source { Style::default().fg(Color::DarkGray) } else { Style::default().fg(Color::Yellow) }),
                ];

                Row::new(cells).height(1).bottom_margin(0)
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(4),   // ID
                Constraint::Length(12),  // Type
                Constraint::Length(20),  // Name
                Constraint::Length(22),  // Aliases
                Constraint::Min(20),     // Context
                Constraint::Length(8),   // Mentions
            ],
        )
        .header(header_row)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Cyan))
                .title(format!("Entities ({} total)", self.entities.len())),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

        f.render_stateful_widget(table, main_chunks[1], &mut self.entity_table_state);

        // Footer (changes based on entity mode)
        let footer_text = match self.entity_mode {
            EntityMode::Browse => "↑↓: Navigate | Enter: Details | A: Add | E: Edit | D: Delete | M: Merge | R: Refresh | Esc: Back".to_string(),
            EntityMode::Adding => "Tab/↑↓: Switch Field | Type chars | Space: Cycle Type | Enter: Save | Esc: Cancel".to_string(),
            EntityMode::Editing => "Tab/↑↓: Switch Field | Type chars | Space: Cycle Type | Esc: Save & Close".to_string(),
            EntityMode::DeleteConfirm => "Y: Confirm Delete | N/Esc: Cancel".to_string(),
            EntityMode::MergeSelectTarget => {
                let src_name = self.merge_source_entity.as_ref()
                    .map(|e| e.canonical_name.as_str())
                    .unwrap_or("?");
                format!("↑↓: Select target | Enter: Confirm | Esc: Cancel  (merging '{}')", src_name)
            }
            EntityMode::MergeConfirm => "Y: Confirm Merge | N/Esc: Cancel".to_string(),
        };
        let footer = Paragraph::new(footer_text)
            .style(Style::default().fg(Color::White))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Blue))
                    .title("Controls"),
            );
        f.render_widget(footer, main_chunks[2]);

        // Popups
        if self.show_entity_detail {
            self.render_entity_detail_popup(f, area);
        }
        if self.entity_mode == EntityMode::Adding {
            self.render_entity_add_popup(f, area);
        }
        if self.entity_mode == EntityMode::Editing {
            self.render_entity_edit_popup(f, area);
        }
        if self.entity_mode == EntityMode::DeleteConfirm {
            self.render_entity_delete_confirm(f, area);
        }
        if self.entity_mode == EntityMode::MergeConfirm {
            self.render_entity_merge_confirm(f, area);
        }
    }

    fn render_entity_detail_popup(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(10),
                Constraint::Percentage(80),
                Constraint::Percentage(10),
            ])
            .split(area)[1];

        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(10),
                Constraint::Percentage(80),
                Constraint::Percentage(10),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        if let Some(entity) = &self.selected_entity {
            let aliases = entity.aliases_list().join(", ");
            let aliases_display = if aliases.is_empty() {
                "-".to_string()
            } else {
                aliases
            };

            let context_display = entity
                .context
                .as_ref()
                .map(|c| c.clone())
                .unwrap_or_else(|| "(no context)".to_string());

            let type_label = match entity.entity_type.as_str() {
                "person" => "person",
                "organization" => "org",
                _ => "other",
            };

            let content = vec![
                Line::from(vec![
                    Span::styled(
                        format!("[{}] ", type_label),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        &entity.canonical_name,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Type: ", Style::default().fg(Color::Green)),
                    Span::styled(
                        &entity.entity_type,
                        Style::default().fg(Color::White),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("ID: ", Style::default().fg(Color::Green)),
                    Span::styled(
                        entity.id.unwrap_or(0).to_string(),
                        Style::default().fg(Color::White),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Mentions: ", Style::default().fg(Color::Green)),
                    Span::styled(
                        entity.mention_count.to_string(),
                        Style::default().fg(Color::Yellow),
                    ),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Aliases: ", Style::default().fg(Color::Green)),
                    Span::styled(
                        aliases_display,
                        Style::default().fg(Color::Gray),
                    ),
                ]),
                Line::from(""),
                Line::from(vec![Span::styled(
                    "Context:",
                    Style::default().fg(Color::Green),
                )]),
                Line::from(vec![Span::styled(
                    context_display,
                    Style::default().fg(Color::White),
                )]),
                Line::from(""),
                Line::from(""),
                Line::from(vec![Span::styled(
                    "Press ESC to close | E to edit | D to delete | M to merge",
                    Style::default().fg(Color::Blue),
                )]),
            ];

            let detail_paragraph = Paragraph::new(content)
                .style(Style::default().fg(Color::White))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!("Entity Details - {}", entity.canonical_name))
                        .title_style(
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        )
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .wrap(Wrap { trim: true });

            f.render_widget(detail_paragraph, popup_area);
        }
    }

    fn render_entity_add_popup(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(15),
                Constraint::Percentage(70),
                Constraint::Percentage(15),
            ])
            .split(area)[1];
        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(15),
                Constraint::Percentage(70),
                Constraint::Percentage(15),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        let cursor = "\u{2588}";
        let name_style = if self.entity_add_field == EntityEditField::Name {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let context_style = if self.entity_add_field == EntityEditField::Context {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let name_display = if self.entity_add_field == EntityEditField::Name {
            format!("{}{}", self.entity_add_name, cursor)
        } else if self.entity_add_name.is_empty() {
            "(type a name)".to_string()
        } else {
            self.entity_add_name.clone()
        };

        let type_display: Vec<Span> = ENTITY_TYPES.iter().map(|t| {
            if *t == self.entity_add_type {
                Span::styled(format!(" [{}] ", t), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            } else {
                Span::styled(format!("  {}  ", t), Style::default().fg(Color::Gray))
            }
        }).collect();

        let context_display = if self.entity_add_field == EntityEditField::Context {
            format!("{}{}", self.entity_add_context, cursor)
        } else if self.entity_add_context.is_empty() {
            "(optional)".to_string()
        } else {
            self.entity_add_context.clone()
        };

        let content = vec![
            Line::from(vec![
                Span::styled("Name: ", Style::default().fg(Color::Green)),
            ]),
            Line::from(vec![Span::styled(name_display, name_style)]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Type: ", Style::default().fg(Color::Green)),
            ]),
            Line::from(type_display),
            Line::from(""),
            Line::from(vec![
                Span::styled("Context: ", Style::default().fg(Color::Green)),
            ]),
            Line::from(vec![Span::styled(context_display, context_style)]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Tab/↑↓: Switch Field | Space: Cycle Type | Enter: Save | Esc: Cancel", Style::default().fg(Color::DarkGray)),
            ]),
        ];

        let add_paragraph = Paragraph::new(content)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" ADD NEW ENTITY ")
                    .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                    .border_style(Style::default().fg(Color::Yellow)),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(add_paragraph, popup_area);
    }

    fn render_entity_edit_popup(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(15),
                Constraint::Percentage(70),
                Constraint::Percentage(15),
            ])
            .split(area)[1];
        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(15),
                Constraint::Percentage(70),
                Constraint::Percentage(15),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        let name_style = if self.entity_edit_field == EntityEditField::Name {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let context_style = if self.entity_edit_field == EntityEditField::Context {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let cursor = "█";
        let name_display = if self.entity_edit_field == EntityEditField::Name {
            format!("{}{}", self.entity_edit_name, cursor)
        } else {
            self.entity_edit_name.clone()
        };

        let type_display: Vec<Span> = ENTITY_TYPES.iter().map(|t| {
            if *t == self.entity_edit_type {
                Span::styled(format!(" [{}] ", t), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            } else {
                Span::styled(format!("  {}  ", t), Style::default().fg(Color::Gray))
            }
        }).collect();

        let context_display = if self.entity_edit_field == EntityEditField::Context {
            format!("{}{}", self.entity_edit_context, cursor)
        } else {
            if self.entity_edit_context.is_empty() { "(empty)".to_string() } else { self.entity_edit_context.clone() }
        };

        let content = vec![
            Line::from(vec![
                Span::styled("Name: ", Style::default().fg(Color::Green)),
            ]),
            Line::from(vec![Span::styled(name_display, name_style)]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Type: ", Style::default().fg(Color::Green)),
            ]),
            Line::from(type_display),
            Line::from(""),
            Line::from(vec![
                Span::styled("Context: ", Style::default().fg(Color::Green)),
            ]),
            Line::from(vec![Span::styled(context_display, context_style)]),
        ];

        let edit_paragraph = Paragraph::new(content)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Edit Entity")
                    .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                    .border_style(Style::default().fg(Color::Yellow)),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(edit_paragraph, popup_area);
    }

    fn render_entity_delete_confirm(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(35),
                Constraint::Length(7),
                Constraint::Percentage(35),
            ])
            .split(area)[1];
        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(60),
                Constraint::Percentage(20),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        let entity_name = self.selected_entity.as_ref()
            .map(|e| e.canonical_name.as_str())
            .unwrap_or("?");

        let content = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Delete entity: ", Style::default().fg(Color::White)),
                Span::styled(entity_name, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::styled("?", Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  [Y] Yes  ", Style::default().fg(Color::Red)),
                Span::styled("  [N] No  ", Style::default().fg(Color::Green)),
            ]),
        ];

        let paragraph = Paragraph::new(content)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Confirm Delete")
                    .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                    .border_style(Style::default().fg(Color::Red)),
            );

        f.render_widget(paragraph, popup_area);
    }

    fn render_entity_merge_confirm(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(30),
                Constraint::Length(9),
                Constraint::Percentage(30),
            ])
            .split(area)[1];
        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(15),
                Constraint::Percentage(70),
                Constraint::Percentage(15),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        let source_name = self.merge_source_entity.as_ref()
            .map(|e| e.canonical_name.as_str())
            .unwrap_or("?");
        let target_name = self.selected_entity.as_ref()
            .map(|e| e.canonical_name.as_str())
            .unwrap_or("?");

        let content = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Merge ", Style::default().fg(Color::White)),
                Span::styled(source_name, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(" INTO ", Style::default().fg(Color::White)),
                Span::styled(target_name, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("?", Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                format!("'{}' becomes an alias. Contexts combined. Mentions transferred.", source_name),
                Style::default().fg(Color::Gray),
            )]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  [Y] Yes  ", Style::default().fg(Color::Yellow)),
                Span::styled("  [N] No  ", Style::default().fg(Color::Green)),
            ]),
        ];

        let paragraph = Paragraph::new(content)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Confirm Merge")
                    .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                    .border_style(Style::default().fg(Color::Yellow)),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(paragraph, popup_area);
    }

    fn render_message_popup(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(30),
                Constraint::Length(5),
                Constraint::Percentage(65),
            ])
            .split(area)[1];

        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(60),
                Constraint::Percentage(20),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        let para = Paragraph::new(self.message.clone())
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Red))
                    .title("Message (press Esc)"),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(para, popup_area);
    }

    fn render_transcript_popup(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(3),
                Constraint::Percentage(94),
                Constraint::Percentage(3),
            ])
            .split(area)[1];

        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(2),
                Constraint::Percentage(96),
                Constraint::Percentage(2),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        // Check if we have enrichment data
        let has_enrichment = self.transcript_summary.is_some()
            || self.transcript_topics.is_some()
            || self.transcript_entities.is_some();

        // Dynamic chat panel height for transcript view
        let has_chat_content = !self.chat.messages.is_empty()
            || !self.chat.streaming_content.is_empty()
            || self.chat.is_generating;

        let chat_panel_h: u16 = if has_chat_content {
            let available = popup_area.height;
            (available * 45 / 100).max(8)
        } else {
            (self.chat.suggestions.len() as u16 + 4).max(5).min(8)
        };

        // Build constraints dynamically
        let mut constraints: Vec<Constraint> = Vec::new();

        if has_enrichment {
            constraints.push(Constraint::Length(self.calculate_enrichment_height()));
        }
        constraints.push(Constraint::Min(5)); // Transcript (flexible)
        constraints.push(Constraint::Length(chat_panel_h)); // Unified chat panel

        let content_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(popup_area);

        let mut chunk_idx = 0;

        if has_enrichment {
            self.render_enrichment_section(f, content_chunks[chunk_idx]);
            chunk_idx += 1;
        }

        // Transcript
        self.render_transcript_content(f, content_chunks[chunk_idx]);
        chunk_idx += 1;

        // Unified chat panel
        self.render_chat_panel(f, content_chunks[chunk_idx]);
    }

    fn calculate_enrichment_height(&self) -> u16 {
        let mut height: u16 = 2; // Border

        if self.transcript_summary.is_some() {
            height += 3; // Summary label + content + spacing
        }
        if self.transcript_topics.is_some() {
            height += 2; // Topics line + spacing
        }
        if self.transcript_entities.is_some() {
            height += 2; // Entities line + spacing
        }
        if self.transcript_key_points.as_ref().map_or(false, |kp| !kp.is_empty()) {
            height += 3; // Key points label + first point + spacing
        }

        height.min(12) // Cap at reasonable size
    }

    fn render_enrichment_section(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let mut lines: Vec<Line> = Vec::new();

        // Summary
        if let Some(summary) = &self.transcript_summary {
            lines.push(Line::from(vec![
                Span::styled("Summary: ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(summary, Style::default().fg(Color::White)),
            ]));
            lines.push(Line::from(""));
        }

        // Topics
        if let Some(topics) = &self.transcript_topics {
            if !topics.is_empty() {
                let topic_spans: Vec<Span> = std::iter::once(
                    Span::styled("Topics: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                )
                .chain(topics.iter().enumerate().flat_map(|(i, topic)| {
                    let mut spans = vec![Span::styled(
                        format!("[{}]", topic),
                        Style::default().fg(Color::Cyan),
                    )];
                    if i < topics.len() - 1 {
                        spans.push(Span::raw(" "));
                    }
                    spans
                }))
                .collect();
                lines.push(Line::from(topic_spans));
            }
        }

        // Entities
        if let Some(entities) = &self.transcript_entities {
            if !entities.is_empty() {
                let entity_spans: Vec<Span> = std::iter::once(
                    Span::styled("Entities: ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
                )
                .chain(entities.iter().enumerate().flat_map(|(i, (name, etype))| {
                    let color = match etype.as_str() {
                        "person" => Color::Green,
                        "organization" => Color::Blue,
                        _ => Color::Gray,
                    };
                    let mut spans = vec![Span::styled(name, Style::default().fg(color))];
                    if i < entities.len() - 1 {
                        spans.push(Span::raw(", "));
                    }
                    spans
                }))
                .collect();
                lines.push(Line::from(entity_spans));
            }
        }

        // Key points (just first one to save space)
        if let Some(key_points) = &self.transcript_key_points {
            if !key_points.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("Key: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::styled(&key_points[0], Style::default().fg(Color::White)),
                ]));
            }
        }

        let enrichment = Paragraph::new(lines)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green))
                    .title("Knowledge Extract")
                    .title_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(enrichment, area);
    }

    fn render_transcript_content(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        // Calculate available height and width for content (subtract borders)
        let content_height = area.height.saturating_sub(2) as usize;
        let content_width = area.width.saturating_sub(4) as usize;

        // Handle text wrapping for very long lines
        let wrapped_lines = self.wrap_text_to_lines(&self.transcript_content, content_width);
        let total_lines = wrapped_lines.len();

        // Create scrollable content
        let (visible_content, scroll_info) = if total_lines > content_height {
            let max_scroll = total_lines.saturating_sub(content_height);
            let actual_offset = std::cmp::min(self.transcript_scroll_offset, max_scroll);
            let end = std::cmp::min(actual_offset + content_height, total_lines);

            let visible_lines = wrapped_lines[actual_offset..end].join("\n");
            let scroll_info = format!(
                "Transcript [{}/{}] - up/down: scroll, C: copy, T: re-transcribe, ESC: close",
                actual_offset + 1,
                total_lines
            );
            (visible_lines, scroll_info)
        } else {
            (
                self.transcript_content.clone(),
                "Transcript - C: copy, T: re-transcribe, ESC: close".to_string(),
            )
        };

        let para = Paragraph::new(visible_content)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Cyan))
                    .title(scroll_info),
            )
            .scroll((0, 0));

        f.render_widget(para, area);
    }

    fn render_delete_confirmation_popup(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(35),
                Constraint::Length(7),
                Constraint::Percentage(35),
            ])
            .split(area)[1];
        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(60),
                Constraint::Percentage(20),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        let recording_name = if let Some(recording) = &self.delete_candidate {
            recording
                .display_name
                .as_ref()
                .unwrap_or(&recording.directory_name)
                .as_str()
        } else {
            "?"
        };

        let content = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Delete recording: ", Style::default().fg(Color::White)),
                Span::styled(recording_name, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::styled("?", Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  [Y] Yes  ", Style::default().fg(Color::Red)),
                Span::styled("  [N] No  ", Style::default().fg(Color::Green)),
            ]),
        ];

        let paragraph = Paragraph::new(content)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Confirm Delete")
                    .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                    .border_style(Style::default().fg(Color::Red)),
            );

        f.render_widget(paragraph, popup_area);
    }

    fn render_search_input(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(40),
                Constraint::Length(3),
                Constraint::Percentage(40),
            ])
            .split(area)[1];

        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(60),
                Constraint::Percentage(20),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        let search_input = Paragraph::new(format!("Search: {}", self.search_query))
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Cyan))
                    .title("Search Recordings"),
            );

        f.render_widget(search_input, popup_area);
    }

    async fn execute_record_and_transcribe(&mut self) -> Result<()> {
        // Check if already recording or transcribing
        if self.recording_task.is_some() || self.transcription_task.is_some() {
            self.message = "Recording or transcription already in progress".to_string();
            self.show_message = true;
            return Ok(());
        }

        // Show immediate progress animation
        self.progress_animation = Some("Recording... (Press Esc to stop)".to_string());
        self.progress_frame = 0;
        self.show_message = true;
        self.recording_mode = Some(RecordingMode::RecordAndTranscribe);

        // Generate filename and start recording task (no name prompt, consistent with A command)
        let recording_name = generate_recording_name(None);
        self.start_recording_task(recording_name).await?;

        Ok(())
    }

    async fn execute_add_external_file(&mut self) -> Result<()> {
        // Check if already recording or transcribing
        if self.recording_task.is_some() || self.transcription_task.is_some() {
            self.message = "Recording or transcription already in progress".to_string();
            self.show_message = true;
            return Ok(());
        }

        // Show file dialog for importing audio file
        self.show_file_dialog = true;
        self.file_dialog_stage = FileDialogStage::FilePath;
        self.file_path_input.clear();
        self.file_name_input.clear();

        Ok(())
    }

    async fn start_file_import(&mut self, file_path: String, display_name: String) -> Result<()> {
        // Track import+transcription inline — no blocking popup
        self.transcribing_recording_name = Some(display_name.clone());
        self.progress_frame = 0;

        let source_path = PathBuf::from(file_path.trim());
        let transcription_mode = self.config.transcription.clone();
        let display_name_clone = display_name.clone();

        self.transcription_task = Some(tokio::spawn(async move {
            let mut workflow = WorkflowManager::new().unwrap();
            workflow
                .complete_import_workflow_silent(
                    &source_path,
                    Some(display_name_clone),
                    Some(transcription_mode),
                )
                .await
                .map(|_| ())
        }));

        Ok(())
    }

    // Removed unused execute_transcribe_file; dashboard uses TranscribeSelected (T) instead

    async fn execute_transcribe_selected(&mut self) -> Result<()> {
        // Check if transcription is already running
        if self.transcription_task.is_some() {
            self.message = "Transcription already in progress. Please wait...".to_string();
            self.show_message = true;
            return Ok(());
        }

        // Get the selected recording
        let selected_index = match self.table_state.selected() {
            Some(i) => i,
            None => {
                self.message = "No recording selected".to_string();
                self.show_message = true;
                return Ok(());
            }
        };

        let selected_recording = match self.recordings.get(selected_index) {
            Some(recording) => recording.clone(),
            None => {
                self.message = "Invalid recording selection".to_string();
                self.show_message = true;
                return Ok(());
            }
        };

        // Check if transcript already exists
        let has_transcript = if let Some(id) = selected_recording.id {
            self.db
                .get_transcript_by_recording_id(id)
                .is_ok_and(|t| t.is_some())
        } else {
            false
        };

        if has_transcript {
            // Check if this is the second press on the same recording
            if self.last_transcribe_warning == Some(selected_index) {
                // User confirmed overwrite - proceed with transcription
                self.last_transcribe_warning = None;
            } else {
                // First press - show warning and remember this recording
                self.last_transcribe_warning = Some(selected_index);
                self.message =
                    "Recording already has transcript. Press T again to overwrite.".to_string();
                self.show_message = true;
                return Ok(());
            }
        } else {
            // Clear any previous warning state
            self.last_transcribe_warning = None;
        }

        // Track transcription inline — no blocking popup
        let directory_name = selected_recording.directory_name.clone();
        self.transcribing_recording_name = Some(directory_name.clone());
        self.progress_frame = 0;

        // Start transcription in background
        let transcription_mode = self.config.transcription.clone();

        self.transcription_task = Some(tokio::spawn(async move {
            let mut workflow = WorkflowManager::new().unwrap();
            workflow
                .retranscribe_recording_silent(&directory_name, transcription_mode)
                .await
        }));

        Ok(())
    }

    fn format_duration(&self, seconds: i64) -> String {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        let secs = seconds % 60;

        if hours > 0 {
            format!("{}h {}m", hours, minutes)
        } else if minutes > 0 {
            format!("{}m {}s", minutes, secs)
        } else {
            format!("{}s", secs)
        }
    }

    async fn create_stereo_temp_file(
        &self,
        mono_file_path: &std::path::Path,
    ) -> Result<std::path::PathBuf> {
        use std::fs;

        // Create a temporary file path for the stereo version
        let temp_dir = std::env::temp_dir();
        let temp_filename = format!(
            "scriba_stereo_{}.wav",
            mono_file_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
        );
        let temp_path = temp_dir.join(temp_filename);

        // Use Rust's hound crate to convert mono to stereo
        let mono_reader =
            hound::WavReader::open(mono_file_path).context("Failed to open mono audio file")?;

        let spec = mono_reader.spec();

        // Create stereo spec (2 channels)
        let stereo_spec = hound::WavSpec {
            channels: 2,
            sample_rate: spec.sample_rate,
            bits_per_sample: spec.bits_per_sample,
            sample_format: spec.sample_format,
        };

        let mut stereo_writer = hound::WavWriter::create(&temp_path, stereo_spec)
            .context("Failed to create stereo audio file")?;

        // Convert samples based on format
        match spec.sample_format {
            hound::SampleFormat::Float => {
                // 32-bit float samples
                for sample in mono_reader.into_samples::<f32>() {
                    match sample {
                        Ok(s) => {
                            // Write the same sample to both left and right channels
                            stereo_writer.write_sample(s)?; // Left
                            stereo_writer.write_sample(s)?; // Right
                        }
                        Err(e) => {
                            return Err(anyhow::anyhow!("Error processing audio sample: {}", e))
                        }
                    }
                }
            }
            hound::SampleFormat::Int => {
                // Integer samples (16-bit or 24-bit)
                if spec.bits_per_sample == 16 {
                    for sample in mono_reader.into_samples::<i16>() {
                        match sample {
                            Ok(s) => {
                                stereo_writer.write_sample(s)?; // Left
                                stereo_writer.write_sample(s)?; // Right
                            }
                            Err(e) => {
                                return Err(anyhow::anyhow!(
                                    "Error processing audio sample: {}",
                                    e
                                ));
                            }
                        }
                    }
                } else if spec.bits_per_sample == 24 {
                    for sample in mono_reader.into_samples::<i32>() {
                        match sample {
                            Ok(s) => {
                                stereo_writer.write_sample(s)?; // Left
                                stereo_writer.write_sample(s)?; // Right
                            }
                            Err(e) => {
                                return Err(anyhow::anyhow!(
                                    "Error processing audio sample: {}",
                                    e
                                ));
                            }
                        }
                    }
                } else {
                    return Err(anyhow::anyhow!(
                        "Unsupported bit depth: {}",
                        spec.bits_per_sample
                    ));
                }
            }
        }

        // Finalize the stereo file
        stereo_writer
            .finalize()
            .context("Failed to finalize stereo audio file")?;

        // Schedule cleanup of temp file after a delay
        let temp_path_clone = temp_path.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            let _ = fs::remove_file(&temp_path_clone);
        });

        Ok(temp_path)
    }

    fn emergency_stop_all_audio_players(&self) -> Result<()> {
        // Kill all common audio players as a fallback when PID is not available
        #[cfg(unix)]
        {
            use std::process::Command;
            // Try to kill common audio players
            let players = ["mpv", "ffplay", "afplay"];
            for player in &players {
                let _ = Command::new("killall").arg(player).output();
            }
        }

        #[cfg(windows)]
        {
            use std::process::Command;
            // Try to kill common audio players on Windows
            let players = ["mpv.exe", "ffplay.exe"];
            for player in &players {
                let _ = Command::new("taskkill")
                    .arg("/IM")
                    .arg(player)
                    .arg("/F")
                    .output();
            }
        }
        Ok(())
    }

    fn stop_audio_playback(&self, pid: u32) -> Result<()> {
        #[cfg(unix)]
        {
            use std::process::Command;
            // Use SIGTERM first for graceful shutdown, then SIGKILL if needed
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .output();

            // Give a very brief moment for graceful shutdown
            std::thread::sleep(std::time::Duration::from_millis(10));

            // Use SIGKILL immediately for faster termination (audio players can be stubborn)
            let kill_result = Command::new("kill")
                .arg("-KILL")
                .arg(pid.to_string())
                .output();

            // Also try pkill in case the process spawned children
            let _ = Command::new("pkill")
                .arg("-P")
                .arg(pid.to_string())
                .output();

            match kill_result {
                Ok(_) => Ok(()),
                Err(_e) => {
                    // If direct kill fails, try killall on common audio players
                    let _ = Command::new("killall")
                        .arg("mpv")
                        .arg("ffplay")
                        .arg("afplay")
                        .output();
                    Ok(())
                }
            }
        }

        #[cfg(windows)]
        {
            use std::process::Command;
            let _ = Command::new("taskkill")
                .arg("/PID")
                .arg(pid.to_string())
                .arg("/F")
                .output();
            Ok(())
        }
    }

    // start_progress_animation not used; progress is updated directly via fields

    fn stop_progress_animation(&mut self) {
        self.progress_animation = None;
    }

    fn update_progress_message(&mut self) {
        if let Some(base_msg) = &self.progress_animation {
            let spinners = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let spinner = spinners[self.progress_frame % spinners.len()];

            // If recording is active, show volume level instead of progress bar
            if self.recording_task.is_some() {
                let volume_bar = self.create_volume_bar(self.current_volume_level);
                self.message = format!("{} {} [{}]", spinner, base_msg, volume_bar);
            } else {
                // Regular progress bar for transcription
                let bar_width = 20;
                let progress_pos = (self.progress_frame / 2) % (bar_width * 2);
                let mut bar = vec!["▱"; bar_width];

                if progress_pos < bar_width {
                    for i in 0..=progress_pos.min(bar_width - 1) {
                        bar[i] = "▰";
                    }
                } else {
                    let reverse_pos = (bar_width * 2 - 1) - progress_pos;
                    for i in reverse_pos..bar_width {
                        bar[i] = "▰";
                    }
                }

                let bar_str = bar.join("");
                self.message = format!("{} {} [{}]", spinner, base_msg, bar_str);
            }

            self.progress_frame += 1;
        }
    }

    fn create_volume_bar(&self, level: f32) -> String {
        let bar_width = 20;
        // Scale the level (0.0 to 1.0) to bar width and apply some amplification for visibility
        let scaled_level = (level * 50.0).min(1.0); // Amplify for visibility
        let filled_chars = (scaled_level * bar_width as f32) as usize;

        let mut bar = vec!["▱"; bar_width];
        for i in 0..filled_chars.min(bar_width) {
            bar[i] = "▰";
        }

        format!("{}|{}%", bar.join(""), (scaled_level * 100.0) as u8)
    }

    async fn start_recording_task(&mut self, recording_name: String) -> Result<()> {
        // Create channels for recording control
        let (stop_tx, stop_rx) = mpsc::channel(1);
        let (level_tx, level_rx) = mpsc::channel(100);

        // Store the channels for control and feedback
        self.recording_stop_tx = Some(stop_tx);
        self.recording_level_rx = Some(level_rx);

        // Use speech-optimized compression settings
        let compression_settings = CompressionSettings::speech_optimized();

        // Determine if auto-transcription is enabled based on recording mode
        let _auto_transcribe = matches!(
            self.recording_mode,
            Some(RecordingMode::RecordAndTranscribe)
        );

        // Silence auto-stop timeout from config
        let silence_timeout = if self.config.silence_auto_stop.enabled {
            Some(Duration::from_secs(self.config.silence_auto_stop.timeout_seconds as u64))
        } else {
            None
        };

        // Use unified recording function with TUI control channels
        let output_path = PathBuf::from(&recording_name);

        self.recording_task = Some(tokio::spawn(async move {
            record_audio(
                output_path,
                RecordOptions {
                    compression_settings: Some(compression_settings),
                    stop_rx: Some(stop_rx),
                    level_tx: Some(level_tx),
                    verbose: false,
                    silence_timeout,
                },
            )
            .await
        }));

        Ok(())
    }

    // ── Voice Mode ("Scriba Forever") ────────────────────────────────────

    async fn toggle_voice_mode(&mut self) {
        if self.voice_mode_active {
            // Shut down voice detector
            if let Some(handle) = self.voice_detector_handle.take() {
                // If recording, stop it first
                if handle.listening_state() == VoiceListeningState::Recording {
                    let _ = handle.stop_recording();
                }
                handle.shutdown();
            }
            self.voice_command_rx = None;
            self.voice_mode_active = false;
            self.notification_message = Some(("Voice mode disabled".to_string(), 30));
        } else {
            // Start voice detector
            let (tx, rx) = mpsc::channel(8);
            match start_voice_detector(&self.config.voice, tx).await {
                Ok(handle) => {
                    self.voice_detector_handle = Some(handle);
                    self.voice_command_rx = Some(rx);
                    self.voice_mode_active = true;
                    self.notification_message = Some((
                        "Voice mode active -- say \"Scriba record\" to start".to_string(),
                        60,
                    ));
                }
                Err(e) => {
                    self.notification_message = Some((
                        format!("Failed to start voice mode: {}", e),
                        60,
                    ));
                }
            }
        }
    }

    async fn handle_voice_record_command(&mut self) {
        // Don't start if already recording
        if self.recording_task.is_some() {
            return;
        }

        let recording_name = generate_recording_name(None);

        if let Some(ref handle) = self.voice_detector_handle {
            if let Err(e) = handle.start_recording(&recording_name) {
                self.notification_message = Some((
                    format!("Voice record failed: {}", e),
                    60,
                ));
                return;
            }
        }

        self.notification_message = Some((
            "Voice: Recording started! Say \"Scriba stop\" to finish.".to_string(),
            60,
        ));
    }

    async fn handle_voice_stop_command(&mut self) {
        let result = if let Some(ref handle) = self.voice_detector_handle {
            handle.stop_recording()
        } else {
            return;
        };

        match result {
            Ok(Some((dir_name, wav_path))) => {
                self.notification_message = Some((
                    "Voice: Recording stopped. Transcribing...".to_string(),
                    60,
                ));

                // Save to database
                let dir_name_clone = dir_name.clone();
                if let Ok(mut db) = Database::new() {
                    let meta = crate::core::FileManager::extract_audio_metadata(&wav_path);
                    if let Ok(meta) = meta {
                        let recording = Recording {
                            id: None,
                            directory_name: dir_name.clone(),
                            display_name: None,
                            created_at: chrono::Utc::now(),
                            updated_at: chrono::Utc::now(),
                            duration_seconds: meta.duration_seconds,
                            file_size_bytes: meta.file_size_bytes,
                            audio_format: meta.audio_format,
                            sample_rate: meta.sample_rate,
                            channels: meta.channels,
                            has_transcript: false,
                            transcript_status: "pending".to_string(),
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
                            audio_path: wav_path
                                .file_name()
                                .unwrap()
                                .to_string_lossy()
                                .to_string(),
                            transcript_path: None,
                        };
                        let _ = db.insert_recording(&recording);
                    }
                }

                // Start transcription pipeline
                let transcription_mode = self.config.transcription.clone();
                self.transcribing_recording_name = Some(dir_name_clone.clone());
                self.progress_frame = 0;
                let _ = self.load_recordings();
                let _ = self.load_stats();

                self.transcription_task = Some(tokio::spawn(async move {
                    let mut workflow = WorkflowManager::new().unwrap();
                    workflow
                        .retranscribe_recording_silent(&dir_name_clone, transcription_mode)
                        .await
                }));
            }
            Ok(None) => {
                self.notification_message = Some(("Voice: No recording to stop.".to_string(), 30));
            }
            Err(e) => {
                self.notification_message = Some((
                    format!("Voice stop failed: {}", e),
                    60,
                ));
            }
        }
    }

    fn render_file_dialog_popup(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Length(12),
                Constraint::Percentage(65),
            ])
            .split(area)[1];

        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(15),
                Constraint::Percentage(70),
                Constraint::Percentage(15),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        let (title, prompt, current_input, hint) = match self.file_dialog_stage {
            FileDialogStage::FilePath => (
                "Import Audio File - Step 1/2",
                "Enter the full path to the audio file:",
                &self.file_path_input,
                "Example: /path/to/your/audio.mp3 or ~/Downloads/recording.wav",
            ),
            FileDialogStage::FileName => (
                "Import Audio File - Step 2/2",
                "Enter a display name for this recording:",
                &self.file_name_input,
                "This name will be shown in your recordings list",
            ),
        };

        let content = vec![
            Line::from(vec![Span::styled(
                prompt,
                Style::default().fg(Color::White),
            )]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Input: ", Style::default().fg(Color::Green)),
                Span::styled(
                    format!("{}_", current_input),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(hint, Style::default().fg(Color::Gray))]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Press ENTER to continue, ESC to cancel",
                Style::default().fg(Color::Blue),
            )]),
        ];

        let dialog_paragraph = Paragraph::new(content)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(dialog_paragraph, popup_area);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Ask Scriba: Chat Implementation
    // ─────────────────────────────────────────────────────────────────────────

    fn generate_suggestions(&self) -> Vec<String> {
        match &self.chat.context {
            ChatContext::Global => {
                let mut suggestions = vec!["What have I been talking about recently?".to_string()];

                // Top entity suggestion
                if !self.entities.is_empty() {
                    if let Some(top) = self.entities.first() {
                        suggestions.push(format!("Summarize my conversations about {}", top.canonical_name));
                    }
                }

                // Action items
                let has_action_items = self.recordings.iter().any(|r| {
                    r.action_items.as_ref().map_or(false, |a| !a.is_empty() && a != "[]")
                });
                if has_action_items {
                    suggestions.push("What action items do I have pending?".to_string());
                }

                // People
                let has_people = self.entities.iter().any(|e| e.entity_type == "person");
                if has_people {
                    suggestions.push("Who have I been meeting with most?".to_string());
                }

                suggestions.truncate(4);
                suggestions
            }
            ChatContext::Recording { .. } => {
                let mut suggestions = vec!["Summarize the key takeaways".to_string()];

                // Draft email if action items exist
                if let Some(recording) = &self.current_transcript_recording {
                    let has_actions = recording.action_items.as_ref()
                        .map_or(false, |a| !a.is_empty() && a != "[]");
                    let has_key_points = recording.key_points.as_ref()
                        .map_or(false, |k| !k.is_empty() && k != "[]");

                    if has_actions || has_key_points {
                        suggestions.push("Draft a follow-up email".to_string());
                    }

                    if has_actions {
                        suggestions.push("What were the action items?".to_string());
                    }
                }

                // Entity cross-reference
                if let Some(entities) = &self.transcript_entities {
                    if let Some((name, _)) = entities.first() {
                        suggestions.push(format!("What other recordings mention {}?", name));
                    }
                }

                suggestions.truncate(4);
                suggestions
            }
        }
    }

    fn init_global_chat(&mut self) {
        // Load world context
        let world = WorldContext::load().ok()
            .and_then(|wc| WorldData::from_json(&wc.content).ok())
            .unwrap_or_default();

        let owner_name = if world.owner.name.is_empty() {
            "User".to_string()
        } else {
            world.owner.name.clone()
        };

        let world_json = world.to_json().unwrap_or_default();

        // Stats summary
        let stats_summary = if let Some(stats) = &self.stats {
            format!(
                "{} recordings, {} total duration, {} transcribed",
                stats.total_recordings,
                stats.format_duration(),
                stats.transcribed_count
            )
        } else {
            "No stats available".to_string()
        };

        // Recent recordings summary
        let recent_recordings: String = self.recordings.iter().take(10).map(|r| {
            let name = r.display_name.as_ref().unwrap_or(&r.directory_name);
            let summary = r.summary.as_deref().unwrap_or("(no summary)");
            format!("- {} [{}]: {}\n", name, r.created_at.format("%m/%d"), summary)
        }).collect();

        let entities_summary = world.entities_summary();

        self.chat.system_prompt = chat_prompts::build_global_chat_prompt(
            &owner_name,
            &world_json,
            &stats_summary,
            &recent_recordings,
            &entities_summary,
        );

        self.chat.context = ChatContext::Global;
        self.chat.suggestions = self.generate_suggestions();
        self.chat.show_suggestions = self.chat.messages.is_empty();
    }

    fn init_recording_chat(&mut self, recording: &Recording) {
        // Stash global messages
        self.global_chat_messages = self.chat.messages.clone();

        let world = WorldContext::load().ok()
            .and_then(|wc| WorldData::from_json(&wc.content).ok())
            .unwrap_or_default();

        let owner_name = if world.owner.name.is_empty() {
            "User".to_string()
        } else {
            world.owner.name.clone()
        };

        let recording_name = recording.display_name.as_ref()
            .unwrap_or(&recording.directory_name)
            .clone();

        let summary = recording.summary.as_deref().unwrap_or("");
        let action_items = recording.action_items.as_deref().unwrap_or("");
        let key_points_str = recording.key_points.as_ref()
            .and_then(|kp| serde_json::from_str::<Vec<String>>(kp).ok())
            .map(|v| v.join("\n- "))
            .unwrap_or_default();

        let topics_str = self.transcript_topics.as_ref()
            .map(|t| t.join(", "))
            .unwrap_or_default();

        let entities_str = self.transcript_entities.as_ref()
            .map(|e| e.iter().map(|(n, t)| format!("{} ({})", n, t)).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();

        let world_json = world.to_json().unwrap_or_default();

        self.chat.system_prompt = chat_prompts::build_recording_chat_prompt(
            &owner_name,
            &recording_name,
            &self.transcript_content,
            summary,
            &topics_str,
            &entities_str,
            &key_points_str,
            action_items,
            &world_json,
        );

        self.chat.context = ChatContext::Recording {
            recording_id: recording.id.unwrap_or(0),
            recording_name,
        };
        self.chat.messages.clear();
        self.chat.input_buffer.clear();
        self.chat.streaming_content.clear();
        self.chat.current_status = None;
        self.chat.is_generating = false;
        self.chat.scroll_offset = 0;

        self.current_transcript_recording = Some(recording.clone());
        self.chat.suggestions = self.generate_suggestions();
        self.chat.show_suggestions = true;
    }

    fn restore_global_chat(&mut self) {
        self.chat.messages = std::mem::take(&mut self.global_chat_messages);
        self.chat.context = ChatContext::Global;
        self.chat.input_buffer.clear();
        self.chat.streaming_content.clear();
        self.chat.current_status = None;
        self.chat.is_generating = false;
        self.chat.scroll_offset = 0;
        self.current_transcript_recording = None;
        self.chat.suggestions = self.generate_suggestions();
        self.chat.show_suggestions = self.chat.messages.is_empty();
        self.chat.focus = ChatFocus::Table;
    }

    fn send_chat_message(&mut self) {
        let user_msg = if self.chat.show_suggestions && !self.chat.suggestions.is_empty() {
            // Check if "Ask Scriba anything..." (last option) is selected
            if self.chat.selected_suggestion >= self.chat.suggestions.len() {
                // Switch to free-form input mode
                self.chat.show_suggestions = false;
                return;
            }
            // Use selected suggestion
            let msg = self.chat.suggestions[self.chat.selected_suggestion].clone();
            self.chat.show_suggestions = false;
            msg
        } else if !self.chat.input_buffer.is_empty() {
            let msg = self.chat.input_buffer.clone();
            self.chat.input_buffer.clear();
            self.chat.show_suggestions = false;
            msg
        } else {
            return;
        };

        // Add user message
        self.chat.messages.push(ChatMessage {
            role: ChatRole::User,
            content: user_msg.clone(),
        });

        // Spawn generation pipeline
        let (event_tx, event_rx) = mpsc::channel::<ChatStreamEvent>(100);
        self.chat.stream_rx = Some(event_rx);
        self.chat.is_generating = true;
        self.chat.auto_scroll = true; // re-engage for new response
        self.chat.streaming_content.clear();
        self.chat.current_status = Some("Preparing...".to_string());

        let config = self.config.enrichment.clone();
        let system_prompt = self.chat.system_prompt.clone();
        let messages: Vec<(String, String)> = self.chat.messages.iter().map(|m| {
            let role = match m.role {
                ChatRole::User => "User",
                ChatRole::Assistant => "Assistant",
                ChatRole::System => "System",
            };
            (role.to_string(), m.content.clone())
        }).collect();

        // Collect entity names for cross-referencing
        let entity_names: Vec<(String, i64)> = self.entities.iter()
            .filter_map(|e| e.id.map(|id| (e.canonical_name.clone(), id)))
            .collect();

        self.chat.generation_task = Some(tokio::spawn(async move {
            chat_generation_pipeline(config, system_prompt, messages, user_msg, entity_names, event_tx).await;
        }));
    }

    fn poll_chat_stream(&mut self) {
        if let Some(ref mut rx) = self.chat.stream_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ChatStreamEvent::Status(msg) => {
                        self.chat.current_status = Some(msg);
                    }
                    ChatStreamEvent::Chunk(text) => {
                        self.chat.current_status = None;
                        self.chat.streaming_content.push_str(&text);
                    }
                    ChatStreamEvent::Done => {
                        let content = std::mem::take(&mut self.chat.streaming_content);
                        self.chat.messages.push(ChatMessage {
                            role: ChatRole::Assistant,
                            content,
                        });
                        self.chat.is_generating = false;
                        self.chat.stream_rx = None;
                        self.chat.current_status = None;
                        // Auto-send queued message if any
                        if self.chat.pending_message.is_some() {
                            let msg = self.chat.pending_message.take().unwrap();
                            self.chat.input_buffer = msg;
                            self.send_chat_message();
                        }
                        return;
                    }
                    ChatStreamEvent::Error(msg) => {
                        self.chat.messages.push(ChatMessage {
                            role: ChatRole::System,
                            content: format!("Error: {}", msg),
                        });
                        self.chat.is_generating = false;
                        self.chat.stream_rx = None;
                        self.chat.current_status = None;
                        self.chat.streaming_content.clear();
                        // Drop pending message on error
                        self.chat.pending_message = None;
                        return;
                    }
                }
            }
        }
    }

    fn handle_chat_key(&mut self, key_code: KeyCode) -> bool {
        match key_code {
            KeyCode::Tab => {
                // Toggle focus
                self.chat.focus = match self.chat.focus {
                    ChatFocus::Table => ChatFocus::ChatInput,
                    ChatFocus::ChatInput => ChatFocus::Table,
                };
                true
            }
            _ if self.chat.focus == ChatFocus::ChatInput => {
                match key_code {
                    KeyCode::Enter => {
                        if self.chat.is_generating {
                            // Queue the message — will auto-send when generation completes
                            if !self.chat.input_buffer.is_empty() {
                                self.chat.pending_message = Some(self.chat.input_buffer.clone());
                                self.chat.input_buffer.clear();
                            }
                        } else {
                            self.send_chat_message();
                        }
                    }
                    KeyCode::Esc => {
                        if !self.chat.input_buffer.is_empty() {
                            self.chat.input_buffer.clear();
                        } else {
                            self.chat.focus = ChatFocus::Table;
                        }
                    }
                    KeyCode::Backspace => {
                        self.chat.input_buffer.pop();
                    }
                    KeyCode::Up => {
                        if self.chat.show_suggestions && !self.chat.suggestions.is_empty() {
                            if self.chat.selected_suggestion > 0 {
                                self.chat.selected_suggestion -= 1;
                            }
                        } else {
                            // Disengage auto-scroll when user scrolls up
                            if self.chat.auto_scroll {
                                // Snap scroll_offset to current bottom before disengaging
                                self.chat.scroll_offset = usize::MAX; // will be clamped in render
                            }
                            self.chat.auto_scroll = false;
                            self.chat.scroll_offset = self.chat.scroll_offset.saturating_sub(1);
                        }
                    }
                    KeyCode::Down => {
                        if self.chat.show_suggestions && !self.chat.suggestions.is_empty() {
                            let max_idx = self.chat.suggestions.len(); // last = free-form
                            if self.chat.selected_suggestion < max_idx {
                                self.chat.selected_suggestion += 1;
                            }
                        } else if !self.chat.auto_scroll {
                            self.chat.scroll_offset += 1;
                            // Re-engage auto-scroll if we've scrolled to the bottom
                            // (the render will clamp, so just check a generous threshold)
                            self.chat.auto_scroll = true; // will be re-checked — if user scrolls up again, it disengages
                        }
                    }
                    KeyCode::Char(c) => {
                        self.chat.input_buffer.push(c);
                        self.chat.show_suggestions = false;
                    }
                    _ => {}
                }
                true
            }
            // Auto-focus: printable chars when table is focused go to chat
            KeyCode::Char(c) if self.chat.focus == ChatFocus::Table
                && !self.show_transcript
                && !self.search_mode
                && c.is_alphanumeric()
                && !matches!(c, 'q' | 'h' | 'H' | 's' | 'S' | 'r' | 'R' | 'a' | 'A' | 't' | 'T' | 'w' | 'W' | 'd' | 'p' | 'P' | '/' | '[' | ']') => {
                self.chat.focus = ChatFocus::ChatInput;
                self.chat.input_buffer.push(c);
                self.chat.show_suggestions = false;
                true
            }
            _ => false,
        }
    }

    fn handle_transcript_chat_key(&mut self, key_code: KeyCode) -> bool {
        if key_code == KeyCode::Tab {
            self.chat.focus = match self.chat.focus {
                ChatFocus::Table => ChatFocus::ChatInput, // Table = transcript scroll in this context
                ChatFocus::ChatInput => ChatFocus::Table,
            };
            return true;
        }

        if self.chat.focus != ChatFocus::ChatInput {
            return false;
        }

        match key_code {
            KeyCode::Enter => {
                if self.chat.is_generating {
                    if !self.chat.input_buffer.is_empty() {
                        self.chat.pending_message = Some(self.chat.input_buffer.clone());
                        self.chat.input_buffer.clear();
                    }
                } else {
                    self.send_chat_message();
                }
            }
            KeyCode::Backspace => {
                self.chat.input_buffer.pop();
            }
            KeyCode::Up => {
                if self.chat.show_suggestions && !self.chat.suggestions.is_empty() {
                    if self.chat.selected_suggestion > 0 {
                        self.chat.selected_suggestion -= 1;
                    }
                } else {
                    if self.chat.auto_scroll {
                        self.chat.scroll_offset = usize::MAX;
                    }
                    self.chat.auto_scroll = false;
                    self.chat.scroll_offset = self.chat.scroll_offset.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if self.chat.show_suggestions && !self.chat.suggestions.is_empty() {
                    let max_idx = self.chat.suggestions.len();
                    if self.chat.selected_suggestion < max_idx {
                        self.chat.selected_suggestion += 1;
                    }
                } else if !self.chat.auto_scroll {
                    self.chat.scroll_offset += 1;
                    self.chat.auto_scroll = true;
                }
            }
            KeyCode::Char(c) => {
                self.chat.input_buffer.push(c);
                self.chat.show_suggestions = false;
            }
            KeyCode::Esc => {
                if !self.chat.input_buffer.is_empty() {
                    self.chat.input_buffer.clear();
                } else {
                    self.chat.focus = ChatFocus::Table;
                }
            }
            _ => {}
        }
        true
    }

    // ── Mouse Handling ────────────────────────────────────────────────────

    /// Map a mouse position to a (content_line, char_col) in the chat content.
    fn mouse_to_content_pos(&self, mouse_col: u16, mouse_row: u16) -> Option<(usize, usize)> {
        let rect = self.chat.panel_rect;
        let click_row = (mouse_row - rect.y).saturating_sub(1) as usize; // subtract top border
        let char_col = (mouse_col - rect.x).saturating_sub(1) as usize; // subtract left border

        let inner_height = rect.height.saturating_sub(2) as usize;
        let has_conv = !self.chat.messages.is_empty() || self.chat.is_generating;
        let reserved = if has_conv { 2 } else { 1 };
        let chat_height = inner_height.saturating_sub(reserved);

        let scroll_y = if self.chat.auto_scroll || self.chat.total_content_lines <= chat_height {
            self.chat.total_content_lines.saturating_sub(chat_height)
        } else {
            let max_scroll = self.chat.total_content_lines.saturating_sub(chat_height);
            self.chat.scroll_offset.min(max_scroll)
        };

        let lines_to_show = self.chat.total_content_lines.saturating_sub(scroll_y);
        let pad = chat_height.saturating_sub(lines_to_show.min(chat_height));
        if click_row < pad {
            return None; // clicked on padding
        }
        let content_line = scroll_y + (click_row - pad);
        if content_line >= self.chat.total_content_lines {
            return None;
        }
        Some((content_line, char_col))
    }

    fn handle_mouse_event(&mut self, mouse: crossterm::event::MouseEvent) {
        let rect = self.chat.panel_rect;
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        // Only handle if mouse is within the chat panel
        if mouse.column < rect.x || mouse.column >= rect.x + rect.width
            || mouse.row < rect.y || mouse.row >= rect.y + rect.height
        {
            return;
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if self.chat.auto_scroll {
                    self.chat.scroll_offset = self.chat.total_content_lines;
                }
                self.chat.auto_scroll = false;
                self.chat.scroll_offset = self.chat.scroll_offset.saturating_sub(3);
            }
            MouseEventKind::ScrollDown => {
                if !self.chat.auto_scroll {
                    self.chat.scroll_offset += 3;
                    let inner_height = rect.height.saturating_sub(2) as usize;
                    let has_conv = !self.chat.messages.is_empty() || self.chat.is_generating;
                    let reserved = if has_conv { 2 } else { 1 };
                    let chat_height = inner_height.saturating_sub(reserved);
                    if self.chat.scroll_offset + chat_height >= self.chat.total_content_lines {
                        self.chat.auto_scroll = true;
                    }
                }
            }
            MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                // Start text selection
                if let Some(pos) = self.mouse_to_content_pos(mouse.column, mouse.row) {
                    self.chat.selection_anchor = Some(pos);
                    self.chat.selection_end = None; // reset until drag
                    self.chat.focus = ChatFocus::ChatInput;
                }
            }
            MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
                // Extend selection + auto-scroll at edges
                if self.chat.selection_anchor.is_some() {
                    let edge_zone = 2u16; // rows from edge that trigger auto-scroll
                    let top_edge = rect.y + 1; // inside top border
                    let inner_height = rect.height.saturating_sub(2) as usize;
                    let has_conv = !self.chat.messages.is_empty() || self.chat.is_generating;
                    let reserved = if has_conv { 2 } else { 1 };
                    let chat_height = inner_height.saturating_sub(reserved);
                    let bottom_edge = rect.y + 1 + chat_height as u16;

                    if mouse.row < top_edge + edge_zone && mouse.row >= rect.y {
                        // Dragging near top — scroll up
                        if self.chat.auto_scroll {
                            self.chat.scroll_offset = self.chat.total_content_lines;
                        }
                        self.chat.auto_scroll = false;
                        self.chat.scroll_offset = self.chat.scroll_offset.saturating_sub(2);
                    } else if mouse.row >= bottom_edge.saturating_sub(edge_zone) && mouse.row < rect.y + rect.height {
                        // Dragging near bottom — scroll down
                        if !self.chat.auto_scroll {
                            self.chat.scroll_offset += 2;
                            if self.chat.scroll_offset + chat_height >= self.chat.total_content_lines {
                                self.chat.auto_scroll = true;
                            }
                        }
                    }

                    if let Some(pos) = self.mouse_to_content_pos(mouse.column, mouse.row) {
                        self.chat.selection_end = Some(pos);
                    }
                }
            }
            MouseEventKind::Up(crossterm::event::MouseButton::Left) => {
                // Finalize selection and copy
                if let (Some(anchor), Some(end)) = (self.chat.selection_anchor, self.chat.selection_end) {
                    let selected = self.extract_selected_text(anchor, end);
                    if !selected.trim().is_empty() {
                        use arboard::Clipboard;
                        if let Ok(mut clipboard) = Clipboard::new() {
                            let _ = clipboard.set_text(&selected);
                            self.notification_message = Some(("Copied to clipboard".to_string(), 15));
                        }
                    }
                    // Keep selection visible (don't clear anchor/end) — cleared on next click
                } else {
                    // Single click (no drag) — clear any existing selection
                    self.chat.selection_anchor = None;
                    self.chat.selection_end = None;
                }
            }
            _ => {}
        }
    }

    /// Extract the plain text between two content positions.
    fn extract_selected_text(&self, anchor: (usize, usize), end: (usize, usize)) -> String {
        // Normalize: start is the earlier position
        let (start, end) = if anchor <= end { (anchor, end) } else { (end, anchor) };
        let (start_line, start_col) = start;
        let (end_line, end_col) = end;

        let texts = &self.chat.content_texts;
        if texts.is_empty() || start_line >= texts.len() {
            return String::new();
        }

        let mut result = String::new();
        for line_idx in start_line..=end_line.min(texts.len() - 1) {
            let line_text = &texts[line_idx];
            let chars: Vec<char> = line_text.chars().collect();

            let from = if line_idx == start_line { start_col.min(chars.len()) } else { 0 };
            let to = if line_idx == end_line { end_col.min(chars.len()) } else { chars.len() };

            if from < to {
                let slice: String = chars[from..to].iter().collect();
                result.push_str(&slice);
            }
            if line_idx < end_line {
                result.push('\n');
            }
        }

        result
    }

    // ── Markdown Parsing ─────────────────────────────────────────────────

    /// Parse simple markdown into styled spans: **bold**, *italic*, `code`.
    /// Returns owned spans to avoid lifetime issues in closures.
    /// Strip leading `# ` / `## ` etc. prefix spans emitted by tui_markdown for headings.
    fn strip_heading_prefix(mut line: Line<'static>) -> Line<'static> {
        if let Some(first) = line.spans.first() {
            let trimmed = first.content.trim_end();
            if !trimmed.is_empty() && trimmed.chars().all(|c| c == '#') {
                line.spans.remove(0);
            }
        }
        line
    }

    /// Wrap a styled Line to fit within max_width, preserving span styles.
    fn wrap_styled_line(line: Line<'static>, max_width: usize) -> Vec<Line<'static>> {
        let char_count: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
        if char_count <= max_width || max_width == 0 {
            return vec![line];
        }

        let styled_chars: Vec<(char, Style)> = line
            .spans
            .iter()
            .flat_map(|span| span.content.chars().map(move |c| (c, span.style)))
            .collect();

        let mut result = Vec::new();
        let mut pos = 0;

        while pos < styled_chars.len() {
            let end = (pos + max_width).min(styled_chars.len());
            let actual_end = if end >= styled_chars.len() {
                end
            } else {
                styled_chars[pos..end]
                    .iter()
                    .rposition(|(c, _)| *c == ' ')
                    .map(|p| pos + p + 1)
                    .unwrap_or(end)
            };

            let chunk = &styled_chars[pos..actual_end];
            let mut spans: Vec<Span<'static>> = Vec::new();
            if let Some(&(first_c, first_style)) = chunk.first() {
                let mut current_text = String::new();
                current_text.push(first_c);
                let mut current_style = first_style;
                for &(c, style) in &chunk[1..] {
                    if style != current_style {
                        spans.push(Span::styled(current_text, current_style));
                        current_text = String::new();
                        current_style = style;
                    }
                    current_text.push(c);
                }
                spans.push(Span::styled(current_text, current_style));
            }
            result.push(Line::from(spans));

            pos = actual_end;
            while pos < styled_chars.len() && styled_chars[pos].0 == ' ' {
                pos += 1;
            }
        }

        result
    }

    /// Apply a highlight background to a portion of a Line (for text selection).
    fn apply_selection_highlight(line: Line<'static>, sel_start: usize, sel_end: usize) -> Line<'static> {
        if sel_start >= sel_end {
            return line;
        }
        let highlight_bg = Color::Indexed(237); // subtle dark gray background
        let mut result_spans: Vec<Span<'static>> = Vec::new();
        let mut col: usize = 0;

        for span in line.spans {
            let span_char_count = span.content.chars().count();
            let span_start = col;
            let span_end = col + span_char_count;

            if span_end <= sel_start || span_start >= sel_end {
                // Entirely outside selection
                result_spans.push(span);
            } else {
                let chars: Vec<char> = span.content.chars().collect();

                // Before selection
                let hl_start = sel_start.saturating_sub(span_start);
                if hl_start > 0 {
                    let before: String = chars[..hl_start].iter().collect();
                    result_spans.push(Span::styled(before, span.style));
                }

                // Selected portion
                let hl_end = (sel_end - span_start).min(chars.len());
                let selected: String = chars[hl_start..hl_end].iter().collect();
                result_spans.push(Span::styled(selected, span.style.bg(highlight_bg)));

                // After selection
                if hl_end < chars.len() {
                    let after: String = chars[hl_end..].iter().collect();
                    result_spans.push(Span::styled(after, span.style));
                }
            }

            col = span_end;
        }

        Line::from(result_spans)
    }

    // ── Chat Rendering ──────────────────────────────────────────────────────

    fn render_chat_panel(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        self.chat.panel_rect = area;
        if area.height < 3 {
            return;
        }

        let is_focused = self.chat.focus == ChatFocus::ChatInput;
        let border_color = if is_focused { Color::Cyan } else { Color::DarkGray };

        let title = match &self.chat.context {
            ChatContext::Global => "Ask Scriba",
            ChatContext::Recording { .. } => "Ask about this recording",
        };

        // Content height inside the bordered block (minus top/bottom border)
        let inner_height = area.height.saturating_sub(2) as usize;
        let has_conversation = !self.chat.messages.is_empty() || self.chat.is_generating;
        // Reserve lines at bottom: 1 for input + 1 for separator when conversation active
        let reserved = if has_conversation { 2 } else { 1 };
        let chat_height = inner_height.saturating_sub(reserved);
        let content_width = area.width.saturating_sub(4) as usize; // borders + padding

        let mut final_lines: Vec<Line> = Vec::with_capacity(inner_height);

        // Suggestions mode: no messages yet
        let show_suggestions = self.chat.show_suggestions
            && !self.chat.suggestions.is_empty()
            && self.chat.messages.is_empty()
            && self.chat.streaming_content.is_empty()
            && !self.chat.is_generating;

        // Compute selection range (normalized: start <= end)
        let selection = match (self.chat.selection_anchor, self.chat.selection_end) {
            (Some(a), Some(e)) => {
                let (start, end) = if a <= e { (a, e) } else { (e, a) };
                Some((start, end))
            }
            _ => None,
        };

        if show_suggestions {
            let mut all_lines: Vec<Line> = Vec::new();
            let mut content_texts: Vec<String> = Vec::new();
            let total_options = self.chat.suggestions.len() + 1;
            for (i, s) in self.chat.suggestions.iter().enumerate() {
                let text = if i == self.chat.selected_suggestion {
                    format!("  > {}", s)
                } else {
                    format!("    {}", s)
                };
                let style = if i == self.chat.selected_suggestion {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                content_texts.push(text.clone());
                all_lines.push(Line::from(Span::styled(text, style)));
            }
            let free_form_idx = total_options - 1;
            let text = if self.chat.selected_suggestion == free_form_idx {
                "  > Ask Scriba anything..."
            } else {
                "    Ask Scriba anything..."
            };
            let style = if self.chat.selected_suggestion == free_form_idx {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            content_texts.push(text.to_string());
            all_lines.push(Line::from(Span::styled(text.to_string(), style)));

            self.chat.content_texts = content_texts;
            let total_content = all_lines.len();
            self.chat.total_content_lines = total_content;
            let scroll_y: u16 = if self.chat.auto_scroll || total_content <= chat_height {
                total_content.saturating_sub(chat_height) as u16
            } else {
                let max_scroll = total_content.saturating_sub(chat_height);
                self.chat.scroll_offset.min(max_scroll) as u16
            };
            let lines_to_show = total_content.saturating_sub(scroll_y as usize);
            let pad = chat_height.saturating_sub(lines_to_show.min(chat_height));
            for _ in 0..pad {
                final_lines.push(Line::from(""));
            }
            for (vis_idx, line) in all_lines.into_iter().skip(scroll_y as usize).take(chat_height).enumerate() {
                let content_idx = scroll_y as usize + vis_idx;
                if let Some(((sel_start_line, sel_start_col), (sel_end_line, sel_end_col))) = selection {
                    if content_idx >= sel_start_line && content_idx <= sel_end_line {
                        let line_start = if content_idx == sel_start_line { sel_start_col } else { 0 };
                        let line_text_len = self.chat.content_texts.get(content_idx)
                            .map(|t| t.chars().count()).unwrap_or(0);
                        let line_end = if content_idx == sel_end_line { sel_end_col } else { line_text_len };
                        final_lines.push(Self::apply_selection_highlight(line, line_start, line_end));
                        continue;
                    }
                }
                final_lines.push(line);
            }
        } else {
            // ── Phase 1: Completed messages (cached) ───────────────────────
            let msg_count = self.chat.messages.len();
            let cache_valid = msg_count == self.chat.cached_msg_count
                && content_width == self.chat.cached_width;

            if !cache_valid {
                let mut cached_lines: Vec<Line<'static>> = Vec::new();
                let mut cached_texts: Vec<String> = Vec::new();
                let wrap_width = content_width.saturating_sub(2);

                for msg in &self.chat.messages {
                    match msg.role {
                        ChatRole::User => {
                            let header = "You:".to_string();
                            cached_texts.push(header.clone());
                            cached_lines.push(Line::from(Span::styled(
                                header,
                                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                            )));
                            let style = Style::default().fg(Color::Cyan);
                            for line in msg.content.lines() {
                                let wrapped = textwrap::wrap(line, wrap_width);
                                if wrapped.is_empty() {
                                    cached_texts.push("  ".to_string());
                                    cached_lines.push(Line::from(Span::styled("  ".to_string(), style)));
                                } else {
                                    for w in &wrapped {
                                        let full = format!("  {}", w);
                                        cached_texts.push(full.clone());
                                        cached_lines.push(Line::from(Span::styled(full, style)));
                                    }
                                }
                            }
                            cached_texts.push(String::new());
                            cached_lines.push(Line::from(""));
                        }
                        ChatRole::Assistant => {
                            let header = "(o,o):".to_string();
                            cached_texts.push(header.clone());
                            cached_lines.push(Line::from(Span::styled(
                                header,
                                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                            )));
                            let md_text = tui_markdown::from_str(&msg.content);
                            for md_line in md_text.lines {
                                let owned_line = Line::from(
                                    md_line
                                        .spans
                                        .into_iter()
                                        .map(|span| Span::styled(span.content.to_string(), span.style))
                                        .collect::<Vec<_>>(),
                                );
                                let owned_line = Self::strip_heading_prefix(owned_line);
                                let wrapped_lines = Self::wrap_styled_line(owned_line, wrap_width);
                                for wl in wrapped_lines {
                                    let plain: String =
                                        wl.spans.iter().map(|s| s.content.as_ref()).collect();
                                    cached_texts.push(format!("  {}", plain));
                                    let mut indented = vec![Span::raw("  ".to_string())];
                                    indented.extend(wl.spans);
                                    cached_lines.push(Line::from(indented));
                                }
                            }
                            cached_texts.push(String::new());
                            cached_lines.push(Line::from(""));
                        }
                        ChatRole::System => {
                            let style = Style::default().fg(Color::Red).add_modifier(Modifier::ITALIC);
                            for line in msg.content.lines() {
                                let wrapped = textwrap::wrap(line, content_width);
                                if wrapped.is_empty() {
                                    cached_texts.push(String::new());
                                    cached_lines.push(Line::from(Span::styled(String::new(), style)));
                                } else {
                                    for w in &wrapped {
                                        let full = w.to_string();
                                        cached_texts.push(full.clone());
                                        cached_lines.push(Line::from(Span::styled(full, style)));
                                    }
                                }
                            }
                            cached_texts.push(String::new());
                            cached_lines.push(Line::from(""));
                        }
                    }
                }

                self.chat.cached_msg_lines = cached_lines;
                self.chat.cached_msg_texts = cached_texts;
                self.chat.cached_msg_count = msg_count;
                self.chat.cached_width = content_width;
            }

            // ── Phase 2: Dynamic content (always rebuilt) ──────────────────
            let mut dynamic_lines: Vec<Line<'static>> = Vec::new();
            let mut dynamic_texts: Vec<String> = Vec::new();

            if !self.chat.streaming_content.is_empty() {
                let header = "(o,o):".to_string();
                dynamic_texts.push(header.clone());
                dynamic_lines.push(Line::from(Span::styled(
                    header,
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )));
                let wrap_width = content_width.saturating_sub(2);
                let md_text = tui_markdown::from_str(&self.chat.streaming_content);
                for md_line in md_text.lines {
                    let owned_line = Line::from(
                        md_line
                            .spans
                            .into_iter()
                            .map(|span| Span::styled(span.content.to_string(), span.style))
                            .collect::<Vec<_>>(),
                    );
                    let owned_line = Self::strip_heading_prefix(owned_line);
                    let wrapped_lines = Self::wrap_styled_line(owned_line, wrap_width);
                    for wl in wrapped_lines {
                        let plain: String =
                            wl.spans.iter().map(|s| s.content.as_ref()).collect();
                        dynamic_texts.push(format!("  {}", plain));
                        let mut indented = vec![Span::raw("  ".to_string())];
                        indented.extend(wl.spans);
                        dynamic_lines.push(Line::from(indented));
                    }
                }
            }

            if let Some(status) = &self.chat.current_status {
                let spinners = ['◐', '◑', '◒', '◓'];
                let spinner = spinners[self.chat.spinner_frame % spinners.len()];
                let text = format!(" {} {}", spinner, status);
                dynamic_texts.push(text);
                dynamic_lines.push(Line::from(vec![
                    Span::styled(
                        format!(" {} ", spinner),
                        Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(status.clone(), Style::default().fg(Color::Yellow)),
                ]));
            }

            // ── Phase 3: Assemble content_texts, compute total ─────────────
            let cached_len = self.chat.cached_msg_lines.len();
            let total_content = cached_len + dynamic_lines.len();
            self.chat.total_content_lines = total_content;

            let mut content_texts = self.chat.cached_msg_texts.clone();
            content_texts.extend(dynamic_texts);
            self.chat.content_texts = content_texts;

            // ── Scroll calculation ─────────────────────────────────────────
            let scroll_y: u16 = if self.chat.auto_scroll || total_content <= chat_height {
                total_content.saturating_sub(chat_height) as u16
            } else {
                let max_scroll = total_content.saturating_sub(chat_height);
                self.chat.scroll_offset.min(max_scroll) as u16
            };
            let lines_to_show = total_content.saturating_sub(scroll_y as usize);
            let pad = chat_height.saturating_sub(lines_to_show.min(chat_height));
            for _ in 0..pad {
                final_lines.push(Line::from(""));
            }

            // ── Phase 4: Build visible window (only visible lines cloned) ──
            let start = scroll_y as usize;
            let end = (start + chat_height).min(total_content);
            for i in start..end {
                let line = if i < cached_len {
                    self.chat.cached_msg_lines[i].clone()
                } else {
                    dynamic_lines[i - cached_len].clone()
                };
                let content_idx = i;
                if let Some(((sel_start_line, sel_start_col), (sel_end_line, sel_end_col))) = selection {
                    if content_idx >= sel_start_line && content_idx <= sel_end_line {
                        let line_start = if content_idx == sel_start_line { sel_start_col } else { 0 };
                        let line_text_len = self.chat.content_texts.get(content_idx)
                            .map(|t| t.chars().count()).unwrap_or(0);
                        let line_end = if content_idx == sel_end_line { sel_end_col } else { line_text_len };
                        final_lines.push(Self::apply_selection_highlight(line, line_start, line_end));
                        continue;
                    }
                }
                final_lines.push(line);
            }
        }

        // ── Separator + Input line (always last lines inside the block) ────
        // Visual breathing room between chat content and input
        if has_conversation {
            let sep_width = content_width.min(area.width.saturating_sub(4) as usize);
            let sep = "─".repeat(sep_width);
            final_lines.push(Line::from(Span::styled(
                format!("  {}", sep),
                Style::default().fg(Color::DarkGray),
            )));
        }

        let cursor = if is_focused { "▎" } else { "" };
        let has_pending = self.chat.pending_message.is_some();
        let input_line = if has_pending && self.chat.input_buffer.is_empty() {
            // Message queued while generating
            let queued_msg = self.chat.pending_message.as_deref().unwrap_or("");
            Line::from(vec![
                Span::styled("  ▸ ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("{} ", queued_msg),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled("(queued)", Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
            ])
        } else if !self.chat.input_buffer.is_empty() {
            Line::from(vec![
                Span::styled("  ▸ ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("{}{}", self.chat.input_buffer, cursor),
                    Style::default().fg(Color::White),
                ),
            ])
        } else {
            let prompt_color = if is_focused { Color::Cyan } else { Color::DarkGray };
            Line::from(Span::styled(
                format!("  ▸ {}", cursor),
                Style::default().fg(prompt_color),
            ))
        };
        final_lines.push(input_line);

        let para = Paragraph::new(final_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color))
                    .title(title)
                    .title_style(Style::default().fg(if is_focused { Color::Cyan } else { Color::DarkGray })),
            );
        f.render_widget(para, area);

        // ── Scroll position indicator (right border) ────────────────────────
        let total_content = self.chat.total_content_lines;
        if total_content > chat_height && chat_height > 0 {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_symbol("█")
                .track_symbol(Some("│"))
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::default().fg(Color::Cyan))
                .track_style(Style::default().fg(Color::Indexed(237)));
            let scroll_y = if self.chat.auto_scroll || total_content <= chat_height {
                total_content.saturating_sub(chat_height)
            } else {
                let max_scroll = total_content.saturating_sub(chat_height);
                self.chat.scroll_offset.min(max_scroll)
            };
            let mut scrollbar_state = ScrollbarState::new(total_content)
                .position(scroll_y);
            // Render inside the block border area (shrink by 1 on each side)
            let scrollbar_area = ratatui::layout::Rect {
                x: area.x,
                y: area.y + 1,
                width: area.width,
                height: area.height.saturating_sub(2),
            };
            f.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
        }
    }

    fn render_compressed_footer(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        // Show notification if present (auto-dismissing)
        if let Some((ref msg, _)) = self.notification_message {
            let is_error = msg.contains("failed") || msg.contains("Failed");
            let style = if is_error {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            };
            let para = Paragraph::new(msg.as_str())
                .style(style)
                .alignment(Alignment::Center);
            f.render_widget(para, area);
            return;
        }

        let voice_status = if self.voice_mode_active { " | Voice" } else { "" };
        let transcribing = if self.transcribing_recording_name.is_some() { " | Transcribing..." } else { "" };
        let controls = format!(
            "TAB: Focus | ↑↓: Nav | /: Search | R: Record | A: Add | W: World | S: Settings | H: Help | Q: Quit{}{}",
            transcribing, voice_status
        );
        let para = Paragraph::new(controls)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        f.render_widget(para, area);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Onboarding: Scriba the Owl
    // ─────────────────────────────────────────────────────────────────────────

    async fn handle_onboarding_keys(&mut self, key_code: KeyCode) -> Result<DashboardAction> {
        let ob = match self.onboarding.as_mut() {
            Some(ob) => ob,
            None => return Ok(DashboardAction::Continue),
        };

        // Esc at any step → skip onboarding
        if matches!(key_code, KeyCode::Esc) {
            self.onboarding = None;
            self.current_view = DashboardView::Main;
            return Ok(DashboardAction::Continue);
        }

        match ob.step {
            OnboardingStep::Entrance => {
                // No key handling during entrance animation
            }
            OnboardingStep::Intro => {
                if !ob.text_complete {
                    ob.complete_text();
                } else if matches!(key_code, KeyCode::Enter) {
                    ob.step = OnboardingStep::ModeSelection;
                    ob.anim_frame = 0;
                    ob.set_step_text(
                        "First, how should I think?\n\n\
                         I can use a cloud AI (smarter, needs API key)\n\
                         or run locally on your machine (private, needs Ollama).\n\n\
                         [1] Cloud Provider (recommended)\n\
                         [2] Privacy Mode (local Ollama)"
                    );
                }
            }
            OnboardingStep::ModeSelection => {
                if !ob.text_complete {
                    ob.complete_text();
                } else {
                    match key_code {
                        KeyCode::Char('1') => {
                            ob.selected_mode = 0;
                            ob.step = OnboardingStep::ProviderSelection;
                            ob.anim_frame = 0;
                            ob.set_step_text(
                                "Great choice! Which cloud provider?\n\n\
                                 [1] Anthropic (Claude) -- Best for nuanced understanding\n\
                                 [2] OpenAI (GPT) -- Widely used, reliable\n\
                                 [3] Google (Gemini) -- Fast and cost-effective"
                            );
                        }
                        KeyCode::Char('2') => {
                            ob.selected_mode = 1;
                            // Set local mode in config and skip to AskName
                            self.config.enrichment.mode = EnrichmentMode::Local {
                                ollama_endpoint: "http://localhost:11434".to_string(),
                                ollama_model: "mistral:latest".to_string(),
                            };
                            let _ = self.config.save();
                            ob.step = OnboardingStep::AskName;
                            ob.anim_frame = 0;
                            ob.set_step_text(
                                "Privacy mode it is! Make sure Ollama is running.\n\n\
                                 So, who am I working for?\n\nWhat's your name?"
                            );
                        }
                        _ => {}
                    }
                }
            }
            OnboardingStep::ProviderSelection => {
                if !ob.text_complete {
                    ob.complete_text();
                } else {
                    let provider = match key_code {
                        KeyCode::Char('1') => Some(CloudProvider::Anthropic),
                        KeyCode::Char('2') => Some(CloudProvider::OpenAI),
                        KeyCode::Char('3') => Some(CloudProvider::Google),
                        _ => None,
                    };
                    if let Some(p) = provider {
                        ob.selected_provider = match &p {
                            CloudProvider::Anthropic => 0,
                            CloudProvider::OpenAI => 1,
                            CloudProvider::Google => 2,
                        };
                        self.config.enrichment.mode = EnrichmentMode::Cloud {
                            provider: p.clone(),
                            api_key: String::new(),
                            model: None,
                        };
                        ob.step = OnboardingStep::ApiKeyEntry;
                        ob.anim_frame = 0;
                        ob.set_step_text(&format!(
                            "Excellent! Now I need your {} API key.\n\n\
                             Paste it below (it won't be shown):",
                            p.display_name()
                        ));
                    }
                }
            }
            OnboardingStep::ApiKeyEntry => {
                if !ob.text_complete {
                    ob.complete_text();
                } else {
                    match key_code {
                        KeyCode::Enter => {
                            if !ob.api_key_input.trim().is_empty() {
                                let key = ob.api_key_input.trim().to_string();
                                if let EnrichmentMode::Cloud { api_key, .. } = &mut self.config.enrichment.mode {
                                    *api_key = key.clone();
                                }
                                let _ = self.config.save();

                                // Start validation
                                ob.step = OnboardingStep::ApiKeyValidation;
                                ob.anim_frame = 0;
                                ob.api_key_valid = None;
                                ob.set_step_text("Testing your API key...");

                                let config = self.config.enrichment.clone();
                                ob.validation_task = Some(tokio::spawn(async move {
                                    let provider = crate::enrichment::create_provider(&config);
                                    match provider.health_check().await {
                                        Ok(()) => Ok(true),
                                        Err(_) => Ok(false),
                                    }
                                }));
                            }
                        }
                        KeyCode::Char(c) => {
                            ob.api_key_input.push(c);
                        }
                        KeyCode::Backspace => {
                            ob.api_key_input.pop();
                        }
                        _ => {}
                    }
                }
            }
            OnboardingStep::ApiKeyValidation => {
                // Validation result is resolved by the tick handler.
                // Here we only handle retry/skip choices after validation fails.
                if ob.api_key_valid == Some(false) {
                    // Handle retry/skip
                    match key_code {
                        KeyCode::Char('1') => {
                            ob.step = OnboardingStep::ApiKeyEntry;
                            ob.api_key_input.clear();
                            ob.anim_frame = 0;
                            let provider_name = self.config.enrichment.provider_display_name().to_string();
                            ob.set_step_text(&format!(
                                "Let's try again. Paste your {} API key:",
                                provider_name
                            ));
                        }
                        KeyCode::Char('2') => {
                            ob.step = OnboardingStep::AskName;
                            ob.anim_frame = 0;
                            ob.set_step_text(
                                "No worries! You can set the key later in Settings.\n\n\
                                 So, who am I working for?\n\nWhat's your name?"
                            );
                        }
                        _ => {}
                    }
                }
            }
            OnboardingStep::AskName => {
                if !ob.text_complete {
                    ob.complete_text();
                } else {
                    match key_code {
                        KeyCode::Enter => {
                            if !ob.user_name.trim().is_empty() {
                                ob.step = OnboardingStep::AskRole;
                                ob.anim_frame = 0;
                                let name = ob.user_name.clone();
                                ob.set_step_text(&format!(
                                    "{}! Great name. Love it.\n\n\
                                     Now tell me about yourself --\n\
                                     What do you do? What's your company?\n\
                                     Who do you work with?\n\n\
                                     Just write naturally, I'll figure it out.",
                                    name
                                ));
                            }
                        }
                        KeyCode::Char(c) => {
                            ob.user_name.push(c);
                        }
                        KeyCode::Backspace => {
                            ob.user_name.pop();
                        }
                        _ => {}
                    }
                }
            }
            OnboardingStep::AskRole => {
                if !ob.text_complete {
                    ob.complete_text();
                } else {
                    match key_code {
                        KeyCode::Enter => {
                            if !ob.user_role.trim().is_empty() {
                                ob.step = OnboardingStep::AskAliases;
                                ob.anim_frame = 0;
                                ob.set_step_text(
                                    "Noted! One last thing --\n\n\
                                     Any nicknames, company name variations, or\n\
                                     words that get mangled in transcripts?\n\n\
                                     (Press ENTER to skip)"
                                );
                            }
                        }
                        KeyCode::Char(c) => {
                            ob.user_role.push(c);
                        }
                        KeyCode::Backspace => {
                            ob.user_role.pop();
                        }
                        _ => {}
                    }
                }
            }
            OnboardingStep::AskAliases => {
                if !ob.text_complete {
                    ob.complete_text();
                } else {
                    match key_code {
                        KeyCode::Enter => {
                            // Enter with empty is OK (skip aliases)
                            ob.step = OnboardingStep::Processing;
                            ob.anim_frame = 0;
                            ob.set_step_text("Let me digest all of that...");
                            self.start_onboarding_processing();
                        }
                        KeyCode::Char(c) => {
                            ob.user_aliases.push(c);
                        }
                        KeyCode::Backspace => {
                            ob.user_aliases.pop();
                        }
                        _ => {}
                    }
                }
            }
            OnboardingStep::Processing => {
                // If Ollama failed and text is complete, Enter advances
                if ob.text_complete && ob.processing_task.is_none() && !ob.ollama_available {
                    if matches!(key_code, KeyCode::Enter) {
                        ob.step = OnboardingStep::Done;
                        ob.anim_frame = 0;
                        ob.set_step_text(
                            "Your world is ready!\n\n\
                             I'll remember all of this. Every recording\n\
                             you make, I'll enrich with what I know about\n\
                             you and your world.\n\n\
                             Time to fly!"
                        );
                    }
                }
            }
            OnboardingStep::Confirmation => {
                if !ob.text_complete {
                    ob.complete_text();
                } else {
                    match key_code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            ob.step = OnboardingStep::Done;
                            ob.anim_frame = 0;
                            ob.set_step_text(
                                "Your world is ready!\n\n\
                                 I'll remember all of this. Every recording\n\
                                 you make, I'll enrich with what I know about\n\
                                 you and your world.\n\n\
                                 Time to fly!"
                            );
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') => {
                            // Go back to AskName with values preserved
                            ob.step = OnboardingStep::AskName;
                            ob.anim_frame = 0;
                            ob.set_step_text(
                                "So, who am I working for?\n\nWhat's your name?"
                            );
                            ob.complete_text(); // Show instantly
                            // Delete the world.md that was created during processing
                            let _ = std::fs::remove_file(WorldContext::file_path());
                        }
                        _ => {}
                    }
                }
            }
            OnboardingStep::Done => {
                // Keys can speed up transition
                if !ob.text_complete {
                    ob.complete_text();
                }
            }
        }

        Ok(DashboardAction::Continue)
    }

    fn start_onboarding_processing(&mut self) {
        let ob = match self.onboarding.as_mut() {
            Some(ob) => ob,
            None => return,
        };

        // Build seed content from user inputs
        let mut seed = format!("My name is {}. ", ob.user_name.trim());
        seed.push_str(ob.user_role.trim());
        if !ob.user_aliases.trim().is_empty() {
            seed.push_str(&format!(
                "\nCommon misspellings or aliases: {}",
                ob.user_aliases.trim()
            ));
        }

        let config = self.config.clone();

        ob.processing_task = Some(tokio::spawn(async move {
            let mut db = Database::new()?;
            let result = initialize_world_from_seed(&mut db, &config, &seed).await?;
            if result.is_some() {
                Ok((result, None))
            } else {
                // Provider unavailable — diagnose why
                let hint = if config.enrichment.is_local() {
                    let endpoint = config.enrichment.ollama_endpoint();
                    let model = config.enrichment.ollama_model();
                    let client = OllamaClient::new(&endpoint, &model);
                    client.diagnose().await.hint()
                } else {
                    let provider_name = config.enrichment.provider_display_name().to_string();
                    if config.enrichment.resolve_api_key().is_none() {
                        Some(format!(
                            "No API key set for {}.\n\nSet it with:\n  scriba config set-enrichment-key <your-key>",
                            provider_name
                        ))
                    } else {
                        Some(format!(
                            "{} is not reachable.\n\nCheck your API key and network connection.",
                            provider_name
                        ))
                    }
                };
                Ok((None, hint))
            }
        }));
    }

    fn render_onboarding(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let ob = match &self.onboarding {
            Some(ob) => ob,
            None => return,
        };

        // Magic transition overlay
        if ob.transitioning {
            self.render_magic_transition(f, area, ob.transition_frame);
            return;
        }

        // Full-screen onboarding box
        let outer_block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Double)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " SCRIBA ",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ))
            .title_alignment(Alignment::Center);
        f.render_widget(outer_block, area);

        // Inner area (inside border)
        let inner = ratatui::layout::Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(4),
            height: area.height.saturating_sub(2),
        };

        // Layout: Owl + Speech Bubble + Input + Step Dots + Footer
        let content_width = 56u16.min(inner.width);
        let h_offset = (inner.width.saturating_sub(content_width)) / 2;
        let centered = ratatui::layout::Rect {
            x: inner.x + h_offset,
            y: inner.y,
            width: content_width,
            height: inner.height,
        };

        match ob.step {
            OnboardingStep::Entrance => {
                self.render_owl_entrance(f, centered, ob);
            }
            _ => {
                // Vertical layout within centered area
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(4),  // Owl
                        Constraint::Length(1),  // Spacer
                        Constraint::Min(6),     // Speech bubble
                        Constraint::Length(3),  // Footer
                    ])
                    .split(centered);

                // Render owl
                self.render_owl(f, chunks[0], ob);

                // Render speech bubble + input
                self.render_speech_bubble(f, chunks[2], ob);

                // Render step dots + footer hint
                self.render_onboarding_footer(f, chunks[3], ob);
            }
        }
    }

    fn render_owl_entrance(&self, f: &mut Frame, area: ratatui::layout::Rect, ob: &OnboardingState) {
        // 40-frame entrance (~4s): sparkle trail → owl flies in → lands with flourish
        let frame = ob.anim_frame.min(39);
        let total_width = area.width as usize;
        let center = total_width / 2;
        let owl_y = area.y + area.height / 2;

        // Phase 1 (frames 0-10): Sparkles gather on the right side, hinting something is coming
        if frame <= 10 {
            let density = (frame as f64 + 1.0) / 12.0;
            let seed = frame * 4219;
            let mut lines: Vec<Line> = Vec::new();
            for y in 0..area.height {
                let mut spans: Vec<Span> = Vec::new();
                for x in 0..area.width {
                    let hash = ((x as usize).wrapping_mul(37).wrapping_add(y as usize).wrapping_mul(13).wrapping_add(seed)) % 100;
                    // Only sparkle in the right third of the screen
                    let in_right_zone = (x as usize) > (total_width * 2 / 3);
                    let threshold = (density * 100.0) as usize;
                    if in_right_zone && hash < threshold {
                        let ch = SPARKLE_CHARS[hash % 4]; // skip space
                        let color = match hash % 3 {
                            0 => Color::Yellow,
                            1 => Color::Cyan,
                            _ => Color::White,
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    } else {
                        spans.push(Span::raw(" "));
                    }
                }
                lines.push(Line::from(spans));
            }
            let p = Paragraph::new(lines);
            f.render_widget(p, area);
            return;
        }

        // Phase 2 (frames 11-32): Owl flies across with sparkle trail
        if frame <= 32 {
            let fly_progress = (frame - 11) as f64 / 21.0;
            // Ease-out cubic for graceful deceleration
            let eased = 1.0 - (1.0 - fly_progress).powi(3);
            let start = total_width.saturating_sub(4);
            let x_pos = start - ((start - center) as f64 * eased) as usize;
            let x_pos = x_pos.min(total_width.saturating_sub(7));

            // Vertical bob: slight sine wave during flight
            let bob = ((fly_progress * std::f64::consts::PI * 3.0).sin() * 1.5) as i16;
            let y_pos = (owl_y as i16 + bob).max(area.y as i16) as u16;

            // Draw sparkle trail behind the owl
            let trail_len = 8usize;
            let seed = frame * 2311;
            for t in 1..=trail_len {
                let trail_x = x_pos + t * 2;
                if trail_x < total_width {
                    let age = t as f64 / trail_len as f64;
                    let ch = if age < 0.3 { '*' } else if age < 0.6 { '+' } else { '.' };
                    let color = if age < 0.4 { Color::Yellow } else { Color::DarkGray };
                    // Slight vertical scatter for trail
                    let scatter = ((seed + t) % 3) as i16 - 1;
                    let ty = (y_pos as i16 + scatter).max(area.y as i16).min((area.y + area.height - 1) as i16) as u16;
                    if trail_x < area.width as usize {
                        let trail_area = ratatui::layout::Rect {
                            x: area.x + trail_x as u16,
                            y: ty,
                            width: 1,
                            height: 1,
                        };
                        let p = Paragraph::new(ch.to_string())
                            .style(Style::default().fg(color));
                        f.render_widget(p, trail_area);
                    }
                }
            }

            // Draw the owl
            let owl_text = if fly_progress < 0.5 {
                OWL_FLYING
            } else if fly_progress < 0.85 {
                OWL_APPROACH
            } else {
                OWL_APPROACH
            };

            let owl_area = ratatui::layout::Rect {
                x: area.x + x_pos as u16,
                y: y_pos,
                width: (owl_text.len() as u16).min(area.width.saturating_sub(x_pos as u16)),
                height: 1,
            };
            let p = Paragraph::new(owl_text)
                .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
            f.render_widget(p, owl_area);
            return;
        }

        // Phase 3 (frames 33-39): Landed — owl settles with a little sparkle burst
        let settle_frame = frame - 33;

        // Sparkle burst around the landing spot (fades over frames)
        if settle_frame < 5 {
            let burst_density = (5 - settle_frame) as f64 / 8.0;
            let seed = frame * 1571;
            let burst_radius = 6u16;
            let cx = (area.x + center as u16).saturating_sub(1);
            let cy = owl_y;
            for dy in 0..burst_radius {
                for dx in 0..(burst_radius * 2) {
                    let bx = cx.saturating_sub(burst_radius) + dx;
                    let by = cy.saturating_sub(burst_radius / 2) + dy;
                    if bx >= area.x && bx < area.x + area.width && by >= area.y && by < area.y + area.height {
                        let hash = ((bx as usize).wrapping_mul(29).wrapping_add(by as usize).wrapping_mul(19).wrapping_add(seed)) % 100;
                        let threshold = (burst_density * 100.0) as usize;
                        if hash < threshold {
                            let ch = SPARKLE_CHARS[hash % 4];
                            let color = match hash % 3 { 0 => Color::Yellow, 1 => Color::Cyan, _ => Color::White };
                            let spark_area = ratatui::layout::Rect { x: bx, y: by, width: 1, height: 1 };
                            let p = Paragraph::new(ch.to_string()).style(Style::default().fg(color));
                            f.render_widget(p, spark_area);
                        }
                    }
                }
            }
        }

        // The landed owl
        let owl_area = ratatui::layout::Rect {
            x: area.x + center.saturating_sub(4) as u16,
            y: owl_y.saturating_sub(1),
            width: 12.min(area.width),
            height: 3,
        };
        let p = Paragraph::new(OWL_LANDED)
            .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
        f.render_widget(p, owl_area);
    }

    fn render_owl(&self, f: &mut Frame, area: ratatui::layout::Rect, ob: &OnboardingState) {
        let owl_text = match ob.step {
            OnboardingStep::ApiKeyValidation if ob.validation_task.is_some() => {
                // Thinking animation while validating API key
                OWL_THINKING[(ob.anim_frame / 3) % 8]
            }
            OnboardingStep::Processing if ob.processing_task.is_some() => {
                // Slow: one frame change every ~300ms
                OWL_THINKING[(ob.anim_frame / 3) % 8]
            }
            OnboardingStep::Done if !ob.transitioning => {
                // Slow celebration: one frame change every ~250ms
                OWL_CELEBRATE[(ob.anim_frame / 2) % 4]
            }
            _ => {
                // Idle with blink every ~30 frames
                if ob.anim_frame % 30 < 2 {
                    OWL_IDLE[1] // blink
                } else {
                    OWL_IDLE[0] // normal
                }
            }
        };

        // Center the owl
        let owl_lines: Vec<&str> = owl_text.split('\n').collect();
        let owl_width = owl_lines.iter().map(|l| l.len()).max().unwrap_or(0) as u16;
        let x_offset = (area.width.saturating_sub(owl_width)) / 2;

        let owl_area = ratatui::layout::Rect {
            x: area.x + x_offset,
            y: area.y,
            width: owl_width + 4,
            height: (owl_lines.len() as u16).min(area.height),
        };

        let p = Paragraph::new(owl_text)
            .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
        f.render_widget(p, owl_area);
    }

    fn render_speech_bubble(&self, f: &mut Frame, area: ratatui::layout::Rect, ob: &OnboardingState) {
        let visible = ob.visible_text();

        // Build content lines
        let mut lines: Vec<Line> = visible
            .split('\n')
            .map(|l| Line::from(Span::styled(l, Style::default().fg(Color::Green))))
            .collect();

        // Add input field for input steps
        let show_input = ob.text_complete && matches!(
            ob.step,
            OnboardingStep::ApiKeyEntry | OnboardingStep::AskName | OnboardingStep::AskRole | OnboardingStep::AskAliases
        );

        // Add confirmation choices
        let show_choices = ob.text_complete && ob.step == OnboardingStep::Confirmation;

        if show_choices {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  [Y] Hoo yes!  ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled("  [N] Let me fix that", Style::default().fg(Color::Yellow)),
            ]));
        }

        // Determine how much height for speech vs input
        let speech_height = if show_input {
            area.height.saturating_sub(4) // Leave room for input box
        } else {
            area.height
        };

        // Speech bubble
        let speech_area = ratatui::layout::Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: speech_height,
        };

        let bubble = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(Span::styled(
                        " SCRIBA ",
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    )),
            )
            .wrap(Wrap { trim: false });
        f.render_widget(bubble, speech_area);

        // Input field
        if show_input {
            let input_value = match ob.step {
                OnboardingStep::ApiKeyEntry => &ob.api_key_input,
                OnboardingStep::AskName => &ob.user_name,
                OnboardingStep::AskRole => &ob.user_role,
                OnboardingStep::AskAliases => &ob.user_aliases,
                _ => &ob.user_name,
            };

            let input_area = ratatui::layout::Rect {
                x: area.x,
                y: area.y + speech_height,
                width: area.width,
                height: 3,
            };

            // Mask API key input: show first 4 chars + dots
            let display_value = if ob.step == OnboardingStep::ApiKeyEntry && !input_value.is_empty() {
                let visible = input_value.chars().take(4).collect::<String>();
                let hidden = "*".repeat(input_value.len().saturating_sub(4));
                format!("{}{}", visible, hidden)
            } else {
                input_value.to_string()
            };

            let input_title = if ob.step == OnboardingStep::ApiKeyEntry {
                " API Key "
            } else {
                " Your answer "
            };

            let input_text = format!("{}_", display_value);
            let input = Paragraph::new(input_text)
                .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Yellow))
                        .title(Span::styled(
                            input_title,
                            Style::default().fg(Color::Yellow),
                        )),
                );
            f.render_widget(input, input_area);
        }
    }

    fn render_onboarding_footer(&self, f: &mut Frame, area: ratatui::layout::Rect, ob: &OnboardingState) {
        // Step indicator dots — 9 logical steps
        let steps = [
            OnboardingStep::Intro,
            OnboardingStep::ModeSelection,
            OnboardingStep::ProviderSelection,
            OnboardingStep::ApiKeyEntry,
            OnboardingStep::AskName,
            OnboardingStep::AskRole,
            OnboardingStep::AskAliases,
            OnboardingStep::Processing,
            OnboardingStep::Confirmation,
            OnboardingStep::Done,
        ];

        let current_idx = match ob.step {
            OnboardingStep::Entrance => 0,
            OnboardingStep::Intro => 0,
            OnboardingStep::ModeSelection => 1,
            OnboardingStep::ProviderSelection => 2,
            OnboardingStep::ApiKeyEntry | OnboardingStep::ApiKeyValidation => 3,
            OnboardingStep::AskName => 4,
            OnboardingStep::AskRole => 5,
            OnboardingStep::AskAliases => 6,
            OnboardingStep::Processing => 7,
            OnboardingStep::Confirmation => 8,
            OnboardingStep::Done => 9,
        };

        let mut dots: Vec<Span> = Vec::new();
        for (i, _) in steps.iter().enumerate() {
            if i == current_idx {
                dots.push(Span::styled(" @ ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)));
            } else if i < current_idx {
                dots.push(Span::styled(" O ", Style::default().fg(Color::Cyan)));
            } else {
                dots.push(Span::styled(" . ", Style::default().fg(Color::DarkGray)));
            }
        }

        let hint = match ob.step {
            OnboardingStep::Intro => {
                if ob.text_complete { "ENTER: Continue" } else { "ENTER: Skip text" }
            }
            OnboardingStep::ModeSelection => {
                if ob.text_complete { "[1] Cloud  [2] Local" } else { "ENTER: Skip text" }
            }
            OnboardingStep::ProviderSelection => {
                if ob.text_complete { "[1] Anthropic  [2] OpenAI  [3] Google" } else { "ENTER: Skip text" }
            }
            OnboardingStep::ApiKeyEntry => {
                "Paste your API key, ENTER to validate"
            }
            OnboardingStep::ApiKeyValidation => {
                if ob.validation_task.is_some() { "Validating..." }
                else if ob.api_key_valid == Some(false) { "[1] Retry  [2] Skip" }
                else { "" }
            }
            OnboardingStep::AskName | OnboardingStep::AskRole => {
                "Type your answer, ENTER to continue"
            }
            OnboardingStep::AskAliases => {
                "Type aliases, ENTER to continue (or skip)"
            }
            OnboardingStep::Processing => {
                if ob.processing_task.is_some() { "" }
                else if !ob.ollama_available { "ENTER: Continue" }
                else { "" }
            }
            OnboardingStep::Confirmation => {
                if ob.text_complete { "[Y] / [N]" } else { "" }
            }
            OnboardingStep::Done => "",
            _ => "",
        };

        let mut footer_lines = vec![
            Line::from(dots),
        ];
        if !hint.is_empty() {
            footer_lines.push(Line::from(vec![
                Span::styled("ESC: Skip  |  ", Style::default().fg(Color::DarkGray)),
                Span::styled(hint, Style::default().fg(Color::DarkGray)),
            ]));
        } else {
            footer_lines.push(Line::from(Span::styled(
                "ESC: Skip",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let footer = Paragraph::new(footer_lines)
            .alignment(Alignment::Center);
        f.render_widget(footer, area);
    }

    fn render_magic_transition(&self, f: &mut Frame, area: ratatui::layout::Rect, frame: usize) {
        if frame <= 10 {
            // Sparkle scatter phase (builds up over ~1s)
            let density = match frame {
                0..=2 => 0.1,
                3..=4 => 0.2,
                5..=6 => 0.35,
                7..=8 => 0.55,
                _ => 0.8,
            };

            let mut lines: Vec<Line> = Vec::new();
            let seed = frame * 7919;
            for y in 0..area.height {
                let mut spans: Vec<Span> = Vec::new();
                for x in 0..area.width {
                    let hash = ((x as usize).wrapping_mul(31).wrapping_add(y as usize).wrapping_mul(17).wrapping_add(seed)) % 100;
                    let threshold = (density * 100.0) as usize;
                    if hash < threshold {
                        let ch_idx = (hash + frame) % SPARKLE_CHARS.len();
                        let ch = SPARKLE_CHARS[ch_idx];
                        let color = match (hash + frame) % 3 {
                            0 => Color::Cyan,
                            1 => Color::Yellow,
                            _ => Color::White,
                        };
                        spans.push(Span::styled(
                            ch.to_string(),
                            Style::default().fg(color),
                        ));
                    } else {
                        spans.push(Span::raw(" "));
                    }
                }
                lines.push(Line::from(spans));
            }
            let p = Paragraph::new(lines);
            f.render_widget(p, area);
        } else if frame <= 14 {
            // Flash frame — bright cyan
            let flash_char = if frame <= 12 { "*" } else { "." };
            let mut lines: Vec<Line> = Vec::new();
            for _ in 0..area.height {
                let line_str: String = std::iter::repeat(flash_char).take(area.width as usize).collect();
                lines.push(Line::from(Span::styled(
                    line_str,
                    Style::default().fg(Color::Cyan),
                )));
            }
            let p = Paragraph::new(lines);
            f.render_widget(p, area);
        } else {
            // Fade to empty — sparse sparkles fading out (~1.5s)
            let density = match frame {
                15..=17 => 0.2,
                18..=20 => 0.12,
                21..=23 => 0.06,
                24..=26 => 0.02,
                _ => 0.0,
            };

            let mut lines: Vec<Line> = Vec::new();
            let seed = frame * 3571;
            for y in 0..area.height {
                let mut spans: Vec<Span> = Vec::new();
                for x in 0..area.width {
                    let hash = ((x as usize).wrapping_mul(31).wrapping_add(y as usize).wrapping_mul(17).wrapping_add(seed)) % 100;
                    let threshold = (density * 100.0) as usize;
                    if hash < threshold {
                        spans.push(Span::styled(".", Style::default().fg(Color::DarkGray)));
                    } else {
                        spans.push(Span::raw(" "));
                    }
                }
                lines.push(Line::from(spans));
            }
            let p = Paragraph::new(lines);
            f.render_widget(p, area);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Chat Generation Pipeline (free function, runs as spawned task)
// ─────────────────────────────────────────────────────────────────────────────

async fn chat_generation_pipeline(
    config: crate::core::config::EnrichmentConfig,
    system_prompt: String,
    messages: Vec<(String, String)>,
    user_message: String,
    entity_names: Vec<(String, i64)>,
    tx: mpsc::Sender<ChatStreamEvent>,
) {
    // Step 1: Entity cross-referencing
    let _ = tx.send(ChatStreamEvent::Status("Searching for related recordings...".to_string())).await;
    let mut extra_context = String::new();

    if let Ok(db) = crate::database::Database::new() {
        for (name, entity_id) in &entity_names {
            if user_message.to_lowercase().contains(&name.to_lowercase()) {
                if let Ok(recordings) = db.get_recordings_for_entity(*entity_id) {
                    if !recordings.is_empty() {
                        let recording_summaries: Vec<(String, String)> = recordings.iter().take(5).map(|r| {
                            let rname = r.display_name.as_ref().unwrap_or(&r.directory_name).clone();
                            let summary = r.summary.as_deref().unwrap_or("(no summary)").to_string();
                            (rname, summary)
                        }).collect();
                        extra_context += &chat_prompts::format_related_recordings(name, &recording_summaries);
                    }
                }
            }
        }
    }

    // Step 2: Build prompt
    let _ = tx.send(ChatStreamEvent::Status("Building context...".to_string())).await;
    let full_prompt = chat_prompts::build_conversation(&system_prompt, &messages, &user_message, &extra_context);

    // Step 3: Stream response
    let _ = tx.send(ChatStreamEvent::Status("Thinking...".to_string())).await;
    let provider = crate::enrichment::create_provider(&config);
    let (chunk_tx, mut chunk_rx) = mpsc::channel::<String>(100);

    let stream_handle = tokio::spawn(async move {
        provider.generate_text_stream(&full_prompt, chunk_tx).await
    });

    // Forward chunks
    while let Some(chunk) = chunk_rx.recv().await {
        let _ = tx.send(ChatStreamEvent::Chunk(chunk)).await;
    }

    match stream_handle.await {
        Ok(Ok(())) => { let _ = tx.send(ChatStreamEvent::Done).await; }
        Ok(Err(e)) => { let _ = tx.send(ChatStreamEvent::Error(format!("{}", e))).await; }
        Err(e) => { let _ = tx.send(ChatStreamEvent::Error(format!("Task error: {}", e))).await; }
    }
}
