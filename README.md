# Scriba v0.4.0

A modern CLI tool for recording audio and transcribing it using OpenAI's Whisper API, featuring an enhanced recording library with integrated statistics.

## Features

- **🎙️ Audio Recording**: Capture audio directly from your microphone
- **📝 AI Transcription**: Convert audio to text using OpenAI's Whisper API
- **🔄 Combined Workflow**: Record and automatically transcribe in one command
- **📚 Enhanced Recording Library**: Interactive interface with integrated statistics display
- **🗃️ Database Storage**: SQLite database for organized recording metadata
- **🔍 Full-Text Search**: Search through your transcripts
- **📊 Always-On Statistics**: Recording statistics and usage metrics always visible
- **▶️ Audio Playback**: Play recordings directly from the library
- **🗑️ Safe Deletion**: Delete recordings with confirmation prompts
- **📁 Smart Organization**: All recordings stored in `~/scriba_recordings/`

## Prerequisites

- Rust (1.70.0 or later)
- OpenAI API key
- Audio system (microphone for recording, speakers for playback)

## Installation

1. Clone the repository:
   ```bash
   git clone https://github.com/giovannialberto/scriba
   cd scriba
   ```

2. Build the project:
   ```bash
   cargo build --release
   ```

3. The binary will be available at `target/release/scriba`

## Configuration

### Option 1: Environment Variable
Create a `.env` file in the project root:
```env
OPENAI_API_KEY=your_api_key_here
```

### Option 2: Command Line Parameter
Pass your API key directly when running commands:
```bash
scriba record --api-key your_api_key_here
```

Get an API key from [OpenAI's platform](https://platform.openai.com/api-keys).

## Usage

### Interactive Mode (Default)

Simply run `scriba` without arguments to enter interactive mode:

```bash
scriba
```

This launches the main menu with options:
1. **Record Audio + Auto-Transcribe** - Record and transcribe in one step
2. **Record Audio Only** - Record without transcription
3. **Transcribe Existing File** - Transcribe an audio file
4. **Exit**
**D. Recording Library with Statistics** - Enhanced library interface

### Recording Library with Statistics

The enhanced library interface provides:
- **Always-Visible Statistics**: Recording metrics always displayed at the bottom
- **Navigation**: Use arrow keys to browse recordings with table view
- **View Transcripts**: Press `Enter` to read transcripts in popup
- **Play Audio**: Press `P` to play recordings with external players
- **Delete**: Press `D` to delete recordings with confirmation dialog
- **Search**: Press `/` to search through all transcripts
- **Quick Actions**: Press `R`/`A`/`T` for recording and transcription actions
- **Help**: Press `H` for complete help guide
- **Pagination**: Use `PgUp`/`PgDn` to navigate large libraries

### Command Line Interface

#### Record Audio
```bash
# Basic recording with auto-transcription
scriba record

# Custom name
scriba record --name "meeting-notes"

# Skip transcription
scriba record --skip-transcription

# With API key
scriba record --api-key "your_api_key_here"
```

#### Transcribe Audio
```bash
# Basic transcription
scriba transcribe path/to/audio.wav

# With custom name
scriba transcribe audio.wav --name "interview-transcript"

# With API key
scriba transcribe audio.wav --api-key "your_api_key_here"
```

### File Organization

All recordings are organized in `~/scriba_recordings/`:

```
~/scriba_recordings/
├── scriba.db                              # SQLite database
├── 2025-08-10_14-30-25_meeting-notes/
│   ├── recording.wav
│   └── transcript.txt
├── 2025-08-10_15-45-12_recording/
│   ├── recording.wav
│   └── transcript.txt
└── 2025-08-10_16-20-30_interview/
    ├── recording.wav
    └── transcript.txt
```

## Library Features

### Search & Browse
- Full-text search through all transcripts
- Filter recordings by transcription status
- Paginated results for large libraries

### Playback
- Cross-platform audio playback
- Automatic player detection (afplay, mpv, ffplay)
- External playback with status updates

### Integrated Statistics Display
- Always visible at bottom of library interface
- Total recordings and duration in real-time
- Storage usage tracking with formatted sizes
- Transcription progress metrics and percentages
- Word count statistics across all transcripts

### Management
- Safe deletion with confirmation
- Database integrity checks
- Automatic corruption recovery

## Development

### Building
```bash
cargo build
```

### Running in Development
```bash
cargo run
cargo run -- record --name "test"
cargo run -- transcribe test.wav
```

### Running Tests
```bash
cargo test
```

## Supported Audio Formats

Recording: WAV (primary)
Transcription: WAV, MP3, MP4, MPEG, MPGA, M4A, WEBM, FLAC, OGG, OPUS, AIFF, CAF

## Database

Scriba uses SQLite for:
- Recording metadata storage
- Transcript content and full-text search
- Usage statistics
- Automatic schema management
- Foreign key constraints for data integrity

## License

This project is licensed under the MIT License.

## Version History

**v0.4.0** - Improved Audio Playback Experience
- Fixed mono audio playback to output to both stereo channels
- Enhanced audio player compatibility across platforms (macOS, Linux, Windows)
- Added stereo forcing arguments for mpv, ffplay, and aplay players
- Improved audio experience for users with headphones and speakers
- Better Windows audio support with multiple player fallbacks

**v0.3.0** - Enhanced Library with Integrated Statistics
- Merged recording library with always-visible statistics display
- Enhanced table-based recording browser with improved UX
- Integrated quick action controls (R/A/T) directly in library
- Popup-based transcript viewing for better readability
- Streamlined main menu (removed separate library option)
- Improved help system with comprehensive key bindings
- Better CLI launch preference while maintaining TUI functionality

**v0.2.0** - Recording Library & Database
- Interactive TUI library for managing recordings
- SQLite database with full-text search
- Audio playback functionality
- Statistics and usage tracking
- Safe deletion with confirmation prompts
- Improved CLI interface

**v0.1.x** - Basic Recording & Transcription
- Command-line audio recording
- OpenAI Whisper API integration
- Basic file organization