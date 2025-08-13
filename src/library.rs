use crate::database::{Database, Recording, RecordingStats};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::io::{self, stdout, Write};
use std::path::PathBuf;
use dirs::home_dir;

pub struct RecordingLibrary {
    db: Database,
    current_page: usize,
    page_size: usize,
    selected_index: usize,
    recordings: Vec<Recording>,
}

impl RecordingLibrary {
    pub fn new() -> Result<Self> {
        let db = Database::new()?;
        Ok(Self {
            db,
            current_page: 0,
            page_size: 10,
            selected_index: 0,
            recordings: Vec::new(),
        })
    }

    pub async fn show_library(&mut self) -> Result<()> {
        self.load_recordings()?;

        if self.recordings.is_empty() {
            self.show_empty_library();
            return Ok(());
        }

        loop {
            self.display_library()?;
            
            match self.get_user_input()? {
                LibraryAction::Quit => break,
                LibraryAction::NextPage => self.next_page(),
                LibraryAction::PrevPage => self.prev_page(),
                LibraryAction::Up => self.move_selection_up(),
                LibraryAction::Down => self.move_selection_down(),
                LibraryAction::Play => {
                    if let Some(recording) = self.get_selected_recording() {
                        self.play_recording(recording)?;
                    }
                }
                LibraryAction::Delete => {
                    if let Some(recording) = self.get_selected_recording() {
                        self.delete_recording(recording.clone())?;
                        self.load_recordings()?;
                    }
                }
                LibraryAction::ShowStats => self.show_stats()?,
                LibraryAction::Search => self.search_recordings().await?,
                LibraryAction::Invalid => {
                    // Show help briefly
                    println!("❌ Invalid command. Use: ↑/↓ to navigate, Enter to play, D to delete, Q to quit");
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            }
        }

        Ok(())
    }

    fn load_recordings(&mut self) -> Result<()> {
        let offset = (self.current_page * self.page_size) as i64;
        self.recordings = self.db.list_recordings(Some(self.page_size as i64), Some(offset))?;
        
        // Reset selection if we're beyond the current page
        if self.selected_index >= self.recordings.len() && !self.recordings.is_empty() {
            self.selected_index = 0;
        }
        
        Ok(())
    }

    fn display_library(&self) -> Result<()> {
        // Clear screen
        print!("\x1B[2J\x1B[H");
        stdout().flush()?;

        println!("╔────────────────────────────────────────────────────────╗");
        println!("║                   RECORDING LIBRARY                    ║");
        println!("╠────────────────────────────────────────────────────────╣");
        println!("║  Navigation: ↑/↓ • Play: Enter • Delete: D • Quit: Q  ║");
        println!("║  Stats: S • Search: / • Pages: ←/→                    ║");
        println!("╚────────────────────────────────────────────────────────╝");
        println!();

        if self.recordings.is_empty() {
            println!("┌────────────────────────────────────────────────────────┐");
            println!("│                    No recordings found                 │");
            println!("│              Start recording to see them here!        │");
            println!("└────────────────────────────────────────────────────────┘");
            return Ok(());
        }

        // Show current page info
        let total_recordings = self.recordings.len();
        let start_index = self.current_page * self.page_size + 1;
        let end_index = std::cmp::min(start_index + total_recordings - 1, start_index + self.page_size - 1);
        
        println!("📄 Page {} • Showing {}-{} of {} recordings\n", 
                 self.current_page + 1, start_index, end_index, total_recordings);

        // Display recordings
        for (i, recording) in self.recordings.iter().enumerate() {
            let is_selected = i == self.selected_index;
            let selection_marker = if is_selected { "►" } else { " " };
            
            let display_name = recording.display_name.as_ref()
                .unwrap_or(&recording.directory_name);
            
            let duration = recording.duration_seconds
                .map(|d| self.format_duration(d))
                .unwrap_or_else(|| "Unknown".to_string());
                
            let transcript_status = if recording.has_transcript {
                "📝"
            } else {
                "🎵"
            };
            
            let created_date = recording.created_at.format("%Y-%m-%d %H:%M");
            
            if is_selected {
                println!("┌────────────────────────────────────────────────────────┐");
                println!("│{}🎙️  {} {}                                    │", 
                         selection_marker, transcript_status, display_name);
                println!("│   📅 {}  ⏱️  {}                        │", 
                         created_date, duration);
                println!("└────────────────────────────────────────────────────────┘");
            } else {
                println!("│{}🎙️  {} {}                                    │", 
                         selection_marker, transcript_status, display_name);
                println!("│   📅 {}  ⏱️  {}                        │", 
                         created_date, duration);
                println!("├────────────────────────────────────────────────────────┤");
            }
        }

        println!("\n💡 Tip: Press 'S' to see your recording statistics!");
        Ok(())
    }

    fn show_empty_library(&self) {
        println!("╔────────────────────────────────────────────────────────╗");
        println!("║                   RECORDING LIBRARY                    ║");
        println!("╚────────────────────────────────────────────────────────╝");
        println!();
        println!("┌────────────────────────────────────────────────────────┐");
        println!("│                    📭 Empty Library                    │");
        println!("│                                                        │");
        println!("│  You haven't made any recordings yet!                 │");
        println!("│  Go back to the main menu to start your first         │");
        println!("│  recording session.                                   │");
        println!("│                                                        │");
        println!("│                Press Enter to continue                │");
        println!("└────────────────────────────────────────────────────────┘");
        
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
    }

    fn get_user_input(&self) -> Result<LibraryAction> {
        print!(">> ");
        stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim().to_lowercase();

        match input.as_str() {
            "q" | "quit" | "exit" => Ok(LibraryAction::Quit),
            "j" | "down" => Ok(LibraryAction::Down),
            "k" | "up" => Ok(LibraryAction::Up),
            "" | "enter" => Ok(LibraryAction::Play),
            "d" | "delete" => Ok(LibraryAction::Delete),
            "s" | "stats" => Ok(LibraryAction::ShowStats),
            "/" | "search" => Ok(LibraryAction::Search),
            "h" | "left" => Ok(LibraryAction::PrevPage),
            "l" | "right" => Ok(LibraryAction::NextPage),
            _ => Ok(LibraryAction::Invalid),
        }
    }

    fn get_selected_recording(&self) -> Option<&Recording> {
        self.recordings.get(self.selected_index)
    }

    fn move_selection_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    fn move_selection_down(&mut self) {
        if self.selected_index + 1 < self.recordings.len() {
            self.selected_index += 1;
        }
    }

    fn next_page(&mut self) {
        if self.recordings.len() == self.page_size {
            self.current_page += 1;
            self.selected_index = 0;
            if let Err(_) = self.load_recordings() {
                // If loading fails, go back to previous page
                self.current_page -= 1;
                let _ = self.load_recordings();
            }
        }
    }

    fn prev_page(&mut self) {
        if self.current_page > 0 {
            self.current_page -= 1;
            self.selected_index = 0;
            let _ = self.load_recordings();
        }
    }

    fn play_recording(&self, recording: &Recording) -> Result<()> {
        println!("\n🎵 Playing: {}", 
                 recording.display_name.as_ref().unwrap_or(&recording.directory_name));
        
        let base_path = home_dir()
            .context("Could not find home directory")?
            .join("scriba_recordings");
        let audio_path = base_path.join(&recording.directory_name).join("recording.wav");
        
        if !audio_path.exists() {
            println!("❌ Audio file not found: {}", audio_path.display());
            println!("Press Enter to continue...");
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            return Ok(());
        }

        // For now, just show file info. In the future, we'll add actual playback
        println!("📁 Location: {}", audio_path.display());
        if let Some(duration) = recording.duration_seconds {
            println!("⏱️  Duration: {}", self.format_duration(duration));
        }
        if recording.has_transcript {
            println!("📝 Transcript available");
            
            // Show transcript if it exists
            let transcript_path = base_path.join(&recording.directory_name).join("transcript.txt");
            if transcript_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&transcript_path) {
                    println!("\n╭─ TRANSCRIPT ───────────────────────────────────────────╮");
                    let lines: Vec<&str> = content.lines().collect();
                    for line in lines.iter().take(5) {
                        if line.len() > 54 {
                            println!("│ {}... │", &line[..51]);
                        } else {
                            println!("│ {}{} │", line, " ".repeat(54 - line.len()));
                        }
                    }
                    if lines.len() > 5 {
                        println!("│ ... ({} more lines)                                  │", lines.len() - 5);
                    }
                    println!("╰────────────────────────────────────────────────────────╯");
                }
            }
        }

        println!("\n💡 Future: Audio playback will be implemented here!");
        println!("Press Enter to continue...");
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        
        Ok(())
    }

    fn delete_recording(&mut self, recording: Recording) -> Result<()> {
        println!("\n⚠️  Are you sure you want to delete this recording?");
        println!("   📁 {}", recording.display_name.as_ref().unwrap_or(&recording.directory_name));
        print!("   Type 'yes' to confirm: ");
        stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        
        if input.trim().to_lowercase() == "yes" {
            // Delete from database
            if let Some(id) = recording.id {
                self.db.delete_recording(id)?;
                
                // Delete files
                let base_path = home_dir()
                    .context("Could not find home directory")?
                    .join("scriba_recordings");
                let recording_dir = base_path.join(&recording.directory_name);
                
                if recording_dir.exists() {
                    std::fs::remove_dir_all(&recording_dir)?;
                }
                
                println!("✅ Recording deleted successfully!");
            }
        } else {
            println!("❌ Deletion cancelled.");
        }
        
        println!("Press Enter to continue...");
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        
        Ok(())
    }

    fn show_stats(&self) -> Result<()> {
        let stats = self.db.get_stats()?;
        
        println!("\n╔────────────────────────────────────────────────────────╗");
        println!("║                  RECORDING STATISTICS                  ║");
        println!("╠────────────────────────────────────────────────────────╣");
        println!("║  📊 Total Recordings: {:>32}      ║", stats.total_recordings);
        println!("║  ⏱️  Total Duration: {:>33}      ║", stats.format_duration());
        println!("║  💾 Total Storage: {:>35}      ║", stats.format_size());
        println!("║  📝 Transcribed: {:>37}      ║", stats.transcribed_count);
        println!("║  🔤 Total Words: {:>37}      ║", stats.total_words);
        if stats.total_recordings > 0 {
            let avg_duration = stats.total_duration_seconds / stats.total_recordings;
            println!("║  📊 Avg Duration: {:>34}      ║", self.format_duration(avg_duration));
        }
        println!("╚────────────────────────────────────────────────────────╝");
        
        println!("\n💡 Your personal knowledge base is growing!");
        println!("Press Enter to continue...");
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        
        Ok(())
    }

    async fn search_recordings(&mut self) -> Result<()> {
        print!("\n🔍 Search recordings (enter keywords): ");
        stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let query = input.trim();
        
        if query.is_empty() {
            return Ok(());
        }

        println!("🔍 Searching for '{}'...", query);
        
        match self.db.search_transcripts(query, Some(10)) {
            Ok(results) => {
                if results.is_empty() {
                    println!("❌ No recordings found matching '{}'", query);
                } else {
                    println!("✅ Found {} matching recording(s):", results.len());
                    println!();
                    
                    for (i, (recording, _transcript)) in results.iter().enumerate() {
                        println!("{}. 🎙️  {}", 
                                 i + 1, 
                                 recording.display_name.as_ref().unwrap_or(&recording.directory_name));
                        println!("   📅 {}", recording.created_at.format("%Y-%m-%d %H:%M"));
                        println!();
                    }
                }
            }
            Err(e) => {
                println!("❌ Search failed: {}", e);
                println!("💡 Make sure you have transcribed recordings to search through.");
            }
        }
        
        println!("Press Enter to continue...");
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        
        Ok(())
    }

    fn format_duration(&self, seconds: i64) -> String {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        let secs = seconds % 60;
        
        if hours > 0 {
            format!("{}h {}m {}s", hours, minutes, secs)
        } else if minutes > 0 {
            format!("{}m {}s", minutes, secs)
        } else {
            format!("{}s", secs)
        }
    }
}

#[derive(Debug)]
enum LibraryAction {
    Quit,
    Up,
    Down,
    NextPage,
    PrevPage,
    Play,
    Delete,
    ShowStats,
    Search,
    Invalid,
}