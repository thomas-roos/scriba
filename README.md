# Scriba

A modern CLI tool for recording audio and transcribing it using OpenAI's Whisper API.

## ✨ Features

- 🎙️ **Audio Recording** - High-quality microphone recording with smart MP3 compression
- 📝 **AI Transcription** - OpenAI Whisper API integration with progress indicators  
- 📊 **Interactive Dashboard** - Browse, search, and manage recordings with live statistics
- 🔍 **Full-Text Search** - Find recordings by searching transcript content
- ▶️ **Audio Playback** - Play recordings directly from the dashboard
- 📋 **Transcript Management** - Copy, view, and transcribe recordings with one keystroke

## 🚀 Quick Start

### Installation
```bash
# Homebrew (recommended)
brew tap giovannialberto/scriba
brew install scriba

# From source
cargo install --git https://github.com/giovannialberto/scriba
```

### Setup
Set your OpenAI API key:
```bash
export OPENAI_API_KEY="your-api-key-here"
```

### Usage
```bash
# Launch interactive dashboard
scriba

# Record and transcribe in CLI
scriba record
scriba transcribe audio-file.wav
```

## 📊 Dashboard Controls

| Key | Action |
|-----|--------|
| **R** | Record + Auto-transcribe |
| **A** | Record audio only |
| **T** | Transcribe selected recording |
| **Enter** | View transcript |
| **C** | Copy transcript to clipboard |
| **P** | Play recording |
| **/** | Search transcripts |
| **D** | Delete recording |
| **H** | Show help |

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

- **OpenAI API key** - Get one at [platform.openai.com](https://platform.openai.com/api-keys)
- **FFmpeg** - For audio compression (`brew install ffmpeg`)
- **Audio system** - Microphone for recording, speakers for playback

## 🎯 Key Benefits

- **80-90% file size reduction** with speech-optimized MP3 compression
- **Instant transcription** with animated progress indicators
- **Seamless workflow** - record, transcribe, and manage from one interface
- **Smart search** - find any recording by searching transcript text
- **Cross-platform** - works on macOS, Linux, and Windows

## 📄 License

MIT License - see [LICENSE](LICENSE) file for details.