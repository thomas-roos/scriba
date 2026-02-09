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
4. "people": An array of INDIVIDUAL HUMAN PERSONS explicitly NAMED in the transcript (NOT companies/organizations), each with:
   - "name": The person's actual name as mentioned in the transcript
   - "context": Brief context about this person from the transcript (role, relationship, etc.)
   - Do NOT include unnamed speakers, narrators, or placeholders like "Author", "Speaker", "Autore", etc.
5. "organizations": An array of companies, organizations, institutions mentioned (e.g., OpenAI, Google, Anthropic, universities, governments), each with:
   - "name": The organization name
   - "context": Brief context about this organization
6. "key_points": An array of key points or insights (3-5 items)
7. "action_items": An array of action items or tasks mentioned (if any)

IMPORTANT: If the same person or organization appears multiple times in the transcript (even with slight spelling variations), include them ONLY ONCE with combined context. Do NOT return duplicate entries.

Return ONLY valid JSON, no additional text. If a field has no relevant content, use an empty array [] or empty string "".

Example JSON structure (DO NOT copy these example values - extract from the actual transcript above):
{{
  "title": "<descriptive title from transcript>",
  "summary": "<summary of actual content>",
  "topics": ["<topic1>", "<topic2>"],
  "people": [
    {{"name": "<person name>", "context": "<their role/context>"}}
  ],
  "organizations": [
    {{"name": "<org name>", "context": "<org context>"}}
  ],
  "key_points": ["<key point from transcript>"],
  "action_items": ["<action item if any>"]
}}

Now analyze the transcript above and return JSON with the ACTUAL content:"#,
        transcript = transcript
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
        .map(|(text, ctx)| format!("- \"{}\" mentioned in: \"{}\"", text, ctx))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"You are building a knowledge base about people and organizations. Your job is to accumulate factual information about entities over time.

ENTITY TO UPDATE:
- Name: "{entity_name}"
- Type: {entity_type}
- Current known information: "{existing_context}"

NEW TRANSCRIPT EXCERPT(S) mentioning this entity:
{mentions_text}

TASK: Extract NEW factual information about "{entity_name}" from the transcript excerpt(s) and merge it with existing knowledge.

Look for facts like:
- For people: job title, role, company, expertise, relationships, location, responsibilities
- For organizations: industry, products, location, size, what they do

RULES:
1. ONLY include facts that are explicitly stated or clearly implied in the text
2. DO NOT invent or assume information not in the text
3. PRESERVE all existing facts that are still relevant
4. ADD new facts discovered in the transcript
5. If new info contradicts old info, prefer the new info
6. Keep it factual and concise (max 150 words)

Return a JSON object:
{{
  "updated_context": "<merged context with old + new facts>",
  "new_facts": ["<specific new fact 1>", "<specific new fact 2>"]
}}

Return ONLY valid JSON:"#,
        entity_name = entity_name,
        entity_type = entity_type,
        existing_context = existing_context,
        mentions_text = mentions_text
    )
}

/// Build a prompt for evolving the world description based on a new transcript.
///
/// This prompt asks the LLM to return ONLY the new information to add to the
/// owner's world, as a conservative JSON delta. The merge happens in code.
pub fn build_world_evolution_prompt(
    current_world: &str,
    _transcript: &str,
    extraction_summary: &str,
) -> String {
    format!(
        r#"You maintain a structured knowledge base about a person. Given their current profile and information extracted from a new recording, identify ONLY genuinely new facts to add.

CURRENT WORLD (what we already know):
{current_world}

NEW INFORMATION FROM LATEST RECORDING:
{extraction_summary}

IMPORTANT: Transcripts often contain misspellings or mis-transcriptions of names. Before adding a new person or organization, check carefully if it could be a misspelling of someone/something already in the current world. For example, "Exane" is likely "Exein", "Jon" could be "John". When in doubt, assume it refers to the known entity and do NOT create a duplicate.

YOUR TASK: Return a JSON object containing ONLY new information learned from this recording. Be very conservative:
- Do NOT repeat or rephrase information already in the current world
- Do NOT include entities/organizations already known unless there is genuinely new information about them
- Do NOT create new entries for misspellings of known entities — they are the SAME entity
- Do NOT include transient details (specific numbers, dates, meeting agendas)
- Do NOT invent fields or categories not in the schema below
- ONLY include lasting facts that are NEW: new people, new organizations, new interests, new projects, new beliefs
- If the recording adds nothing meaningful to the world, return an empty object: {{}}

The JSON schema (include only fields with new content):
{{
  "owner": {{
    "name": "only if name needs correction",
    "aliases": ["only NEW aliases not already known"],
    "role": "only if role changed",
    "organization": "only if changed",
    "location": "only if changed"
  }},
  "organizations": [
    {{"name": "org name", "description": "what they do", "aliases": ["misspellings"]}}
  ],
  "people": [
    {{"name": "person name", "relationship": "who they are to the owner", "aliases": ["alternate spellings"]}}
  ],
  "interests": ["only genuinely new recurring interests"],
  "projects": [
    {{"name": "project name", "description": "brief description"}}
  ],
  "beliefs": ["only strong opinions explicitly expressed by the owner"]
}}

RULES:
- Omit any field/section that has no new information
- For people: only include if they are clearly important (not just briefly mentioned)
- For interests: only include if this is a recurring theme, not a one-off topic
- For projects: only include active projects the owner is working on
- For beliefs: only include strong convictions the owner clearly expressed
- Keep descriptions SHORT (under 15 words each)

Return ONLY valid JSON, nothing else:"#,
        current_world = current_world,
        extraction_summary = extraction_summary
    )
}

/// Build a prompt to extract a structured world profile from a seed description.
///
/// This is used when initializing the world to build the structured JSON
/// from the owner's free-form description.
pub fn build_world_seed_extraction_prompt(world_content: &str) -> String {
    format!(
        r#"You are analyzing a personal description to build a structured profile.

DESCRIPTION:
{world_content}

Extract a structured profile. The first person mentioned is the OWNER of this system.

Return a JSON object matching this EXACT schema:
{{
  "owner": {{
    "name": "full name of the owner",
    "aliases": ["common nicknames or short names"],
    "role": "their job title or role",
    "organization": "their primary organization",
    "location": "where they are based"
  }},
  "organizations": [
    {{"name": "org name", "description": "what they do", "aliases": ["known misspellings"]}}
  ],
  "people": [
    {{"name": "person name", "relationship": "who they are to the owner", "aliases": ["alternate spellings"]}}
  ],
  "interests": [],
  "projects": [],
  "beliefs": []
}}

RULES:
- ONLY extract information explicitly stated in the description
- Do NOT invent or assume information not present
- Leave arrays empty if nothing relevant is mentioned
- Keep descriptions SHORT (under 15 words)

Return ONLY valid JSON, nothing else."#,
        world_content = world_content
    )
}

/// Build a prompt to extract entities from a world description.
///
/// This is used when initializing the world to automatically create
/// entities mentioned in the owner's description.
pub fn build_world_entity_extraction_prompt(world_content: &str) -> String {
    format!(
        r#"You are analyzing a personal world description to extract entities (people and organizations).

WORLD DESCRIPTION:
{world_content}

Extract all people and organizations mentioned. For each entity:
- Identify the canonical name (how they should be referred to)
- Note any aliases or alternative names/spellings mentioned
- Extract context about who they are or what they do

The FIRST person mentioned is typically the OWNER of this system - mark them with "is_owner": true.

Return a JSON object with:
{{
  "people": [
    {{
      "name": "canonical name",
      "aliases": ["alias1", "full name if different"],
      "context": "brief description of who they are",
      "is_owner": true/false
    }}
  ],
  "organizations": [
    {{
      "name": "canonical name",
      "aliases": ["common misspelling", "abbreviation"],
      "context": "brief description of what they do"
    }}
  ]
}}

Return ONLY valid JSON, no additional text."#,
        world_content = world_content
    )
}

/// Build extraction prompt with world context and inline entity resolution.
///
/// The world context contains everything the LLM needs: owner profile,
/// known people, organizations, etc. The LLM resolves entity mentions
/// against the world and indicates matches via `resolved_to`.
pub fn build_full_context_extraction_prompt(
    transcript: &str,
    world_context: Option<&str>,
) -> String {
    if world_context.is_none() || world_context.map(|w| w.trim().is_empty()).unwrap_or(true) {
        return build_extraction_prompt(transcript);
    }

    let world = world_context.unwrap();

    format!(
        r#"You are a knowledge extraction assistant. You have access to the owner's world context, which describes who they are and the people, organizations, and projects they know about.

OWNER'S WORLD:
{world}

Use this world context to understand the transcript, resolve ambiguous or misspelled names, and identify known entities. The owner is involved in every recording as a speaker or listener.

TRANSCRIPT:
---
{transcript}
---

Extract the following information and return it as a JSON object:

1. "title": A concise, descriptive title for this recording (5-10 words max)
2. "summary": A brief summary of the main content (2-3 sentences)
3. "topics": An array of main topics discussed (3-7 items)
4. "people": An array of INDIVIDUAL HUMAN PERSONS explicitly NAMED in the transcript (NOT companies/organizations), each with:
   - "name": The name as it appears in the transcript
   - "context": Brief context about this person from the transcript
   - "resolved_to": If this person matches someone in the OWNER'S WORLD above (even if misspelled or abbreviated), set this to their EXACT name from the world. Otherwise set to null.
   - Do NOT include unnamed speakers, narrators, or placeholders like "Author", "Speaker", "Autore", etc.
5. "organizations": An array of companies, organizations, institutions mentioned, each with:
   - "name": The name as it appears in the transcript
   - "context": Brief context about this organization
   - "resolved_to": If this organization matches one in the OWNER'S WORLD above (even if misspelled — e.g. "Exane" for "Exein"), set this to the EXACT name from the world. Otherwise set to null.
6. "key_points": An array of key points or insights (3-5 items)
7. "action_items": An array of action items or tasks mentioned (if any)

ENTITY RESOLUTION RULES:
- Transcripts often contain misspellings due to speech-to-text errors. "Exane", "Xane", "Hexane" could all be "Exein". Use the world context to resolve these.
- If a name closely resembles a known person or organization from the world, resolve it. Check BOTH names AND aliases in the world context.
- If a name is genuinely new (not in the world at all), set "resolved_to" to null.
- When in doubt, resolve to the known entity rather than creating a new one.
- DEDUPLICATION: If the same person or organization appears multiple times in the transcript (even with slight spelling variations), include them ONLY ONCE with combined context. Do NOT return duplicate entries.

Return ONLY valid JSON, no additional text. If a field has no relevant content, use an empty array [] or empty string "".

{{
  "title": "<title>",
  "summary": "<summary>",
  "topics": ["<topic>"],
  "people": [
    {{"name": "<transcript name>", "context": "<context>", "resolved_to": "<world name or null>"}}
  ],
  "organizations": [
    {{"name": "<transcript name>", "context": "<context>", "resolved_to": "<world name or null>"}}
  ],
  "key_points": ["<point>"],
  "action_items": ["<item>"]
}}

Now analyze the transcript above and return JSON:"#,
        world = world,
        transcript = transcript
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
    fn test_world_evolution_prompt() {
        let current = r#"{"owner":{"name":"Giovanni"}}"#;
        let transcript = "Today we met with Luca to discuss the roadmap.";
        let extraction = "People: Luca (team member)";

        let prompt = build_world_evolution_prompt(current, transcript, extraction);

        assert!(prompt.contains("Giovanni"));
        assert!(prompt.contains("Luca"));
        assert!(prompt.contains("ONLY new information"));
        assert!(prompt.contains("conservative"));
    }

    #[test]
    fn test_full_context_extraction_prompt_with_world() {
        let world = r#"{"owner":{"name":"Giovanni"},"organizations":[{"name":"Exein"}]}"#;

        let prompt = build_full_context_extraction_prompt(
            "Meeting transcript here",
            Some(world),
        );

        assert!(prompt.contains("Giovanni"));
        assert!(prompt.contains("Exein"));
        assert!(prompt.contains("Meeting transcript here"));
        assert!(prompt.contains("resolved_to"));
        assert!(prompt.contains("OWNER'S WORLD"));
    }

    #[test]
    fn test_full_context_extraction_prompt_no_context() {
        let prompt = build_full_context_extraction_prompt(
            "Simple transcript",
            None,
        );

        assert!(prompt.contains("Simple transcript"));
        assert!(prompt.contains("title"));
        // Should NOT contain resolved_to (basic prompt)
        assert!(!prompt.contains("resolved_to"));
    }
}
