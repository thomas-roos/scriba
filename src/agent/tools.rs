//! Tool definitions and executor for the Scriba agent.
//!
//! Each tool maps to existing Database/WorldContext methods and is read-only.

use crate::database::Database;
use crate::enrichment::WorldContext;
use serde_json::{json, Value};

/// Return all tool definitions as Anthropic-format JSON schemas.
pub fn all_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "list_recordings",
            "description": "List all recordings with metadata (id, name, date, duration, summary snippet). Use this to see what recordings are available.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of recordings to return. Omit for all."
                    }
                },
                "required": []
            }
        }),
        json!({
            "name": "get_recording",
            "description": "Get full recording metadata including summary, topics, key_points, action_items, and speakers.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "Recording ID"
                    }
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "get_transcript",
            "description": "Fetch the full transcript text for a recording. Returns the entire content with no word limit.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "recording_id": {
                        "type": "integer",
                        "description": "Recording ID to fetch transcript for"
                    }
                },
                "required": ["recording_id"]
            }
        }),
        json!({
            "name": "search_transcripts",
            "description": "Full-text search across all transcripts. Returns matching recording IDs with snippets. Use this to find recordings about a topic.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query (FTS5 syntax supported)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results to return. Default: 10."
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "list_entities",
            "description": "List known entities (people, organizations) with context and mention counts.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "entity_type": {
                        "type": "string",
                        "description": "Filter by type: 'person' or 'organization'. Omit for all.",
                        "enum": ["person", "organization"]
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum entities to return. Omit for all."
                    }
                },
                "required": []
            }
        }),
        json!({
            "name": "get_entity",
            "description": "Get entity details including all mentions and which recordings reference this entity.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "Entity ID"
                    }
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "get_recordings_for_entity",
            "description": "Find all recordings that mention a specific entity. Great for cross-correlating information about a person or organization.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "entity_id": {
                        "type": "integer",
                        "description": "Entity ID to find recordings for"
                    }
                },
                "required": ["entity_id"]
            }
        }),
        json!({
            "name": "get_world_context",
            "description": "Read the full world.md knowledge base containing info about the owner, their relationships, organizations, projects, and beliefs.",
            "input_schema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "get_stats",
            "description": "Get recording statistics: total recordings, total hours, total words, entity counts.",
            "input_schema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
    ]
}

/// Execute a tool by name with the given parameters.
/// Returns the tool output as a string (JSON or plain text).
pub fn execute_tool(name: &str, input: &Value, db: &Database) -> String {
    match name {
        "list_recordings" => exec_list_recordings(input, db),
        "get_recording" => exec_get_recording(input, db),
        "get_transcript" => exec_get_transcript(input, db),
        "search_transcripts" => exec_search_transcripts(input, db),
        "list_entities" => exec_list_entities(input, db),
        "get_entity" => exec_get_entity(input, db),
        "get_recordings_for_entity" => exec_get_recordings_for_entity(input, db),
        "get_world_context" => exec_get_world_context(),
        "get_stats" => exec_get_stats(db),
        _ => format!("Unknown tool: {}", name),
    }
}

/// Summarize a tool result for display in the UI (short one-liner).
pub fn summarize_tool_result(name: &str, result: &str) -> String {
    let char_count = result.len();
    match name {
        "list_recordings" => {
            let count = result.matches("\"id\"").count();
            format!("{} recordings", count)
        }
        "get_recording" => {
            if result.contains("not found") {
                "not found".to_string()
            } else {
                format!("{} chars", char_count)
            }
        }
        "get_transcript" => {
            let word_count = result.split_whitespace().count();
            format!("{} words", word_count)
        }
        "search_transcripts" => {
            let count = result.matches("\"recording_id\"").count();
            format!("{} results", count)
        }
        "list_entities" => {
            let count = result.matches("\"id\"").count();
            format!("{} entities", count)
        }
        "get_entity" => {
            if result.contains("not found") {
                "not found".to_string()
            } else {
                format!("{} chars", char_count)
            }
        }
        "get_recordings_for_entity" => {
            let count = result.matches("\"id\"").count();
            format!("{} recordings", count)
        }
        "get_world_context" => {
            if result.is_empty() || result == "(no world context)" {
                "empty".to_string()
            } else {
                let word_count = result.split_whitespace().count();
                format!("{} words", word_count)
            }
        }
        "get_stats" => "ok".to_string(),
        _ => format!("{} chars", char_count),
    }
}

// ─── Tool Implementations ────────────────────────────────────────────────────

fn exec_list_recordings(input: &Value, db: &Database) -> String {
    let limit = input.get("limit").and_then(|v| v.as_i64());
    match db.list_recordings(limit, None) {
        Ok(recordings) => {
            let items: Vec<Value> = recordings
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "name": r.display_name.as_ref().unwrap_or(&r.directory_name),
                        "created_at": r.created_at.to_rfc3339(),
                        "duration_seconds": r.duration_seconds,
                        "has_transcript": r.has_transcript,
                        "summary": r.summary.as_deref().unwrap_or("").chars().take(200).collect::<String>(),
                    })
                })
                .collect();
            serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string())
        }
        Err(e) => format!("Error listing recordings: {}", e),
    }
}

fn exec_get_recording(input: &Value, db: &Database) -> String {
    let id = match input.get("id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => return "Error: missing 'id' parameter".to_string(),
    };
    match db.get_recording(id) {
        Ok(Some(r)) => {
            let result = json!({
                "id": r.id,
                "name": r.display_name.as_ref().unwrap_or(&r.directory_name),
                "directory_name": r.directory_name,
                "created_at": r.created_at.to_rfc3339(),
                "duration_seconds": r.duration_seconds,
                "has_transcript": r.has_transcript,
                "language": r.language_code,
                "summary": r.summary,
                "key_points": r.key_points,
                "action_items": r.action_items,
                "speakers": r.speakers,
                "topics": r.categories,
                "tags": r.tags,
            });
            serde_json::to_string_pretty(&result).unwrap_or_default()
        }
        Ok(None) => format!("Recording {} not found", id),
        Err(e) => format!("Error: {}", e),
    }
}

fn exec_get_transcript(input: &Value, db: &Database) -> String {
    let recording_id = match input.get("recording_id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => return "Error: missing 'recording_id' parameter".to_string(),
    };
    match db.get_transcript_by_recording_id(recording_id) {
        Ok(Some(t)) => t.content,
        Ok(None) => format!("No transcript found for recording {}", recording_id),
        Err(e) => format!("Error: {}", e),
    }
}

fn exec_search_transcripts(input: &Value, db: &Database) -> String {
    let query = match input.get("query").and_then(|v| v.as_str()) {
        Some(q) => q,
        None => return "Error: missing 'query' parameter".to_string(),
    };
    let limit = input.get("limit").and_then(|v| v.as_i64()).or(Some(10));
    match db.search_transcripts(query, limit) {
        Ok(results) => {
            let items: Vec<Value> = results
                .iter()
                .map(|(r, t)| {
                    // Extract a snippet around the match
                    let snippet = extract_snippet(&t.content, query, 150);
                    json!({
                        "recording_id": r.id,
                        "recording_name": r.display_name.as_ref().unwrap_or(&r.directory_name),
                        "created_at": r.created_at.to_rfc3339(),
                        "word_count": t.word_count,
                        "snippet": snippet,
                    })
                })
                .collect();
            serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string())
        }
        Err(e) => format!("Error searching: {}", e),
    }
}

fn exec_list_entities(input: &Value, db: &Database) -> String {
    let entity_type = input.get("entity_type").and_then(|v| v.as_str());
    let limit = input.get("limit").and_then(|v| v.as_i64());
    match db.list_entities(entity_type, limit) {
        Ok(entities) => {
            let items: Vec<Value> = entities
                .iter()
                .map(|e| {
                    json!({
                        "id": e.id,
                        "type": e.entity_type,
                        "name": e.canonical_name,
                        "aliases": e.aliases_list(),
                        "context": e.context,
                        "mention_count": e.mention_count,
                    })
                })
                .collect();
            serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string())
        }
        Err(e) => format!("Error: {}", e),
    }
}

fn exec_get_entity(input: &Value, db: &Database) -> String {
    let id = match input.get("id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => return "Error: missing 'id' parameter".to_string(),
    };
    let entity = match db.get_entity(id) {
        Ok(Some(e)) => e,
        Ok(None) => return format!("Entity {} not found", id),
        Err(e) => return format!("Error: {}", e),
    };
    let mentions = db.get_mentions_for_entity(id).unwrap_or_default();
    let recordings = db.get_recordings_for_entity(id).unwrap_or_default();

    let result = json!({
        "id": entity.id,
        "type": entity.entity_type,
        "name": entity.canonical_name,
        "aliases": entity.aliases_list(),
        "context": entity.context,
        "mention_count": entity.mention_count,
        "mentions": mentions.iter().take(20).map(|m| {
            json!({
                "recording_id": m.recording_id,
                "mention_text": m.mention_text,
                "context_snippet": m.context_snippet,
            })
        }).collect::<Vec<_>>(),
        "recordings": recordings.iter().map(|r| {
            json!({
                "id": r.id,
                "name": r.display_name.as_ref().unwrap_or(&r.directory_name),
                "summary": r.summary.as_deref().unwrap_or("").chars().take(150).collect::<String>(),
            })
        }).collect::<Vec<_>>(),
    });
    serde_json::to_string_pretty(&result).unwrap_or_default()
}

fn exec_get_recordings_for_entity(input: &Value, db: &Database) -> String {
    let entity_id = match input.get("entity_id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => return "Error: missing 'entity_id' parameter".to_string(),
    };
    match db.get_recordings_for_entity(entity_id) {
        Ok(recordings) => {
            let items: Vec<Value> = recordings
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "name": r.display_name.as_ref().unwrap_or(&r.directory_name),
                        "created_at": r.created_at.to_rfc3339(),
                        "summary": r.summary.as_deref().unwrap_or("").chars().take(200).collect::<String>(),
                    })
                })
                .collect();
            serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string())
        }
        Err(e) => format!("Error: {}", e),
    }
}

fn exec_get_world_context() -> String {
    match WorldContext::load() {
        Ok(wc) => {
            if wc.content.is_empty() {
                "(no world context)".to_string()
            } else {
                wc.content
            }
        }
        Err(e) => format!("Error loading world context: {}", e),
    }
}

fn exec_get_stats(db: &Database) -> String {
    match db.get_stats() {
        Ok(stats) => {
            let entity_count = db.list_entities(None, None).map(|e| e.len()).unwrap_or(0);
            let result = json!({
                "total_recordings": stats.total_recordings,
                "total_duration": stats.format_duration(),
                "total_size": stats.format_size(),
                "transcribed_count": stats.transcribed_count,
                "total_words": stats.total_words,
                "total_entities": entity_count,
            });
            serde_json::to_string_pretty(&result).unwrap_or_default()
        }
        Err(e) => format!("Error: {}", e),
    }
}

/// Extract a snippet of text around a search query match.
/// Uses char-level indexing to avoid panics on multi-byte UTF-8.
fn extract_snippet(content: &str, query: &str, max_chars: usize) -> String {
    let content_lower = content.to_lowercase();
    let first_word = query.split_whitespace().next().unwrap_or(query).to_lowercase();

    // Find byte position in the lowered string, then convert to char offset
    let match_char_offset = if let Some(byte_pos) = content_lower.find(&first_word) {
        content_lower[..byte_pos].chars().count()
    } else {
        return content.chars().take(max_chars).collect::<String>();
    };

    let total_chars = content.chars().count();
    let half = max_chars / 2;
    let start_char = match_char_offset.saturating_sub(half);
    let end_char = (match_char_offset + half).min(total_chars);

    let snippet: String = content.chars().skip(start_char).take(end_char - start_char).collect();

    // Trim to word boundaries
    let snippet = if start_char > 0 {
        if let Some(pos) = snippet.find(' ') {
            snippet[pos + 1..].to_string()
        } else {
            snippet
        }
    } else {
        snippet
    };
    let snippet = if end_char < total_chars {
        if let Some(pos) = snippet.rfind(' ') {
            snippet[..pos].to_string()
        } else {
            snippet
        }
    } else {
        snippet
    };

    if start_char > 0 {
        format!("...{}...", snippet)
    } else {
        format!("{}...", snippet)
    }
}
