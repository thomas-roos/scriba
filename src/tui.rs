use crate::database::{Database, Recording};
use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{
        Block, Borders, Clear, Paragraph, Row, Table, TableState, Wrap, Cell,
    },
    Frame, Terminal,
};
use std::io;
use std::path::PathBuf;
use tokio::process::Command as TokioCommand;
use dirs::home_dir;
use anyhow::Context;
use std::time::Duration;

pub struct TuiRecordingLibrary {
    db: Database,
    recordings: Vec<Recording>,
    table_state: TableState,
    current_page: usize,
    page_size: usize,
    show_help: bool,
    show_stats: bool,
    search_mode: bool,
    search_query: String,
    show_message: bool,
    message: String,
    show_transcript: bool,
    transcript_content: String,
    show_delete_confirm: bool,
    delete_candidate: Option<Recording>,
}

impl TuiRecordingLibrary {
    pub fn new() -> Result<Self> {
        let db = Database::new()?;
        let mut table_state = TableState::default();
        table_state.select(Some(0));

        Ok(Self {
            db,
            recordings: Vec::new(),
            table_state,
            current_page: 0,
            page_size: 10,
            show_help: false,
            show_stats: false,
            search_mode: false,
            search_query: String::new(),
            show_message: false,
            message: String::new(),
            show_transcript: false,
            transcript_content: String::new(),
            show_delete_confirm: false,
            delete_candidate: None,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        self.load_recordings()?;
        let result = self.run_app(&mut terminal).await;

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

    async fn run_app<B: ratatui::backend::Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        loop {
            terminal.draw(|f| self.ui(f))?;

            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match self.handle_key_event(key.code).await {
                            Ok(AppAction::Continue) => continue,
                            Ok(AppAction::Quit) => break,
                            Err(e) => return Err(e),
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn load_recordings(&mut self) -> Result<()> {
        let offset = (self.current_page * self.page_size) as i64;
        
        self.recordings = if self.search_query.is_empty() {
            self.db.list_recordings(Some(self.page_size as i64), Some(offset))?
        } else {
            let search_results = self.db.search_transcripts(&self.search_query, None)?;
            search_results.into_iter().map(|(recording, _)| recording).collect()
        };

        if !self.recordings.is_empty() {
            self.table_state.select(Some(0));
        } else {
            self.table_state.select(None);
        }

        Ok(())
    }

    async fn handle_key_event(&mut self, key_code: KeyCode) -> Result<AppAction> {
        if self.show_message {
            // Any key press closes the message popup
            self.show_message = false;
            self.message.clear();
            return Ok(AppAction::Continue);
        }

        if self.search_mode {
            return self.handle_search_input(key_code).await;
        }

        if self.show_help {
            self.show_help = false;
            return Ok(AppAction::Continue);
        }

        if self.show_stats {
            self.show_stats = false;
            return Ok(AppAction::Continue);
        }

        if self.show_transcript {
            self.show_transcript = false;
            self.transcript_content.clear();
            return Ok(AppAction::Continue);
        }

        if self.show_delete_confirm {
            return self.handle_delete_confirmation(key_code).await;
        }

        match key_code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(AppAction::Quit),
            KeyCode::Char('h') | KeyCode::F(1) => {
                self.show_help = true;
            },
            KeyCode::Char('s') => {
                self.show_stats = true;
            },
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_query.clear();
            },
            KeyCode::Up => {
                self.previous_recording();
            },
            KeyCode::Down => {
                self.next_recording();
            },
            KeyCode::PageUp => {
                self.previous_page().await?;
            },
            KeyCode::PageDown => {
                self.next_page().await?;
            },
            KeyCode::Enter => {
                match self.show_selected_transcript_only().await {
                    Ok(()) => {}
                    Err(e) => {
                        self.message = format!("❌ Failed to load transcript: {}", e);
                        self.show_message = true;
                    }
                }
            },
            KeyCode::Char('d') => {
                self.show_delete_confirmation();
            },
            KeyCode::Delete => {
                self.show_delete_confirmation();
            },
            KeyCode::Char('p') | KeyCode::Char('P') => {
                match self.play_selected_recording().await {
                    Ok(()) => {}
                    Err(e) => {
                        self.message = format!("❌ Failed to play recording: {}", e);
                        self.show_message = true;
                    }
                }
            },
            _ => {}
        }

        Ok(AppAction::Continue)
    }

    async fn handle_search_input(&mut self, key_code: KeyCode) -> Result<AppAction> {
        match key_code {
            KeyCode::Esc => {
                self.search_mode = false;
                self.search_query.clear();
                self.load_recordings()?;
            },
            KeyCode::Enter => {
                self.search_mode = false;
                self.load_recordings()?;
            },
            KeyCode::Backspace => {
                self.search_query.pop();
            },
            KeyCode::Char(c) => {
                self.search_query.push(c);
            },
            _ => {},
        }
        Ok(AppAction::Continue)
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
        if self.recordings.len() == self.page_size {
            self.current_page += 1;
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


    async fn show_selected_transcript_only(&mut self) -> Result<()> {
        if let Some(selected) = self.table_state.selected() {
            if let Some(recording) = self.recordings.get(selected) {
                if recording.has_transcript {
                    match self.load_transcript_content(recording) {
                        Ok(content) => {
                            self.transcript_content = content;
                            self.show_transcript = true;
                        }
                        Err(e) => {
                            self.message = format!("❌ Failed to load transcript: {}", e);
                            self.show_message = true;
                        }
                    }
                } else {
                    self.message = "❌ No transcript available for this recording. Use P to play instead.".to_string();
                    self.show_message = true;
                }
            }
        }
        Ok(())
    }

    async fn play_selected_recording(&mut self) -> Result<()> {
        use anyhow::anyhow;
        if let Some(selected) = self.table_state.selected() {
            if let Some(recording) = self.recordings.get(selected) {
                // Locate the audio file in ~/scriba_recordings/<directory_name>/
                let audio_path = self
                    .find_audio_file(recording)
                    .ok_or_else(|| anyhow!("Could not find an audio file for this recording"))?;

                // Candidate players differ by platform. We'll try a few in order.
                #[cfg(target_os = "macos")]
                let candidates: Vec<(&str, &[&str])> = vec![
                    ("afplay", &[]),             // Native macOS
                    ("mpv", &["--really-quiet"]),
                    ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
                ];

                #[cfg(all(unix, not(target_os = "macos")))]
                let candidates: Vec<(&str, &[&str])> = vec![
                    ("mpv", &["--really-quiet"]),
                    ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
                    ("aplay", &[]),             // WAV-only fallback
                ];

                #[cfg(target_os = "windows")]
                let candidates: Vec<(&str, &[&str])> = vec![
                    ("powershell", &["-NoProfile", "-Command", "(New-Object Media.SoundPlayer '" ]), // will be handled specially
                ];

                // Try each candidate until one spawns successfully
                let mut launched_with: Option<String> = None;

                #[cfg(not(target_os = "windows"))]
                for (prog, base_args) in candidates {
                    let mut cmd = TokioCommand::new(prog);
                    for a in base_args { cmd.arg(a); }
                    cmd.arg(&audio_path);
                    match cmd.spawn() {
                        Ok(_child) => { launched_with = Some(prog.to_string()); break; }
                        Err(_e) => { /* try next */ }
                    }
                }

                #[cfg(target_os = "windows")]
                {
                    // Use PowerShell SoundPlayer fallback
                    let escaped = audio_path.to_string_lossy().replace("'", "''");
                    let ps = format!(
                        "$p=New-Object Media.SoundPlayer '{}';$p.Play();",
                        escaped
                    );
                    match TokioCommand::new("powershell")
                        .arg("-NoProfile")
                        .arg("-Command")
                        .arg(ps)
                        .spawn()
                    {
                        Ok(_child) => { launched_with = Some("powershell".to_string()); }
                        Err(_e) => {}
                    }
                }

                if let Some(player) = launched_with {
                    let name = recording
                        .display_name
                        .as_ref()
                        .unwrap_or(&recording.directory_name);
                    self.message = format!("▶ Playing: {}\nUsing player: {}\n\n(Playback runs externally; return here when done.)", name, player);
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
            } else { Ok(()) }
        } else { Ok(()) }
    }

    fn find_audio_file(&self, recording: &Recording) -> Option<PathBuf> {
        let base_path = home_dir()?.join("scriba_recordings").join(&recording.directory_name);
        if !base_path.exists() { return None; }
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

    fn show_delete_confirmation(&mut self) {
        if let Some(selected) = self.table_state.selected() {
            if let Some(recording) = self.recordings.get(selected).cloned() {
                self.delete_candidate = Some(recording);
                self.show_delete_confirm = true;
            }
        }
    }

    async fn handle_delete_confirmation(&mut self, key_code: KeyCode) -> Result<AppAction> {
        match key_code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // Confirm deletion
                if let Some(recording) = self.delete_candidate.take() {
                    match self.perform_delete_recording(recording).await {
                        Ok(()) => {}
                        Err(e) => {
                            self.message = format!("❌ Failed to delete recording: {}", e);
                            self.show_message = true;
                        }
                    }
                }
                self.show_delete_confirm = false;
                self.delete_candidate = None;
            },
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                // Cancel deletion
                self.show_delete_confirm = false;
                self.delete_candidate = None;
            },
            _ => {
                // Any other key, just ignore
            }
        }
        Ok(AppAction::Continue)
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
                    self.message = "✅ Recording deleted successfully".to_string();
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
            return Err(anyhow::anyhow!("Selected recording has no database ID; cannot delete."));
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
        
        // Fallback: try to load from file
        let base_path = home_dir()
            .context("Could not find home directory")?
            .join("scriba_recordings");
        let transcript_path = base_path.join(&recording.directory_name).join("transcript.txt");
        
        if transcript_path.exists() {
            std::fs::read_to_string(&transcript_path)
                .context("Failed to read transcript file")
        } else {
            Err(anyhow::anyhow!("Transcript file not found"))
        }
    }

    fn ui(&mut self, f: &mut Frame) {
        let size = f.size();

        if self.show_help {
            self.render_help_popup(f, size);
            return;
        }

        if self.show_stats {
            self.render_stats_popup(f, size);
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

        // Main layout
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // Header
                Constraint::Min(0),     // Table
                Constraint::Length(3),  // Footer/Controls
            ])
            .split(size);

        // Header
        let header = Paragraph::new("🎵 SCRIBA RECORDING LIBRARY 🎵")
            .style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Cyan))
                    .border_type(ratatui::widgets::BorderType::Double),
            );
        f.render_widget(header, chunks[0]);

        // Table
        self.render_recordings_table(f, chunks[1]);

        // Footer
        self.render_controls(f, chunks[2]);

        // Search input overlay
        if self.search_mode {
            self.render_search_input(f, size);
        }
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
                    .title("Message (press any key)")
            )
            .wrap(Wrap { trim: true });

        f.render_widget(para, popup_area);
    }

    fn render_transcript_popup(&self, f: &mut Frame, area: ratatui::layout::Rect) {
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
                Constraint::Percentage(5),
                Constraint::Percentage(90),
                Constraint::Percentage(5),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        let para = Paragraph::new(self.transcript_content.clone())
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Cyan))
                    .title("📝 Transcript (press any key to close)")
            )
            .wrap(Wrap { trim: true });

        f.render_widget(para, popup_area);
    }

    fn render_delete_confirmation_popup(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Length(12),
                Constraint::Percentage(63),
            ])
            .split(area)[1];

        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(50),
                Constraint::Percentage(25),
            ])
            .split(popup_area)[1];

        f.render_widget(Clear, popup_area);

        let recording_name = if let Some(recording) = &self.delete_candidate {
            recording.display_name.as_ref()
                .unwrap_or(&recording.directory_name)
                .clone()
        } else {
            "Unknown".to_string()
        };

        let confirmation_text = format!("⚠️  DELETE CONFIRMATION  ⚠️\n\nAre you sure you want to permanently delete:\n\n\"{}\"?\n\nThis action cannot be undone!\n\n[Y] Yes, delete it    [N] No, cancel\n[ESC] Cancel", recording_name);

        let para = Paragraph::new(confirmation_text)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Red))
                    .title("⚠️  Confirm Deletion")
            )
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(para, popup_area);
    }

    fn render_recordings_table(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        let header_cells = ["#", "Status", "Name", "Duration", "Created"]
            .iter()
            .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
        
        let header = Row::new(header_cells)
            .style(Style::default())
            .height(1)
            .bottom_margin(1);

        let rows: Vec<Row> = self.recordings.iter().enumerate().map(|(i, recording)| {
            let display_name = recording.display_name.as_ref()
                .unwrap_or(&recording.directory_name);
            
            let duration = recording.duration_seconds
                .map(|d| self.format_duration(d))
                .unwrap_or_else(|| "Unknown".to_string());

            let status = if recording.has_transcript { "[T]" } else { "[A]" };
            let created = recording.created_at.format("%m/%d %H:%M").to_string();

            let cells = vec![
                Cell::from((i + 1).to_string()),
                Cell::from(status).style(
                    if recording.has_transcript {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::Blue)
                    }
                ),
                Cell::from(display_name.clone()),
                Cell::from(duration),
                Cell::from(created),
            ];

            Row::new(cells).height(1).bottom_margin(0)
        }).collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(4),   // #
                Constraint::Length(8),   // Status
                Constraint::Min(20),     // Name (flexible)
                Constraint::Length(12),  // Duration
                Constraint::Length(12),  // Created
            ]
        )
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Cyan))
                    .title("Recordings")
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            )
            .highlight_symbol("▶ ");

        f.render_stateful_widget(table, area, &mut self.table_state);
    }

    fn render_controls(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let controls = if self.search_mode {
            "ESC: Cancel | ENTER: Search | Type to search..."
        } else {
            "↑↓: Navigate | ENTER: Transcript | P: Play | D/Delete: Delete | /: Search | S: Stats | H: Help | Q: Quit"
        };

        let controls_paragraph = Paragraph::new(controls)
            .style(Style::default().fg(Color::White))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Blue))
                    .title("Controls")
            );

        f.render_widget(controls_paragraph, area);
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

        let search_input = Paragraph::new(format!("🔍 Search: {}", self.search_query))
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Cyan))
                    .title("Search Recordings")
            );

        f.render_widget(search_input, popup_area);
    }

    fn render_help_popup(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(60),
                Constraint::Percentage(20),
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

        let help_text = vec![
            Line::from("🎵 SCRIBA RECORDING LIBRARY HELP 🎵"),
            Line::from(""),
            Line::from("Navigation:"),
            Line::from("  ↑/↓        - Navigate recordings"),
            Line::from("  PgUp/PgDn  - Change pages"),
            Line::from("  ENTER      - View transcript"),
            Line::from("  P          - Play recording"),
            Line::from(""),
            Line::from("Actions:"),
            Line::from("  D          - Delete recording (with confirmation)"),
            Line::from("  /          - Search recordings"),
            Line::from("  S          - Show statistics"),
            Line::from("  H/F1       - Show this help"),
            Line::from("  Q/ESC      - Quit"),
            Line::from(""),
            Line::from("Press any key to continue..."),
        ];

        let help_paragraph = Paragraph::new(help_text)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Yellow))
                    .title("Help")
            )
            .wrap(Wrap { trim: true });

        f.render_widget(help_paragraph, popup_area);
    }

    fn render_stats_popup(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(50),
                Constraint::Percentage(25),
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

        let stats = match self.db.get_stats() {
            Ok(stats) => stats,
            Err(_) => return,
        };

        let stats_text = vec![
            Line::from("📊 RECORDING STATISTICS 📊"),
            Line::from(""),
            Line::from(format!("Total Recordings: {}", stats.total_recordings)),
            Line::from(format!("Total Duration: {}", stats.format_duration())),
            Line::from(format!("Total Storage: {}", stats.format_size())),
            Line::from(format!("Transcribed: {}", stats.transcribed_count)),
            Line::from(format!("Total Words: {}", stats.total_words)),
            Line::from(""),
            Line::from("💡 Your personal knowledge base is growing!"),
            Line::from(""),
            Line::from("Press any key to continue..."),
        ];

        let stats_paragraph = Paragraph::new(stats_text)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Magenta))
                    .title("Statistics")
            );

        f.render_widget(stats_paragraph, popup_area);
    }

    fn format_duration(&self, seconds: i64) -> String {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        let secs = seconds % 60;
        
        if hours > 0 {
            format!("{}h {}m {}s", hours, minutes, secs)
        } else if minutes > 0 {
            format!("{}m {}s", minutes, secs)
        } else {
            format!("{}s", secs)
        }
    }
}

#[derive(Debug)]
enum AppAction {
    Continue,
    Quit,
}