Scriba 0.11.1 — Unified Core, Smoother UX, Reliable Builds

Highlights

- Unified recording: Introduced a single `record_audio` entrypoint with `RecordOptions`, removing duplicate logic and thin wrappers. CLI/TUI differences are now configuration, not code paths.
- Unified transcription: Added `transcribe_audio` to handle both local (Whisper via whisper-rs) and OpenAI API modes with a simple verbose flag for CLI/TUI.
- TUI import UX: The “A” flow now imports + transcribes in the background with a live progress animation instead of blocking the UI.
- Clean transcripts: Standardized on `transcript.txt` everywhere; removed `recording.txt` legacy fallback.
- Accurate metadata: Centralized audio metadata extraction using `FileManager::extract_audio_metadata` for consistent DB entries across record/import.
- Robust stop handling: Fixed CLI Ctrl+C and TUI stop behavior by offloading waits to OS threads, keeping async tasks responsive and Send-safe.
- Core deduplication: Reduced duplication of silent vs. verbose flows in `core.rs` via internal helpers.
- Documentation: Updated README to reflect Local/API modes, dashboard “A” behavior, and progress indicators.
- CI reliability: Fixed macOS ARM build by disabling ggml i8mm in the release workflow for broader Apple Silicon compatibility.

Changelog

- feat(core): unify recording/transcription entrypoints and simplify workflows
- feat(tui): non-blocking import + transcription with progress
- refactor: remove legacy `recording.txt` and associated fallbacks
- refactor: centralize metadata extraction for DB save
- fix(cli/tui): stop signal handling without blocking the runtime
- docs: update README for modes, dashboard keys, and progress UX
- ci: disable ggml i8mm on aarch64 macOS to stabilize builds

Notes

- Local transcription auto-downloads Whisper models on first use if not found under `~/scriba_recordings/models/`.
- API transcription uses `OPENAI_API_KEY` from the dashboard settings or CLI `--api-key`.

