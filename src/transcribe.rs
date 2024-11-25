use std::path::PathBuf;
use anyhow::Context;
use reqwest::Client;
use reqwest::multipart::{Form, Part};
use std::fs::File;
use std::io::BufWriter;
use lazy_static::lazy_static;
use dirs::home_dir;
use std::io::Write;

lazy_static! {
    static ref BASE_PATH: PathBuf = home_dir().expect("error home dir").join("scriba_recordings");
}

// Mock transcription function
pub async fn transcribe_file(
    input_path: &PathBuf,
    output_path: &PathBuf,
    api_key: &str,
) -> Result<(), anyhow::Error> {
    // Read the content of the WAV file
    let audio_file_path = BASE_PATH.join(input_path).join("recording.wav");
    let audio_file = std::fs::read(&audio_file_path)
        .context("Unable to read the input file")?;

    // Set up the reqwest client
    let client = Client::new();

    // Create a multipart form with the audio file and model parameter
    let form = Form::new()
        .part("file", Part::bytes(audio_file).file_name("audio.m4a"))
        .text("model", "whisper-1");

    // Make the POST request to the OpenAI API
    let response = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .multipart(form)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "multipart/form-data")
        .send()
        .await
        .context("Failed to send request")?;

    if response.status().is_success() {
        let text = response.text().await.context("Failed to read response body")?;

        // Save the transcript to the specified or default file
        let transcript_file_path = BASE_PATH.join(output_path).join("transcript.txt");
        let transcript_file = File::create(&transcript_file_path)?;
        let mut transcript_writer = BufWriter::new(transcript_file);
        transcript_writer.write_all(text.as_bytes())?;

        println!("Transcription saved to: {:?}", output_path);
        println!("Transcription: {text}");
    

        Ok(())
    } else {
        eprintln!("Error: {}", response.status());

        let text = response.text().await.context("Failed to read response body")?;
        eprintln!("Error Body: {text:#?}");
        Err(anyhow::Error::msg("Transcription failed"))
    }
}
