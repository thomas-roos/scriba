use chrono::Local;
use dirs::home_dir;
use lazy_static::lazy_static;
use std::path::PathBuf;

lazy_static! {
    /// Base path for all scriba recordings and data
    pub static ref BASE_PATH: PathBuf = {
        let path = home_dir()
            .expect("Could not find home directory")
            .join("scriba_recordings");
        if !path.exists() {
            std::fs::create_dir_all(&path).expect("Failed to create scriba_recordings directory");
        }
        path
    };
}

/// Generate a timestamp-based filename for recordings
/// Sanitizes user-provided names and ensures filesystem compatibility
pub fn generate_recording_name(name: Option<String>) -> String {
    let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    match name {
        Some(n) => {
            let sanitized = sanitize_filename(&n);
            format!("{}_{}", timestamp, sanitized)
        }
        None => format!("{}_recording", timestamp),
    }
}

/// Sanitize a string to be safe for use as a filename
/// Replaces spaces with dashes and removes/replaces unsafe characters
pub fn sanitize_filename(input: &str) -> String {
    input
        .replace(' ', "-")
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
}
