use std::path::PathBuf;
use anyhow::Context;
use reqwest::Client;
use reqwest::multipart::{Form, Part};
use std::fs::File;
use std::io::{BufWriter, Write};
use lazy_static::lazy_static;
use dirs::home_dir;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use std::io::stdout;
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

lazy_static! {
    static ref BASE_PATH: PathBuf = home_dir().expect("error home dir").join("scriba_recordings");
}

struct TranscriptionProgress {
    start_time: Instant,
    animation_frame: usize,
}

impl TranscriptionProgress {
    fn new() -> Self {
        Self {
            start_time: Instant::now(),
            animation_frame: 0,
        }
    }

    async fn show_progress(&mut self) {
        let elapsed = self.start_time.elapsed().as_secs();
        
        // Rotating spinner animation
        let spinner_chars = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let spinner = spinner_chars[self.animation_frame % spinner_chars.len()];
        
        // Progress messages that change over time
        let message = match elapsed {
            0..=3 => "Uploading audio file",
            4..=8 => "OpenAI is processing your audio",
            9..=15 => "Converting speech to text",
            16..=25 => "This is taking longer than usual",
            _ => "Almost there, hang tight"
        };
        
        // Show elapsed time
        let time_display = if elapsed < 60 {
            format!("{}s", elapsed)
        } else {
            format!("{}m {}s", elapsed / 60, elapsed % 60)
        };
        
        // Animated progress bar
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
        
        print!("\r🎵 {} [{}] {} - {}", spinner, bar_str, message, time_display);
        stdout().flush().unwrap();
        
        self.animation_frame += 1;
        sleep(Duration::from_millis(100)).await;
    }
    
}

async fn show_transcription_typing_effect(text: &str) {
    const BOX_WIDTH: usize = 58;
    const CONTENT_WIDTH: usize = BOX_WIDTH - 4; // Account for "│ " and " │"
    
    println!("\n╭─ TRANSCRIPTION RESULT ─────────────────────────────────╮");
    
    // Split text into words and wrap properly
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut current_line = String::new();
    let mut lines = Vec::new();
    
    for word in words {
        if current_line.is_empty() {
            current_line = word.to_string();
        } else {
            let test_line = format!("{} {}", current_line, word);
            if test_line.width() <= CONTENT_WIDTH {
                current_line = test_line;
            } else {
                lines.push(current_line);
                current_line = word.to_string();
            }
        }
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }
    
    // Show each line with typing effect
    for line in lines {
        print!("│ ");
        stdout().flush().unwrap();
        
        for char in line.chars() {
            print!("{}", char);
            stdout().flush().unwrap();
            sleep(Duration::from_millis(20)).await;
        }
        
        // Add padding to reach the box width using Unicode-aware width calculation
        let line_width = line.width();
        let padding = CONTENT_WIDTH - line_width;
        print!("{} │", " ".repeat(padding));
        println!();
    }
    
    println!("╰────────────────────────────────────────────────────────╯");
}

// Enhanced transcription function with delightful UX
pub async fn transcribe_file(
    input_path: &PathBuf,
    output_path: &PathBuf,
    api_key: &str,
) -> Result<(), anyhow::Error> {
    // Read the content of the WAV file
    let audio_file_path = BASE_PATH.join(input_path).join("recording.wav");
    let audio_file = std::fs::read(&audio_file_path)
        .context("Unable to read the input file")?;

    // Create progress indicator
    let mut progress = TranscriptionProgress::new();
    
    // Show initial progress
    println!("\n🎤 → 📝 Transcribing your audio...\n");

    // Set up the reqwest client
    let client = Client::new();

    // Create a multipart form with the audio file and model parameter
    let form = Form::new()
        .part("file", Part::bytes(audio_file).file_name("audio.m4a"))
        .text("model", "whisper-1");

    // Start the progress animation in background
    let progress_task = tokio::spawn(async move {
        loop {
            progress.show_progress().await;
        }
    });

    // Make the POST request to the OpenAI API
    let response = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .multipart(form)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .context("Failed to send request")?;
    
    // Stop the progress animation
    progress_task.abort();
    
    // Clear progress line
    print!("\r{}", " ".repeat(80));
    print!("\r");
    stdout().flush().unwrap();

    if response.status().is_success() {
        let text = response.text().await.context("Failed to read response body")?;

        // Parse the JSON response to extract just the text
        let json: Value = serde_json::from_str(&text)?;
        let transcription_text = json["text"].as_str().unwrap_or(&text);

        // Save the transcript to the specified or default file
        let transcript_file_path = BASE_PATH.join(output_path).join("transcript.txt");
        
        // Ensure the directory exists
        if let Some(parent) = transcript_file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        let transcript_file = File::create(&transcript_file_path)?;
        let mut transcript_writer = BufWriter::new(transcript_file);
        transcript_writer.write_all(transcription_text.as_bytes())?;

        // Show success animation
        println!("✨ Transcription complete! ✨");
        
        // Show the transcription with typing effect
        show_transcription_typing_effect(transcription_text).await;

        println!("\n📁 Transcript saved to: ~/scriba_recordings/{}/transcript.txt", output_path.display());

        Ok(())
    } else {
        println!("❌ Transcription failed!");
        eprintln!("Error: {}", response.status());

        let text = response.text().await.context("Failed to read response body")?;
        eprintln!("Error Body: {text:#?}");
        Err(anyhow::Error::msg("Transcription failed"))
    }
}
