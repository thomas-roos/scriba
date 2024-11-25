use std::env;
use std::path::PathBuf;
use structopt::StructOpt;
use lazy_static::lazy_static;
use dirs::home_dir;
use scriba::record::record;
use scriba::transcribe::transcribe_file;

lazy_static! {
    static ref BASE_PATH: PathBuf = home_dir().expect("error home dir").join("scriba_recordings");
}

#[derive(Debug, StructOpt)]
enum Command {
    Record {
        #[structopt(parse(from_os_str), help = "Specify the output audio file path")]
        audio_output: PathBuf,
        #[structopt(short = "s", long = "skip_transcription", help = "Skip transcription after recording")]
        skip_transcription: bool,
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
        } => {
            // Start the recording task
            let record_result = record(audio_output.clone()).await;

            if !skip_transcription {
                // Transcribe the recorded file
                if let Some(output_path) = &transcript_output {
                    match transcribe_file(&audio_output, output_path, &api_key).await {
                        Ok(()) => {
                            println!("Transcription saved to: {:?}", output_path);
                        }
                        Err(err) => {
                            eprintln!("Error during transcription: {err}");
                        }
                    }
                } else {
                    println!("Transcription skipped.");
                }
            }

            // Return the result of the record task
            record_result
        }
        Command::Transcribe { input, output } => {
            // Transcribe the specified file
            let transcription = transcribe_file(&input, &output, &api_key).await;
            transcription
        }
    };

    if let Err(err) = result {
        eprintln!("An error happened: {err}");
    }
}
