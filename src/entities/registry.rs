//! Entity registry for managing known entities.

use anyhow::Result;
use chrono::Utc;

use crate::database::{Database, Entity, EntityMentionRecord};

/// Registry for managing entities (people, organizations, etc.).
pub struct EntityRegistry<'a> {
    db: &'a mut Database,
}

impl<'a> EntityRegistry<'a> {
    /// Create a new entity registry.
    pub fn new(db: &'a mut Database) -> Self {
        Self { db }
    }

    /// Create a new entity.
    pub fn create_entity(
        &mut self,
        entity_type: &str,
        canonical_name: &str,
        context: Option<&str>,
    ) -> Result<Entity> {
        let now = Utc::now();
        let entity = Entity {
            id: None,
            entity_type: entity_type.to_string(),
            canonical_name: canonical_name.to_string(),
            aliases: None,
            context: context.map(|s| s.to_string()),
            metadata: None,
            mention_count: 1,
            first_seen_at: Some(now),
            last_seen_at: Some(now),
            created_at: now,
            updated_at: now,
        };

        let id = self.db.insert_entity(&entity)?;

        Ok(Entity {
            id: Some(id),
            ..entity
        })
    }

    /// Get an entity by ID.
    pub fn get_entity(&self, id: i64) -> Result<Option<Entity>> {
        self.db.get_entity(id)
    }

    /// Get an entity by canonical name.
    pub fn get_entity_by_name(&self, name: &str) -> Result<Option<Entity>> {
        self.db.get_entity_by_name(name)
    }

    /// List all entities, optionally filtered by type.
    pub fn list_entities(
        &self,
        entity_type: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Vec<Entity>> {
        self.db.list_entities(entity_type, limit)
    }

    /// Find entities that might match a mention text.
    /// Checks canonical name and aliases (case-insensitive).
    pub fn find_candidate_entities(&self, mention_text: &str) -> Result<Vec<Entity>> {
        let all_entities = self.db.list_entities(None, None)?;
        let mention_lower = mention_text.to_lowercase();

        let candidates: Vec<Entity> = all_entities
            .into_iter()
            .filter(|e| {
                // Check canonical name
                if e.canonical_name.to_lowercase().contains(&mention_lower)
                    || mention_lower.contains(&e.canonical_name.to_lowercase())
                {
                    return true;
                }

                // Check aliases
                let aliases = e.aliases_list();
                aliases
                    .iter()
                    .any(|a| a.to_lowercase() == mention_lower || mention_lower.contains(&a.to_lowercase()))
            })
            .collect();

        Ok(candidates)
    }

    /// Update an entity's context.
    pub fn update_entity_context(&mut self, id: i64, new_context: &str) -> Result<()> {
        if let Some(mut entity) = self.db.get_entity(id)? {
            entity.context = Some(new_context.to_string());
            self.db.update_entity(&entity)?;
        }
        Ok(())
    }

    /// Add an alias to an entity.
    pub fn add_entity_alias(&mut self, id: i64, alias: &str) -> Result<()> {
        if let Some(mut entity) = self.db.get_entity(id)? {
            entity.add_alias(alias);
            self.db.update_entity(&entity)?;
        }
        Ok(())
    }

    /// Increment mention count for an entity.
    pub fn record_mention(&mut self, entity_id: i64) -> Result<()> {
        self.db.increment_entity_mention(entity_id)
    }

    /// Create a mention record.
    pub fn create_mention(
        &mut self,
        recording_id: i64,
        mention_text: &str,
        context_snippet: Option<&str>,
        entity_id: Option<i64>,
        confidence: f64,
    ) -> Result<i64> {
        let now = Utc::now();
        let mention = EntityMentionRecord {
            id: None,
            entity_id,
            recording_id,
            mention_text: mention_text.to_string(),
            context_snippet: context_snippet.map(|s| s.to_string()),
            confidence,
            linked_at: entity_id.map(|_| now),
            created_at: now,
        };

        self.db.insert_entity_mention(&mention)
    }

    /// Get all mentions for a recording.
    pub fn get_mentions_for_recording(&self, recording_id: i64) -> Result<Vec<EntityMentionRecord>> {
        self.db.get_mentions_for_recording(recording_id)
    }

    /// Get all mentions for an entity.
    pub fn get_mentions_for_entity(&self, entity_id: i64) -> Result<Vec<EntityMentionRecord>> {
        self.db.get_mentions_for_entity(entity_id)
    }

    /// Get unlinked mentions.
    pub fn get_unlinked_mentions(&self, limit: Option<i64>) -> Result<Vec<EntityMentionRecord>> {
        self.db.get_unlinked_mentions(limit)
    }

    /// Link a mention to an entity.
    pub fn link_mention(&mut self, mention_id: i64, entity_id: i64, confidence: f64) -> Result<()> {
        self.db.link_mention_to_entity(mention_id, entity_id, confidence)?;
        self.db.increment_entity_mention(entity_id)?;
        Ok(())
    }

    /// Unlink a mention from its entity.
    pub fn unlink_mention(&mut self, mention_id: i64) -> Result<()> {
        self.db.unlink_mention(mention_id)
    }

    /// Get recordings that mention a specific entity.
    pub fn get_recordings_for_entity(&self, entity_id: i64) -> Result<Vec<crate::database::Recording>> {
        self.db.get_recordings_for_entity(entity_id)
    }

    /// Delete an entity.
    pub fn delete_entity(&mut self, id: i64) -> Result<()> {
        self.db.delete_entity(id)
    }
}
