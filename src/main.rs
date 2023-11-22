use std::env;
use std::path::PathBuf;
use anyhow::Context;
use structopt::StructOpt;
use reqwest::Client;
use reqwest::multipart::{Form, Part};
use std::sync::{Arc, Mutex};
use std::fs::File;
use cpal::traits::{HostTrait, DeviceTrait,StreamTrait};
use cpal::{FromSample, Sample};
use std::io::BufWriter;
use tokio::signal;
use lazy_static::lazy_static;
use dirs::home_dir;
use std::io::Write;

lazy_static! {
    static ref BASE_PATH: PathBuf = home_dir().expect("error home dir").join("scriba_recordings");
}

#[derive(Debug, StructOpt)]
enum Command {
    Record {
        #[structopt(parse(from_os_str), help = "Specify the output audio file path")]
        audio_output: PathBuf,
        #[structopt(
            long = "skip_transcription",
            help = "Skip transcription after recording",
        )]
        skip_transcription: bool,
        #[structopt(
            long = "transcript_output",
            help = "Specify the output transcript file path (optional)"
        )]
        transcript_output: PathBuf,
    },
    Transcribe {
        #[structopt(parse(from_os_str), help = "Specify the input recording file path")]
        input: PathBuf,
        #[structopt(parse(from_os_str), help = "Specify the output transcript file path")]
        output: PathBuf,
    },
}


#[derive(Debug, StructOpt)]
#[structopt(name = "scriba", about = "A simple CLI tool for recording and transcribing")]
struct Cli {
    #[structopt(subcommand)]
    command: Command,
}

// Mock transcription function
async fn transcribe_file(
    input_path: &PathBuf,
    output_path: &PathBuf,
    api_key: &str,
) -> Result<(), anyhow::Error> {
    // Read the content of the WAV file
    let audio_file_path = BASE_PATH.join(input_path);
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
        let transcript_file_path = BASE_PATH.join(output_path);
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

fn write_input_data<T, U>(input: &[T], writer: &WavWriterHandle)
where
    T: Sample,
    U: Sample + hound::Sample + FromSample<T>,
{
    if let Ok(mut guard) = writer.try_lock() {
        if let Some(writer) = guard.as_mut() {
            for &sample in input.iter() {
                let sample: U = U::from_sample(sample);
                writer.write_sample(sample).ok();
            }
        }
    }
}

// Main recording function
async fn record(output_path: PathBuf) -> Result<(), anyhow::Error> {

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
    let file_path = BASE_PATH.join(output_path);
    let spec = wav_spec_from_config(&config);
    let writer = hound::WavWriter::create(&file_path, spec)?;
    let writer = Arc::new(Mutex::new(Some(writer)));

    // A flag to indicate that recording is in progress.
    println!("Begin recording...");

    // Run the input stream on a separate thread.
    let writer_2 = writer.clone();

    let err_fn = move |err| {
        eprintln!("an error occurred on stream: {}", err);
    };

    let stream = match config.sample_format() {
        cpal::SampleFormat::I8 => device.build_input_stream(
            &config.into(),
            move |data, _: &_| write_input_data::<i8, i8>(data, &writer_2),
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config.into(),
            move |data, _: &_| write_input_data::<i16, i16>(data, &writer_2),
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I32 => device.build_input_stream(
            &config.into(),
            move |data, _: &_| write_input_data::<i32, i32>(data, &writer_2),
            err_fn,
            None,
        )?,
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config.into(),
            move |data, _: &_| write_input_data::<f32, f32>(data, &writer_2),
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

    writer.lock().unwrap().take().unwrap().finalize()?;
    println!("Recording {} complete!", file_path.display());
    Ok(())
}

#[tokio::main]
async fn main() {
    // Load environment variables from .env file
    dotenv::dotenv().ok();

    // Retrieve the OpenAI API key from the environment
    let api_key = env::var("OPENAI_API_KEY")
        .expect("Please set the OPENAI_API_KEY environment variable in the .env file");

    let cli = Cli::from_args();

    let result = match cli.command {
        Command::Record {
            audio_output,
            skip_transcription,
            transcript_output,
        } => {
            // Start the recording task
            let record_result = record(audio_output.clone()).await;

            if !skip_transcription {
                // Transcribe the recorded file
                match transcribe_file(&audio_output, &transcript_output, &api_key).await {
                    Ok(()) => {
                        println!("Transcription saved to: {:?}", transcript_output);
                    }
                    Err(err) => {
                        eprintln!("Error during transcription: {err}");
                    }
                }
            }

            // Return the result of the record task
            record_result
        }
        Command::Transcribe { input, output } => {
            // Transcribe the specified file
            let _transcription = transcribe_file(&input, &output, &api_key).await;
            _transcription
        }
    };

    if let Err(err) = result {
        eprintln!("An error happened: {err}");
    }
}
