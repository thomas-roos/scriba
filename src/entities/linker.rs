//! Smart entity linking using LLM.

use anyhow::Result;

use crate::database::Database;
use crate::enrichment::{EnrichmentService, EntityMention, ExtractionResult};

use super::registry::EntityRegistry;

/// Smart entity linker that uses LLM to correlate mentions with known entities.
pub struct EntityLinker {
    enrichment_service: EnrichmentService,
    confidence_threshold: f32,
}

impl EntityLinker {
    /// Create a new entity linker.
    pub fn new(enrichment_service: EnrichmentService, confidence_threshold: f32) -> Self {
        Self {
            enrichment_service,
            confidence_threshold,
        }
    }

    /// Process extraction results and link/create entities.
    pub async fn process_extraction(
        &self,
        db: &mut Database,
        recording_id: i64,
        extraction: &ExtractionResult,
    ) -> Result<LinkingReport> {
        let mut report = LinkingReport::default();

        // Process all entity mentions (people + organizations)
        let mentions = extraction.all_mentions();

        for mention in mentions {
            let result = self
                .process_mention(db, recording_id, &mention)
                .await?;

            match result {
                MentionResult::LinkedToExisting { entity_id, confidence } => {
                    report.linked_existing += 1;
                    report.details.push(LinkDetail {
                        mention_text: mention.mention_text.clone(),
                        action: LinkAction::LinkedExisting,
                        entity_id: Some(entity_id),
                        confidence,
                    });
                }
                MentionResult::CreatedNew { entity_id } => {
                    report.created_new += 1;
                    report.details.push(LinkDetail {
                        mention_text: mention.mention_text.clone(),
                        action: LinkAction::CreatedNew,
                        entity_id: Some(entity_id),
                        confidence: 1.0,
                    });
                }
            }
        }

        Ok(report)
    }

    /// Process a single mention - either link to existing entity or create new one.
    async fn process_mention(
        &self,
        db: &mut Database,
        recording_id: i64,
        mention: &EntityMention,
    ) -> Result<MentionResult> {
        let mut registry = EntityRegistry::new(db);

        // Find candidate entities that might match this mention
        let candidates = registry.find_candidate_entities(&mention.mention_text)?;

        if candidates.is_empty() {
            // No candidates - create a new entity
            let entity = registry.create_entity(
                &mention.entity_type,
                &mention.mention_text,
                Some(&mention.context_snippet),
            )?;

            // Create the mention record linked to the new entity
            registry.create_mention(
                recording_id,
                &mention.mention_text,
                Some(&mention.context_snippet),
                entity.id,
                1.0,
            )?;

            return Ok(MentionResult::CreatedNew {
                entity_id: entity.id.unwrap(),
            });
        }

        // Try to match with existing entities using LLM
        for candidate in &candidates {
            let linking_result = self
                .enrichment_service
                .check_entity_match(
                    &mention.mention_text,
                    &mention.context_snippet,
                    &candidate.canonical_name,
                    &candidate.entity_type,
                    candidate.context.as_deref().unwrap_or(""),
                )
                .await;

            match linking_result {
                Ok(result) if result.is_match && result.confidence >= self.confidence_threshold => {
                    // Found a match - link to existing entity
                    let entity_id = candidate.id.unwrap();

                    // Create mention record
                    registry.create_mention(
                        recording_id,
                        &mention.mention_text,
                        Some(&mention.context_snippet),
                        Some(entity_id),
                        result.confidence as f64,
                    )?;

                    // Record the mention (increment count)
                    registry.record_mention(entity_id)?;

                    // Add the mention text as an alias if different from canonical name
                    if mention.mention_text.to_lowercase()
                        != candidate.canonical_name.to_lowercase()
                    {
                        registry.add_entity_alias(entity_id, &mention.mention_text)?;
                    }

                    // Update entity context with new information
                    self.update_entity_context_with_mention(
                        db,
                        entity_id,
                        &mention.mention_text,
                        &mention.context_snippet,
                    )
                    .await?;

                    return Ok(MentionResult::LinkedToExisting {
                        entity_id,
                        confidence: result.confidence,
                    });
                }
                Ok(_) => {
                    // Not a match or below threshold, continue to next candidate
                    continue;
                }
                Err(e) => {
                    // LLM error - log and continue
                    eprintln!("Entity linking error: {}", e);
                    continue;
                }
            }
        }

        // No match found among candidates - create a new entity
        let entity = registry.create_entity(
            &mention.entity_type,
            &mention.mention_text,
            Some(&mention.context_snippet),
        )?;

        registry.create_mention(
            recording_id,
            &mention.mention_text,
            Some(&mention.context_snippet),
            entity.id,
            1.0,
        )?;

        Ok(MentionResult::CreatedNew {
            entity_id: entity.id.unwrap(),
        })
    }

    /// Update an entity's context with information from a new mention.
    async fn update_entity_context_with_mention(
        &self,
        db: &mut Database,
        entity_id: i64,
        mention_text: &str,
        context_snippet: &str,
    ) -> Result<()> {
        let entity = db.get_entity(entity_id)?;
        if let Some(entity) = entity {
            let existing_context = entity.context.as_deref().unwrap_or("");

            // Use LLM to update context
            let update_result = self
                .enrichment_service
                .update_entity_context(
                    &entity.canonical_name,
                    &entity.entity_type,
                    existing_context,
                    &[(mention_text, context_snippet)],
                )
                .await;

            if let Ok(result) = update_result {
                let mut registry = EntityRegistry::new(db);
                registry.update_entity_context(entity_id, &result.updated_context)?;
            }
        }

        Ok(())
    }
}

/// Result of processing a single mention.
enum MentionResult {
    /// Linked to an existing entity.
    LinkedToExisting { entity_id: i64, confidence: f32 },
    /// Created a new entity.
    CreatedNew { entity_id: i64 },
}

/// Report of entity linking results.
#[derive(Debug, Default)]
pub struct LinkingReport {
    /// Number of mentions linked to existing entities.
    pub linked_existing: usize,
    /// Number of new entities created.
    pub created_new: usize,
    /// Number of mentions left unlinked.
    pub unlinked: usize,
    /// Detailed results for each mention.
    pub details: Vec<LinkDetail>,
}

/// Detail of a single linking operation.
#[derive(Debug)]
pub struct LinkDetail {
    pub mention_text: String,
    pub action: LinkAction,
    pub entity_id: Option<i64>,
    pub confidence: f32,
}

/// Action taken for a mention.
#[derive(Debug)]
pub enum LinkAction {
    LinkedExisting,
    CreatedNew,
}
