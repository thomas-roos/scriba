# Scriba Agent Architecture

## Overview

The Scriba Agent transforms the "Ask Scriba" chat from a single-turn text pipeline into a real autonomous agent that can fetch data, search transcripts, and cross-correlate across recordings using tool calls.

**Provider requirement**: The agent loop requires **Anthropic (Claude)** as the provider. When using Ollama, OpenAI, or Google, the chat falls back to the original single-turn pipeline (no regression).

## Architecture Diagram

```
User types question
        |
        v
  send_chat_message()
        |
        |--- Is Anthropic? --yes--> chat_agent_pipeline()
        |                                   |
        |                                   v
        |                           run_agent_loop()
        |                             /          \
        |                    Anthropic API    Tool Executor
        |                    (streaming)      (read-only DB)
        |                             \          /
        |                           AgentEvent stream
        |                                   |
        |                                   v
        |                    Bridge: AgentEvent -> ChatStreamEvent
        |
        |--- Not Anthropic? ---> chat_generation_pipeline()
                                 (existing single-turn pipeline)
        |
        v
  poll_chat_stream() --- renders events into TUI
```

## Module Structure

```
src/agent/
  mod.rs           - Module exports
  tools.rs         - 9 tool definitions + executor + summarizer
  loop_runner.rs   - Agent loop, SSE parser, event types
```

## The Agent Loop (`src/agent/loop_runner.rs`)

### Flow

1. **Initialize**: Open database, build tool definitions, convert chat history to Anthropic message format.
2. **Send request**: POST to Anthropic Messages API with `system`, `messages`, `tools`, and `stream: true`.
3. **Parse streaming SSE**: Extract text deltas (streamed live to TUI) and `tool_use` blocks (id, name, input JSON).
4. **Execute tools**: For each `tool_use` block, run the tool against the database, emit `ToolCall` and `ToolResult` events.
5. **Append results**: Add assistant message (with tool_use blocks) and user message (with tool_result blocks) to conversation.
6. **Loop or stop**: If `stop_reason == "end_turn"`, we're done. If `stop_reason == "tool_use"`, loop back to step 2.
7. **Safety**: Hard limit of 10 iterations to prevent infinite loops.

### Event Types (`AgentEvent`)

| Event | Description |
|-------|-------------|
| `Status(String)` | Status text (e.g. "Thinking...") |
| `Chunk(String)` | Streamed text from the assistant |
| `ToolCall { name, input_summary }` | Agent is about to call a tool |
| `ToolResult { name, output_summary }` | Tool returned a result |
| `Done` | Agent finished |
| `Error(String)` | Something went wrong |

### Streaming SSE Parser

The parser handles Anthropic's streaming format:

- `content_block_start` with `type: "tool_use"` — starts accumulating tool input JSON
- `content_block_delta` with `type: "text_delta"` — streams text to TUI in real time
- `content_block_delta` with `type: "input_json_delta"` — accumulates tool input JSON fragments
- `content_block_stop` — finalizes the current tool_use block
- `message_delta` — captures `stop_reason`

## Tools (`src/agent/tools.rs`)

All tools are **read-only** and safe to call in a loop.

| Tool | Description | DB Method | Returns |
|------|-------------|-----------|---------|
| `list_recordings` | List all recordings with metadata | `list_recordings(limit, None)` | JSON array: id, name, date, duration, summary snippet |
| `get_recording` | Full recording metadata | `get_recording(id)` | JSON: summary, key_points, action_items, speakers, topics |
| `get_transcript` | Full transcript text | `get_transcript_by_recording_id(id)` | Raw transcript content (no truncation) |
| `search_transcripts` | Full-text search across transcripts | `search_transcripts(query, limit)` | JSON array: recording_id, name, word_count, snippet |
| `list_entities` | List known entities | `list_entities(type, limit)` | JSON array: id, type, name, aliases, context, mention_count |
| `get_entity` | Entity details + mentions + recordings | `get_entity(id)` + `get_mentions_for_entity(id)` + `get_recordings_for_entity(id)` | JSON: full entity with mentions and recording list |
| `get_recordings_for_entity` | Recordings mentioning an entity | `get_recordings_for_entity(id)` | JSON array: id, name, date, summary |
| `get_world_context` | Full world.md knowledge base | `WorldContext::load()` | Raw world.md content |
| `get_stats` | Recording statistics | `get_stats()` + `list_entities()` | JSON: totals for recordings, duration, words, entities |

### Tool Definition Format

Each tool is defined as an Anthropic-format JSON schema:

```json
{
  "name": "search_transcripts",
  "description": "Full-text search across all transcripts...",
  "input_schema": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "Search query" },
      "limit": { "type": "integer", "description": "Max results" }
    },
    "required": ["query"]
  }
}
```

### Tool Result Summarization

Each tool has a `summarize_tool_result()` function that produces a short one-liner for the TUI display (e.g. "3 results", "2,847 words", "5 entities").

## TUI Integration (`src/tui/app.rs`)

### New Types

```rust
struct ToolCallDisplay {
    name: String,           // Tool name
    input_summary: String,  // e.g. "\"machine learning\""
    output_summary: String, // e.g. "3 results"
    is_complete: bool,      // Animates spinner while false
}
```

`ChatStreamEvent` gained two new variants:
- `ToolCall { name, input_summary }` — displayed as spinning yellow status
- `ToolResult { name, output_summary }` — marks tool complete, shows dim gray

### Rendering

**During generation** (dynamic content):
- Pending tool calls render above streaming text
- Incomplete tools show an animated spinner in yellow
- Complete tools show a checkmark in gray: `✓ search_transcripts("AI") -> 3 results`

**After generation** (cached messages):
- Tool calls render as dim gray lines before the assistant's text:
  ```
  ✓ list_recordings(all) -> 12 recordings
  ✓ get_transcript(id=5) -> 2847 words
  (o,o):
    Based on your recordings, ...
  ```

### Provider Gating

In `send_chat_message()`:
```rust
let use_agent = matches!(
    &config.mode,
    EnrichmentMode::Cloud { provider: CloudProvider::Anthropic, .. }
);
```

- **Anthropic** → `chat_agent_pipeline()` → agent loop with tools
- **Others** → `chat_generation_pipeline()` → original single-turn pipeline

### System Prompts

**Agent mode** (Anthropic): Lightweight prompt telling the agent to use tools.
- `build_agent_global_prompt(owner)` — global context, instructs tool usage
- `build_agent_recording_prompt(owner, id, name, summary)` — recording context with pre-loaded summary

**Fallback mode** (others): Full context-stuffed prompt (unchanged from before).
- `build_global_chat_prompt(...)` — world JSON, stats, recordings, entities all inlined
- `build_recording_chat_prompt(...)` — full transcript, summary, all metadata inlined

## Key Design Decisions

1. **Self-contained HTTP client**: The agent loop uses its own `reqwest::Client` rather than modifying `anthropic.rs`. This avoids breaking the existing enrichment pipeline.

2. **Bridge pattern**: `AgentEvent` → `ChatStreamEvent` bridge in `chat_agent_pipeline()` keeps the agent loop decoupled from the TUI event system.

3. **No modifications to existing providers**: The `LlmProvider` trait and all existing provider implementations remain untouched.

4. **Tool results in conversation**: Tool results are appended as `user` messages with `tool_result` content blocks, following Anthropic's API contract for multi-turn tool use.

5. **Streaming**: Text is streamed in real-time as the model generates it. Tool calls and results appear as status updates between text chunks.

6. **Hard iteration limit**: 10 iterations maximum prevents runaway loops or excessive API costs.

## Example Interaction Flow

User asks: "What did I talk about with Marco last week?"

```
1. Agent receives question
2. → search_transcripts("Marco") -> 4 results
3. → list_recordings(limit=20) -> 20 recordings
4. → get_transcript(id=42) -> 3,200 words
5. → get_transcript(id=45) -> 1,800 words
6. Agent synthesizes answer:
   "In your meeting with Marco on Feb 24th, you discussed the Q1 roadmap
    and the hiring plan for the engineering team. In your follow-up on
    Feb 27th, you covered the budget approval for the new project..."
```

Each tool call is visible in the chat as a dim status line, giving the user transparency into the agent's reasoning process.
