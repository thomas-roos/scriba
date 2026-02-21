//! Chat prompt builders for the "Ask Scriba" conversational AI feature.

/// Build the system prompt for the global (home page) chat context.
pub fn build_global_chat_prompt(
    owner_name: &str,
    world_json: &str,
    stats_summary: &str,
    recent_recordings: &str,
    entities_summary: &str,
) -> String {
    format!(
        r#"You are Scriba, a wise and friendly owl who serves as {owner}'s personal audio assistant. You record, transcribe, and remember everything. You speak concisely and helpfully, with occasional owl-like charm.

## Your Knowledge

### World Context
{world}

### Recording Statistics
{stats}

### Recent Recordings
{recent}

### Known Entities
{entities}

## Guidelines
- Answer questions about recordings, people, organizations, and topics you know about.
- When asked to summarize or find information, reference specific recordings when possible.
- Be concise — most answers should be 2-5 sentences unless the user asks for detail.
- If you don't have enough information, say so honestly.
- You can help draft emails, summarize themes, find patterns across recordings, and answer questions about the owner's world.
- Never fabricate information about recordings or entities you don't know about."#,
        owner = owner_name,
        world = world_json,
        stats = stats_summary,
        recent = recent_recordings,
        entities = entities_summary,
    )
}

/// Build the system prompt for a recording-specific chat context.
pub fn build_recording_chat_prompt(
    owner_name: &str,
    recording_name: &str,
    transcript: &str,
    summary: &str,
    topics: &str,
    entities: &str,
    key_points: &str,
    action_items: &str,
    world_json: &str,
) -> String {
    // Truncate transcript to ~6000 words to stay within context limits
    let truncated_transcript = truncate_to_words(transcript, 6000);

    format!(
        r#"You are Scriba, a wise and friendly owl who serves as {owner}'s personal audio assistant. You are currently helping with a specific recording.

## Recording: {name}

### Summary
{summary}

### Topics
{topics}

### Key Points
{key_points}

### Action Items
{action_items}

### Entities Mentioned
{entities}

### Full Transcript
{transcript}

### World Context
{world}

## Guidelines
- Answer questions specifically about this recording and its content.
- Reference specific parts of the transcript when relevant.
- You can summarize, extract action items, draft follow-up emails, or explain what was discussed.
- Be concise — most answers should be 2-5 sentences unless the user asks for detail.
- If asked about topics not in this recording, mention that and suggest checking the global view."#,
        owner = owner_name,
        name = recording_name,
        summary = if summary.is_empty() { "(no summary available)" } else { summary },
        topics = if topics.is_empty() { "(no topics extracted)" } else { topics },
        key_points = if key_points.is_empty() { "(no key points extracted)" } else { key_points },
        action_items = if action_items.is_empty() { "(no action items)" } else { action_items },
        entities = if entities.is_empty() { "(no entities)" } else { entities },
        transcript = truncated_transcript,
        world = world_json,
    )
}

/// Assemble a full conversation prompt from system prompt + history + latest message.
pub fn build_conversation(
    system_prompt: &str,
    history: &[(String, String)], // (role, content) pairs
    latest_user_msg: &str,
    extra_context: &str,
) -> String {
    let mut prompt = format!("SYSTEM:\n{}\n\n", system_prompt);

    // Include last 16 messages of history
    let history_slice = if history.len() > 16 {
        &history[history.len() - 16..]
    } else {
        history
    };

    for (role, content) in history_slice {
        prompt.push_str(&format!("{}:\n{}\n\n", role.to_uppercase(), content));
    }

    if !extra_context.is_empty() {
        prompt.push_str(&format!("CONTEXT (related recordings):\n{}\n\n", extra_context));
    }

    prompt.push_str(&format!("USER:\n{}\n\nASSISTANT:\n", latest_user_msg));
    prompt
}

/// Build a specialized prompt for drafting follow-up emails.
pub fn build_email_draft_prompt(
    recording_name: &str,
    summary: &str,
    action_items: &str,
    key_points: &str,
) -> String {
    format!(
        r#"Based on this recording, draft a concise follow-up email.

Recording: {name}
Summary: {summary}
Action Items: {actions}
Key Points: {points}

Write a professional but friendly follow-up email that:
1. Thanks participants for the discussion
2. Summarizes key decisions
3. Lists action items with owners (if mentioned)
4. Notes any follow-up meetings or deadlines

Keep it concise and professional."#,
        name = recording_name,
        summary = summary,
        actions = if action_items.is_empty() { "(none)" } else { action_items },
        points = if key_points.is_empty() { "(none)" } else { key_points },
    )
}

/// Format related recordings as context for cross-referencing.
pub fn format_related_recordings(entity_name: &str, recordings: &[(String, String)]) -> String {
    let mut result = format!("\n## Recordings mentioning \"{}\":\n", entity_name);
    for (name, summary) in recordings.iter().take(5) {
        result.push_str(&format!("- {}: {}\n", name, summary));
    }
    result
}

/// Truncate text to approximately `max_words` words.
fn truncate_to_words(text: &str, max_words: usize) -> &str {
    let mut word_count = 0;
    let mut last_end = 0;

    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            word_count += 1;
            if word_count >= max_words {
                return &text[..i];
            }
            last_end = i;
        }
    }

    let _ = last_end;
    text
}
