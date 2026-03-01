//! Chat module for "Ask Scriba" — types, state, rendering, and pipeline functions.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};
use tokio::sync::mpsc;

use crate::enrichment::chat_prompts;

// ─────────────────────────────────────────────────────────────────────────────
// Chat types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ChatStreamEvent {
    Status(String),
    Chunk(String),
    ToolCall { name: String, input_summary: String },
    ToolResult { name: String, output_summary: String },
    Usage { input_tokens: u32, output_tokens: u32 },
    Compacted { summary: String, removed_count: usize },
    Done,
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatContext {
    Global,
    Recording { recording_id: i64, recording_name: String },
}

#[derive(Debug, Clone)]
pub struct ToolCallDisplay {
    pub name: String,
    pub input_summary: String,
    pub output_summary: String,
    pub is_complete: bool,
}

#[derive(Debug, Clone)]
pub enum ChatBlock {
    Text(String),
    ToolCall(ToolCallDisplay),
    CompactionMarker,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub blocks: Vec<ChatBlock>,
}

impl ChatMessage {
    /// Create a simple text-only message (user, system, or fallback assistant).
    pub fn text(role: ChatRole, content: String) -> Self {
        Self { role, blocks: vec![ChatBlock::Text(content)] }
    }

    /// Concatenate all text blocks for LLM conversation history.
    pub fn content(&self) -> String {
        let mut s = String::new();
        for block in &self.blocks {
            if let ChatBlock::Text(t) = block {
                s.push_str(t);
            }
        }
        s
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ChatFocus {
    Table,
    ChatInput,
}

// ─────────────────────────────────────────────────────────────────────────────
// ChatState
// ─────────────────────────────────────────────────────────────────────────────

pub struct ChatState {
    pub context: ChatContext,
    pub messages: Vec<ChatMessage>,
    pub input_buffer: String,
    pub scroll_offset: usize,

    // Streaming state
    pub stream_rx: Option<mpsc::Receiver<ChatStreamEvent>>,
    pub current_status: Option<String>,
    pub is_generating: bool,

    // Blocks accumulated during the current generation (text + tool calls, in order)
    pub pending_blocks: Vec<ChatBlock>,

    // Generation task handle
    pub generation_task: Option<tokio::task::JoinHandle<()>>,

    // Suggestions
    pub suggestions: Vec<String>,
    pub show_suggestions: bool,
    pub selected_suggestion: usize,

    // Pre-built system prompt
    pub system_prompt: String,

    // Focus
    pub focus: ChatFocus,

    // Spinner frame
    pub spinner_frame: usize,

    // Auto-scroll: stays true until user manually scrolls up
    pub auto_scroll: bool,

    // Queued message: submitted while generating, will auto-send when done
    pub pending_message: Option<String>,

    // Last rendered chat panel area (for mouse hit-testing)
    pub panel_rect: Rect,

    // Total content lines (for scroll clamping in mouse handler)
    pub total_content_lines: usize,

    // Text selection state (click-drag to select, auto-copy on release)
    pub selection_anchor: Option<(usize, usize)>, // (content_line, char_col) where drag started
    pub selection_end: Option<(usize, usize)>,    // (content_line, char_col) current drag position
    pub content_texts: Vec<String>,               // plain text of each content line for extraction

    // Context window tracking (Anthropic only)
    pub context_window_max: u32,
    pub context_input_tokens: u32,
    pub context_output_tokens: u32,
    usage_baseline_set: bool, // true after first Usage event per generation

    // Rendering cache for completed messages
    cached_msg_lines: Vec<Line<'static>>,
    cached_msg_texts: Vec<String>,
    cached_msg_count: usize,
    cached_width: usize,
}

impl ChatState {
    pub fn new() -> Self {
        Self {
            context: ChatContext::Global,
            messages: Vec::new(),
            input_buffer: String::new(),
            scroll_offset: 0,
            stream_rx: None,
            current_status: None,
            is_generating: false,
            pending_blocks: Vec::new(),
            generation_task: None,
            suggestions: Vec::new(),
            show_suggestions: true,
            selected_suggestion: 0,
            system_prompt: String::new(),
            focus: ChatFocus::Table,
            spinner_frame: 0,
            auto_scroll: true,
            pending_message: None,
            panel_rect: Rect::default(),
            total_content_lines: 0,
            selection_anchor: None,
            selection_end: None,
            content_texts: Vec::new(),
            context_window_max: std::env::var("SCRIBA_CTX_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(200_000),
            context_input_tokens: 0,
            context_output_tokens: 0,
            usage_baseline_set: false,
            cached_msg_lines: Vec::new(),
            cached_msg_texts: Vec::new(),
            cached_msg_count: 0,
            cached_width: 0,
        }
    }

    pub fn context_usage_fraction(&self) -> f64 {
        let total = self.context_input_tokens + self.context_output_tokens;
        total as f64 / self.context_window_max as f64
    }

    pub fn needs_compaction(&self) -> bool {
        self.context_usage_fraction() > 0.80
    }

    // ── Stream Polling ──────────────────────────────────────────────────────

    /// Poll the chat stream for new events. Returns `true` if a pending message
    /// should be re-sent (i.e. generation finished and a queued message exists).
    pub fn poll_stream(&mut self) -> bool {
        let mut should_resend = false;
        if let Some(ref mut rx) = self.stream_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ChatStreamEvent::Status(msg) => {
                        self.current_status = Some(msg);
                    }
                    ChatStreamEvent::Chunk(text) => {
                        // Append text to the last Text block, or create one
                        if let Some(ChatBlock::Text(ref mut s)) = self.pending_blocks.last_mut() {
                            s.push_str(&text);
                        } else {
                            self.pending_blocks.push(ChatBlock::Text(text));
                        }
                        // Text is rendered progressively — clear status so it doesn't compete
                        self.current_status = None;
                    }
                    ChatStreamEvent::ToolCall { name, input_summary } => {
                        self.current_status = None; // tool call line is the indicator
                        self.pending_blocks.push(ChatBlock::ToolCall(ToolCallDisplay {
                            name,
                            input_summary,
                            output_summary: String::new(),
                            is_complete: false,
                        }));
                    }
                    ChatStreamEvent::ToolResult { name, output_summary } => {
                        // Find last matching incomplete tool call in blocks
                        for block in self.pending_blocks.iter_mut().rev() {
                            if let ChatBlock::ToolCall(ref mut tc) = block {
                                if tc.name == name && !tc.is_complete {
                                    tc.output_summary = output_summary.clone();
                                    tc.is_complete = true;
                                    break;
                                }
                            }
                        }
                        self.current_status = Some("Thinking...".to_string());
                    }
                    ChatStreamEvent::Usage { input_tokens, output_tokens } => {
                        // Only update input_tokens from the FIRST API call of each
                        // generation. That call reflects the true conversation baseline.
                        // Subsequent calls within the same turn include transient tool-use
                        // overhead that would inflate the bar and cause apparent "resets"
                        // when the next turn starts with a smaller baseline.
                        if !self.usage_baseline_set {
                            self.context_input_tokens = input_tokens;
                            self.usage_baseline_set = true;
                        }
                        // Always update output_tokens (current response contribution)
                        self.context_output_tokens = output_tokens;
                    }
                    ChatStreamEvent::Compacted { summary, removed_count } => {
                        let total_msgs = self.messages.len();
                        if removed_count <= total_msgs {
                            let remaining: Vec<ChatMessage> = self.messages.drain(removed_count..).collect();
                            self.messages.clear();
                            self.messages.push(ChatMessage::text(
                                ChatRole::System,
                                format!("Context compacted: {} messages summarized, {} kept",
                                    removed_count, remaining.len()),
                            ));
                            self.messages.push(ChatMessage {
                                role: ChatRole::System,
                                blocks: vec![ChatBlock::CompactionMarker],
                            });
                            self.messages.push(ChatMessage::text(ChatRole::System, summary));
                            self.messages.extend(remaining);
                        } else {
                            // Mismatch — just insert the summary without restructuring
                            self.messages.push(ChatMessage {
                                role: ChatRole::System,
                                blocks: vec![ChatBlock::CompactionMarker],
                            });
                            self.messages.push(ChatMessage::text(ChatRole::System, summary));
                        }
                        self.cached_msg_count = 0;
                        self.context_input_tokens = 0;
                        self.context_output_tokens = 0;
                        self.usage_baseline_set = false;
                    }
                    ChatStreamEvent::Done => {
                        let blocks = std::mem::take(&mut self.pending_blocks);
                        self.messages.push(ChatMessage {
                            role: ChatRole::Assistant,
                            blocks,
                        });
                        self.is_generating = false;
                        self.stream_rx = None;
                        self.current_status = None;
                        self.usage_baseline_set = false;
                        if self.pending_message.is_some() {
                            should_resend = true;
                        }
                        return should_resend;
                    }
                    ChatStreamEvent::Error(msg) => {
                        self.pending_blocks.clear();
                        self.messages.push(ChatMessage::text(
                            ChatRole::System,
                            format!("Error: {}", msg),
                        ));
                        self.is_generating = false;
                        self.stream_rx = None;
                        self.current_status = None;
                        self.usage_baseline_set = false;
                        self.pending_message = None;
                        return false;
                    }
                }
            }
        }
        should_resend
    }

    // ── Mouse Helpers ───────────────────────────────────────────────────────

    /// Map a mouse position to a (content_line, char_col) in the chat content.
    pub fn mouse_to_content_pos(&self, mouse_col: u16, mouse_row: u16) -> Option<(usize, usize)> {
        let rect = self.panel_rect;
        let click_row = (mouse_row - rect.y).saturating_sub(1) as usize;
        let char_col = (mouse_col - rect.x).saturating_sub(1) as usize;

        let inner_height = rect.height.saturating_sub(2) as usize;
        let has_conv = !self.messages.is_empty() || self.is_generating;
        let reserved = if has_conv { 2 } else { 1 };
        let chat_height = inner_height.saturating_sub(reserved);

        let scroll_y = if self.auto_scroll || self.total_content_lines <= chat_height {
            self.total_content_lines.saturating_sub(chat_height)
        } else {
            let max_scroll = self.total_content_lines.saturating_sub(chat_height);
            self.scroll_offset.min(max_scroll)
        };

        let lines_to_show = self.total_content_lines.saturating_sub(scroll_y);
        let pad = chat_height.saturating_sub(lines_to_show.min(chat_height));
        if click_row < pad {
            return None;
        }
        let content_line = scroll_y + (click_row - pad);
        if content_line >= self.total_content_lines {
            return None;
        }
        Some((content_line, char_col))
    }

    /// Extract the plain text between two content positions.
    pub fn extract_selected_text(&self, anchor: (usize, usize), end: (usize, usize)) -> String {
        let (start, end) = if anchor <= end { (anchor, end) } else { (end, anchor) };
        let (start_line, start_col) = start;
        let (end_line, end_col) = end;

        let texts = &self.content_texts;
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

    // ── Rendering ───────────────────────────────────────────────────────────

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        self.panel_rect = area;
        if area.height < 3 {
            return;
        }

        let is_focused = self.focus == ChatFocus::ChatInput;
        let border_color = if is_focused { Color::Cyan } else { Color::DarkGray };

        let title = match &self.context {
            ChatContext::Global => "Ask Scriba",
            ChatContext::Recording { .. } => "Ask about this recording",
        };

        let inner_height = area.height.saturating_sub(2) as usize;
        let has_conversation = !self.messages.is_empty() || self.is_generating;
        let content_width = area.width.saturating_sub(4) as usize;

        let input_line_count = if !self.input_buffer.is_empty() {
            let cursor_ch = if is_focused { "▎" } else { "" };
            let display = format!("{}{}", self.input_buffer, cursor_ch);
            let wrap_width = content_width.saturating_sub(4);
            if wrap_width > 0 { textwrap::wrap(&display, wrap_width).len() } else { 1 }
        } else {
            1
        };
        let reserved = if has_conversation { 1 + input_line_count } else { input_line_count };
        let chat_height = inner_height.saturating_sub(reserved);

        let mut final_lines: Vec<Line> = Vec::with_capacity(inner_height);

        let show_suggestions = self.show_suggestions
            && !self.suggestions.is_empty()
            && self.messages.is_empty()
            && self.pending_blocks.is_empty()
            && !self.is_generating;

        // Compute selection range (normalized: start <= end)
        let selection = match (self.selection_anchor, self.selection_end) {
            (Some(a), Some(e)) => {
                let (start, end) = if a <= e { (a, e) } else { (e, a) };
                Some((start, end))
            }
            _ => None,
        };

        if show_suggestions {
            let mut all_lines: Vec<Line> = Vec::new();
            let mut content_texts: Vec<String> = Vec::new();
            let total_options = self.suggestions.len() + 1;
            for (i, s) in self.suggestions.iter().enumerate() {
                let text = if i == self.selected_suggestion {
                    format!("  > {}", s)
                } else {
                    format!("    {}", s)
                };
                let style = if i == self.selected_suggestion {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                content_texts.push(text.clone());
                all_lines.push(Line::from(Span::styled(text, style)));
            }
            let free_form_idx = total_options - 1;
            let text = if self.selected_suggestion == free_form_idx {
                "  > Ask Scriba anything..."
            } else {
                "    Ask Scriba anything..."
            };
            let style = if self.selected_suggestion == free_form_idx {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            content_texts.push(text.to_string());
            all_lines.push(Line::from(Span::styled(text.to_string(), style)));

            self.content_texts = content_texts;
            let total_content = all_lines.len();
            self.total_content_lines = total_content;
            let scroll_y: u16 = if self.auto_scroll || total_content <= chat_height {
                total_content.saturating_sub(chat_height) as u16
            } else {
                let max_scroll = total_content.saturating_sub(chat_height);
                self.scroll_offset.min(max_scroll) as u16
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
                        let line_text_len = self.content_texts.get(content_idx)
                            .map(|t| t.chars().count()).unwrap_or(0);
                        let line_end = if content_idx == sel_end_line { sel_end_col } else { line_text_len };
                        final_lines.push(apply_selection_highlight(line, line_start, line_end));
                        continue;
                    }
                }
                final_lines.push(line);
            }
        } else {
            // ── Phase 1: Completed messages (cached) ───────────────────────
            let msg_count = self.messages.len();
            let cache_valid = msg_count == self.cached_msg_count
                && content_width == self.cached_width;

            if !cache_valid {
                let mut cached_lines: Vec<Line<'static>> = Vec::new();
                let mut cached_texts: Vec<String> = Vec::new();
                let wrap_width = content_width.saturating_sub(2);

                for msg in &self.messages {
                    match msg.role {
                        ChatRole::User => {
                            let content = msg.content();
                            for line in content.lines() {
                                let wrapped = textwrap::wrap(line, wrap_width);
                                if wrapped.is_empty() {
                                    cached_texts.push("  ".to_string());
                                    cached_lines.push(Line::from(Span::styled(
                                        "  ".to_string(),
                                        Style::default().fg(Color::Cyan),
                                    )));
                                } else {
                                    for (j, w) in wrapped.iter().enumerate() {
                                        if j == 0 {
                                            let text = format!("  \u{25B8} {}", w);
                                            cached_texts.push(text.clone());
                                            cached_lines.push(Line::from(Span::styled(
                                                text,
                                                Style::default().fg(Color::Cyan),
                                            )));
                                        } else {
                                            let text = format!("    {}", w);
                                            cached_texts.push(text.clone());
                                            cached_lines.push(Line::from(Span::styled(
                                                text,
                                                Style::default().fg(Color::Cyan),
                                            )));
                                        }
                                    }
                                }
                            }
                            cached_texts.push(String::new());
                            cached_lines.push(Line::from(""));
                        }
                        ChatRole::Assistant => {
                            // Render blocks in chronological order — no header label
                            for block in &msg.blocks {
                                match block {
                                    ChatBlock::ToolCall(tc) => {
                                        render_tool_call_cached(tc, &mut cached_lines, &mut cached_texts);
                                    }
                                    ChatBlock::Text(text) => {
                                        for wl in safe_markdown_lines(text, wrap_width) {
                                            let plain: String =
                                                wl.spans.iter().map(|s| s.content.as_ref()).collect();
                                            cached_texts.push(format!("  {}", plain));
                                            let mut indented = vec![Span::raw("  ".to_string())];
                                            indented.extend(wl.spans);
                                            cached_lines.push(Line::from(indented));
                                        }
                                    }
                                    ChatBlock::CompactionMarker => {
                                        let dashes = "─".repeat((content_width.saturating_sub(22)) / 2);
                                        let marker = format!("  {} context compacted {}", dashes, dashes);
                                        cached_texts.push(marker.clone());
                                        cached_lines.push(Line::from(Span::styled(
                                            marker,
                                            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                                        )));
                                    }
                                }
                            }
                            cached_texts.push(String::new());
                            cached_lines.push(Line::from(""));
                        }
                        ChatRole::System => {
                            // Check for CompactionMarker blocks first
                            let has_marker = msg.blocks.iter().any(|b| matches!(b, ChatBlock::CompactionMarker));
                            if has_marker {
                                for block in &msg.blocks {
                                    match block {
                                        ChatBlock::CompactionMarker => {
                                            let dashes = "─".repeat((content_width.saturating_sub(22)) / 2);
                                            let marker = format!("  {} context compacted {}", dashes, dashes);
                                            cached_texts.push(marker.clone());
                                            cached_lines.push(Line::from(Span::styled(
                                                marker,
                                                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                                            )));
                                        }
                                        ChatBlock::Text(text) if !text.is_empty() => {
                                            let style = Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
                                            for line in text.lines() {
                                                let wrapped = textwrap::wrap(line, content_width);
                                                for w in &wrapped {
                                                    let full = w.to_string();
                                                    cached_texts.push(full.clone());
                                                    cached_lines.push(Line::from(Span::styled(full, style)));
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            } else {
                                let content = msg.content();
                                let style = Style::default().fg(Color::Red).add_modifier(Modifier::ITALIC);
                                for line in content.lines() {
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
                            }
                            cached_texts.push(String::new());
                            cached_lines.push(Line::from(""));
                        }
                    }
                }

                self.cached_msg_lines = cached_lines;
                self.cached_msg_texts = cached_texts;
                self.cached_msg_count = msg_count;
                self.cached_width = content_width;
            }

            // ── Phase 2: Dynamic content (progressive block rendering) ────
            // Blocks render as they arrive: text with markdown, tool calls
            // with spinners. Builds up the answer in real-time.
            let mut dynamic_lines: Vec<Line<'static>> = Vec::new();
            let mut dynamic_texts: Vec<String> = Vec::new();
            let wrap_width = content_width.saturating_sub(2);

            for block in &self.pending_blocks {
                match block {
                    ChatBlock::Text(text) => {
                        // Render text progressively with markdown — no header label
                        for wl in safe_markdown_lines(text, wrap_width) {
                            let plain: String =
                                wl.spans.iter().map(|s| s.content.as_ref()).collect();
                            dynamic_texts.push(format!("  {}", plain));
                            let mut indented = vec![Span::raw("  ".to_string())];
                            indented.extend(wl.spans);
                            dynamic_lines.push(Line::from(indented));
                        }
                    }
                    ChatBlock::ToolCall(tc) => {
                        let mut spans: Vec<Span<'static>> = Vec::new();
                        if tc.is_complete {
                            spans.push(Span::styled("  \u{2713} ", Style::default().fg(Color::Green)));
                        } else {
                            let spinners = ['\u{25D0}', '\u{25D1}', '\u{25D2}', '\u{25D3}'];
                            let icon = spinners[self.spinner_frame % spinners.len()];
                            spans.push(Span::styled(
                                format!("  {} ", icon),
                                Style::default().fg(Color::Magenta),
                            ));
                        }
                        spans.push(Span::styled(
                            tc.name.clone(),
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                        ));
                        if !tc.input_summary.is_empty() {
                            spans.push(Span::styled(
                                format!("({})", tc.input_summary),
                                Style::default().fg(Color::White),
                            ));
                        }
                        if tc.is_complete && !tc.output_summary.is_empty() {
                            spans.push(Span::styled(
                                format!(" \u{2192} {}", tc.output_summary),
                                Style::default().fg(Color::DarkGray),
                            ));
                        }
                        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
                        dynamic_texts.push(text);
                        dynamic_lines.push(Line::from(spans));
                    }
                    ChatBlock::CompactionMarker => {
                        let dashes = "─".repeat((content_width.saturating_sub(22)) / 2);
                        let marker = format!("  {} context compacted {}", dashes, dashes);
                        dynamic_texts.push(marker.clone());
                        dynamic_lines.push(Line::from(Span::styled(
                            marker,
                            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                        )));
                    }
                }
            }

            if let Some(status) = &self.current_status {
                let spinners = ['◐', '◑', '◒', '◓'];
                let spinner = spinners[self.spinner_frame % spinners.len()];
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
            let cached_len = self.cached_msg_lines.len();
            let total_content = cached_len + dynamic_lines.len();
            self.total_content_lines = total_content;

            // Clamp stale scroll offset to valid range
            let max_offset = total_content.saturating_sub(chat_height);
            if self.scroll_offset > max_offset {
                self.scroll_offset = max_offset;
            }

            let mut content_texts = self.cached_msg_texts.clone();
            content_texts.extend(dynamic_texts);
            self.content_texts = content_texts;

            // ── Scroll calculation ─────────────────────────────────────────
            let scroll_y: u16 = if self.auto_scroll || total_content <= chat_height {
                total_content.saturating_sub(chat_height) as u16
            } else {
                let max_scroll = total_content.saturating_sub(chat_height);
                self.scroll_offset.min(max_scroll) as u16
            };
            let lines_to_show = total_content.saturating_sub(scroll_y as usize);
            let pad = chat_height.saturating_sub(lines_to_show.min(chat_height));
            for _ in 0..pad {
                final_lines.push(Line::from(""));
            }

            // ── Phase 4: Build visible window ──────────────────────────────
            let start = scroll_y as usize;
            let end = (start + chat_height).min(total_content);
            for i in start..end {
                let line = if i < cached_len {
                    self.cached_msg_lines[i].clone()
                } else {
                    let dyn_idx = i - cached_len;
                    if dyn_idx < dynamic_lines.len() {
                        dynamic_lines[dyn_idx].clone()
                    } else {
                        Line::from("")
                    }
                };
                let content_idx = i;
                if let Some(((sel_start_line, sel_start_col), (sel_end_line, sel_end_col))) = selection {
                    if content_idx >= sel_start_line && content_idx <= sel_end_line {
                        let line_start = if content_idx == sel_start_line { sel_start_col } else { 0 };
                        let line_text_len = self.content_texts.get(content_idx)
                            .map(|t| t.chars().count()).unwrap_or(0);
                        let line_end = if content_idx == sel_end_line { sel_end_col } else { line_text_len };
                        final_lines.push(apply_selection_highlight(line, line_start, line_end));
                        continue;
                    }
                }
                final_lines.push(line);
            }
        }

        // ── Separator + Usage bar + Input line ─────────────────────────────
        if has_conversation {
            let sep_width = content_width.min(area.width.saturating_sub(4) as usize);
            let sep = "─".repeat(sep_width);
            final_lines.push(Line::from(Span::styled(
                format!("  {}", sep),
                Style::default().fg(Color::DarkGray),
            )));
        }

        let cursor = if is_focused { "▎" } else { "" };
        let has_pending = self.pending_message.is_some();
        if has_pending && self.input_buffer.is_empty() {
            let queued_msg = self.pending_message.as_deref().unwrap_or("");
            final_lines.push(Line::from(vec![
                Span::styled("  ▸ ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("{} ", queued_msg),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled("(queued)", Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
            ]));
        } else if !self.input_buffer.is_empty() {
            let display = format!("{}{}", self.input_buffer, cursor);
            let wrap_width = content_width.saturating_sub(4);
            let wrapped = textwrap::wrap(&display, wrap_width);
            for (j, w) in wrapped.iter().enumerate() {
                if j == 0 {
                    final_lines.push(Line::from(vec![
                        Span::styled("  ▸ ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        Span::styled(w.to_string(), Style::default().fg(Color::White)),
                    ]));
                } else {
                    final_lines.push(Line::from(Span::styled(
                        format!("    {}", w),
                        Style::default().fg(Color::White),
                    )));
                }
            }
        } else {
            let prompt_color = if is_focused { Color::Cyan } else { Color::DarkGray };
            final_lines.push(Line::from(Span::styled(
                format!("  ▸ {}", cursor),
                Style::default().fg(prompt_color),
            )));
        };

        let mut chat_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(title)
            .title_style(Style::default().fg(if is_focused { Color::Cyan } else { Color::DarkGray }));

        if self.context_input_tokens > 0 {
            let fraction = self.context_usage_fraction().min(1.0);
            let pct = (fraction * 100.0) as u32;
            let bar_len: usize = 10;
            let filled = ((bar_len as f64) * fraction) as usize;
            let empty = bar_len.saturating_sub(filled);

            let bar_color = if fraction <= 0.60 {
                Color::Green
            } else if fraction <= 0.80 {
                Color::Yellow
            } else {
                Color::Red
            };

            chat_block = chat_block.title_bottom(
                Line::from(vec![
                    Span::styled(" ctx [", Style::default().fg(Color::DarkGray)),
                    Span::styled("█".repeat(filled), Style::default().fg(bar_color)),
                    Span::styled("░".repeat(empty), Style::default().fg(Color::Indexed(237))),
                    Span::styled(format!("] {}% ", pct), Style::default().fg(Color::DarkGray)),
                ]).right_aligned()
            );
        }

        let para = Paragraph::new(final_lines).block(chat_block);
        f.render_widget(para, area);

        // ── Scroll position indicator ───────────────────────────────────────
        let total_content = self.total_content_lines;
        if total_content > chat_height && chat_height > 0 {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_symbol("█")
                .track_symbol(Some("│"))
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::default().fg(Color::Cyan))
                .track_style(Style::default().fg(Color::Indexed(237)));
            let scroll_y = if self.auto_scroll || total_content <= chat_height {
                total_content.saturating_sub(chat_height)
            } else {
                let max_scroll = total_content.saturating_sub(chat_height);
                self.scroll_offset.min(max_scroll)
            };
            let mut scrollbar_state = ScrollbarState::new(total_content)
                .position(scroll_y);
            let scrollbar_area = Rect {
                x: area.x,
                y: area.y + 1,
                width: area.width,
                height: area.height.saturating_sub(2),
            };
            f.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
        }

    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Static helper functions
// ─────────────────────────────────────────────────────────────────────────────

/// Render a completed tool call into the cached lines/texts.
fn render_tool_call_cached(
    tc: &ToolCallDisplay,
    cached_lines: &mut Vec<Line<'static>>,
    cached_texts: &mut Vec<String>,
) {
    let mut spans = vec![
        Span::styled("  \u{2713} ", Style::default().fg(Color::Green)),
        Span::styled(
            tc.name.clone(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
    ];
    if !tc.input_summary.is_empty() {
        spans.push(Span::styled(
            format!("({})", tc.input_summary),
            Style::default().fg(Color::White),
        ));
    }
    if !tc.output_summary.is_empty() {
        spans.push(Span::styled(
            format!(" \u{2192} {}", tc.output_summary),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    cached_texts.push(text);
    cached_lines.push(Line::from(spans));
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
    let highlight_bg = Color::Indexed(237);
    let mut result_spans: Vec<Span<'static>> = Vec::new();
    let mut col: usize = 0;

    for span in line.spans {
        let span_char_count = span.content.chars().count();
        let span_start = col;
        let span_end = col + span_char_count;

        if span_end <= sel_start || span_start >= sel_end {
            result_spans.push(span);
        } else {
            let chars: Vec<char> = span.content.chars().collect();

            let hl_start = sel_start.saturating_sub(span_start);
            if hl_start > 0 {
                let before: String = chars[..hl_start].iter().collect();
                result_spans.push(Span::styled(before, span.style));
            }

            let hl_end = (sel_end - span_start).min(chars.len());
            let selected: String = chars[hl_start..hl_end].iter().collect();
            result_spans.push(Span::styled(selected, span.style.bg(highlight_bg)));

            if hl_end < chars.len() {
                let after: String = chars[hl_end..].iter().collect();
                result_spans.push(Span::styled(after, span.style));
            }
        }

        col = span_end;
    }

    Line::from(result_spans)
}

/// Safe markdown renderer — handles headers, bold, italic, inline code, code blocks,
/// and list items with styled spans. Cannot panic (no third-party markdown crate).
fn safe_markdown_lines(text: &str, wrap_width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_code_block = false;

    for raw_line in text.lines() {
        // Code fence toggle
        if raw_line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            // Don't render the fence line itself
            continue;
        }

        if in_code_block {
            // Render code block content with a distinct style
            let styled = Line::from(Span::styled(
                raw_line.to_string(),
                Style::default().fg(Color::Green),
            ));
            lines.extend(wrap_styled_line(styled, wrap_width));
            continue;
        }

        // Headers: strip leading `#`s and style as bold
        if let Some(rest) = strip_heading_md(raw_line) {
            let styled = Line::from(Span::styled(
                rest,
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ));
            lines.extend(wrap_styled_line(styled, wrap_width));
            continue;
        }

        // List items: `- text` or `* text`
        let trimmed = raw_line.trim_start();
        let is_list = trimmed.starts_with("- ") || trimmed.starts_with("* ");
        let (prefix, body) = if is_list {
            let indent = raw_line.len() - trimmed.len();
            let bullet_prefix = format!("{}\u{2022} ", " ".repeat(indent));
            (bullet_prefix, &trimmed[2..])
        } else {
            (String::new(), raw_line)
        };

        // Parse inline styles: **bold**, *italic*, `code`
        let mut spans = Vec::new();
        if !prefix.is_empty() {
            spans.push(Span::styled(prefix, Style::default().fg(Color::Yellow)));
        }
        parse_inline_markdown(body, &mut spans);

        let styled_line = Line::from(spans);
        lines.extend(wrap_styled_line(styled_line, wrap_width));
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines
}

/// Strip `# ` / `## ` / `### ` etc. from the start of a line, returning the heading text.
fn strip_heading_md(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes > 0 && hashes <= 6 {
        let rest = trimmed[hashes..].trim_start();
        if !rest.is_empty() {
            return Some(rest.to_string());
        }
    }
    None
}

/// Parse inline markdown (`**bold**`, `*italic*`, `` `code` ``) into styled spans.
fn parse_inline_markdown(text: &str, spans: &mut Vec<Span<'static>>) {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut plain = String::new();
    let base_style = Style::default();

    while i < len {
        // Inline code: `...`
        if chars[i] == '`' {
            if !plain.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut plain), base_style));
            }
            i += 1;
            let mut code = String::new();
            while i < len && chars[i] != '`' {
                code.push(chars[i]);
                i += 1;
            }
            if i < len { i += 1; } // skip closing `
            spans.push(Span::styled(code, Style::default().fg(Color::Green)));
            continue;
        }

        // Bold: **...**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if !plain.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut plain), base_style));
            }
            i += 2;
            let mut bold = String::new();
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') {
                bold.push(chars[i]);
                i += 1;
            }
            if i + 1 < len { i += 2; } // skip closing **
            spans.push(Span::styled(bold, Style::default().add_modifier(Modifier::BOLD)));
            continue;
        }

        // Italic: *...*
        if chars[i] == '*' {
            if !plain.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut plain), base_style));
            }
            i += 1;
            let mut italic = String::new();
            while i < len && chars[i] != '*' {
                italic.push(chars[i]);
                i += 1;
            }
            if i < len { i += 1; } // skip closing *
            spans.push(Span::styled(italic, Style::default().add_modifier(Modifier::ITALIC)));
            continue;
        }

        plain.push(chars[i]);
        i += 1;
    }

    if !plain.is_empty() {
        spans.push(Span::styled(plain, base_style));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Async pipeline functions
// ─────────────────────────────────────────────────────────────────────────────

pub async fn chat_agent_pipeline(
    config: crate::core::config::EnrichmentConfig,
    system_prompt: String,
    messages: Vec<(String, String)>,
    user_message: String,
    needs_compaction: bool,
    tx: mpsc::Sender<ChatStreamEvent>,
) {
    use crate::agent::loop_runner::{AgentEvent, run_agent_loop};
    use crate::agent::providers::create_agent_provider;

    let provider = create_agent_provider(&config);

    let mut effective_system_prompt = system_prompt;
    let mut effective_messages = messages;

    // Auto-compact if context is above 80% and there are enough messages
    if needs_compaction && effective_messages.len() >= 4 {
        let _ = tx.send(ChatStreamEvent::Status(
            format!("Compacting context ({} messages)...", effective_messages.len()),
        )).await;

        let keep_count = 4; // keep last 4 messages
        let compact_end = effective_messages.len() - keep_count;
        let to_compact: Vec<(String, String)> = effective_messages[..compact_end].to_vec();
        let remaining: Vec<(String, String)> = effective_messages[compact_end..].to_vec();

        let prompt = chat_prompts::build_compaction_prompt(&to_compact);
        match provider.compact_history(&prompt).await {
            Ok(summary) => {
                let removed_count = compact_end;
                let _ = tx.send(ChatStreamEvent::Compacted {
                    summary: summary.clone(),
                    removed_count,
                }).await;
                let _ = tx.send(ChatStreamEvent::Status(
                    format!("Compacted {} messages into summary, {} kept", removed_count, remaining.len()),
                )).await;

                // Augment system prompt with the summary
                effective_system_prompt = format!(
                    "{}\n\n## Previous Conversation Summary\n{}",
                    effective_system_prompt, summary
                );
                effective_messages = remaining;
            }
            Err(e) => {
                let _ = tx.send(ChatStreamEvent::Error(
                    format!("Compaction failed: {}", e),
                )).await;
                return;
            }
        }
    }

    let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(100);

    // Spawn the agent loop
    let agent_handle = tokio::spawn(async move {
        run_agent_loop(provider, effective_system_prompt, effective_messages, user_message, agent_tx).await;
    });

    // Bridge AgentEvent -> ChatStreamEvent
    while let Some(event) = agent_rx.recv().await {
        let chat_event = match event {
            AgentEvent::Status(msg) => ChatStreamEvent::Status(msg),
            AgentEvent::Chunk(text) => ChatStreamEvent::Chunk(text),
            AgentEvent::ToolCall { name, input_summary } => {
                ChatStreamEvent::ToolCall { name, input_summary }
            }
            AgentEvent::ToolResult { name, output_summary } => {
                ChatStreamEvent::ToolResult { name, output_summary }
            }
            AgentEvent::Usage { input_tokens, output_tokens } => {
                ChatStreamEvent::Usage { input_tokens, output_tokens }
            }
            AgentEvent::Done => ChatStreamEvent::Done,
            AgentEvent::Error(msg) => ChatStreamEvent::Error(msg),
        };
        if tx.send(chat_event).await.is_err() {
            break;
        }
    }

    // If the agent loop panicked, report the error
    if let Err(e) = agent_handle.await {
        let _ = tx.send(ChatStreamEvent::Error(format!("Agent error: {}", e))).await;
    }
}
