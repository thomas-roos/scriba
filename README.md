# Scriba

A simple CLI tool for recording audio and transcribing it using OpenAI's Whisper API.

## Features

- **Record Audio**: Capture audio directly from your microphone
- **Transcribe Audio**: Convert audio files to text using OpenAI's Whisper API
- **Combined Workflow**: Record and automatically transcribe in one command
- **Automatic File Naming**: Smart timestamp-based naming with optional descriptions
- **Flexible API Key**: Pass API key via command line or environment variable
- **Organized Storage**: All recordings stored in `~/scriba_recordings/`

## Prerequisites

- Rust (1.70.0 or later)
- OpenAI API key

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

### Option 1: Environment Variable (Optional)

Create a `.env` file in the project root and add your OpenAI API key:

```env
OPENAI_API_KEY=your_api_key_here
```

### Option 2: Command Line Parameter (Recommended)

Pass your API key directly when running commands:

```bash
scriba record --api-key your_api_key_here
```

You can get an API key from [OpenAI's platform](https://platform.openai.com/api-keys).

## Usage

### Record Audio

#### Basic Recording (Auto-generated filename)
```bash
# Simple recording with automatic naming
scriba record

# Records to: ~/scriba_recordings/2025-08-10_14-30-25_recording/
# Creates: recording.wav and transcript.txt (if not skipped)
```

#### Recording with Custom Name
```bash
# Recording with descriptive name
scriba record --name "meeting-notes"

# Records to: ~/scriba_recordings/2025-08-10_14-30-25_meeting-notes/
# Creates: recording.wav and transcript.txt
```

#### Skip Automatic Transcription
```bash
# Record only, no transcription
scriba record --skip-transcription
scriba record --name "interview" --skip-transcription
```

#### With API Key Parameter
```bash
# Pass API key directly (no .env file needed)
scriba record --api-key "your_api_key_here"
scriba record --name "meeting" --api-key "your_api_key_here"
```

### Transcribe Audio

#### Transcribe Existing File
```bash
# Transcribe with auto-generated name
scriba transcribe path/to/audio.wav

# Transcribe with custom name
scriba transcribe path/to/audio.wav --name "interview-transcript"

# With API key parameter
scriba transcribe audio.wav --api-key "your_api_key_here"
```

### File Organization

All files are organized in `~/scriba_recordings/` with automatic naming:

```
~/scriba_recordings/
├── 2025-08-10_14-30-25_meeting-notes/
│   ├── recording.wav
│   └── transcript.txt
├── 2025-08-10_15-45-12_recording/
│   ├── recording.wav
│   └── transcript.txt
└── 2025-08-10_16-20-30_interview-transcript.txt
```

### Examples

```bash
# Quick start - just record and transcribe
scriba record --api-key "sk-..."

# Record a meeting with custom name
scriba record --name "team-standup" --api-key "sk-..."

# Record without transcription
scriba record --name "music-practice" --skip-transcription

# Transcribe an existing file
scriba transcribe old-recording.wav --name "notes" --api-key "sk-..."
```

## Development

### Building

```bash
cargo build
```

### Running in Development

```bash
cargo run -- record test.wav
cargo run -- transcribe test.wav transcript.txt
```

### Running Tests

```bash
cargo test
```

## Supported Audio Formats

The tool supports various audio formats that are compatible with OpenAI's Whisper API, including:
- WAV
- MP3
- MP4
- MPEG
- MPGA
- M4A
- WEBM

## License

This project is licensed under the MIT License.