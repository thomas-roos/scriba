use std::env;
use std::path::PathBuf;
use structopt::StructOpt;
use scriba::record::record;
use scriba::transcribe::transcribe_file;
use anyhow::{Context, Result};
use chrono::Local;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const ASCII_ART: &str = r#"
 ███████  ██████ ██████  ██ ██████   █████  
██      ██      ██   ██ ██ ██   ██ ██   ██ 
███████ ██      ██████  ██ ██████  ███████ 
     ██ ██      ██   ██ ██ ██   ██ ██   ██ 
███████  ██████ ██   ██ ██ ██████  ██   ██ 
                                           
"#;

#[derive(Debug, StructOpt)]
enum Command {
    Record {
        #[structopt(short = "n", long = "name", help = "Optional name/description for the recording (auto-generated if not provided)")]
        name: Option<String>,
        #[structopt(short = "s", long = "skip-transcription", help = "Skip transcription after recording")]
        skip_transcription: bool,
        #[structopt(long = "api-key", help = "OpenAI API key (optional, falls back to OPENAI_API_KEY env var)")]
        api_key: Option<String>,
    },
    Transcribe {
        #[structopt(parse(from_os_str), help = "Path to the input recording file or directory name")]
        input: PathBuf,
        #[structopt(short = "n", long = "name", help = "Optional name for the transcript (auto-generated if not provided)")]
        name: Option<String>,
        #[structopt(long = "api-key", help = "OpenAI API key (optional, falls back to OPENAI_API_KEY env var)")]
        api_key: Option<String>,
    },
}


#[derive(Debug, StructOpt)]
#[structopt(name = "scriba", about = "A simple CLI tool for recording and transcribing", version = VERSION)]
struct Cli {
    #[structopt(subcommand)]
    command: Command,
}

fn get_api_key(provided_key: Option<String>) -> Result<String> {
    if let Some(key) = provided_key {
        Ok(key)
    } else {
        env::var("OPENAI_API_KEY")
            .context("Please provide an API key using --api-key or set the OPENAI_API_KEY environment variable")
    }
}

fn generate_filename(name: Option<String>) -> String {
    let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    match name {
        Some(n) => {
            let sanitized = n.replace(' ', "-").replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
            format!("{}_{}", timestamp, sanitized)
        },
        None => format!("{}_recording", timestamp),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Display ASCII art banner
    println!("{}", ASCII_ART);
    println!("Scriba v{} - Audio Recording and Transcription Tool", VERSION);
    println!("{}", "=".repeat(55));
    println!();

    // Load environment variables from .env file
    dotenv::dotenv().ok();

    let cli = Cli::from_args();

    let result = match cli.command {
        Command::Record {
            name,
            skip_transcription,
            api_key,
        } => {
            // Generate automatic filename
            let recording_name = generate_filename(name.clone());
            let audio_output = PathBuf::from(&recording_name);
            
            // Get API key from parameter or environment
            let api_key = get_api_key(api_key)?;
            // Start the recording task
            let record_result = record(audio_output.clone()).await;

            if !skip_transcription {
                // Use the same directory as the recording
                let transcript_output = audio_output.clone();
                
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
        Command::Transcribe { input, name, api_key } => {
            // Generate automatic transcript filename if needed
            let output = if let Some(n) = name {
                let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
                let sanitized = n.replace(' ', "-").replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
                PathBuf::from(format!("{}_{}_transcript", timestamp, sanitized))
            } else {
                let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
                PathBuf::from(format!("{}_transcript", timestamp))
            };
            
            // Get API key from parameter or environment
            let api_key = get_api_key(api_key)?;
            // Transcribe the specified file
            let transcription = transcribe_file(&input, &output, &api_key).await;
            transcription
        }
    };

    if let Err(err) = result {
        eprintln!("An error happened: {err}");
    }
    
    Ok(())
}
