//! Web search for verifying unresolved entities.
//!
//! When the LLM fails to resolve an entity against the world context (e.g. due
//! to a misspelling from speech-to-text), we search the web for the entity name
//! and feed the results back to the LLM in a second resolution pass.

use anyhow::Result;

use super::extractor::ExtractionResult;

/// A single web search result.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Web search results for a single unresolved entity.
#[derive(Debug, Clone)]
pub struct EntitySearchResults {
    pub entity_name: String,
    pub entity_type: String,
    pub entity_context: String,
    pub results: Vec<SearchResult>,
}

/// Search DuckDuckGo HTML for a query and return up to `max_results` results.
///
/// Uses the HTML-only endpoint (`html.duckduckgo.com/html/`) to avoid
/// JavaScript rendering. On any error, returns an empty vec — search must
/// never block enrichment.
pub async fn web_search(query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoded(query)
    );

    let response = client.get(&url).send().await?;
    let html = response.text().await?;

    let document = scraper::Html::parse_document(&html);
    let result_selector = scraper::Selector::parse(".result").unwrap();
    let title_selector = scraper::Selector::parse(".result__a").unwrap();
    let snippet_selector = scraper::Selector::parse(".result__snippet").unwrap();

    let mut results = Vec::new();

    for element in document.select(&result_selector) {
        if results.len() >= max_results {
            break;
        }

        let title = element
            .select(&title_selector)
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default()
            .trim()
            .to_string();

        let href = element
            .select(&title_selector)
            .next()
            .and_then(|el| el.value().attr("href"))
            .unwrap_or_default()
            .to_string();

        let snippet = element
            .select(&snippet_selector)
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default()
            .trim()
            .to_string();

        if !title.is_empty() {
            results.push(SearchResult {
                title,
                url: href,
                snippet,
            });
        }
    }

    Ok(results)
}

/// Search for each unresolved entity in the extraction result.
///
/// Filters entities where `resolved_to` is `None`, builds search queries,
/// and calls `web_search` per entity with a 500ms politeness delay.
pub async fn search_unresolved_entities(
    extraction: &ExtractionResult,
    max_results_per_entity: usize,
) -> Vec<EntitySearchResults> {
    let mut all_results = Vec::new();

    // Collect unresolved entities with their types
    let unresolved: Vec<(&str, &str, &str)> = extraction
        .people
        .iter()
        .filter(|e| e.resolved_to.is_none())
        .map(|e| (e.name.as_str(), "person", e.context.as_str()))
        .chain(
            extraction
                .organizations
                .iter()
                .filter(|e| e.resolved_to.is_none())
                .map(|e| (e.name.as_str(), "organization", e.context.as_str())),
        )
        .collect();

    for (i, (name, entity_type, context)) in unresolved.iter().enumerate() {
        // Politeness delay between searches
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        // Build search query: quoted name + first few context words
        let query = build_search_query(name, context);

        match web_search(&query, max_results_per_entity).await {
            Ok(results) if !results.is_empty() => {
                all_results.push(EntitySearchResults {
                    entity_name: name.to_string(),
                    entity_type: entity_type.to_string(),
                    entity_context: context.to_string(),
                    results,
                });
            }
            Ok(_) => {} // empty results — skip
            Err(_) => {} // search failed — skip silently
        }
    }

    all_results
}

/// Build a search query from an entity name and its context.
///
/// Uses the quoted entity name and appends the first few significant
/// context words for disambiguation.
fn build_search_query(name: &str, context: &str) -> String {
    let context_words: Vec<&str> = context
        .split_whitespace()
        .filter(|w| w.len() > 3)
        .take(3)
        .collect();

    if context_words.is_empty() {
        format!("\"{}\"", name)
    } else {
        format!("\"{}\" {}", name, context_words.join(" "))
    }
}

/// Simple URL-encoding for query parameters.
fn urlencoded(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            ' ' => "+".to_string(),
            '"' => "%22".to_string(),
            '&' => "%26".to_string(),
            '=' => "%3D".to_string(),
            '+' => "%2B".to_string(),
            '#' => "%23".to_string(),
            '?' => "%3F".to_string(),
            '/' => "%2F".to_string(),
            _ if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' => {
                c.to_string()
            }
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_search_query_with_context() {
        let query = build_search_query("Gianni Cuozzi", "discussing budget at Exein");
        assert_eq!(query, "\"Gianni Cuozzi\" discussing budget Exein");
    }

    #[test]
    fn test_build_search_query_no_context() {
        let query = build_search_query("Gianni Cuozzi", "");
        assert_eq!(query, "\"Gianni Cuozzi\"");
    }

    #[test]
    fn test_build_search_query_short_context_words() {
        let query = build_search_query("John", "is a CTO");
        // "is" and "a" are filtered out (<=3 chars)
        assert_eq!(query, "\"John\"");
    }

    #[test]
    fn test_urlencoded() {
        assert_eq!(urlencoded("hello world"), "hello+world");
        assert_eq!(urlencoded("\"test\""), "%22test%22");
        assert_eq!(urlencoded("a&b"), "a%26b");
    }

    #[test]
    fn test_search_result_creation() {
        let result = SearchResult {
            title: "Test".to_string(),
            url: "https://example.com".to_string(),
            snippet: "A test result".to_string(),
        };
        assert_eq!(result.title, "Test");
    }
}
