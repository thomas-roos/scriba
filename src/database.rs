use rusqlite::{Connection, Result as SqliteResult, params};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use dirs::home_dir;
use anyhow::{Context, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    pub id: Option<i64>,
    pub directory_name: String,
    pub display_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub duration_seconds: Option<i64>,
    pub file_size_bytes: Option<i64>,
    pub audio_format: String,
    pub sample_rate: i64,
    pub channels: i64,
    pub has_transcript: bool,
    pub transcript_status: String,
    pub language_code: String,
    pub model_used: String,
    pub tags: Option<String>, // JSON
    pub summary: Option<String>,
    pub key_points: Option<String>, // JSON
    pub action_items: Option<String>, // JSON
    pub speakers: Option<String>, // JSON
    pub sentiment_score: Option<f64>,
    pub search_index: Option<String>,
    pub categories: Option<String>, // JSON
    pub confidence_score: Option<f64>,
    pub audio_path: String,
    pub transcript_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub id: Option<i64>,
    pub recording_id: i64,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub word_count: Option<i64>,
    pub character_count: Option<i64>,
    pub language_detected: Option<String>,
    pub confidence_scores: Option<String>, // JSON
    pub segments: Option<String>, // JSON
    pub entities: Option<String>, // JSON
    pub topics: Option<String>, // JSON
}

#[derive(Debug, Clone)]
pub struct Tag {
    pub id: Option<i64>,
    pub name: String,
    pub color: String,
    pub created_at: DateTime<Utc>,
    pub usage_count: i64,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new() -> Result<Self> {
        let db_path = Self::get_database_path()?;
        
        // Ensure the directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        let conn = Connection::open(&db_path)
            .context("Failed to open database connection")?;
        
        let mut db = Database { conn };
        db.initialize()?;
        Ok(db)
    }
    
    fn get_database_path() -> Result<PathBuf> {
        let home = home_dir().context("Could not find home directory")?;
        Ok(home.join("scriba_recordings").join("scriba.db"))
    }
    
    fn initialize(&mut self) -> Result<()> {
        // Read and execute schema
        let schema = include_str!("../schema.sql");
        self.conn.execute_batch(schema)
            .context("Failed to initialize database schema")?;
        Ok(())
    }
    
    // Recording operations
    pub fn insert_recording(&mut self, recording: &Recording) -> Result<i64> {
        let sql = r#"
            INSERT INTO recordings (
                directory_name, display_name, created_at, updated_at,
                duration_seconds, file_size_bytes, audio_format, sample_rate,
                channels, has_transcript, transcript_status, language_code,
                model_used, audio_path
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14
            )
        "#;
        
        let result = self.conn.execute(sql, params![
            recording.directory_name,
            recording.display_name,
            recording.created_at,
            recording.updated_at,
            recording.duration_seconds,
            recording.file_size_bytes,
            recording.audio_format,
            recording.sample_rate,
            recording.channels,
            recording.has_transcript,
            recording.transcript_status,
            recording.language_code,
            recording.model_used,
            recording.audio_path,
        ])?;
        
        Ok(self.conn.last_insert_rowid())
    }
    
    pub fn get_recording_by_directory(&self, directory_name: &str) -> Result<Option<Recording>> {
        let sql = "SELECT * FROM recordings WHERE directory_name = ?1";
        
        let mut stmt = self.conn.prepare(sql)?;
        let mut rows = stmt.query_map([directory_name], |row| {
            Ok(Recording {
                id: Some(row.get(0)?),
                directory_name: row.get(1)?,
                display_name: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                duration_seconds: row.get(5)?,
                file_size_bytes: row.get(6)?,
                audio_format: row.get(7)?,
                sample_rate: row.get(8)?,
                channels: row.get(9)?,
                has_transcript: row.get(10)?,
                transcript_status: row.get(11)?,
                language_code: row.get(12)?,
                model_used: row.get(13)?,
                tags: row.get(14)?,
                summary: row.get(15)?,
                key_points: row.get(16)?,
                action_items: row.get(17)?,
                speakers: row.get(18)?,
                sentiment_score: row.get(19)?,
                search_index: row.get(20)?,
                categories: row.get(21)?,
                confidence_score: row.get(22)?,
                audio_path: row.get(23)?,
                transcript_path: row.get(24)?,
            })
        })?;
        
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }
    
    pub fn list_recordings(&self, limit: Option<i64>, offset: Option<i64>) -> Result<Vec<Recording>> {
        let sql = match (limit, offset) {
            (Some(l), Some(o)) => format!("SELECT * FROM recordings ORDER BY created_at DESC LIMIT {} OFFSET {}", l, o),
            (Some(l), None) => format!("SELECT * FROM recordings ORDER BY created_at DESC LIMIT {}", l),
            _ => "SELECT * FROM recordings ORDER BY created_at DESC".to_string(),
        };
        
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(Recording {
                id: Some(row.get(0)?),
                directory_name: row.get(1)?,
                display_name: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                duration_seconds: row.get(5)?,
                file_size_bytes: row.get(6)?,
                audio_format: row.get(7)?,
                sample_rate: row.get(8)?,
                channels: row.get(9)?,
                has_transcript: row.get(10)?,
                transcript_status: row.get(11)?,
                language_code: row.get(12)?,
                model_used: row.get(13)?,
                tags: row.get(14)?,
                summary: row.get(15)?,
                key_points: row.get(16)?,
                action_items: row.get(17)?,
                speakers: row.get(18)?,
                sentiment_score: row.get(19)?,
                search_index: row.get(20)?,
                categories: row.get(21)?,
                confidence_score: row.get(22)?,
                audio_path: row.get(23)?,
                transcript_path: row.get(24)?,
            })
        })?;
        
        let mut recordings = Vec::new();
        for row in rows {
            recordings.push(row?);
        }
        Ok(recordings)
    }
    
    pub fn update_recording_transcript_status(&mut self, id: i64, status: &str, has_transcript: bool) -> Result<()> {
        let sql = "UPDATE recordings SET transcript_status = ?1, has_transcript = ?2, updated_at = ?3 WHERE id = ?4";
        self.conn.execute(sql, params![status, has_transcript, Utc::now(), id])?;
        Ok(())
    }
    
    pub fn delete_recording(&mut self, id: i64) -> Result<()> {
        let sql = "DELETE FROM recordings WHERE id = ?1";
        self.conn.execute(sql, params![id])?;
        Ok(())
    }
    
    // Transcript operations
    pub fn insert_transcript(&mut self, transcript: &Transcript) -> Result<i64> {
        let sql = r#"
            INSERT INTO transcripts (
                recording_id, content, created_at, updated_at,
                word_count, character_count, language_detected
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#;
        
        let word_count = transcript.content.split_whitespace().count() as i64;
        let character_count = transcript.content.chars().count() as i64;
        
        self.conn.execute(sql, params![
            transcript.recording_id,
            transcript.content,
            transcript.created_at,
            transcript.updated_at,
            word_count,
            character_count,
            transcript.language_detected,
        ])?;
        
        Ok(self.conn.last_insert_rowid())
    }
    
    pub fn get_transcript_by_recording_id(&self, recording_id: i64) -> Result<Option<Transcript>> {
        let sql = "SELECT * FROM transcripts WHERE recording_id = ?1";
        
        let mut stmt = self.conn.prepare(sql)?;
        let mut rows = stmt.query_map([recording_id], |row| {
            Ok(Transcript {
                id: Some(row.get(0)?),
                recording_id: row.get(1)?,
                content: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                word_count: row.get(5)?,
                character_count: row.get(6)?,
                language_detected: row.get(7)?,
                confidence_scores: row.get(8)?,
                segments: row.get(9)?,
                entities: row.get(10)?,
                topics: row.get(11)?,
            })
        })?;
        
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }
    
    // Search operations (for future AI features)
    pub fn search_transcripts(&self, query: &str, limit: Option<i64>) -> Result<Vec<(Recording, Transcript)>> {
        let sql = match limit {
            Some(l) => format!(r#"
                SELECT r.*, t.* FROM recordings r
                JOIN transcripts t ON r.id = t.recording_id
                JOIN transcripts_fts ON transcripts_fts.rowid = t.id
                WHERE transcripts_fts MATCH ?1
                ORDER BY rank
                LIMIT {}
            "#, l),
            None => r#"
                SELECT r.*, t.* FROM recordings r
                JOIN transcripts t ON r.id = t.recording_id
                JOIN transcripts_fts ON transcripts_fts.rowid = t.id
                WHERE transcripts_fts MATCH ?1
                ORDER BY rank
            "#.to_string(),
        };
        
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([query], |row| {
            let recording = Recording {
                id: Some(row.get(0)?),
                directory_name: row.get(1)?,
                display_name: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                duration_seconds: row.get(5)?,
                file_size_bytes: row.get(6)?,
                audio_format: row.get(7)?,
                sample_rate: row.get(8)?,
                channels: row.get(9)?,
                has_transcript: row.get(10)?,
                transcript_status: row.get(11)?,
                language_code: row.get(12)?,
                model_used: row.get(13)?,
                tags: row.get(14)?,
                summary: row.get(15)?,
                key_points: row.get(16)?,
                action_items: row.get(17)?,
                speakers: row.get(18)?,
                sentiment_score: row.get(19)?,
                search_index: row.get(20)?,
                categories: row.get(21)?,
                confidence_score: row.get(22)?,
                audio_path: row.get(23)?,
                transcript_path: row.get(24)?,
            };
            
            let transcript = Transcript {
                id: Some(row.get(25)?),
                recording_id: row.get(26)?,
                content: row.get(27)?,
                created_at: row.get(28)?,
                updated_at: row.get(29)?,
                word_count: row.get(30)?,
                character_count: row.get(31)?,
                language_detected: row.get(32)?,
                confidence_scores: row.get(33)?,
                segments: row.get(34)?,
                entities: row.get(35)?,
                topics: row.get(36)?,
            };
            
            Ok((recording, transcript))
        })?;
        
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }
    
    // Statistics for future dashboard
    pub fn get_stats(&self) -> Result<RecordingStats> {
        let sql = r#"
            SELECT 
                COUNT(*) as total_recordings,
                SUM(duration_seconds) as total_duration,
                SUM(file_size_bytes) as total_size,
                COUNT(CASE WHEN has_transcript = 1 THEN 1 END) as transcribed_count,
                SUM(CASE WHEN t.word_count IS NOT NULL THEN t.word_count ELSE 0 END) as total_words
            FROM recordings r
            LEFT JOIN transcripts t ON r.id = t.recording_id
        "#;
        
        let mut stmt = self.conn.prepare(sql)?;
        let row = stmt.query_row([], |row| {
            Ok(RecordingStats {
                total_recordings: row.get(0)?,
                total_duration_seconds: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                total_size_bytes: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                transcribed_count: row.get(3)?,
                total_words: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
            })
        })?;
        
        Ok(row)
    }
}

#[derive(Debug)]
pub struct RecordingStats {
    pub total_recordings: i64,
    pub total_duration_seconds: i64,
    pub total_size_bytes: i64,
    pub transcribed_count: i64,
    pub total_words: i64,
}

impl RecordingStats {
    pub fn format_duration(&self) -> String {
        let hours = self.total_duration_seconds / 3600;
        let minutes = (self.total_duration_seconds % 3600) / 60;
        let seconds = self.total_duration_seconds % 60;
        
        if hours > 0 {
            format!("{}h {}m {}s", hours, minutes, seconds)
        } else if minutes > 0 {
            format!("{}m {}s", minutes, seconds)
        } else {
            format!("{}s", seconds)
        }
    }
    
    pub fn format_size(&self) -> String {
        const KB: i64 = 1024;
        const MB: i64 = KB * 1024;
        const GB: i64 = MB * 1024;
        
        if self.total_size_bytes >= GB {
            format!("{:.1} GB", self.total_size_bytes as f64 / GB as f64)
        } else if self.total_size_bytes >= MB {
            format!("{:.1} MB", self.total_size_bytes as f64 / MB as f64)
        } else if self.total_size_bytes >= KB {
            format!("{:.1} KB", self.total_size_bytes as f64 / KB as f64)
        } else {
            format!("{} bytes", self.total_size_bytes)
        }
    }
}