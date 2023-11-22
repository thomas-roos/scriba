use std::path::PathBuf;
use anyhow::Context;
use std::sync::{Arc, Mutex};
use std::fs::File;
use cpal::traits::{HostTrait, DeviceTrait,StreamTrait};
use cpal::{FromSample, Sample};
use std::io::BufWriter;
use tokio::signal;
use lazy_static::lazy_static;
use dirs::home_dir;

lazy_static! {
    static ref BASE_PATH: PathBuf = {
        let path = home_dir().expect("error home dir").join("scriba_recordings");
        if !path.exists() {
            std::fs::create_dir_all(&path).expect("Failed to create directory");
        }
        path
    };
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