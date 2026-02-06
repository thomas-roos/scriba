//! Knowledge extraction and enrichment module.
//!
//! This module provides AI-powered extraction of metadata from transcripts,
//! including summaries, topics, entities (people, organizations), and action items.

mod ollama;
mod extractor;
mod prompts;

pub use ollama::{OllamaClient, OllamaError};
pub use extractor::{EnrichmentService, ExtractionResult, EntityMention, ExtractedEntity};
