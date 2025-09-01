# Scriba

Modern CLI and TUI to record audio and transcribe it locally using Whisper via whisper-rs, or via the OpenAI API. Local mode requires no API key; API mode uses your `OPENAI_API_KEY`.

## ✨ Features

- 🎙️ **Audio Recording** - High-quality microphone recording with smart MP3 compression
- 📝 **Transcription (Local or API)** - Whisper.cpp engine via whisper-rs with progress indicators, or OpenAI API  
- 📊 **Interactive Dashboard** - Browse, search, and manage recordings with model display
- 🔍 **Full-Text Search** - Find recordings by searching transcript content
- ▶️ **Audio Playback** - Play recordings directly from the dashboard
- 📋 **Transcript Management** - Copy, view, re-transcribe, and manage recordings
- 🏷️ **Model Tracking** - See which transcription model was used for each recording
- 🔄 **Re-transcription** - Easily re-transcribe with different models from transcript view
- 📁 **External Audio Import** - Import and transcribe external audio files (from the dashboard or CLI)
- 🤖 **MCP Server Integration** - Connect with Claude Desktop and other MCP clients to access transcripts via AI assistants

## 🚀 Quick Start

### Installation
```bash
# Homebrew (recommended)
brew tap giovannialberto/scriba
brew install scriba

# From source
cargo install --git https://github.com/giovannialberto/scriba
```

### Whisper Models (Local Mode)
You can let Scriba auto-download models on first use, or pre-place a GGML/GGUF model under `~/scriba_recordings/models/`.
Examples: `ggml-base.bin`, `base.gguf`, `ggml-large-v3-turbo-q5_0.gguf`.

### Usage
```bash
# Launch interactive dashboard
scriba

# Record and transcribe in CLI (choose model or API)
scriba record --model turbo
scriba record --local --model medium
scriba record --api-key $OPENAI_API_KEY

# Import external audio file and transcribe via CLI
scriba transcribe /path/to/audio/file.mp3 -n "My call" --model large
scriba transcribe /path/to/audio/file.wav --api-key $OPENAI_API_KEY

# Run as MCP server for Claude Desktop integration
scriba mcp
```

## 📊 Dashboard Controls (TUI)

### Main Dashboard
| Key | Action |
|-----|--------|
| **R** | Record + Auto-transcribe |
| **A** | Add external audio file & transcribe |
| **T** | Transcribe selected recording |
| **Enter** | View transcript |
| **C** | Copy transcript to clipboard |
| **P** | Play recording |
| **/** | Search transcripts |
| **D** | Delete recording |
| **H** | Show help |

### Transcript View
| Key | Action |
|-----|--------|
| **T** | Re-transcribe with current model |
| **C** | Copy transcript to clipboard |
| **Esc** | Return to dashboard |

## 🗂️ File Organization

All recordings are stored in `~/scriba_recordings/`:
```
~/scriba_recordings/
├── 2025-08-26_14-30-25_meeting/
│   ├── recording.mp3
│   └── transcript.txt
└── scriba.db
```

## 🔧 Requirements

- **Whisper model (Local)** - GGML/GGUF file under `~/scriba_recordings/models/` (auto-download supported)
- **CMake** - Required to build whisper-rs (`brew install cmake` on macOS)
- **FFmpeg** - For audio compression and resampling (`brew install ffmpeg`)
- **Audio system** - Microphone for recording, speakers for playback

### Modes and Model Selection
- Local mode uses Whisper models: `--model tiny|base|small|medium|large|turbo`.
- API mode uses your OpenAI key: `--api-key $OPENAI_API_KEY` or configure in the dashboard settings.
- Dashboard lets you toggle between Local and API, and cycle model sizes.
- Turbo defaults to a GGUF build (`ggml-large-v3-turbo-q5_0.gguf`). You can override by placing a different model file in `~/scriba_recordings/models/`.

## 🤖 MCP Server Integration

Scriba includes a Model Context Protocol (MCP) server that allows AI assistants like Claude Desktop to access your transcripts directly.

### Setup for Claude Desktop
1. Run `scriba mcp` to start the MCP server
2. Add this configuration to your Claude Desktop config file:
   - **macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
   - **Windows**: `%APPDATA%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "scriba": {
      "command": "scriba",
      "args": ["mcp"]
    }
  }
}
```

### Available Tools
- **list_transcripts** - Browse all recordings with metadata
- **get_transcript** - Retrieve full transcript content by ID or name
- **search_transcripts** - Full-text search across all transcripts
- **get_recording_info** - Get detailed recording metadata

## 🎯 Key Benefits

- **80-90% file size reduction** with speech-optimized MP3 compression
- **Responsive UX** with animated progress indicators for recording, importing, and transcribing
- **Seamless workflow** - record, transcribe, re-transcribe, and manage from one interface
- **Smart search** - find any recording by searching transcript text
- **Model flexibility** - easily compare transcriptions using different Whisper models
- **AI integration** - Direct access to transcripts from Claude Desktop and other MCP clients
- **Cross-platform** - works on macOS, Linux, and Windows

## 📄 License

MIT License - see [LICENSE](LICENSE) file for details.
