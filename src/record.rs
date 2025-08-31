use crate::audio::{
    convert_wav_to_mp3, create_encoder, AudioEncoder, AudioFormat, CompressionSettings,
};
use crate::database::{Database, Recording};
use crate::core::FileManager;
use crate::utils::BASE_PATH;
use anyhow::Context;
use chrono::Utc;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::signal;
use tokio::sync::mpsc;

/// Monitors audio input levels for real-time feedback during recording
/// Provides smoothed RMS level calculations for TUI display
pub struct AudioLevelMonitor {
    last_update: Instant,
    level: f32,
}

impl AudioLevelMonitor {
    /// Create a new audio level monitor
    pub fn new() -> Self {
        Self {
            last_update: Instant::now(),
            level: 0.0,
        }
    }

    /// Update the audio level with new samples
    /// Returns the current level if enough time has passed for UI updates
    pub fn update_level(&mut self, samples: &[f32]) -> Option<f32> {
        // Calculate RMS (Root Mean Square) for audio level
        let sum_of_squares: f32 = samples.iter().map(|&s| s * s).sum();
        let rms = (sum_of_squares / samples.len() as f32).sqrt();

        // Smooth the level with a simple moving average
        self.level = self.level * 0.8 + rms * 0.2;

        // Return level every 100ms for TUI
        if self.last_update.elapsed() >= Duration::from_millis(100) {
            self.last_update = Instant::now();
            Some(self.level)
        } else {
            None
        }
    }
}

/// Get file paths and format info for recording based on compression settings
/// Returns (wav_path, final_path, format_string, needs_conversion)
fn setup_recording_paths(
    output_path: &PathBuf,
    compression_settings: &Option<CompressionSettings>,
) -> (PathBuf, PathBuf, String, bool) {
    let wav_file_path = BASE_PATH.join(&output_path).join("recording.wav");

    let (final_file_path, audio_format_str, needs_conversion) =
        if let Some(ref settings) = compression_settings {
            match settings.format {
                AudioFormat::Mp3 => {
                    let mp3_path = BASE_PATH.join(&output_path).join("recording.mp3");
                    (mp3_path, "mp3".to_string(), true)
                }
                _ => {
                    let filename = settings.get_filename("recording");
                    (
                        BASE_PATH.join(&output_path).join(filename),
                        settings.format.to_string().to_lowercase(),
                        false,
                    )
                }
            }
        } else {
            (wav_file_path.clone(), "wav".to_string(), false)
        };

    (
        wav_file_path,
        final_file_path,
        audio_format_str,
        needs_conversion,
    )
}

/// Create a database recording entry using unified metadata extraction
fn create_recording_entry(
    output_path: &PathBuf,
    final_file_path: &PathBuf,
) -> Result<Recording, anyhow::Error> {
    let meta = FileManager::extract_audio_metadata(final_file_path)?;

    Ok(Recording {
        id: None,
        directory_name: output_path.to_string_lossy().to_string(),
        display_name: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        duration_seconds: meta.duration_seconds,
        file_size_bytes: meta.file_size_bytes,
        audio_format: meta.audio_format,
        sample_rate: meta.sample_rate,
        channels: meta.channels,
        has_transcript: false,
        transcript_status: "pending".to_string(),
        language_code: "auto".to_string(),
        model_used: "whisper.cpp".to_string(),
        tags: None,
        summary: None,
        key_points: None,
        action_items: None,
        speakers: None,
        sentiment_score: None,
        search_index: None,
        categories: None,
        confidence_score: None,
        audio_path: final_file_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string(),
        transcript_path: None,
    })
}

/// Unified recording options controlling behavior and integrations
pub struct RecordOptions {
    pub compression_settings: Option<CompressionSettings>,
    pub stop_rx: Option<mpsc::Receiver<()>>, // None => wait for Ctrl+C
    pub level_tx: Option<mpsc::Sender<f32>>, // Some => send levels for UI
    pub verbose: bool,                       // Print CLI messages
}

impl Default for RecordOptions {
    fn default() -> Self {
        Self {
            compression_settings: None,
            stop_rx: None,
            level_tx: None,
            verbose: false,
        }
    }
}

/// Single, unified recording function.
/// - Records from default input device to WAV, optionally converts based on compression settings
/// - Stops on Ctrl+C or a provided stop channel
/// - Optionally streams audio levels to a provided channel
/// - Saves metadata to DB and returns the recording directory name
fn record_core(
    output_path: PathBuf,
    compression_settings: Option<CompressionSettings>,
    level_tx_opt: Option<mpsc::Sender<f32>>,
    verbose: bool,
    wait_stop: Box<dyn FnOnce()>,
) -> Result<String, anyhow::Error> {
    // Resolve input device and config
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No input device available"))?;

    let config = device
        .default_input_config()
        .map_err(|e| anyhow::anyhow!("Failed to get default input config: {}", e))?;

    // Determine final file's properties for DB based on compression settings
    // (kept for potential future use; actual metadata is extracted from the final file)
    let (_sample_rate, _channels) = if let Some(ref settings) = compression_settings {
        (settings.sample_rate as i64, settings.channels as i64)
    } else {
        (config.sample_rate().0 as i64, config.channels() as i64)
    };

    // Setup recording file paths
    let (wav_file_path, final_file_path, _audio_format_str, needs_conversion) =
        setup_recording_paths(&output_path, &compression_settings);

    // Ensure directory exists
    if let Some(parent) = wav_file_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Always encode raw WAV at device-native settings
    let recording_settings = CompressionSettings {
        format: AudioFormat::Wav,
        sample_rate: config.sample_rate().0,
        bitrate_kbps: None,
        channels: config.channels(),
        speech_optimized: false,
    };
    let encoder = create_encoder(&wav_file_path, &recording_settings)?;
    let encoder = Arc::new(Mutex::new(Some(encoder)));

    // Level monitoring
    let level_monitor = Arc::new(Mutex::new(AudioLevelMonitor::new()));

    if verbose {
        println!(
            "🎙️  Begin recording... (Press {} to stop)",
            if level_tx_opt.is_some() { "Esc in UI" } else { "Ctrl+C" }
        );
        println!("Device config: {:?}", config);
    }

    // Clone shared state for stream closure
    let encoder_2 = encoder.clone();
    let level_monitor_2 = level_monitor.clone();
    let level_tx_opt = level_tx_opt;

    let err_fn = move |err| {
        // Be quiet in TUI mode, loud in verbose mode
        eprintln!("an error occurred on stream: {}", err);
    };

    // Build input stream for device sample format
    let stream = match config.sample_format() {
        cpal::SampleFormat::I8 => device.build_input_stream(
            &config.clone().into(),
            move |data: &[i8], _: &_| {
                write_input_data_i8(data, &encoder_2, &level_monitor_2, level_tx_opt.as_ref())
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config.clone().into(),
            move |data: &[i16], _: &_| {
                write_input_data_i16(data, &encoder_2, &level_monitor_2, level_tx_opt.as_ref())
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I32 => device.build_input_stream(
            &config.clone().into(),
            move |data: &[i32], _: &_| {
                write_input_data_i32(data, &encoder_2, &level_monitor_2, level_tx_opt.as_ref())
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config.clone().into(),
            move |data: &[f32], _: &_| {
                write_input_data_f32(data, &encoder_2, &level_monitor_2, level_tx_opt.as_ref())
            },
            err_fn,
            None,
        )?,
        sample_format => {
            return Err(anyhow::Error::msg(format!(
                "Unsupported sample format '{sample_format}'"
            )))
        }
    };

    stream.play()?;

    // Wait for stop signal synchronously so this function remains usable from a Send future
    wait_stop();

    drop(stream);

    // Finalize encoder
    if let Some(mut enc) = encoder.lock().unwrap().take() {
        enc.finalize()?;
    }

    // Convert if required
    if needs_conversion {
        if let Some(ref settings) = compression_settings {
            convert_wav_to_mp3(&wav_file_path, &final_file_path, settings)
                .context("Failed to convert WAV to MP3")?;
            // Remove temp WAV
            let _ = std::fs::remove_file(&wav_file_path);
        }
    }

    let metadata_file_path = if needs_conversion {
        &final_file_path
    } else {
        &wav_file_path
    };

    // Save to DB
    let mut db = Database::new().context("Failed to connect to database")?;
    let recording = create_recording_entry(&output_path, metadata_file_path)?;

    match db.insert_recording(&recording) {
        Ok(id) => {
            if verbose {
                println!("📊 Recording saved to database with ID: {}", id);
            }
        }
        Err(e) => eprintln!("⚠️ Warning: Failed to save recording to database: {}", e),
    }

    if verbose {
        println!("✅ Recording complete: {}", metadata_file_path.display());
    }

    Ok(output_path.to_string_lossy().to_string())
}

pub async fn record_audio(
    output_path: PathBuf,
    options: RecordOptions,
) -> Result<String, anyhow::Error> {
    // Choose a synchronous wait strategy so the future remains Send when spawned
    let wait_strategy: Box<dyn FnOnce()> = match options.stop_rx {
        Some(mut rx) => Box::new(move || {
            // Avoid blocking a Tokio worker: wait on a dedicated OS thread
            let (tx, rx_done) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = rx.blocking_recv();
                let _ = tx.send(());
            });
            let _ = rx_done.recv();
        }),
        None => {
            // In CLI mode, wait for Ctrl+C without blocking a Tokio worker thread.
            // Use a dedicated OS thread that blocks on `ctrl_c()` via the runtime handle,
            // then signal back over a synchronous channel.
            let handle = tokio::runtime::Handle::current();
            Box::new(move || {
                let (tx, rx) = std::sync::mpsc::channel();
                let handle_clone = handle.clone();
                std::thread::spawn(move || {
                    let _ = handle_clone.block_on(signal::ctrl_c());
                    let _ = tx.send(());
                });
                let _ = rx.recv();
            })
        }
    };

    record_core(
        output_path,
        options.compression_settings,
        options.level_tx,
        options.verbose,
        wait_strategy,
    )
}

type AudioEncoderHandle = Arc<Mutex<Option<Box<dyn AudioEncoder>>>>;

// Unified sample handlers that optionally send level updates
fn write_input_data_f32(
    input: &[f32],
    encoder: &AudioEncoderHandle,
    level_monitor: &Arc<Mutex<AudioLevelMonitor>>,
    level_tx: Option<&mpsc::Sender<f32>>,
) {
    // Update the level monitor with f32 samples
    if let Ok(mut monitor) = level_monitor.try_lock() {
        if let Some(level) = monitor.update_level(input) {
            if let Some(tx) = level_tx {
                let _ = tx.try_send(level);
            }
        }
    }

    // Write audio data
    if let Ok(mut guard) = encoder.try_lock() {
        if let Some(encoder) = guard.as_mut() {
            let _ = encoder.encode_samples(input);
        }
    }
}

fn write_input_data_i16(
    input: &[i16],
    encoder: &AudioEncoderHandle,
    level_monitor: &Arc<Mutex<AudioLevelMonitor>>,
    level_tx: Option<&mpsc::Sender<f32>>,
) {
    let f32_samples: Vec<f32> = input.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
    write_input_data_f32(&f32_samples, encoder, level_monitor, level_tx);
}

fn write_input_data_i32(
    input: &[i32],
    encoder: &AudioEncoderHandle,
    level_monitor: &Arc<Mutex<AudioLevelMonitor>>,
    level_tx: Option<&mpsc::Sender<f32>>,
) {
    let f32_samples: Vec<f32> = input.iter().map(|&s| s as f32 / i32::MAX as f32).collect();
    write_input_data_f32(&f32_samples, encoder, level_monitor, level_tx);
}

fn write_input_data_i8(
    input: &[i8],
    encoder: &AudioEncoderHandle,
    level_monitor: &Arc<Mutex<AudioLevelMonitor>>,
    level_tx: Option<&mpsc::Sender<f32>>,
) {
    let f32_samples: Vec<f32> = input.iter().map(|&s| s as f32 / i8::MAX as f32).collect();
    write_input_data_f32(&f32_samples, encoder, level_monitor, level_tx);
}

/// Calculate the duration of an audio file in seconds
/// Supports WAV files (via hound) and MP3 files (via ffprobe with fallback estimation)
pub fn calculate_audio_duration(
    file_path: &std::path::Path,
    _sample_rate: i64,
    _channels: i64,
) -> Result<i64, anyhow::Error> {
    let extension = file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

    match extension.to_lowercase().as_str() {
        "wav" => {
            // Use hound to properly read the WAV file header and get accurate info
            let reader = hound::WavReader::open(file_path)
                .context("Failed to open WAV file for duration calculation")?;

            let spec = reader.spec();
            let wav_sample_rate = spec.sample_rate as i64;

            // Get the total number of samples
            let duration_samples = reader.duration() as i64;

            // Calculate duration in seconds
            let duration_seconds = duration_samples / wav_sample_rate;

            Ok(duration_seconds)
        }
        "mp3" => {
            // For MP3, use ffprobe for accurate duration calculation
            let output = std::process::Command::new("ffprobe")
                .arg("-v")
                .arg("quiet")
                .arg("-show_entries")
                .arg("format=duration")
                .arg("-of")
                .arg("csv=p=0")
                .arg(file_path)
                .output();

            match output {
                Ok(output) if output.status.success() => {
                    let duration_str = String::from_utf8_lossy(&output.stdout);
                    let duration_f64: f64 = duration_str.trim().parse().unwrap_or(1.0);
                    Ok(duration_f64.round() as i64)
                }
                _ => {
                    // Fallback: estimate using 32kbps bitrate (our default)
                    let file_size = std::fs::metadata(file_path)?.len() as i64;
                    // 32kbps = 32000 bits per second = 4000 bytes per second
                    let estimated_duration = file_size / 4000;
                    Ok(estimated_duration.max(1))
                }
            }
        }
        _ => {
            // Fallback: estimate based on provided sample rate and assume reasonable file size
            Ok(1) // Default to 1 second for unknown formats
        }
    }
}
