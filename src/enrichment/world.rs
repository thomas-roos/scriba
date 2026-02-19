//! World context management for Scriba.
//!
//! The "world" is Scriba's evolving understanding of its owner - a structured
//! knowledge base that grows with every conversation, capturing who they are,
//! who they know, what they care about, and what they believe.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::utils::BASE_PATH;

const WORLD_FILENAME: &str = "world.md";

/// Append only genuinely new sentences from `new_text` to `existing`.
/// Splits on periods, checks each sentence against existing text (case-insensitive),
/// and only appends sentences that aren't already present.
pub fn append_new_facts(existing: &mut String, new_text: &str) {
    if existing.is_empty() {
        *existing = new_text.to_string();
        return;
    }
    let existing_lower = existing.to_lowercase();
    for sentence in new_text.split('.').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if !existing_lower.contains(&sentence.to_lowercase()) {
            *existing = format!("{}. {}", existing.trim_end_matches('.'), sentence);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Structured World Data
// ─────────────────────────────────────────────────────────────────────────────

/// The owner of Scriba.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OwnerInfo {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub organization: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub location: String,
}

/// An organization relevant to the owner's world.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OrgInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

/// A person in the owner's world.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersonInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub relationship: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

/// A project the owner is working on.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// Structured representation of Scriba's understanding of the owner's world.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorldData {
    #[serde(default)]
    pub owner: OwnerInfo,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub organizations: Vec<OrgInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<PersonInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interests: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<ProjectInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beliefs: Vec<String>,
}

impl WorldData {
    /// Parse from JSON string.
    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).context("Failed to parse world data as JSON")
    }

    /// Serialize to pretty-printed JSON.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("Failed to serialize world data")
    }


    /// Produce a compact summary of all known entity names and aliases.
    ///
    /// Used in the search resolution prompt so the LLM can cross-reference
    /// search results against the world without needing the full JSON.
    pub fn entities_summary(&self) -> String {
        let mut lines = Vec::new();

        // Owner
        if !self.owner.name.is_empty() {
            let aliases = if self.owner.aliases.is_empty() {
                String::new()
            } else {
                format!(" (aliases: {})", self.owner.aliases.join(", "))
            };
            lines.push(format!("Owner: {}{}", self.owner.name, aliases));
        }

        // People
        let people_parts: Vec<String> = self
            .people
            .iter()
            .map(|p| {
                if p.aliases.is_empty() {
                    p.name.clone()
                } else {
                    format!("{} (aliases: {})", p.name, p.aliases.join(", "))
                }
            })
            .collect();
        if !people_parts.is_empty() {
            lines.push(format!("People: {}", people_parts.join(", ")));
        }

        // Organizations
        let org_parts: Vec<String> = self
            .organizations
            .iter()
            .map(|o| {
                if o.aliases.is_empty() {
                    o.name.clone()
                } else {
                    format!("{} (aliases: {})", o.name, o.aliases.join(", "))
                }
            })
            .collect();
        if !org_parts.is_empty() {
            lines.push(format!("Organizations: {}", org_parts.join(", ")));
        }

        lines.join("\n")
    }

    /// Merge changes into this world data.
    ///
    /// This is conservative: it only adds new items and updates existing ones.
    /// It never removes anything.
    pub fn merge(&mut self, changes: &WorldData) {
        // Owner: only overwrite non-empty fields
        if !changes.owner.name.is_empty() {
            self.owner.name = changes.owner.name.clone();
        }
        if !changes.owner.aliases.is_empty() {
            for alias in &changes.owner.aliases {
                if !self.owner.aliases.iter().any(|a| a.to_lowercase() == alias.to_lowercase()) {
                    self.owner.aliases.push(alias.clone());
                }
            }
        }
        if !changes.owner.role.is_empty() {
            self.owner.role = changes.owner.role.clone();
        }
        if !changes.owner.organization.is_empty() {
            self.owner.organization = changes.owner.organization.clone();
        }
        if !changes.owner.location.is_empty() {
            self.owner.location = changes.owner.location.clone();
        }

        // Organizations: add new, update description only if richer.
        // Also skip new orgs whose name is already an alias of an existing org.
        for new_org in &changes.organizations {
            let new_name_lower = new_org.name.to_lowercase();

            // Find existing org by name OR by alias
            if let Some(existing) = self.organizations.iter_mut().find(|o| {
                o.name.to_lowercase() == new_name_lower
                    || o.aliases.iter().any(|a| a.to_lowercase() == new_name_lower)
            }) {
                // Append new description info if genuinely new (and the name matches exactly)
                if existing.name.to_lowercase() == new_name_lower
                    && !new_org.description.is_empty()
                {
                    append_new_facts(&mut existing.description, &new_org.description);
                }
                for alias in &new_org.aliases {
                    if !existing.aliases.iter().any(|a| a.to_lowercase() == alias.to_lowercase()) {
                        existing.aliases.push(alias.clone());
                    }
                }
            } else {
                self.organizations.push(new_org.clone());
            }
        }

        // People: add new, update relationship only if richer.
        // Also skip new people whose name is already an alias of an existing person.
        for new_person in &changes.people {
            let new_name_lower = new_person.name.to_lowercase();

            // Find existing person by name OR by alias
            if let Some(existing) = self.people.iter_mut().find(|p| {
                p.name.to_lowercase() == new_name_lower
                    || p.aliases.iter().any(|a| a.to_lowercase() == new_name_lower)
            }) {
                // Append new relationship info if genuinely new (not alias match)
                if existing.name.to_lowercase() == new_name_lower
                    && !new_person.relationship.is_empty()
                {
                    append_new_facts(&mut existing.relationship, &new_person.relationship);
                }
                for alias in &new_person.aliases {
                    if !existing.aliases.iter().any(|a| a.to_lowercase() == alias.to_lowercase()) {
                        existing.aliases.push(alias.clone());
                    }
                }
            } else {
                self.people.push(new_person.clone());
            }
        }

        // Interests: add new, deduplicate (case-insensitive)
        for interest in &changes.interests {
            if !self.interests.iter().any(|i| i.to_lowercase() == interest.to_lowercase()) {
                self.interests.push(interest.clone());
            }
        }

        // Projects: add new, update description only if richer
        for new_project in &changes.projects {
            if let Some(existing) = self.projects.iter_mut().find(|p| {
                p.name.to_lowercase() == new_project.name.to_lowercase()
            }) {
                if !new_project.description.is_empty()
                    && new_project.description.len() > existing.description.len()
                {
                    existing.description = new_project.description.clone();
                }
            } else {
                self.projects.push(new_project.clone());
            }
        }

        // Beliefs: add new, deduplicate (case-insensitive)
        for belief in &changes.beliefs {
            if !self.beliefs.iter().any(|b| b.to_lowercase() == belief.to_lowercase()) {
                self.beliefs.push(belief.clone());
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// WorldContext (file management)
// ─────────────────────────────────────────────────────────────────────────────

/// Manages the world file on disk.
#[derive(Debug, Clone)]
pub struct WorldContext {
    /// The raw content of the world file.
    pub content: String,
    /// Path to the world file.
    pub path: PathBuf,
}

impl Default for WorldContext {
    fn default() -> Self {
        Self {
            content: String::new(),
            path: BASE_PATH.join(WORLD_FILENAME),
        }
    }
}

impl WorldContext {
    /// Load world description from file.
    ///
    /// Returns an empty WorldContext if the file doesn't exist.
    pub fn load() -> Result<Self> {
        let path = BASE_PATH.join(WORLD_FILENAME);

        if !path.exists() {
            return Ok(Self {
                content: String::new(),
                path,
            });
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read world file: {}", path.display()))?;

        Ok(Self { content, path })
    }

    /// Save the world description to file.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        std::fs::write(&self.path, &self.content)
            .with_context(|| format!("Failed to write world file: {}", self.path.display()))?;

        Ok(())
    }

    /// Check if the world file exists.
    pub fn exists() -> bool {
        BASE_PATH.join(WORLD_FILENAME).exists()
    }

    /// Get the path to the world file.
    pub fn file_path() -> PathBuf {
        BASE_PATH.join(WORLD_FILENAME)
    }

    /// Create initial world file with seed content.
    pub fn initialize(seed_content: &str) -> Result<Self> {
        let path = BASE_PATH.join(WORLD_FILENAME);

        if path.exists() {
            return Err(anyhow::anyhow!(
                "World file already exists at {}. Use 'scriba world edit' to modify it.",
                path.display()
            ));
        }

        let world = Self {
            content: seed_content.to_string(),
            path,
        };

        world.save()?;
        Ok(world)
    }

    /// Check if the world has meaningful content.
    pub fn has_content(&self) -> bool {
        !self.content.trim().is_empty()
    }

    /// Update the world content and save.
    pub fn update(&mut self, new_content: String) -> Result<()> {
        self.content = new_content;
        self.save()
    }

    /// Try to parse the content as structured WorldData.
    ///
    /// Returns None if the content is not valid JSON (e.g. legacy narrative format).
    pub fn parsed(&self) -> Option<WorldData> {
        WorldData::from_json(&self.content).ok()
    }

    /// Update from structured WorldData and save.
    pub fn update_data(&mut self, data: &WorldData) -> Result<()> {
        self.content = data.to_json()?;
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_world_context() {
        let world = WorldContext::default();
        assert!(world.content.is_empty());
        assert!(!world.has_content());
    }

    #[test]
    fn test_has_content() {
        let mut world = WorldContext::default();
        assert!(!world.has_content());

        world.content = "   ".to_string();
        assert!(!world.has_content());

        world.content = "Some content".to_string();
        assert!(world.has_content());
    }

    #[test]
    fn test_world_data_roundtrip() {
        let data = WorldData {
            owner: OwnerInfo {
                name: "Giovanni".to_string(),
                aliases: vec!["Gio".to_string()],
                role: "CTO".to_string(),
                organization: "Exein".to_string(),
                location: "Rome".to_string(),
            },
            organizations: vec![OrgInfo {
                name: "Exein".to_string(),
                description: "cybersecurity scaleup".to_string(),
                aliases: vec!["Exane".to_string()],
            }],
            people: vec![PersonInfo {
                name: "Gerardo".to_string(),
                relationship: "co-founder, CFO".to_string(),
                ..Default::default()
            }],
            interests: vec!["cybersecurity".to_string()],
            projects: vec![],
            beliefs: vec![],
        };

        let json = data.to_json().unwrap();
        let parsed = WorldData::from_json(&json).unwrap();
        assert_eq!(parsed.owner.name, "Giovanni");
        assert_eq!(parsed.organizations.len(), 1);
        assert_eq!(parsed.people.len(), 1);
    }

    #[test]
    fn test_merge_adds_new_people() {
        let mut world = WorldData {
            people: vec![PersonInfo {
                name: "Alice".to_string(),
                relationship: "colleague".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let changes = WorldData {
            people: vec![PersonInfo {
                name: "Bob".to_string(),
                relationship: "partner".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        world.merge(&changes);
        assert_eq!(world.people.len(), 2);
        assert_eq!(world.people[1].name, "Bob");
    }

    #[test]
    fn test_merge_accumulates_person_relationship() {
        let mut world = WorldData {
            people: vec![PersonInfo {
                name: "Alice".to_string(),
                relationship: "colleague and team lead at Exein".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        // Duplicate info (substring) should not be appended
        let changes = WorldData {
            people: vec![PersonInfo {
                name: "Alice".to_string(),
                relationship: "team lead".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        world.merge(&changes);
        assert_eq!(world.people.len(), 1);
        assert_eq!(world.people[0].relationship, "colleague and team lead at Exein");

        // Genuinely new info should be appended
        let changes2 = WorldData {
            people: vec![PersonInfo {
                name: "Alice".to_string(),
                relationship: "expert in embedded systems".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        world.merge(&changes2);
        assert_eq!(world.people[0].relationship, "colleague and team lead at Exein. expert in embedded systems");
    }

    #[test]
    fn test_merge_fills_empty_person_relationship() {
        let mut world = WorldData {
            people: vec![PersonInfo {
                name: "Alice".to_string(),
                relationship: String::new(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let changes = WorldData {
            people: vec![PersonInfo {
                name: "Alice".to_string(),
                relationship: "team lead".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        world.merge(&changes);
        assert_eq!(world.people.len(), 1);
        assert_eq!(world.people[0].relationship, "team lead");
    }

    #[test]
    fn test_merge_deduplicates_interests() {
        let mut world = WorldData {
            interests: vec!["cybersecurity".to_string()],
            ..Default::default()
        };

        let changes = WorldData {
            interests: vec!["Cybersecurity".to_string(), "AI".to_string()],
            ..Default::default()
        };

        world.merge(&changes);
        assert_eq!(world.interests.len(), 2);
        assert_eq!(world.interests[0], "cybersecurity");
        assert_eq!(world.interests[1], "AI");
    }

    #[test]
    fn test_merge_skips_org_that_is_alias_of_existing() {
        let mut world = WorldData {
            organizations: vec![OrgInfo {
                name: "Exein".to_string(),
                description: "cybersecurity scaleup".to_string(),
                aliases: vec!["Exane".to_string()],
            }],
            ..Default::default()
        };

        // LLM tries to add "Exane" as a separate org — should be skipped
        let changes = WorldData {
            organizations: vec![OrgInfo {
                name: "Exane".to_string(),
                description: "cybersecurity company based in Rome".to_string(),
                aliases: vec![],
            }],
            ..Default::default()
        };

        world.merge(&changes);
        assert_eq!(world.organizations.len(), 1);
        assert_eq!(world.organizations[0].name, "Exein");
        // Description should NOT be updated since the match was by alias, not name
        assert_eq!(world.organizations[0].description, "cybersecurity scaleup");
    }

    #[test]
    fn test_merge_skips_person_that_is_alias_of_existing() {
        let mut world = WorldData {
            people: vec![PersonInfo {
                name: "Giulia".to_string(),
                relationship: "Giovanni's girlfriend".to_string(),
                aliases: vec!["Julia".to_string()],
            }],
            ..Default::default()
        };

        // LLM tries to add "Julia" as a separate person — should be skipped
        let changes = WorldData {
            people: vec![PersonInfo {
                name: "Julia".to_string(),
                relationship: "Giovanni's girlfriend, works at Saatchi".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        world.merge(&changes);
        assert_eq!(world.people.len(), 1);
        assert_eq!(world.people[0].name, "Giulia");
        // Relationship should NOT be updated since the match was by alias
        assert_eq!(world.people[0].relationship, "Giovanni's girlfriend");
    }

    #[test]
    fn test_merge_preserves_existing_on_empty_changes() {
        let mut world = WorldData {
            owner: OwnerInfo {
                name: "Giovanni".to_string(),
                role: "CTO".to_string(),
                ..Default::default()
            },
            interests: vec!["cybersecurity".to_string()],
            ..Default::default()
        };

        let changes = WorldData::default();

        world.merge(&changes);
        assert_eq!(world.owner.name, "Giovanni");
        assert_eq!(world.owner.role, "CTO");
        assert_eq!(world.interests.len(), 1);
    }

    #[test]
    fn test_parsed_returns_none_for_legacy() {
        let world = WorldContext {
            content: "Hello, I am Giovanni.".to_string(),
            path: PathBuf::from("/tmp/world.md"),
        };
        assert!(world.parsed().is_none());
    }

    #[test]
    fn test_parsed_returns_some_for_json() {
        let data = WorldData {
            owner: OwnerInfo {
                name: "Giovanni".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let json = data.to_json().unwrap();

        let world = WorldContext {
            content: json,
            path: PathBuf::from("/tmp/world.md"),
        };
        let parsed = world.parsed().unwrap();
        assert_eq!(parsed.owner.name, "Giovanni");
    }

    #[test]
    fn test_entities_summary() {
        let data = WorldData {
            owner: OwnerInfo {
                name: "Giovanni Alberto Falcione".to_string(),
                aliases: vec!["Gio".to_string()],
                ..Default::default()
            },
            people: vec![PersonInfo {
                name: "Gerardo Gagliardo".to_string(),
                relationship: "co-founder".to_string(),
                aliases: vec![],
            }],
            organizations: vec![OrgInfo {
                name: "Exein".to_string(),
                description: "cybersecurity".to_string(),
                aliases: vec!["Exane".to_string()],
            }],
            ..Default::default()
        };

        let summary = data.entities_summary();
        assert!(summary.contains("Owner: Giovanni Alberto Falcione (aliases: Gio)"));
        assert!(summary.contains("People: Gerardo Gagliardo"));
        assert!(summary.contains("Organizations: Exein (aliases: Exane)"));
    }

    #[test]
    fn test_entities_summary_empty() {
        let data = WorldData::default();
        let summary = data.entities_summary();
        assert!(summary.is_empty());
    }
}
