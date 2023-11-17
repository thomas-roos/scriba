use std::env;
use std::path::PathBuf;
use structopt::StructOpt;
use reqwest::blocking::Client;
use reqwest::blocking::multipart::{Form, Part};

#[derive(Debug, StructOpt)]
enum Command {
    Record {
        #[structopt(parse(from_os_str), help = "Specify the output file path")]
        output: PathBuf,
    },
    Transcribe {
        #[structopt(parse(from_os_str), help = "Specify the input recording file path")]
        input: PathBuf,
    },
}

#[derive(Debug, StructOpt)]
#[structopt(name = "my-cli-tool", about = "A simple CLI tool for recording and transcribing")]
struct Cli {
    #[structopt(subcommand)]
    command: Command,
}

// Mock transcription function
fn transcribe_file(input_path: &PathBuf, api_key: &str) {
    // Get the path to the "recordings" folder
    let recordings_path = std::env::current_dir()
        .expect("Failed to get current directory")
        .join("recordings");

    // Construct the full path to the input file within the "recordings" folder
    let full_path = recordings_path.join(input_path);

    // Read the content of the WAV file
    let audio_file = std::fs::read(&full_path).expect("Unable to read the input file");

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
        .expect("Failed to send request");

    if response.status().is_success() {
        println!("{}", response.text().expect("Failed to read response body"));
    } else {
        eprintln!("Error: {}", response.status());
        eprintln!("Error Body: {:#?}", response.text());
    }
}

fn main() {
    // Load environment variables from .env file
    dotenv::dotenv().ok();

    // Retrieve the OpenAI API key from the environment
    let api_key = env::var("OPENAI_API_KEY")
        .expect("Please set the OPENAI_API_KEY environment variable in the .env file");

    let cli = Cli::from_args();

    match cli.command {
        Command::Record { output } => {
            println!("Recording to file: {:?}", output);
            // Implement recording logic here
        }
        Command::Transcribe { input } => {
            transcribe_file(&input, &api_key);
        }
    }
}
