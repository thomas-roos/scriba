use std::path::PathBuf;
use anyhow::Context;
use std::sync::{Arc, Mutex};
use std::fs::File;
use cpal::traits::{HostTrait, DeviceTrait, StreamTrait};
use cpal::{FromSample, Sample};
use std::io::{BufWriter, Write};
use tokio::signal;
use lazy_static::lazy_static;
use dirs::home_dir;
use std::time::{Duration, Instant};

lazy_static! {
    static ref BASE_PATH: PathBuf = {
        let path = home_dir().expect("error home dir").join("scriba_recordings");
        if !path.exists() {
            std::fs::create_dir_all(&path).expect("Failed to create directory");
        }
        path
    };
}

struct AudioLevelMonitor {
    last_update: Instant,
    level: f32,
}

impl AudioLevelMonitor {
    fn new() -> Self {
        Self {
            last_update: Instant::now(),
            level: 0.0,
        }
    }

    fn update_level(&mut self, samples: &[f32]) {
        // Calculate RMS (Root Mean Square) for audio level
        let sum_of_squares: f32 = samples.iter().map(|&s| s * s).sum();
        let rms = (sum_of_squares / samples.len() as f32).sqrt();
        
        // Smooth the level with a simple moving average
        self.level = self.level * 0.8 + rms * 0.2;
        
        // Update display every 100ms
        if self.last_update.elapsed() >= Duration::from_millis(100) {
            self.display_level();
            self.last_update = Instant::now();
        }
    }

    fn display_level(&self) {
        // Create a retro ASCII VU meter style with better scaling for normal speech
        // Apply logarithmic scaling and amplify for realistic percentages
        let scaled_level = (self.level * 10.0).min(1.0); // Amplify by 10x, cap at 100%
        let level_percent = (scaled_level * 100.0) as usize;
        let level_bars = (scaled_level * 30.0) as usize;
        
        // Build the classic VU meter with different characters for different levels
        let mut meter = String::new();
        for i in 0..30 {
            if i < level_bars {
                if i < 10 {
                    meter.push('=');      // Low levels: ===
                } else if i < 20 {
                    meter.push('#');      // Mid levels: ###
                } else {
                    meter.push('!');      // High levels: !!!
                }
            } else {
                meter.push('-');         // Empty: ---
            }
        }
        
        // Classic terminal style with brackets and percentage
        print!("\r>> REC [{}] {:3}% <<   ", meter, level_percent.min(99));
        std::io::stdout().flush().unwrap();
    }
}

// Main recording function
pub async fn record(output_path: PathBuf) -> Result<(), anyhow::Error> {

    // Get the default input device
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .expect("No input device available");

    // Get the device default configuration
    let config = device
        .default_input_config()
        .expect("Failed to get default input config");
    println!("Default input config: {:?}", config);

    // The WAV file we're recording to.
    let file_path = BASE_PATH.join(output_path).join("recording.wav");
    
    // Ensure the recording directory exists
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    
    let spec = wav_spec_from_config(&config);
    let writer = hound::WavWriter::create(&file_path, spec)?;
    let writer = Arc::new(Mutex::new(Some(writer)));

    // Create audio level monitor
    let level_monitor = Arc::new(Mutex::new(AudioLevelMonitor::new()));

    // A flag to indicate that recording is in progress.
    println!("Begin recording... (Press Ctrl+C to stop)");
    println!();

    // Run the input stream on a separate thread.
    let writer_2 = writer.clone();
    let level_monitor_2 = level_monitor.clone();

    let err_fn = move |err| {
        eprintln!("an error occurred on stream: {}", err);
    };

    let stream = match config.sample_format() {
        cpal::SampleFormat::I8 => device.build_input_stream(
            &config.into(),
            move |data, _: &_| write_input_data_with_monitoring_i8(data, &writer_2, &level_monitor_2),
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config.into(),
            move |data, _: &_| write_input_data_with_monitoring_i16(data, &writer_2, &level_monitor_2),
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I32 => device.build_input_stream(
            &config.into(),
            move |data, _: &_| write_input_data_with_monitoring_i32(data, &writer_2, &level_monitor_2),
            err_fn,
            None,
        )?,
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config.into(),
            move |data, _: &_| write_input_data_with_monitoring_f32(data, &writer_2, &level_monitor_2),
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

    signal::ctrl_c().await.context("ctrl c error")?;

    drop(stream);

    // Clear the level display line and show completion message
    print!("\r");
    println!("Recording {} complete!", file_path.display());
    
    writer.lock().unwrap().take().unwrap().finalize()?;
    Ok(())
}

fn wav_spec_from_config(config: &cpal::SupportedStreamConfig) -> hound::WavSpec {
    hound::WavSpec {
        channels: config.channels() as _,
        sample_rate: config.sample_rate().0 as _,
        bits_per_sample: (config.sample_format().sample_size() * 8) as _,
        sample_format: sample_format(config.sample_format()),
    }
}

type WavWriterHandle = Arc<Mutex<Option<hound::WavWriter<BufWriter<File>>>>>;

fn sample_format(format: cpal::SampleFormat) -> hound::SampleFormat {
    if format.is_float() {
        hound::SampleFormat::Float
    } else {
        hound::SampleFormat::Int
    }
}

// Specialized functions for level monitoring with audio level feedback
fn write_input_data_with_monitoring_f32(input: &[f32], writer: &WavWriterHandle, level_monitor: &Arc<Mutex<AudioLevelMonitor>>) {
    // Update the level monitor with f32 samples
    if let Ok(mut monitor) = level_monitor.try_lock() {
        monitor.update_level(input);
    }
    
    // Write audio data
    if let Ok(mut guard) = writer.try_lock() {
        if let Some(writer) = guard.as_mut() {
            for &sample in input.iter() {
                writer.write_sample(sample).ok();
            }
        }
    }
}

fn write_input_data_with_monitoring_i16(input: &[i16], writer: &WavWriterHandle, level_monitor: &Arc<Mutex<AudioLevelMonitor>>) {
    // Convert i16 to f32 for level calculation
    let f32_samples: Vec<f32> = input.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
    
    if let Ok(mut monitor) = level_monitor.try_lock() {
        monitor.update_level(&f32_samples);
    }
    
    // Write audio data
    if let Ok(mut guard) = writer.try_lock() {
        if let Some(writer) = guard.as_mut() {
            for &sample in input.iter() {
                writer.write_sample(sample).ok();
            }
        }
    }
}

fn write_input_data_with_monitoring_i32(input: &[i32], writer: &WavWriterHandle, level_monitor: &Arc<Mutex<AudioLevelMonitor>>) {
    // Convert i32 to f32 for level calculation
    let f32_samples: Vec<f32> = input.iter().map(|&s| s as f32 / i32::MAX as f32).collect();
    
    if let Ok(mut monitor) = level_monitor.try_lock() {
        monitor.update_level(&f32_samples);
    }
    
    // Write audio data
    if let Ok(mut guard) = writer.try_lock() {
        if let Some(writer) = guard.as_mut() {
            for &sample in input.iter() {
                writer.write_sample(sample).ok();
            }
        }
    }
}

fn write_input_data_with_monitoring_i8(input: &[i8], writer: &WavWriterHandle, level_monitor: &Arc<Mutex<AudioLevelMonitor>>) {
    // Convert i8 to f32 for level calculation
    let f32_samples: Vec<f32> = input.iter().map(|&s| s as f32 / i8::MAX as f32).collect();
    
    if let Ok(mut monitor) = level_monitor.try_lock() {
        monitor.update_level(&f32_samples);
    }
    
    // Write audio data
    if let Ok(mut guard) = writer.try_lock() {
        if let Some(writer) = guard.as_mut() {
            for &sample in input.iter() {
                writer.write_sample(sample).ok();
            }
        }
    }
}