use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::database::{Database, Entity, EntityMentionRecord, Recording, Transcript};
use crate::entities::EntityRegistry;

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse<'a> {
    jsonrpc: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListTranscriptsParams {
    /// Maximum number of items to return
    limit: Option<i64>,
    /// Number of items to skip
    offset: Option<i64>,
    /// Include recordings without transcripts
    include_without_transcripts: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetTranscriptParams {
    /// Recording ID to fetch transcript for
    recording_id: Option<i64>,
    /// Directory name to fetch transcript for
    directory_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchTranscriptsParams {
    /// Search query for full-text search
    query: String,
    /// Maximum number of results to return
    limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListEntitiesParams {
    /// Filter by entity type: "person", "organization", or null for all
    entity_type: Option<String>,
    /// Maximum number of items to return
    limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetEntityParams {
    /// Entity ID to fetch
    entity_id: Option<i64>,
    /// Entity name to fetch (case-insensitive)
    name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchByEntityParams {
    /// Entity ID to search recordings for
    entity_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpdateEntityParams {
    /// Entity ID to update
    entity_id: i64,
    /// New name for the entity (old name becomes an alias)
    new_name: Option<String>,
    /// New context description
    new_context: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AddEntityAliasParams {
    /// Entity ID to add alias to
    entity_id: i64,
    /// Alias to add
    alias: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RemoveEntityAliasParams {
    /// Entity ID to remove alias from
    entity_id: i64,
    /// Alias to remove
    alias: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MergeEntitiesParams {
    /// Source entity ID (will be deleted after merge)
    source_id: i64,
    /// Target entity ID (will receive merged data)
    target_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteEntityParams {
    /// Entity ID to delete
    entity_id: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
struct RecordingInfo {
    id: Option<i64>,
    directory_name: String,
    display_name: Option<String>,
    created_at: String,
    updated_at: String,
    has_transcript: bool,
    transcript_status: Option<String>,
    language_code: Option<String>,
    model_used: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_points: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    action_items: Option<Vec<String>>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct TranscriptInfo {
    id: Option<i64>,
    recording_id: i64,
    created_at: String,
    updated_at: String,
    word_count: Option<i64>,
    character_count: Option<i64>,
    language_detected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entities: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    topics: Option<Vec<String>>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct SearchResult {
    recording: RecordingInfo,
    transcript: TranscriptInfo,
}

#[derive(Debug, Serialize, JsonSchema)]
struct EntityInfo {
    id: Option<i64>,
    entity_type: String,
    canonical_name: String,
    aliases: Option<Vec<String>>,
    context: Option<String>,
    mention_count: i64,
    first_seen_at: Option<String>,
    last_seen_at: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct EntityMentionInfo {
    id: Option<i64>,
    entity_id: Option<i64>,
    recording_id: i64,
    mention_text: String,
    context_snippet: Option<String>,
    confidence: f64,
}

#[derive(Debug, Serialize, JsonSchema)]
struct EntityDetailInfo {
    entity: EntityInfo,
    mentions: Vec<EntityMentionInfo>,
    recordings: Vec<RecordingInfo>,
}

fn make_error(code: i64, message: impl Into<String>, data: Option<Value>) -> JsonRpcError {
    JsonRpcError {
        code,
        message: message.into(),
        data,
    }
}

// MCP STDIO transport: newline-delimited JSON messages
async fn read_json_message(reader: &mut BufReader<tokio::io::Stdin>) -> Result<Option<String>> {
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line).await?;

    if bytes_read == 0 {
        return Ok(None); // EOF
    }

    // Remove trailing newline
    let trimmed = line.trim_end().to_string();
    if trimmed.is_empty() {
        // Skip empty lines
        return Ok(Some(String::new()));
    }

    Ok(Some(trimmed))
}

async fn write_json_message(writer: &mut tokio::io::Stdout, payload: &Value) -> Result<()> {
    let json_str = serde_json::to_string(payload)?;
    writer.write_all(json_str.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

fn response_ok(id: Option<Value>, result: Value) -> Value {
    serde_json::to_value(JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    })
    .unwrap()
}

fn response_err(id: Option<Value>, err: JsonRpcError) -> Value {
    serde_json::to_value(JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(err),
    })
    .unwrap()
}

fn recording_to_info(recording: &Recording) -> RecordingInfo {
    RecordingInfo {
        id: recording.id,
        directory_name: recording.directory_name.clone(),
        display_name: recording.display_name.clone(),
        created_at: recording.created_at.to_rfc3339(),
        updated_at: recording.updated_at.to_rfc3339(),
        has_transcript: recording.has_transcript,
        transcript_status: Some(recording.transcript_status.clone()),
        language_code: Some(recording.language_code.clone()),
        model_used: Some(recording.model_used.clone()),
        summary: recording.summary.clone(),
        key_points: recording.key_points.as_ref().and_then(|s| serde_json::from_str(s).ok()),
        action_items: recording.action_items.as_ref().and_then(|s| serde_json::from_str(s).ok()),
    }
}

fn transcript_to_info(transcript: &Transcript) -> TranscriptInfo {
    TranscriptInfo {
        id: transcript.id,
        recording_id: transcript.recording_id,
        created_at: transcript.created_at.to_rfc3339(),
        updated_at: transcript.updated_at.to_rfc3339(),
        word_count: transcript.word_count,
        character_count: transcript.character_count,
        language_detected: transcript.language_detected.clone(),
        entities: transcript.entities.as_ref().and_then(|s| serde_json::from_str(s).ok()),
        topics: transcript.topics.as_ref().and_then(|s| serde_json::from_str(s).ok()),
    }
}

fn entity_to_info(entity: &Entity) -> EntityInfo {
    EntityInfo {
        id: entity.id,
        entity_type: entity.entity_type.clone(),
        canonical_name: entity.canonical_name.clone(),
        aliases: entity.aliases.as_ref().and_then(|s| serde_json::from_str(s).ok()),
        context: entity.context.clone(),
        mention_count: entity.mention_count,
        first_seen_at: entity.first_seen_at.map(|dt| dt.to_rfc3339()),
        last_seen_at: entity.last_seen_at.map(|dt| dt.to_rfc3339()),
    }
}

fn mention_to_info(mention: &EntityMentionRecord) -> EntityMentionInfo {
    EntityMentionInfo {
        id: mention.id,
        entity_id: mention.entity_id,
        recording_id: mention.recording_id,
        mention_text: mention.mention_text.clone(),
        context_snippet: mention.context_snippet.clone(),
        confidence: mention.confidence,
    }
}

fn tool_schema_list_transcripts() -> Value {
    let schema = schemars::schema_for!(ListTranscriptsParams);
    json!({
        "name": "list_transcripts",
        "description": "List recordings with transcripts, newest first. Returns recording metadata including creation dates, transcript status, and audio format information.",
        "inputSchema": schema
    })
}

fn tool_schema_get_transcript() -> Value {
    let schema = schemars::schema_for!(GetTranscriptParams);
    json!({
        "name": "get_transcript",
        "description": "Fetch the full transcript content for a specific recording by ID or directory name. Returns the complete transcribed text.",
        "inputSchema": schema
    })
}

fn tool_schema_search_transcripts() -> Value {
    let schema = schemars::schema_for!(SearchTranscriptsParams);
    json!({
        "name": "search_transcripts",
        "description": "Full-text search across all transcripts using SQLite FTS. Supports complex queries and phrase matching.",
        "inputSchema": schema
    })
}

fn tool_schema_get_recording_info() -> Value {
    let schema = schemars::schema_for!(GetTranscriptParams);
    json!({
        "name": "get_recording_info",
        "description": "Get detailed metadata about a specific recording including audio format, duration, file size, and transcript status.",
        "inputSchema": schema
    })
}

fn tool_schema_list_entities() -> Value {
    let schema = schemars::schema_for!(ListEntitiesParams);
    json!({
        "name": "list_entities",
        "description": "List all known entities (people, organizations) with their context and mention counts. Use this to discover who and what has been discussed across recordings.",
        "inputSchema": schema
    })
}

fn tool_schema_get_entity() -> Value {
    let schema = schemars::schema_for!(GetEntityParams);
    json!({
        "name": "get_entity",
        "description": "Get full details about a specific entity including all mentions across recordings and accumulated context.",
        "inputSchema": schema
    })
}

fn tool_schema_search_by_entity() -> Value {
    let schema = schemars::schema_for!(SearchByEntityParams);
    json!({
        "name": "search_by_entity",
        "description": "Find all recordings that mention a specific entity. Returns recordings with context about how the entity was mentioned.",
        "inputSchema": schema
    })
}

fn tool_schema_update_entity() -> Value {
    let schema = schemars::schema_for!(UpdateEntityParams);
    json!({
        "name": "update_entity",
        "description": "Update an entity's name or context. When renaming, the old name is automatically added as an alias for smart matching in future transcripts.",
        "inputSchema": schema
    })
}

fn tool_schema_add_entity_alias() -> Value {
    let schema = schemars::schema_for!(AddEntityAliasParams);
    json!({
        "name": "add_entity_alias",
        "description": "Add an alias to an entity. Aliases are used for smart matching - if a transcript mentions the alias, it will be linked to this entity.",
        "inputSchema": schema
    })
}

fn tool_schema_remove_entity_alias() -> Value {
    let schema = schemars::schema_for!(RemoveEntityAliasParams);
    json!({
        "name": "remove_entity_alias",
        "description": "Remove an alias from an entity.",
        "inputSchema": schema
    })
}

fn tool_schema_merge_entities() -> Value {
    let schema = schemars::schema_for!(MergeEntitiesParams);
    json!({
        "name": "merge_entities",
        "description": "Merge two entities into one. The source entity's name becomes an alias of the target, all mentions are transferred, and contexts are combined. Source entity is deleted.",
        "inputSchema": schema
    })
}

fn tool_schema_delete_entity() -> Value {
    let schema = schemars::schema_for!(DeleteEntityParams);
    json!({
        "name": "delete_entity",
        "description": "Delete an entity from the knowledge base. Mentions will be unlinked but not deleted.",
        "inputSchema": schema
    })
}

async fn handle_initialize(id: Option<Value>, params: Option<Value>) -> Value {
    let mut protocol_version = "2024-11-05".to_string();

    if let Some(Value::Object(p)) = params {
        if let Some(Value::String(v)) = p.get("protocolVersion") {
            protocol_version = v.clone();
        }
    }

    // Accept any protocol version for compatibility
    let result = json!({
        "protocolVersion": protocol_version,
        "serverInfo": {
            "name": "scriba-mcp",
            "version": env!("CARGO_PKG_VERSION")
        },
        "capabilities": {
            "tools": {
                "listChanged": false
            },
            "resources": {
                "subscribe": false,
                "listChanged": false
            },
            "prompts": {
                "listChanged": false
            },
            "logging": {}
        }
    });

    response_ok(id, result)
}

async fn handle_tools_list(id: Option<Value>) -> Value {
    let tools = vec![
        tool_schema_list_transcripts(),
        tool_schema_get_transcript(),
        tool_schema_search_transcripts(),
        tool_schema_get_recording_info(),
        tool_schema_list_entities(),
        tool_schema_get_entity(),
        tool_schema_search_by_entity(),
        tool_schema_update_entity(),
        tool_schema_add_entity_alias(),
        tool_schema_remove_entity_alias(),
        tool_schema_merge_entities(),
        tool_schema_delete_entity(),
    ];
    response_ok(id, json!({"tools": tools}))
}

async fn handle_tools_call(id: Option<Value>, params: Option<Value>) -> Value {
    let Some(Value::Object(map)) = params else {
        return response_err(id, make_error(-32602, "Missing params", None));
    };
    let name = match map.get("name").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return response_err(id, make_error(-32602, "Missing tool name", None)),
    };
    let args = map.get("arguments").cloned().unwrap_or_else(|| json!({}));

    // Create fresh database connection to ensure latest data from all Scriba instances
    let db = match Database::new() {
        Ok(database) => database,
        Err(e) => return response_err(id, make_error(-32000, format!("Database error: {}", e), None)),
    };

    match name {
        "list_transcripts" => {
            let params: ListTranscriptsParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => return response_err(id, make_error(-32602, format!("Invalid params: {}", e), None)),
            };

            let recordings = match db.list_recordings(params.limit, params.offset) {
                Ok(list) => list,
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            let include_without = params.include_without_transcripts.unwrap_or(false);
            let filtered: Vec<_> = recordings
                .into_iter()
                .filter(|r| include_without || r.has_transcript)
                .map(|r| recording_to_info(&r))
                .collect();

            response_ok(id, json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&filtered).unwrap()
                }],
                "isError": false
            }))
        }
        "get_transcript" => {
            let params: GetTranscriptParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => return response_err(id, make_error(-32602, format!("Invalid params: {}", e), None)),
            };

            let recording_id = if let Some(idv) = params.recording_id {
                idv
            } else if let Some(dn) = params.directory_name {
                match db.get_recording_by_directory(&dn) {
                    Ok(Some(r)) => r.id.unwrap_or_default(),
                    Ok(None) => return response_err(id, make_error(404, "Recording not found", None)),
                    Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
                }
            } else {
                return response_err(id, make_error(-32602, "Provide recording_id or directory_name", None));
            };

            let transcript = match db.get_transcript_by_recording_id(recording_id) {
                Ok(Some(t)) => t,
                Ok(None) => return response_err(id, make_error(404, "Transcript not found", None)),
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            response_ok(id, json!({
                "content": [{
                    "type": "text",
                    "text": transcript.content
                }],
                "isError": false
            }))
        }
        "search_transcripts" => {
            let params: SearchTranscriptsParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => return response_err(id, make_error(-32602, format!("Invalid params: {}", e), None)),
            };

            let results = match db.search_transcripts(&params.query, params.limit) {
                Ok(v) => v,
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            let search_results: Vec<_> = results
                .iter()
                .map(|(r, t)| SearchResult {
                    recording: recording_to_info(r),
                    transcript: transcript_to_info(t),
                })
                .collect();

            response_ok(id, json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&search_results).unwrap()
                }],
                "isError": false
            }))
        }
        "get_recording_info" => {
            let params: GetTranscriptParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => return response_err(id, make_error(-32602, format!("Invalid params: {}", e), None)),
            };

            let recording = if let Some(id_val) = params.recording_id {
                match db.get_recording(id_val) {
                    Ok(Some(r)) => r,
                    Ok(None) => return response_err(id, make_error(404, "Recording not found", None)),
                    Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
                }
            } else if let Some(dir_name) = params.directory_name {
                match db.get_recording_by_directory(&dir_name) {
                    Ok(Some(r)) => r,
                    Ok(None) => return response_err(id, make_error(404, "Recording not found", None)),
                    Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
                }
            } else {
                return response_err(id, make_error(-32602, "Provide recording_id or directory_name", None));
            };

            let recording_info = recording_to_info(&recording);
            response_ok(id, json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&recording_info).unwrap()
                }],
                "isError": false
            }))
        }
        "list_entities" => {
            let params: ListEntitiesParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => return response_err(id, make_error(-32602, format!("Invalid params: {}", e), None)),
            };

            let entities = match db.list_entities(params.entity_type.as_deref(), params.limit) {
                Ok(list) => list,
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            let entity_infos: Vec<_> = entities.iter().map(entity_to_info).collect();

            response_ok(id, json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&entity_infos).unwrap()
                }],
                "isError": false
            }))
        }
        "get_entity" => {
            let params: GetEntityParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => return response_err(id, make_error(-32602, format!("Invalid params: {}", e), None)),
            };

            let entity = if let Some(entity_id) = params.entity_id {
                match db.get_entity(entity_id) {
                    Ok(Some(e)) => e,
                    Ok(None) => return response_err(id, make_error(404, "Entity not found", None)),
                    Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
                }
            } else if let Some(name) = params.name {
                match db.get_entity_by_name(&name) {
                    Ok(Some(e)) => e,
                    Ok(None) => return response_err(id, make_error(404, "Entity not found", None)),
                    Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
                }
            } else {
                return response_err(id, make_error(-32602, "Provide entity_id or name", None));
            };

            let entity_id = entity.id.unwrap();

            // Get mentions for this entity
            let mentions = match db.get_mentions_for_entity(entity_id) {
                Ok(m) => m,
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            // Get recordings for this entity
            let recordings = match db.get_recordings_for_entity(entity_id) {
                Ok(r) => r,
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            let detail = EntityDetailInfo {
                entity: entity_to_info(&entity),
                mentions: mentions.iter().map(mention_to_info).collect(),
                recordings: recordings.iter().map(recording_to_info).collect(),
            };

            response_ok(id, json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&detail).unwrap()
                }],
                "isError": false
            }))
        }
        "search_by_entity" => {
            let params: SearchByEntityParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => return response_err(id, make_error(-32602, format!("Invalid params: {}", e), None)),
            };

            // Get entity details
            let entity = match db.get_entity(params.entity_id) {
                Ok(Some(e)) => e,
                Ok(None) => return response_err(id, make_error(404, "Entity not found", None)),
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            // Get recordings for this entity
            let recordings = match db.get_recordings_for_entity(params.entity_id) {
                Ok(r) => r,
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            // Get mentions to provide context
            let mentions = match db.get_mentions_for_entity(params.entity_id) {
                Ok(m) => m,
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            let result = json!({
                "entity": entity_to_info(&entity),
                "recording_count": recordings.len(),
                "recordings": recordings.iter().map(recording_to_info).collect::<Vec<_>>(),
                "mention_contexts": mentions.iter().map(|m| json!({
                    "recording_id": m.recording_id,
                    "mention_text": m.mention_text,
                    "context_snippet": m.context_snippet
                })).collect::<Vec<_>>()
            });

            response_ok(id, json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&result).unwrap()
                }],
                "isError": false
            }))
        }
        "update_entity" => {
            let params: UpdateEntityParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => return response_err(id, make_error(-32602, format!("Invalid params: {}", e), None)),
            };

            // Need mutable database for updates
            let mut db = match Database::new() {
                Ok(database) => database,
                Err(e) => return response_err(id, make_error(-32000, format!("Database error: {}", e), None)),
            };
            let mut registry = EntityRegistry::new(&mut db);

            // Check entity exists
            let entity = match registry.get_entity(params.entity_id) {
                Ok(Some(e)) => e,
                Ok(None) => return response_err(id, make_error(404, "Entity not found", None)),
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            let old_name = entity.canonical_name.clone();
            let mut changes = Vec::new();

            // Rename if new_name provided
            if let Some(new_name) = &params.new_name {
                if let Err(e) = registry.rename_entity(params.entity_id, new_name) {
                    return response_err(id, make_error(-32000, format!("Rename error: {}", e), None));
                }
                changes.push(format!("Renamed from '{}' to '{}' (old name added as alias)", old_name, new_name));
            }

            // Update context if provided
            if let Some(new_context) = &params.new_context {
                if let Err(e) = registry.update_entity_context(params.entity_id, new_context) {
                    return response_err(id, make_error(-32000, format!("Context update error: {}", e), None));
                }
                changes.push("Context updated".to_string());
            }

            // Get updated entity
            let updated = match registry.get_entity(params.entity_id) {
                Ok(Some(e)) => e,
                Ok(None) => return response_err(id, make_error(404, "Entity not found after update", None)),
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            let result = json!({
                "success": true,
                "changes": changes,
                "entity": entity_to_info(&updated)
            });

            response_ok(id, json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&result).unwrap()
                }],
                "isError": false
            }))
        }
        "add_entity_alias" => {
            let params: AddEntityAliasParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => return response_err(id, make_error(-32602, format!("Invalid params: {}", e), None)),
            };

            let mut db = match Database::new() {
                Ok(database) => database,
                Err(e) => return response_err(id, make_error(-32000, format!("Database error: {}", e), None)),
            };
            let mut registry = EntityRegistry::new(&mut db);

            // Check entity exists
            match registry.get_entity(params.entity_id) {
                Ok(Some(_)) => {}
                Ok(None) => return response_err(id, make_error(404, "Entity not found", None)),
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            }

            if let Err(e) = registry.add_entity_alias(params.entity_id, &params.alias) {
                return response_err(id, make_error(-32000, format!("Error adding alias: {}", e), None));
            }

            let updated = match registry.get_entity(params.entity_id) {
                Ok(Some(e)) => e,
                Ok(None) => return response_err(id, make_error(404, "Entity not found", None)),
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            let result = json!({
                "success": true,
                "message": format!("Added alias '{}' to entity '{}'", params.alias, updated.canonical_name),
                "entity": entity_to_info(&updated)
            });

            response_ok(id, json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&result).unwrap()
                }],
                "isError": false
            }))
        }
        "remove_entity_alias" => {
            let params: RemoveEntityAliasParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => return response_err(id, make_error(-32602, format!("Invalid params: {}", e), None)),
            };

            let mut db = match Database::new() {
                Ok(database) => database,
                Err(e) => return response_err(id, make_error(-32000, format!("Database error: {}", e), None)),
            };
            let mut registry = EntityRegistry::new(&mut db);

            // Check entity exists
            match registry.get_entity(params.entity_id) {
                Ok(Some(_)) => {}
                Ok(None) => return response_err(id, make_error(404, "Entity not found", None)),
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            }

            if let Err(e) = registry.remove_entity_alias(params.entity_id, &params.alias) {
                return response_err(id, make_error(-32000, format!("Error removing alias: {}", e), None));
            }

            let updated = match registry.get_entity(params.entity_id) {
                Ok(Some(e)) => e,
                Ok(None) => return response_err(id, make_error(404, "Entity not found", None)),
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            let result = json!({
                "success": true,
                "message": format!("Removed alias '{}' from entity '{}'", params.alias, updated.canonical_name),
                "entity": entity_to_info(&updated)
            });

            response_ok(id, json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&result).unwrap()
                }],
                "isError": false
            }))
        }
        "merge_entities" => {
            let params: MergeEntitiesParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => return response_err(id, make_error(-32602, format!("Invalid params: {}", e), None)),
            };

            let mut db = match Database::new() {
                Ok(database) => database,
                Err(e) => return response_err(id, make_error(-32000, format!("Database error: {}", e), None)),
            };
            let mut registry = EntityRegistry::new(&mut db);

            // Get both entities first
            let source = match registry.get_entity(params.source_id) {
                Ok(Some(e)) => e,
                Ok(None) => return response_err(id, make_error(404, "Source entity not found", None)),
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };
            let target = match registry.get_entity(params.target_id) {
                Ok(Some(e)) => e,
                Ok(None) => return response_err(id, make_error(404, "Target entity not found", None)),
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            let source_name = source.canonical_name.clone();
            let target_name = target.canonical_name.clone();

            if let Err(e) = registry.merge_entities(params.source_id, params.target_id) {
                return response_err(id, make_error(-32000, format!("Merge error: {}", e), None));
            }

            // Get updated target entity
            let updated = match registry.get_entity(params.target_id) {
                Ok(Some(e)) => e,
                Ok(None) => return response_err(id, make_error(404, "Target entity not found after merge", None)),
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            let result = json!({
                "success": true,
                "message": format!("Merged '{}' into '{}'. Source entity deleted, name added as alias.", source_name, target_name),
                "entity": entity_to_info(&updated)
            });

            response_ok(id, json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&result).unwrap()
                }],
                "isError": false
            }))
        }
        "delete_entity" => {
            let params: DeleteEntityParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => return response_err(id, make_error(-32602, format!("Invalid params: {}", e), None)),
            };

            let mut db = match Database::new() {
                Ok(database) => database,
                Err(e) => return response_err(id, make_error(-32000, format!("Database error: {}", e), None)),
            };
            let mut registry = EntityRegistry::new(&mut db);

            // Get entity info before deletion
            let entity = match registry.get_entity(params.entity_id) {
                Ok(Some(e)) => e,
                Ok(None) => return response_err(id, make_error(404, "Entity not found", None)),
                Err(e) => return response_err(id, make_error(-32000, format!("DB error: {}", e), None)),
            };

            let entity_name = entity.canonical_name.clone();

            if let Err(e) = registry.delete_entity(params.entity_id) {
                return response_err(id, make_error(-32000, format!("Delete error: {}", e), None));
            }

            let result = json!({
                "success": true,
                "message": format!("Deleted entity '{}' (ID: {})", entity_name, params.entity_id)
            });

            response_ok(id, json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&result).unwrap()
                }],
                "isError": false
            }))
        }
        other => response_err(
            id,
            make_error(-32601, format!("Unknown tool: {}", other), None),
        ),
    }
}

async fn handle_resources_list(id: Option<Value>) -> Value {
    // Resources are not implemented in this version - focus on tools
    response_ok(id, json!({"resources": []}))
}

/// Run the MCP server over stdio
pub async fn run_mcp_server() -> Result<()> {
    // Hint to DB layer to skip heavy integrity checks in MCP mode
    std::env::set_var("SCRIBA_MCP_MODE", "1");

    let mut reader = BufReader::new(io::stdin());
    let mut writer = io::stdout();

    while let Some(line) = read_json_message(&mut reader).await? {
        if line.is_empty() {
            continue; // Skip empty lines
        }

        let req: JsonRpcRequest = match serde_json::from_str::<JsonRpcRequest>(&line) {
            Ok(v) => {
                // Validate JSON-RPC version
                if v.jsonrpc != "2.0" {
                    eprintln!("[scriba-mcp] Invalid JSON-RPC version: {}", v.jsonrpc);
                }
                v
            },
            Err(e) => {
                eprintln!("[scriba-mcp] JSON parse error: {}", e);
                let resp = response_err(None, make_error(-32700, format!("Parse error: {}", e), None));
                if let Err(write_err) = write_json_message(&mut writer, &resp).await {
                    eprintln!("[scriba-mcp] Error writing response: {}", write_err);
                }
                continue;
            }
        };

        let id = req.id.clone();
        let method = req.method.as_str();

        let resp = match method {
            "initialize" => handle_initialize(id, req.params).await,
            "ping" => response_ok(id, json!({})),
            "tools/list" => handle_tools_list(id).await,
            "tools/call" => handle_tools_call(id.clone(), req.params).await,
            "resources/list" => handle_resources_list(id.clone()).await,
            "resources/templates/list" => response_ok(id, json!({"resourceTemplates": []})),
            "prompts/list" => response_ok(id, json!({"prompts": []})),
            "prompts/get" => response_err(id, make_error(404, "Prompt not found", None)),
            "logging/setLevel" => response_ok(id, json!({})),
            "notifications/initialized" => continue, // No response for notifications
            other => response_err(
                id,
                make_error(-32601, format!("Method not found: {}", other), None),
            ),
        };

        if req.id.is_some() {
            if let Err(write_err) = write_json_message(&mut writer, &resp).await {
                eprintln!("[scriba-mcp] Error writing response: {}", write_err);
                return Err(write_err);
            }
        }
    }
    Ok(())
}
