//! Ranking layer: turns fuzzy matches over a snippet's name and content into a
//! single ordered result list, blended with recency and frequency so a
//! developer's command memory surfaces the right thing first.

use crate::fuzzy::fuzzy_match;
use crate::model::{ScoredSnippet, Snippet};

/// Name matches are worth more than content matches: searching "prod" should
/// prefer items whose *title* contains "prod" (per the spec).
const NAME_WEIGHT: f64 = 1.6;
const CONTENT_WEIGHT: f64 = 1.0;

/// How much usage frequency contributes (log-scaled so a few uses help but
/// don't dominate relevance).
const FREQ_WEIGHT: f64 = 12.0;
/// How much recency contributes at most (decays with age).
const RECENCY_WEIGHT: f64 = 40.0;
/// Recency half-life in seconds (~7 days): a snippet used a week ago gets half
/// the recency boost of one used just now.
const RECENCY_HALFLIFE_SECS: f64 = 7.0 * 24.0 * 3600.0;

/// Split a raw query into terms. Multiple whitespace-separated terms are ANDed:
/// every term must match (in either field) for the snippet to be included.
/// This makes "docker compose" behave intuitively while still being fuzzy.
fn terms(query: &str) -> Vec<&str> {
    query.split_whitespace().collect()
}

/// Boost (in score points) from usage frequency and recency.
fn usage_boost(snippet: &Snippet, now: i64) -> f64 {
    let freq = (snippet.use_count.max(0) as f64).ln_1p() * FREQ_WEIGHT;
    let recency = match snippet.last_used_at {
        Some(t) => {
            let age = (now - t).max(0) as f64;
            RECENCY_WEIGHT * 0.5f64.powf(age / RECENCY_HALFLIFE_SECS)
        }
        None => 0.0,
    };
    freq + recency
}

/// Search `snippets` for `query` as of `now` (unix seconds), returning results
/// ordered best-first.
///
/// * Empty query → all snippets ordered purely by recency + frequency (the
///   "what was I just doing" view, ideal for the popup's empty state).
/// * Otherwise → only snippets matching every term, ordered by blended score.
pub fn search(snippets: &[Snippet], query: &str, now: i64) -> Vec<ScoredSnippet> {
    let terms = terms(query);

    let mut results: Vec<ScoredSnippet> = Vec::new();

    for snippet in snippets {
        if terms.is_empty() {
            // Empty-query view: everything, ranked by usage only.
            results.push(ScoredSnippet {
                score: usage_boost(snippet, now).round() as i64,
                name_indices: Vec::new(),
                content_indices: Vec::new(),
                snippet: snippet.clone(),
            });
            continue;
        }

        let mut total = 0.0f64;
        let mut name_indices: Vec<usize> = Vec::new();
        let mut content_indices: Vec<usize> = Vec::new();
        let mut all_matched = true;

        for term in &terms {
            let name_m = fuzzy_match(term, &snippet.name);
            let content_m = fuzzy_match(term, &snippet.content);

            let name_score = name_m.as_ref().map(|m| m.score as f64 * NAME_WEIGHT);
            let content_score = content_m.as_ref().map(|m| m.score as f64 * CONTENT_WEIGHT);

            match (name_score, content_score) {
                (None, None) => {
                    all_matched = false;
                    break;
                }
                _ => {
                    // Take the better field for this term, and record indices
                    // from whichever field(s) matched for highlighting.
                    let best = name_score.unwrap_or(f64::MIN).max(content_score.unwrap_or(f64::MIN));
                    total += best;
                    if let Some(m) = name_m {
                        name_indices.extend(m.indices);
                    }
                    if let Some(m) = content_m {
                        content_indices.extend(m.indices);
                    }
                }
            }
        }

        if !all_matched {
            continue;
        }

        total += usage_boost(snippet, now);

        name_indices.sort_unstable();
        name_indices.dedup();
        content_indices.sort_unstable();
        content_indices.dedup();

        results.push(ScoredSnippet {
            score: total.round() as i64,
            name_indices,
            content_indices,
            snippet: snippet.clone(),
        });
    }

    // Highest score first; tie-break by most-recently-used, then name for
    // deterministic ordering.
    results.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.snippet.last_used_at.cmp(&a.snippet.last_used_at))
            .then_with(|| a.snippet.name.cmp(&b.snippet.name))
    });

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snip(id: i64, name: &str, content: &str) -> Snippet {
        Snippet {
            id,
            name: name.to_string(),
            content: content.to_string(),
            created_at: 0,
            last_used_at: None,
            use_count: 0,
        }
    }

    fn corpus() -> Vec<Snippet> {
        vec![
            snip(1, "Production migration command", "docker compose exec api python manage.py migrate"),
            snip(2, "Local docker compose up", "docker compose up -d"),
            snip(3, "Prod SSH", "ssh deploy@prod.example.com"),
            snip(4, "Random regex", "^\\d{3}-\\d{4}$"),
        ]
    }

    #[test]
    fn mig_finds_production_migration_command() {
        let results = search(&corpus(), "mig", 1000);
        assert!(!results.is_empty());
        assert_eq!(results[0].snippet.id, 1, "expected migration snippet first");
    }

    #[test]
    fn docker_compose_finds_compose_commands() {
        let results = search(&corpus(), "docker compose", 1000);
        let ids: Vec<i64> = results.iter().map(|r| r.snippet.id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
        // The regex snippet should not appear.
        assert!(!ids.contains(&4));
    }

    #[test]
    fn prod_prefers_title_match() {
        let results = search(&corpus(), "prod", 1000);
        assert!(!results.is_empty());
        // "Prod SSH" (title) should beat "Production migration command" only by
        // weight; both are valid. The key requirement: a title-"prod" item ranks
        // at the very top.
        assert_eq!(results[0].snippet.id, 3);
    }

    #[test]
    fn all_terms_must_match() {
        // "docker regex" matches nothing (no snippet has both).
        let results = search(&corpus(), "docker regex", 1000);
        assert!(results.is_empty());
    }

    #[test]
    fn empty_query_returns_everything_by_usage() {
        let mut c = corpus();
        c[2].use_count = 10;
        c[2].last_used_at = Some(1000);
        let results = search(&c, "", 1000);
        assert_eq!(results.len(), 4);
        assert_eq!(results[0].snippet.id, 3, "most-used should be first");
    }

    #[test]
    fn recency_breaks_ties_for_equal_relevance() {
        let mut c = vec![
            snip(1, "deploy staging", "kubectl apply -f staging.yaml"),
            snip(2, "deploy staging", "kubectl apply -f staging.yaml"),
        ];
        c[1].last_used_at = Some(900);
        c[1].use_count = 3;
        let results = search(&c, "deploy", 1000);
        assert_eq!(results[0].snippet.id, 2, "recently/often used wins ties");
    }

    #[test]
    fn highlight_indices_are_returned_for_name() {
        let results = search(&corpus(), "prod", 1000);
        let top = &results[0];
        assert!(!top.name_indices.is_empty());
        // indices must be sorted & unique
        let mut sorted = top.name_indices.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, top.name_indices);
    }
}
