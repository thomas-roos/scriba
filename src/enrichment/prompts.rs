//! Prompts for knowledge extraction from transcripts.

/// Build the extraction prompt for a transcript.
pub fn build_extraction_prompt(transcript: &str) -> String {
    format!(
        r#"You are a knowledge extraction assistant. Analyze the following transcript and extract structured information.

TRANSCRIPT:
---
{transcript}
---

Extract the following information and return it as a JSON object:

1. "title": A concise, descriptive title for this recording (5-10 words max)
2. "summary": A brief summary of the main content (2-3 sentences)
3. "topics": An array of main topics discussed (3-7 items)
4. "people": An array of people mentioned, each with:
   - "name": The name as mentioned in the transcript
   - "context": Brief context about this person from the transcript (role, relationship, etc.)
5. "organizations": An array of organizations mentioned, each with:
   - "name": The organization name
   - "context": Brief context about this organization
6. "key_points": An array of key points or insights (3-5 items)
7. "action_items": An array of action items or tasks mentioned (if any)

Return ONLY valid JSON, no additional text. If a field has no relevant content, use an empty array [] or empty string "".

Example output format:
{{
  "title": "Q2 Budget Planning Meeting",
  "summary": "Discussion of Q2 budget allocation with focus on marketing spend and new hires.",
  "topics": ["budget", "marketing", "hiring", "Q2 planning"],
  "people": [
    {{"name": "John Smith", "context": "CFO, presenting budget numbers"}},
    {{"name": "Sarah", "context": "Marketing lead, requesting increased budget"}}
  ],
  "organizations": [
    {{"name": "Acme Inc", "context": "The company being discussed"}}
  ],
  "key_points": [
    "Marketing budget increased by 20%",
    "Two new engineering positions approved"
  ],
  "action_items": [
    "John to finalize budget spreadsheet by Friday",
    "Sarah to submit revised marketing plan"
  ]
}}

Now analyze the transcript and return the JSON:"#,
        transcript = transcript
    )
}

/// Build a prompt for entity linking - determining if a mention matches a known entity.
pub fn build_entity_linking_prompt(
    mention_text: &str,
    mention_context: &str,
    entity_name: &str,
    entity_type: &str,
    entity_context: &str,
) -> String {
    format!(
        r#"You are an entity resolution assistant. Determine if a mention in a transcript refers to a known entity.

MENTION FROM TRANSCRIPT:
- Text: "{mention_text}"
- Context: "{mention_context}"

KNOWN ENTITY:
- Name: "{entity_name}"
- Type: {entity_type}
- Known context: "{entity_context}"

Does this mention refer to this known entity? Consider:
- Name similarity (nicknames, abbreviations, partial names)
- Context consistency (role, relationships, topics)
- Logical coherence

Return a JSON object with:
- "is_match": true or false
- "confidence": a number between 0.0 and 1.0
- "reasoning": brief explanation of your decision

Return ONLY valid JSON:
{{
  "is_match": true,
  "confidence": 0.85,
  "reasoning": "The mention 'John' likely refers to 'John Smith' based on matching role as CFO and discussion of budget topics."
}}"#,
        mention_text = mention_text,
        mention_context = mention_context,
        entity_name = entity_name,
        entity_type = entity_type,
        entity_context = entity_context
    )
}

/// Build a prompt for updating entity context with new information.
pub fn build_context_update_prompt(
    entity_name: &str,
    entity_type: &str,
    existing_context: &str,
    new_mentions: &[(&str, &str)], // (mention_text, context_snippet)
) -> String {
    let mentions_text: String = new_mentions
        .iter()
        .map(|(text, ctx)| format!("- \"{}\" in context: \"{}\"", text, ctx))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"You are an entity context manager. Update the context for an entity based on new mentions.

ENTITY:
- Name: "{entity_name}"
- Type: {entity_type}
- Existing context: "{existing_context}"

NEW MENTIONS:
{mentions_text}

Create an updated context that:
1. Preserves important existing information
2. Incorporates new relevant details
3. Stays concise (max 200 words)
4. Focuses on factual, useful information

Return a JSON object with:
- "updated_context": the new context string
- "new_facts": array of new facts learned

Return ONLY valid JSON:
{{
  "updated_context": "CTO at Acme Inc. Discussed Q2 budget, marketing strategy. Known for product expertise. Met on 2025-01-15 about roadmap.",
  "new_facts": ["Attended Q2 budget meeting", "Interested in marketing strategy"]
}}"#,
        entity_name = entity_name,
        entity_type = entity_type,
        existing_context = existing_context,
        mentions_text = mentions_text
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extraction_prompt_contains_transcript() {
        let prompt = build_extraction_prompt("Hello, this is a test transcript.");
        assert!(prompt.contains("Hello, this is a test transcript."));
        assert!(prompt.contains("title"));
        assert!(prompt.contains("summary"));
        assert!(prompt.contains("topics"));
    }

    #[test]
    fn test_entity_linking_prompt() {
        let prompt = build_entity_linking_prompt(
            "John",
            "John mentioned the budget",
            "John Smith",
            "person",
            "CFO at Acme Inc",
        );
        assert!(prompt.contains("John"));
        assert!(prompt.contains("John Smith"));
        assert!(prompt.contains("CFO at Acme Inc"));
    }
}
