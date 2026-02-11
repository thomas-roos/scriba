//! Transcription functionality for Scriba.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::{
    multipart::{Form, Part},
    Client,
};
use serde_json::Value;
use std::ffi::{c_char, c_void};
use std::io::{stdout, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::config::{LocalModelSize, ScribaConfig, TranscriptionMode};
use super::files::FileManager;
use crate::database::Database;
use crate::enrichment::{WorldContext, WorldData};
use crate::utils::BASE_PATH;

extern "C" {
    fn whisper_log_set(
        callback: Option<unsafe extern "C" fn(i32, *const c_char, *mut c_void)>,
        user_data: *mut c_void,
    );
}

static INIT_WHISPER_LOG: Once = Once::new();

unsafe extern "C" fn discard_whisper_log(_level: i32, _text: *const c_char, _ud: *mut c_void) {
    // intentionally no-op to keep TUI clean
}

fn ensure_whisper_logs_suppressed() {
    INIT_WHISPER_LOG.call_once(|| unsafe {
        whisper_log_set(Some(discard_whisper_log), std::ptr::null_mut());
    });
}

/// Progress indicator for transcription operations.
pub struct TranscriptionProgress {
    start_time: Instant,
    animation_frame: usize,
}

impl TranscriptionProgress {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            animation_frame: 0,
        }
    }

    pub async fn show_progress(&mut self, mode_message: Option<&str>) {
        let elapsed = self.start_time.elapsed().as_secs();

        let spinner_chars = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let spinner = spinner_chars[self.animation_frame % spinner_chars.len()];

        let message = match elapsed {
            0..=3 => "Preparing audio",
            4..=8 => "Processing",
            9..=25 => "Transcribing",
            _ => "Almost there, hang tight",
        };

        let time_display = if elapsed < 60 {
            format!("{}s", elapsed)
        } else {
            format!("{}m {}s", elapsed / 60, elapsed % 60)
        };

        let bar_width = 30;
        let progress_pos = (elapsed as usize * 2) % (bar_width * 2);
        let mut bar = vec![' '; bar_width];

        if progress_pos < bar_width {
            for i in 0..=progress_pos.min(bar_width - 1) {
                bar[i] = if i == progress_pos { '█' } else { '▓' };
            }
        } else {
            let reverse_pos = (bar_width * 2 - 1) - progress_pos;
            for i in reverse_pos..bar_width {
                bar[i] = if i == reverse_pos { '█' } else { '▓' };
            }
        }

        let bar_str: String = bar.into_iter().collect();

        let display_message = mode_message.unwrap_or(message);
        print!(
            "\r🎵 {} [{}] {} - {}",
            spinner, bar_str, display_message, time_display
        );
        stdout().flush().unwrap();

        self.animation_frame += 1;
        sleep(Duration::from_millis(100)).await;
    }
}

impl Default for TranscriptionProgress {
    fn default() -> Self {
        Self::new()
    }
}

/// Persist transcript to file and update database.
fn save_transcript_to_files_and_db(
    audio_path: &Path,
    transcript_text: &str,
    model_used: &str,
) -> Result<()> {
    let audio_dir = audio_path
        .parent()
        .context("Could not determine audio file directory")?;
    let transcript_file_path = audio_dir.join("transcript.txt");
    std::fs::write(&transcript_file_path, transcript_text).with_context(|| {
        format!(
            "Failed to write transcript to {}",
            transcript_file_path.display()
        )
    })?;

    let mut db = Database::new().context("Failed to connect to database")?;
    let directory_name = audio_dir
        .file_name()
        .and_then(|name| name.to_str())
        .context("Could not determine directory name")?;
    if let Some(recording) = db.get_recording_by_directory(directory_name)? {
        if let Some(recording_id) = recording.id {
            db.upsert_transcript(recording_id, transcript_text)?;
            let _ = db.update_recording_transcript_status_and_model(
                recording_id,
                "completed",
                true,
                model_used,
            );
        }
    }

    Ok(())
}

fn find_ffmpeg() -> Result<String> {
    let possible_paths = [
        "ffmpeg",
        "/opt/homebrew/bin/ffmpeg",
        "/usr/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "C:\\ffmpeg\\bin\\ffmpeg.exe",
    ];

    for path in &possible_paths {
        match Command::new(path).arg("-version").output() {
            Ok(output) => {
                if output.status.success() {
                    return Ok(path.to_string());
                }
            }
            Err(_) => continue,
        }
    }

    match Command::new("ffmpeg").arg("-version").output() {
        Ok(output) if output.status.success() => {
            return Ok("ffmpeg".to_string());
        }
        _ => {}
    }

    Err(anyhow::anyhow!(
        "FFmpeg not found. Please install FFmpeg and ensure it's in your PATH."
    ))
}

fn ensure_mono_16k_wav(input: &Path) -> Result<PathBuf> {
    let out = input
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("_tmp_whisper_16k.wav");

    let ffmpeg_path = find_ffmpeg().context("FFmpeg is required for audio processing")?;

    let output = Command::new(&ffmpeg_path)
        .args([
            "-y",
            "-i",
            input.to_string_lossy().as_ref(),
            "-ar",
            "16000",
            "-ac",
            "1",
            "-f",
            "wav",
            out.to_string_lossy().as_ref(),
        ])
        .output()
        .with_context(|| format!("Failed to run ffmpeg from path: {}", ffmpeg_path))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("ffmpeg conversion failed: {}", stderr));
    }

    Ok(out)
}

async fn download_model_with_timeout(model_size: LocalModelSize) -> Result<PathBuf> {
    ensure_model_path_local(model_size, true).await
}

async fn ensure_model_path_local(size: LocalModelSize, quiet: bool) -> Result<PathBuf> {
    let models_dir = BASE_PATH.join("models");
    std::fs::create_dir_all(&models_dir).ok();

    let local_candidates: Vec<&str> = match size {
        LocalModelSize::Tiny => vec!["ggml-tiny.bin", "tiny.gguf", "ggml-tiny-q5_0.gguf"],
        LocalModelSize::Base => vec!["ggml-base.bin", "base.gguf", "ggml-base-q5_0.gguf"],
        LocalModelSize::Small => vec!["ggml-small.bin", "small.gguf", "ggml-small-q5_0.gguf"],
        LocalModelSize::Medium => vec!["ggml-medium.bin", "medium.gguf", "ggml-medium-q5_0.gguf"],
        LocalModelSize::Large => vec![
            "ggml-large-v3.bin",
            "large-v3.gguf",
            "ggml-large-v3-q5_0.gguf",
        ],
        LocalModelSize::Turbo => vec![
            "ggml-large-v3-turbo.gguf",
            "ggml-large-v3-turbo-q5_0.gguf",
            "large-v3-turbo.gguf",
            "ggml-large-v3-turbo.bin",
        ],
    };

    for name in &local_candidates {
        let p = models_dir.join(name);
        if p.exists() {
            return Ok(p);
        }
    }

    // Download model if not found
    match size {
        LocalModelSize::Tiny => {
            let name = "ggml-tiny.bin";
            let url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin";
            download_model(&models_dir, name, url, quiet).await
        }
        LocalModelSize::Base => {
            let name = "ggml-base.bin";
            let url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin";
            download_model(&models_dir, name, url, quiet).await
        }
        LocalModelSize::Small => {
            let name = "ggml-small.bin";
            let url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin";
            download_model(&models_dir, name, url, quiet).await
        }
        LocalModelSize::Medium => {
            let name = "ggml-medium.bin";
            let url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin";
            download_model(&models_dir, name, url, quiet).await
        }
        LocalModelSize::Large => {
            let name = "ggml-large-v3.bin";
            let url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin";
            download_model(&models_dir, name, url, quiet).await
        }
        LocalModelSize::Turbo => {
            let candidates = vec![
                ("ggml-large-v3-turbo-q5_0.gguf", "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.gguf"),
                ("ggml-large-v3-turbo.gguf", "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.gguf"),
                ("large-v3-turbo.gguf", "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/large-v3-turbo.gguf"),
                ("ggml-large-v3-turbo.bin", "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin"),
            ];
            let mut last_err: Option<anyhow::Error> = None;
            for (name, url) in candidates {
                match download_model(&models_dir, name, url, quiet).await {
                    Ok(path) => return Ok(path),
                    Err(e) => last_err = Some(e),
                }
            }
            Err(last_err.unwrap_or_else(|| {
                anyhow::anyhow!("Failed to download turbo model from all known locations")
            }))
        }
    }
}

async fn download_model(
    models_dir: &Path,
    name: &str,
    url: &str,
    quiet: bool,
) -> Result<PathBuf> {
    let dest = models_dir.join(name);
    if !quiet {
        println!(
            "📥 Downloading Whisper model: {} (this may take a while)",
            name
        );
    }
    download_file_streaming(url, &dest, quiet)
        .await
        .with_context(|| format!("Failed to download model from {}", url))?;
    if !quiet {
        println!("✅ Model downloaded to {}", dest.display());
    }
    Ok(dest)
}

async fn download_file_streaming(url: &str, dest: &Path, quiet: bool) -> Result<()> {
    let client = Client::new();
    let resp = client.get(url).send().await?.error_for_status()?;
    let total = resp.content_length();
    let mut stream = resp.bytes_stream();
    let mut file = std::fs::File::create(dest).context("Failed to create destination file")?;
    let mut downloaded: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        std::io::Write::write_all(&mut file, &chunk)?;
        if !quiet {
            downloaded += chunk.len() as u64;
            if let Some(total) = total {
                let pct = (downloaded as f64 / total as f64) * 100.0;
                if downloaded % (10 * 1024 * 1024) < chunk.len() as u64 {
                    print!("\rDownloading model... {:>6.2}%", pct);
                    let _ = stdout().flush();
                }
            }
        }
    }
    if !quiet {
        println!();
    }
    Ok(())
}

/// Approximate max characters for ~224 Whisper tokens.
const WHISPER_PROMPT_CHAR_LIMIT: usize = 670;

/// Filter aliases to only include genuine spelling variants, not LLM annotations.
fn filter_aliases(aliases: &[String], canonical_name: &str) -> Vec<String> {
    let canonical_lower = canonical_name.to_lowercase();
    aliases
        .iter()
        .filter(|a| {
            let lower = a.to_lowercase();
            lower != canonical_lower && !a.contains('(') && a.len() <= 30
        })
        .cloned()
        .collect()
}

/// Build a Whisper initial prompt from structured world data.
///
/// Returns a natural-language contextual string that primes Whisper's decoder
/// with proper nouns, roles, and relationships for better transcription accuracy.
fn build_prompt_from_world_data(data: &WorldData) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    // 1) Owner — contextual sentence about who owns this instance
    if !data.owner.name.is_empty() {
        let name = &data.owner.name;
        let aliases = filter_aliases(&data.owner.aliases, name);
        let role = &data.owner.role;
        let org = &data.owner.organization;

        let mut sentence = if !role.is_empty() && !org.is_empty() {
            format!("{} is the {} at {}", name, role, org)
        } else if !role.is_empty() {
            format!("{} is a {}", name, role)
        } else if !org.is_empty() {
            format!("{} works at {}", name, org)
        } else {
            format!("{} is the owner of this recording", name)
        };

        if !aliases.is_empty() {
            sentence.push_str(&format!(", also known as {}", aliases.join(", ")));
        }

        sentence.push_str(". It is likely that he is involved in this conversation.");
        parts.push(sentence);
    }

    // 2) People — natural sentence with roles/relationships
    if !data.people.is_empty() {
        let people_descriptions: Vec<String> = data
            .people
            .iter()
            .map(|p| {
                let aliases = filter_aliases(&p.aliases, &p.name);
                let mut desc = p.name.clone();
                if !aliases.is_empty() {
                    desc.push_str(&format!(" ({})", aliases.join(", ")));
                }
                if !p.relationship.is_empty() && !p.relationship.starts_with('{') {
                    desc.push_str(&format!(", {}", p.relationship));
                }
                desc
            })
            .collect();
        parts.push(format!(
            "Common people he interacts with: {}.",
            people_descriptions.join("; ")
        ));
    }

    // 3) Organizations — with descriptions
    if !data.organizations.is_empty() {
        let org_descriptions: Vec<String> = data
            .organizations
            .iter()
            .map(|o| {
                let aliases = filter_aliases(&o.aliases, &o.name);
                let mut desc = o.name.clone();
                if !aliases.is_empty() {
                    desc.push_str(&format!(" ({})", aliases.join(", ")));
                }
                if !o.description.is_empty() && !o.description.starts_with('{') {
                    desc.push_str(&format!(", {}", o.description));
                }
                desc
            })
            .collect();
        parts.push(format!(
            "Organizations: {}.",
            org_descriptions.join("; ")
        ));
    }

    // 4) Projects and interests — domain vocabulary
    let mut topics: Vec<String> = Vec::new();
    for p in &data.projects {
        topics.push(p.name.clone());
    }
    for i in &data.interests {
        topics.push(i.clone());
    }
    if !topics.is_empty() {
        parts.push(format!("Topics often discussed: {}.", topics.join(", ")));
    }

    if parts.is_empty() {
        return None;
    }

    // Assemble with budget enforcement
    let mut prompt = String::new();
    for part in &parts {
        let candidate = if prompt.is_empty() {
            part.clone()
        } else {
            format!("{} {}", prompt, part)
        };
        if candidate.len() > WHISPER_PROMPT_CHAR_LIMIT {
            break;
        }
        prompt = candidate;
    }

    // Strip null bytes (set_initial_prompt panics on them)
    let prompt = prompt.replace('\0', "");

    if prompt.is_empty() {
        None
    } else {
        Some(prompt)
    }
}

/// Load world context and build a Whisper initial prompt.
///
/// Returns `None` if no world context exists or it cannot be parsed.
fn build_whisper_world_prompt() -> Option<String> {
    let world_ctx = WorldContext::load().ok()?;
    let data = world_ctx.parsed()?;
    build_prompt_from_world_data(&data)
}

fn run_whisper_transcription(model_path: &Path, wav_path: &Path) -> Result<String> {
    if !model_path.exists() {
        return Err(anyhow::anyhow!(
            "Whisper model file not found: {}",
            model_path.display()
        ));
    }
    if !wav_path.exists() {
        return Err(anyhow::anyhow!(
            "Audio file not found: {}",
            wav_path.display()
        ));
    }

    let params_ctx = WhisperContextParameters::default();
    let model_path_str = model_path.to_string_lossy();
    let ctx = match WhisperContext::new_with_params(&model_path_str, params_ctx) {
        Ok(ctx) => ctx,
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to load Whisper model '{}': {}. Check if model file is valid and you have enough memory",
                model_path.display(),
                e
            ))
        }
    };

    let mut state = ctx.create_state().with_context(|| {
        format!(
            "Failed to create Whisper state for model: {}",
            model_path.display()
        )
    })?;

    let mut reader = hound::WavReader::open(wav_path)
        .context("Failed to open normalized WAV for transcription")?;
    let spec = reader.spec();

    let mut samples_f32: Vec<f32> = Vec::new();
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for s in reader.samples::<f32>() {
                samples_f32.push(s.unwrap_or(0.0));
            }
        }
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample as i64 - 1)) as f32;
            for s in reader.samples::<i32>() {
                let v = s.unwrap_or(0) as f32 / max;
                samples_f32.push(v);
            }
        }
    }

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    params.set_single_segment(false);
    params.set_n_threads(num_cpus::get() as i32);

    // Anti-hallucination: skip segments that look like no speech
    params.set_no_speech_thold(0.6);
    // Anti-repetition: skip segments with low entropy (repetitive output)
    params.set_entropy_thold(2.4);
    // Anti-hallucination: skip segments with very low token probability
    params.set_logprob_thold(-1.0);
    // Suppress blank outputs at start of segments
    params.set_suppress_blank(true);
    // Suppress non-speech tokens
    params.set_suppress_nst(true);
    // Temperature fallback: retry with higher temperature on failed decodes
    params.set_temperature(0.0);
    params.set_temperature_inc(0.2);

    if let Some(world_prompt) = build_whisper_world_prompt() {
        params.set_initial_prompt(&world_prompt);
    }

    state
        .full(params, &samples_f32)
        .context("Whisper full() failed")?;

    let num_segments = state.full_n_segments();
    let mut text = String::new();
    for i in 0..num_segments {
        if let Some(seg) = state.get_segment(i) {
            if let Ok(segment_text) = seg.to_str() {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(segment_text.trim());
            }
        }
    }

    Ok(text)
}

async fn transcribe_with_openai_api(audio_path: &PathBuf, api_key: &str) -> Result<String> {
    let audio_file = std::fs::read(audio_path).context("Unable to read audio file")?;

    let filename = audio_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("audio")
        .to_string();

    let client = Client::new();

    let part = Part::bytes(audio_file)
        .file_name(filename.clone())
        .mime_str("audio/mpeg")
        .context("Failed to create multipart form data")?;

    let form = Form::new().part("file", part).text("model", "whisper-1");

    let response = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await
        .context("Failed to send transcription request to OpenAI")?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(anyhow::anyhow!(
            "OpenAI API request failed with status {}: {}",
            status,
            error_text
        ));
    }

    let response_json: Value = response
        .json()
        .await
        .context("Failed to parse OpenAI response as JSON")?;

    response_json
        .get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("No 'text' field found in OpenAI response"))
}

/// Unified transcription function.
pub async fn transcribe_audio(
    input_path: &PathBuf,
    mode_override: Option<TranscriptionMode>,
    verbose: bool,
) -> Result<()> {
    if !verbose {
        ensure_whisper_logs_suppressed();
    }

    let audio_file_path = FileManager::resolve_audio_path(input_path)?;
    let config = ScribaConfig::load()?;
    let transcription_mode = mode_override.unwrap_or_else(|| config.transcription.clone());

    let progress = TranscriptionProgress::new();

    if verbose {
        let mode_description = match &transcription_mode {
            TranscriptionMode::Local { model_size } => {
                format!(
                    "🎤 → 📝 Transcribing locally using Whisper {} model...",
                    model_size
                )
            }
            TranscriptionMode::Api { .. } => {
                "🎤 → ☁️ Transcribing using OpenAI Whisper API...".to_string()
            }
        };
        println!("\n{}\n", mode_description);
    }

    let (transcription_text, model_used) = match transcription_mode {
        TranscriptionMode::Local { model_size } => {
            let progress_task = if verbose {
                let mut local_progress = progress;
                Some(tokio::spawn(async move {
                    loop {
                        let message = match local_progress.start_time.elapsed().as_secs() {
                            0..=3 => Some("Preparing audio (16kHz mono)"),
                            4..=8 => Some("Loading Whisper model"),
                            9..=25 => Some("Running local transcription"),
                            _ => Some("Almost there, hang tight"),
                        };
                        local_progress.show_progress(message).await;
                    }
                }))
            } else {
                None
            };

            let wav_path = ensure_mono_16k_wav(&audio_file_path)
                .context("Failed to prepare 16kHz mono WAV for transcription")?;
            let model_path = {
                let download_future = download_model_with_timeout(model_size);
                tokio::time::timeout(Duration::from_secs(300), download_future)
                    .await
                    .context("Model download timed out after 5 minutes")?
                    .context("Failed to download model")?
            };
            let result = run_whisper_transcription(&model_path, &wav_path)
                .context("Local Whisper transcription failed")?;
            if wav_path.file_name() == Some(std::ffi::OsStr::new("_tmp_whisper_16k.wav")) {
                let _ = std::fs::remove_file(&wav_path);
            }
            if let Some(task) = progress_task {
                task.abort();
            }
            let model_name = format!("whisper-{}", model_size);
            (result, model_name)
        }
        TranscriptionMode::Api { api_key } => {
            let progress_task = if verbose {
                let mut api_progress = progress;
                Some(tokio::spawn(async move {
                    loop {
                        let message = match api_progress.start_time.elapsed().as_secs() {
                            0..=3 => Some("Uploading audio file"),
                            4..=8 => Some("OpenAI is processing your audio"),
                            9..=15 => Some("Converting speech to text"),
                            16..=25 => Some("This is taking longer than usual"),
                            _ => Some("Almost there, hang tight"),
                        };
                        api_progress.show_progress(message).await;
                    }
                }))
            } else {
                None
            };

            let result = transcribe_with_openai_api(&audio_file_path, &api_key)
                .await
                .context("OpenAI API transcription failed")?;
            if let Some(task) = progress_task {
                task.abort();
            }
            (result, "whisper-1".to_string())
        }
    };

    if verbose {
        print!("\r{}", " ".repeat(80));
        print!("\r");
        stdout().flush().unwrap();
        println!("✨ Transcription complete! ✨");
    }

    save_transcript_to_files_and_db(&audio_file_path, &transcription_text, &model_used)?;

    if verbose {
        let transcript_file_path = audio_file_path
            .parent()
            .context("Could not determine audio file directory")?
            .join("transcript.txt");
        println!(
            "\n📁 Transcript saved to: {}",
            transcript_file_path.display()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrichment::world::{OrgInfo, OwnerInfo, PersonInfo, ProjectInfo};

    #[test]
    fn test_filter_aliases_removes_canonical_match() {
        let aliases = vec!["giovanni".to_string()];
        let result = filter_aliases(&aliases, "Giovanni");
        assert!(result.is_empty());
    }

    #[test]
    fn test_filter_aliases_removes_annotations_with_parens() {
        let aliases = vec![
            "Gio".to_string(),
            "Giovanni (in the owner's world)".to_string(),
        ];
        let result = filter_aliases(&aliases, "Giovanni");
        assert_eq!(result, vec!["Gio"]);
    }

    #[test]
    fn test_filter_aliases_removes_long_aliases() {
        let aliases = vec![
            "Exane".to_string(),
            "This is an extremely long alias that should be filtered out".to_string(),
        ];
        let result = filter_aliases(&aliases, "Exein");
        assert_eq!(result, vec!["Exane"]);
    }

    #[test]
    fn test_filter_aliases_keeps_genuine_variants() {
        let aliases = vec!["Exane".to_string(), "Saci".to_string()];
        let result = filter_aliases(&aliases, "Exein");
        assert_eq!(result, vec!["Exane", "Saci"]);
    }

    #[test]
    fn test_build_prompt_empty_world() {
        let data = WorldData::default();
        assert!(build_prompt_from_world_data(&data).is_none());
    }

    #[test]
    fn test_build_prompt_owner_only() {
        let data = WorldData {
            owner: OwnerInfo {
                name: "Giovanni".to_string(),
                aliases: vec!["Gio".to_string()],
                role: "CTO".to_string(),
                organization: "Exein".to_string(),
                location: String::new(),
            },
            ..Default::default()
        };
        let prompt = build_prompt_from_world_data(&data).unwrap();
        assert!(prompt.contains("Giovanni is the CTO at Exein"));
        assert!(prompt.contains("also known as Gio"));
        assert!(prompt.contains("likely that he is involved"));
    }

    #[test]
    fn test_build_prompt_full_world() {
        let data = WorldData {
            owner: OwnerInfo {
                name: "Giovanni".to_string(),
                aliases: vec![],
                role: "CTO".to_string(),
                organization: "Exein".to_string(),
                location: String::new(),
            },
            people: vec![
                PersonInfo {
                    name: "Gerardo".to_string(),
                    relationship: "CFO of Exein".to_string(),
                    aliases: vec!["Gerardo Gagliardo".to_string()],
                },
                PersonInfo {
                    name: "Steve".to_string(),
                    relationship: String::new(),
                    aliases: vec![],
                },
            ],
            organizations: vec![OrgInfo {
                name: "Exein".to_string(),
                description: "cybersecurity company".to_string(),
                aliases: vec!["Exane".to_string()],
            }],
            projects: vec![ProjectInfo {
                name: "ASPISEC".to_string(),
                description: String::new(),
            }],
            interests: vec!["cybersecurity".to_string()],
            beliefs: vec![],
        };
        let prompt = build_prompt_from_world_data(&data).unwrap();
        assert!(prompt.contains("Giovanni is the CTO at Exein"));
        assert!(prompt.contains("Common people he interacts with"));
        assert!(prompt.contains("Gerardo (Gerardo Gagliardo), CFO of Exein"));
        assert!(prompt.contains("Steve"));
        assert!(prompt.contains("Exein (Exane), cybersecurity company"));
        assert!(prompt.contains("ASPISEC"));
        assert!(prompt.contains("cybersecurity"));
    }

    #[test]
    fn test_build_prompt_respects_char_limit() {
        let data = WorldData {
            owner: OwnerInfo {
                name: "Owner".to_string(),
                ..Default::default()
            },
            people: (0..200)
                .map(|i| PersonInfo {
                    name: format!("Person{}", i),
                    relationship: String::new(),
                    aliases: vec![],
                })
                .collect(),
            ..Default::default()
        };
        let prompt = build_prompt_from_world_data(&data).unwrap();
        assert!(prompt.len() <= WHISPER_PROMPT_CHAR_LIMIT);
    }

    #[test]
    fn test_build_prompt_strips_null_bytes() {
        let data = WorldData {
            owner: OwnerInfo {
                name: "Test\0Name".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let prompt = build_prompt_from_world_data(&data).unwrap();
        assert!(!prompt.contains('\0'));
        assert!(prompt.contains("TestName"));
    }
}
