//! Extraction service for knowledge extraction from transcripts.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::provider::LlmProvider;
use super::prompts;
use super::search::EntitySearchResults;
use super::world::WorldData;
use crate::core::config::EnrichmentConfig;

/// Result of extracting metadata from a transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    /// AI-generated title for the recording.
    pub title: String,
    /// Brief summary of the content.
    pub summary: String,
    /// Main topics discussed.
    pub topics: Vec<String>,
    /// People mentioned in the transcript.
    pub people: Vec<ExtractedEntity>,
    /// Organizations mentioned in the transcript.
    pub organizations: Vec<ExtractedEntity>,
    /// Key points or insights.
    pub key_points: Vec<String>,
    /// Action items or tasks mentioned.
    pub action_items: Vec<String>,
}

/// An entity (person or organization) extracted from a transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    /// Name as mentioned in the transcript.
    pub name: String,
    /// Context about this entity from the transcript.
    pub context: String,
    /// If this entity matches a known entity from the world, the canonical name.
    /// None means this is a genuinely new entity.
    #[serde(default)]
    pub resolved_to: Option<String>,
}

/// Result of context update.
#[derive(Debug, Clone, Deserialize)]
pub struct ContextUpdateResult {
    pub updated_context: String,
    pub new_facts: Vec<String>,
}

/// Entity extracted from world description.
#[derive(Debug, Clone, Deserialize)]
pub struct WorldEntityPerson {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub is_owner: bool,
}

/// Organization extracted from world description.
#[derive(Debug, Clone, Deserialize)]
pub struct WorldEntityOrganization {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub context: String,
}

/// Result of extracting entities from world description.
#[derive(Debug, Clone, Deserialize)]
pub struct WorldEntityExtractionResult {
    #[serde(default)]
    pub people: Vec<WorldEntityPerson>,
    #[serde(default)]
    pub organizations: Vec<WorldEntityOrganization>,
}

/// Service for extracting knowledge from transcripts using LLM.
pub struct EnrichmentService {
    provider: Box<dyn LlmProvider>,
}

impl EnrichmentService {
    /// Create from an LLM provider.
    pub fn new(provider: Box<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    /// Create from enrichment configuration.
    pub fn from_config(config: &EnrichmentConfig) -> Self {
        Self {
            provider: super::create_provider(config),
        }
    }

    /// Check if the enrichment service is available.
    pub async fn health_check(&self) -> Result<()> {
        self.provider
            .health_check()
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
            .context("Enrichment service health check failed")
    }

    /// Get the provider display name.
    pub fn provider_display_name(&self) -> &str {
        self.provider.display_name()
    }

    /// Get the model name in use.
    pub fn model(&self) -> &str {
        self.provider.model()
    }

    /// Extract metadata from a transcript.
    pub async fn extract(&self, transcript: &str) -> Result<ExtractionResult> {
        let prompt = prompts::build_extraction_prompt(transcript);

        let response = self
            .provider
            .generate(&prompt)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
            .context("Failed to generate extraction")?;

        let cleaned = strip_markdown_fences(&response);

        let result: ExtractionResult =
            serde_json::from_str(cleaned).context("Failed to parse extraction result")?;

        Ok(result)
    }

    /// Update entity context with new mentions.
    pub async fn update_entity_context(
        &self,
        entity_name: &str,
        entity_type: &str,
        existing_context: &str,
        new_mentions: &[(&str, &str)],
    ) -> Result<ContextUpdateResult> {
        let prompt = prompts::build_context_update_prompt(
            entity_name,
            entity_type,
            existing_context,
            new_mentions,
        );

        let response = self
            .provider
            .generate(&prompt)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
            .context("Failed to update entity context")?;

        let cleaned = strip_markdown_fences(&response);

        let result: ContextUpdateResult =
            serde_json::from_str(cleaned).context("Failed to parse context update result")?;

        Ok(result)
    }

    /// Extract metadata with world context.
    ///
    /// The world context contains everything the LLM needs: owner profile,
    /// known people, organizations, etc. The LLM resolves entity mentions
    /// inline against the world.
    pub async fn extract_with_full_context(
        &self,
        transcript: &str,
        world_context: Option<&str>,
    ) -> Result<ExtractionResult> {
        let prompt =
            prompts::build_full_context_extraction_prompt(transcript, world_context);

        let response = self
            .provider
            .generate(&prompt)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
            .context("Failed to generate full-context extraction")?;

        let cleaned = strip_markdown_fences(&response);

        let result: ExtractionResult =
            serde_json::from_str(cleaned).context("Failed to parse extraction result")?;

        Ok(result)
    }

    /// Evolve the world by extracting a conservative JSON delta from a new recording.
    ///
    /// Returns the parsed delta as `WorldData`. The caller is responsible for
    /// merging it into the existing world via `WorldData::merge()`.
    /// Returns `None` if the LLM response couldn't be parsed (non-fatal).
    pub async fn evolve_world(
        &self,
        current_world: &str,
        transcript: &str,
        extraction: &ExtractionResult,
    ) -> Result<Option<WorldData>> {
        let extraction_summary = format!(
            "Title: {}\nSummary: {}\nTopics: {}\nPeople: {}\nOrganizations: {}",
            extraction.title,
            extraction.summary,
            extraction.topics.join(", "),
            extraction
                .people
                .iter()
                .map(|p| format!("{} ({})", p.name, p.context))
                .collect::<Vec<_>>()
                .join(", "),
            extraction
                .organizations
                .iter()
                .map(|o| format!("{} ({})", o.name, o.context))
                .collect::<Vec<_>>()
                .join(", ")
        );

        let prompt =
            prompts::build_world_evolution_prompt(current_world, transcript, &extraction_summary);

        let response = self
            .provider
            .generate(&prompt)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
            .context("Failed to evolve world description")?;

        let cleaned = strip_markdown_fences(&response);

        match WorldData::from_json(cleaned) {
            Ok(delta) => Ok(Some(delta)),
            Err(e) => {
                eprintln!("  Warning: could not parse world evolution response as JSON: {}", e);
                Ok(None)
            }
        }
    }

    /// Extract entities from a world description.
    pub async fn extract_world_entities(
        &self,
        world_content: &str,
    ) -> Result<WorldEntityExtractionResult> {
        let prompt = prompts::build_world_entity_extraction_prompt(world_content);

        let response = self
            .provider
            .generate(&prompt)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
            .context("Failed to extract entities from world description")?;

        let cleaned = strip_markdown_fences(&response);

        let result: WorldEntityExtractionResult = serde_json::from_str(cleaned)
            .context("Failed to parse world entity extraction result")?;

        Ok(result)
    }

    /// Extract a structured world profile from a free-form seed description.
    pub async fn extract_world_seed(&self, seed_content: &str) -> Result<WorldData> {
        let prompt = prompts::build_world_seed_extraction_prompt(seed_content);

        let response = self
            .provider
            .generate(&prompt)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
            .context("Failed to extract world seed")?;

        let cleaned = strip_markdown_fences(&response);

        WorldData::from_json(cleaned).context("Failed to parse world seed extraction result")
    }

    /// Compact an entity's context by merging existing + new info into a clean description.
    pub async fn compact_entity_context(
        &self,
        entity_name: &str,
        entity_type: &str,
        existing_context: &str,
        new_info: &str,
    ) -> Result<String> {
        let prompt = prompts::build_context_compaction_prompt(
            entity_name,
            entity_type,
            existing_context,
            new_info,
        );

        let response = self
            .provider
            .generate(&prompt)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
            .context("Failed to compact entity context")?;

        let compacted = response
            .trim()
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim_matches('"')
            .trim()
            .to_string();

        if compacted.is_empty() || compacted.starts_with('{') || compacted.starts_with('[') {
            Ok(format!("{}. {}", existing_context.trim_end_matches('.'), new_info))
        } else {
            Ok(compacted)
        }
    }

    /// Identify speakers in a diarized transcript using world context.
    pub async fn identify_speakers(
        &self,
        diarized_text: &str,
        world_context: Option<&str>,
        num_speakers: usize,
    ) -> Result<std::collections::HashMap<String, Option<String>>> {
        let prompt = prompts::build_speaker_identification_prompt(
            diarized_text,
            world_context,
            num_speakers,
        );

        let response = self
            .provider
            .generate(&prompt)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
            .context("Failed to identify speakers")?;

        let cleaned = strip_markdown_fences(&response);

        let parsed: serde_json::Value =
            serde_json::from_str(cleaned).context("Failed to parse speaker identification JSON")?;

        let mut result = std::collections::HashMap::new();

        if let Some(speakers) = parsed.get("speakers").and_then(|s| s.as_object()) {
            for (key, value) in speakers {
                let resolved = if value.is_null() {
                    None
                } else {
                    value.as_str().map(|s| s.to_string())
                };
                result.insert(key.clone(), resolved);
            }
        }

        Ok(result)
    }

    /// Resolve unresolved entities using web search results.
    ///
    /// Takes search results for unresolved entities plus a compact world summary,
    /// builds a resolution prompt, and asks the LLM to cross-reference.
    /// Returns a map from transcript name -> canonical name (or None for genuinely new).
    /// On any failure, returns an empty map (non-fatal).
    pub async fn resolve_with_search(
        &self,
        search_results: &[EntitySearchResults],
        world_summary: &str,
    ) -> HashMap<String, Option<String>> {
        // Build the input tuples for the prompt: (name, type, context, formatted_search_results)
        let unresolved: Vec<(String, String, String, String)> = search_results
            .iter()
            .map(|sr| {
                let formatted = sr
                    .results
                    .iter()
                    .map(|r| {
                        format!("    - \"{}\" ({}) — {}", r.title, r.url, r.snippet)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    sr.entity_name.clone(),
                    sr.entity_type.clone(),
                    sr.entity_context.clone(),
                    formatted,
                )
            })
            .collect();

        let prompt = prompts::build_search_resolution_prompt(&unresolved, world_summary);

        let response = match self
            .provider
            .generate(&prompt)
            .await
        {
            Ok(r) => r,
            Err(_) => return HashMap::new(),
        };

        let cleaned = strip_markdown_fences(&response);

        // Parse: { "resolutions": { "Name": "Canonical" | null } }
        let parsed: serde_json::Value = match serde_json::from_str(cleaned) {
            Ok(v) => v,
            Err(_) => return HashMap::new(),
        };

        let mut result = HashMap::new();

        if let Some(resolutions) = parsed.get("resolutions").and_then(|r| r.as_object()) {
            for (key, value) in resolutions {
                let resolved = if value.is_null() {
                    None
                } else {
                    value.as_str().map(|s| s.to_string())
                };
                result.insert(key.clone(), resolved);
            }
        }

        result
    }
}

/// Strip markdown fences from LLM responses.
fn strip_markdown_fences(response: &str) -> &str {
    response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
}

impl ExtractionResult {
    /// Get all extracted entities (people + organizations) with their type.
    pub fn all_entities(&self) -> Vec<(&ExtractedEntity, &str)> {
        let mut entities: Vec<(&ExtractedEntity, &str)> = self.people
            .iter()
            .map(|p| (p, "person"))
            .collect();
        entities.extend(self.organizations.iter().map(|o| (o, "organization")));
        entities
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extraction_result_parsing() {
        let json = r#"{
            "title": "Test Meeting",
            "summary": "A test meeting about things.",
            "topics": ["testing", "meetings"],
            "people": [{"name": "John", "context": "The host"}],
            "organizations": [{"name": "Acme", "context": "The company"}],
            "key_points": ["Point 1"],
            "action_items": ["Do something"]
        }"#;

        let result: ExtractionResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.title, "Test Meeting");
        assert_eq!(result.topics.len(), 2);
        assert_eq!(result.people.len(), 1);
        assert_eq!(result.people[0].name, "John");
        assert!(result.people[0].resolved_to.is_none());
    }

    #[test]
    fn test_extraction_result_with_resolved_to() {
        let json = r#"{
            "title": "Test",
            "summary": "Test",
            "topics": [],
            "people": [{"name": "Gerardo", "context": "discussing budget", "resolved_to": "Gerardo Gagliardo"}],
            "organizations": [{"name": "Exane", "context": "their product", "resolved_to": "Exein"}],
            "key_points": [],
            "action_items": []
        }"#;

        let result: ExtractionResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.people[0].resolved_to.as_deref(), Some("Gerardo Gagliardo"));
        assert_eq!(result.organizations[0].resolved_to.as_deref(), Some("Exein"));
    }

    #[test]
    fn test_all_entities() {
        let result = ExtractionResult {
            title: "Test".to_string(),
            summary: "Test summary".to_string(),
            topics: vec![],
            people: vec![ExtractedEntity {
                name: "John".to_string(),
                context: "Engineer".to_string(),
                resolved_to: None,
            }],
            organizations: vec![ExtractedEntity {
                name: "Acme".to_string(),
                context: "Company".to_string(),
                resolved_to: Some("Acme Corp".to_string()),
            }],
            key_points: vec![],
            action_items: vec![],
        };

        let entities = result.all_entities();
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].1, "person");
        assert_eq!(entities[1].1, "organization");
        assert_eq!(entities[1].0.resolved_to.as_deref(), Some("Acme Corp"));
    }
}
