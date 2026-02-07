//! Knowledge extraction and enrichment module.
//!
//! This module provides AI-powered extraction of metadata from transcripts,
//! including summaries, topics, entities (people, organizations), and action items.
//!
//! The module also manages "Scriba's World" - an evolving understanding of
//! the owner that grows with every conversation.

mod context;
mod extractor;
mod ollama;
mod prompts;
mod world;

pub use extractor::{
    EnrichmentService, ExtractedEntity, ExtractionResult,
    WorldEntityExtractionResult, WorldEntityOrganization, WorldEntityPerson,
};
pub use ollama::{OllamaClient, OllamaError};
pub use world::{WorldContext, WorldData};
