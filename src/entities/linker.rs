//! Entity linking using LLM-resolved entities.
//!
//! The LLM extraction prompt already resolves entities against the world context.
//! This module simply links resolved entities to their DB records and creates
//! new entities for unresolved ones.

use anyhow::Result;

use crate::database::Database;
use crate::enrichment::ExtractionResult;

use super::registry::EntityRegistry;

/// Entity linker that processes LLM-resolved extraction results.
pub struct EntityLinker;

impl EntityLinker {
    /// Create a new entity linker.
    pub fn new() -> Self {
        Self
    }

    /// Process extraction results and link/create entities.
    ///
    /// For each extracted entity:
    /// - If `resolved_to` is set, find the known entity by canonical name and link
    /// - If `resolved_to` is null, create a new entity
    pub fn process_extraction(
        &self,
        db: &mut Database,
        recording_id: i64,
        extraction: &ExtractionResult,
    ) -> Result<LinkingReport> {
        let mut report = LinkingReport::default();

        for (entity, entity_type) in extraction.all_entities() {
            let mut registry = EntityRegistry::new(db);

            // Determine the canonical name: use resolved_to if available, otherwise the transcript name
            let canonical_name = entity.resolved_to.as_deref().unwrap_or(&entity.name);

            // Try to find existing entity by canonical name
            let existing = registry.get_entity_by_name(canonical_name)?;

            if let Some(existing_entity) = existing {
                let entity_id = existing_entity.id.unwrap();

                // Link mention to existing entity
                registry.create_mention(
                    recording_id,
                    &entity.name,
                    Some(&entity.context),
                    Some(entity_id),
                    1.0,
                )?;

                // Increment mention count
                registry.record_mention(entity_id)?;

                // Add transcript name as alias if different from canonical
                if entity.name.to_lowercase() != canonical_name.to_lowercase() {
                    registry.add_entity_alias(entity_id, &entity.name)?;
                }

                report.linked_existing += 1;
                report.details.push(LinkDetail {
                    mention_text: entity.name.clone(),
                    action: LinkAction::LinkedExisting,
                    entity_id: Some(entity_id),
                    canonical_name: canonical_name.to_string(),
                });
            } else {
                // Create new entity
                let new_entity = registry.create_entity(
                    entity_type,
                    canonical_name,
                    Some(&entity.context),
                )?;

                let new_id = new_entity.id.unwrap();

                // Add transcript name as alias if different
                if entity.name.to_lowercase() != canonical_name.to_lowercase() {
                    registry.add_entity_alias(new_id, &entity.name)?;
                }

                // Create mention record
                registry.create_mention(
                    recording_id,
                    &entity.name,
                    Some(&entity.context),
                    new_entity.id,
                    1.0,
                )?;

                report.created_new += 1;
                report.details.push(LinkDetail {
                    mention_text: entity.name.clone(),
                    action: LinkAction::CreatedNew,
                    entity_id: new_entity.id,
                    canonical_name: canonical_name.to_string(),
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
