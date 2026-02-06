//! Extraction service for knowledge extraction from transcripts.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::ollama::OllamaClient;
use super::prompts;

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
}

/// A mention of an entity with surrounding context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityMention {
    /// The text of the mention as it appears in the transcript.
    pub mention_text: String,
    /// Surrounding context (~100 chars around the mention).
    pub context_snippet: String,
    /// Type of entity: "person" or "organization".
    pub entity_type: String,
}

/// Result of entity linking check.
#[derive(Debug, Clone, Deserialize)]
pub struct LinkingResult {
    pub is_match: bool,
    pub confidence: f32,
    pub reasoning: String,
}

/// Result of context update.
#[derive(Debug, Clone, Deserialize)]
pub struct ContextUpdateResult {
    pub updated_context: String,
    pub new_facts: Vec<String>,
}

/// Service for extracting knowledge from transcripts using LLM.
pub struct EnrichmentService {
    client: OllamaClient,
}

impl EnrichmentService {
    /// Create a new enrichment service.
    pub fn new(ollama_endpoint: &str, ollama_model: &str) -> Self {
        Self {
            client: OllamaClient::new(ollama_endpoint, ollama_model),
        }
    }

    /// Create from an existing Ollama client.
    pub fn with_client(client: OllamaClient) -> Self {
        Self { client }
    }

    /// Check if the enrichment service is available (Ollama running with model).
    pub async fn health_check(&self) -> Result<()> {
        self.client
            .health_check()
            .await
            .context("Enrichment service health check failed")
    }

    /// Extract metadata from a transcript.
    pub async fn extract(&self, transcript: &str) -> Result<ExtractionResult> {
        let prompt = prompts::build_extraction_prompt(transcript);

        let response = self
            .client
            .generate(&prompt)
            .await
            .context("Failed to generate extraction")?;

        // Parse the JSON response
        let result: ExtractionResult =
            serde_json::from_str(&response).context("Failed to parse extraction result")?;

        Ok(result)
    }

    /// Check if a mention matches a known entity.
    pub async fn check_entity_match(
        &self,
        mention_text: &str,
        mention_context: &str,
        entity_name: &str,
        entity_type: &str,
        entity_context: &str,
    ) -> Result<LinkingResult> {
        let prompt = prompts::build_entity_linking_prompt(
            mention_text,
            mention_context,
            entity_name,
            entity_type,
            entity_context,
        );

        let response = self
            .client
            .generate(&prompt)
            .await
            .context("Failed to check entity match")?;

        let result: LinkingResult =
            serde_json::from_str(&response).context("Failed to parse linking result")?;

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
            .client
            .generate(&prompt)
            .await
            .context("Failed to update entity context")?;

        let result: ContextUpdateResult =
            serde_json::from_str(&response).context("Failed to parse context update result")?;

        Ok(result)
    }

    /// Get the underlying Ollama client.
    pub fn client(&self) -> &OllamaClient {
        &self.client
    }
}

impl ExtractionResult {
    /// Convert people to entity mentions.
    pub fn people_mentions(&self) -> Vec<EntityMention> {
        self.people
            .iter()
            .map(|p| EntityMention {
                mention_text: p.name.clone(),
                context_snippet: p.context.clone(),
                entity_type: "person".to_string(),
            })
            .collect()
    }

    /// Convert organizations to entity mentions.
    pub fn organization_mentions(&self) -> Vec<EntityMention> {
        self.organizations
            .iter()
            .map(|o| EntityMention {
                mention_text: o.name.clone(),
                context_snippet: o.context.clone(),
                entity_type: "organization".to_string(),
            })
            .collect()
    }

    /// Get all entity mentions (people + organizations).
    pub fn all_mentions(&self) -> Vec<EntityMention> {
        let mut mentions = self.people_mentions();
        mentions.extend(self.organization_mentions());
        mentions
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
    }

    #[test]
    fn test_entity_mentions_conversion() {
        let result = ExtractionResult {
            title: "Test".to_string(),
            summary: "Test summary".to_string(),
            topics: vec![],
            people: vec![ExtractedEntity {
                name: "John".to_string(),
                context: "Engineer".to_string(),
            }],
            organizations: vec![ExtractedEntity {
                name: "Acme".to_string(),
                context: "Company".to_string(),
            }],
            key_points: vec![],
            action_items: vec![],
        };

        let mentions = result.all_mentions();
        assert_eq!(mentions.len(), 2);
        assert_eq!(mentions[0].entity_type, "person");
        assert_eq!(mentions[1].entity_type, "organization");
    }
}
