//! Entity linking using LLM-resolved entities.
//!
//! The LLM extraction prompt already resolves entities against the world context.
//! This module links resolved entities to their DB records (checking both canonical
//! names and aliases) and creates new entities for genuinely unresolved ones.
//! Extraction results are deduplicated before processing to prevent duplicate
//! entity creation within a single extraction.

use std::collections::HashMap;

use anyhow::Result;

use crate::database::Database;
use crate::enrichment::ExtractionResult;

use super::registry::EntityRegistry;

/// Entity linker that processes LLM-resolved extraction results.
pub struct EntityLinker;

/// A deduplicated entity: same canonical name, merged contexts.
struct DeduplicatedEntity {
    canonical_name: String,
    transcript_names: Vec<String>,
    context: String,
    entity_type: String,
}

impl EntityLinker {
    /// Create a new entity linker.
    pub fn new() -> Self {
        Self
    }

    /// Deduplicate extracted entities by canonical name.
    ///
    /// If the LLM returns "Steve" twice with different contexts,
    /// merge them into a single entry with combined context.
    fn deduplicate_entities(extraction: &ExtractionResult) -> Vec<DeduplicatedEntity> {
        let mut seen: HashMap<String, DeduplicatedEntity> = HashMap::new();

        for (entity, entity_type) in extraction.all_entities() {
            let canonical = entity
                .resolved_to
                .as_deref()
                .unwrap_or(&entity.name)
                .to_lowercase();

            if let Some(existing) = seen.get_mut(&canonical) {
                // Merge context if different
                if !existing.context.contains(&entity.context) {
                    existing.context = format!("{}; {}", existing.context, entity.context);
                }
                // Track all transcript names for alias creation
                if !existing
                    .transcript_names
                    .iter()
                    .any(|n| n.to_lowercase() == entity.name.to_lowercase())
                {
                    existing.transcript_names.push(entity.name.clone());
                }
            } else {
                // Use the original-case canonical name
                let canonical_name = entity
                    .resolved_to
                    .as_deref()
                    .unwrap_or(&entity.name)
                    .to_string();

                seen.insert(
                    canonical.clone(),
                    DeduplicatedEntity {
                        canonical_name,
                        transcript_names: vec![entity.name.clone()],
                        context: entity.context.clone(),
                        entity_type: entity_type.to_string(),
                    },
                );
            }
        }

        seen.into_values().collect()
    }

    /// Process extraction results and link/create entities.
    ///
    /// Deduplicates extraction results first, then for each entity:
    /// - Searches by canonical name AND aliases (catches merged entities)
    /// - If found, links to existing entity
    /// - If not found, creates a new entity
    pub fn process_extraction(
        &self,
        db: &mut Database,
        recording_id: i64,
        extraction: &ExtractionResult,
    ) -> Result<LinkingReport> {
        let mut report = LinkingReport::default();

        let deduped = Self::deduplicate_entities(extraction);

        for entity in &deduped {
            let mut registry = EntityRegistry::new(db);

            // Try to find existing entity by canonical name OR alias
            let existing = registry.get_entity_by_name_or_alias(&entity.canonical_name)?;

            if let Some(existing_entity) = existing {
                let entity_id = existing_entity.id.unwrap();

                // Create mention records for each transcript name
                for name in &entity.transcript_names {
                    registry.create_mention(
                        recording_id,
                        name,
                        Some(&entity.context),
                        Some(entity_id),
                        1.0,
                    )?;
                }

                // Increment mention count
                registry.record_mention(entity_id)?;

                // Add transcript names as aliases if different from canonical
                for name in &entity.transcript_names {
                    if name.to_lowercase() != existing_entity.canonical_name.to_lowercase()
                        && !existing_entity
                            .aliases_list()
                            .iter()
                            .any(|a| a.to_lowercase() == name.to_lowercase())
                    {
                        registry.add_entity_alias(entity_id, name)?;
                    }
                }

                report.linked_existing += 1;
                report.details.push(LinkDetail {
                    mention_text: entity.transcript_names.join(", "),
                    action: LinkAction::LinkedExisting,
                    entity_id: Some(entity_id),
                    canonical_name: entity.canonical_name.clone(),
                });
            } else {
                // Create new entity
                let new_entity = registry.create_entity(
                    &entity.entity_type,
                    &entity.canonical_name,
                    Some(&entity.context),
                )?;

                let new_id = new_entity.id.unwrap();

                // Add transcript names as aliases if different
                for name in &entity.transcript_names {
                    if name.to_lowercase() != entity.canonical_name.to_lowercase() {
                        registry.add_entity_alias(new_id, name)?;
                    }
                }

                // Create mention records
                for name in &entity.transcript_names {
                    registry.create_mention(
                        recording_id,
                        name,
                        Some(&entity.context),
                        new_entity.id,
                        1.0,
                    )?;
                }

                report.created_new += 1;
                report.details.push(LinkDetail {
                    mention_text: entity.transcript_names.join(", "),
                    action: LinkAction::CreatedNew,
                    entity_id: new_entity.id,
                    canonical_name: entity.canonical_name.clone(),
                });
            }
        }

        Ok(report)
    }
}

/// Report of entity linking results.
#[derive(Debug, Default)]
pub struct LinkingReport {
    /// Number of mentions linked to existing entities.
    pub linked_existing: usize,
    /// Number of new entities created.
    pub created_new: usize,
    /// Detailed results for each mention.
    pub details: Vec<LinkDetail>,
}

/// Detail of a single linking operation.
#[derive(Debug)]
pub struct LinkDetail {
    pub mention_text: String,
    pub action: LinkAction,
    pub entity_id: Option<i64>,
    pub canonical_name: String,
}

/// Action taken for a mention.
#[derive(Debug)]
pub enum LinkAction {
    LinkedExisting,
    CreatedNew,
}
