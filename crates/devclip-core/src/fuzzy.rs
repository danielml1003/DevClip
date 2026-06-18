//! Fuzzy subsequence matching with quality-oriented scoring.
//!
//! Search quality is one of the most important aspects of DevClip, so the
//! matcher is deliberately written by hand rather than pulled from a crate —
//! this gives us full control over the heuristics that make a developer's
//! command memory feel "smart":
//!
//! * a query matches if all of its characters appear in order (subsequence),
//! * contiguous runs are rewarded (typing `docker` should rank a literal
//!   "docker" run above six scattered letters),
//! * matches at word boundaries / camelCase humps are rewarded (so `mig`
//!   strongly matches the "migration" word inside "Production migration
//!   command"),
//! * an exact substring is rewarded most of all,
//! * leading gaps and total gaps are gently penalised.
//!
//! Matching is case-insensitive. The matched character indices are returned so
//! the UI can highlight exactly what the user typed.

/// Score awarded for every matched character (the baseline).
const SCORE_MATCH: i64 = 16;
/// Extra reward when a matched character immediately follows another match.
const SCORE_CONSECUTIVE: i64 = 18;
/// Reward when a match lands on the first character of a word.
const SCORE_WORD_START: i64 = 30;
/// Reward when a match lands on a camelCase hump (lower -> Upper transition).
const SCORE_CAMEL: i64 = 22;
/// Reward when the whole query appears as a contiguous substring.
const SCORE_SUBSTRING: i64 = 40;
/// Reward when the match begins at the very start of the text.
const SCORE_PREFIX: i64 = 24;
/// Penalty per character skipped before the first match.
const PENALTY_LEADING: i64 = 3;
/// Penalty per gap between matched characters.
const PENALTY_GAP: i64 = 2;
/// Cap on the leading penalty so a late-but-good match is never killed outright.
const PENALTY_LEADING_MAX: i64 = 30;

/// The result of matching one query term against one piece of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub score: i64,
    /// Indices (into `text`'s `char` sequence) that were matched, ascending.
    pub indices: Vec<usize>,
}

/// Classify a character for boundary detection.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// Returns `Some(Match)` if every character of `query` appears in `text` in
/// order (case-insensitive), or `None` otherwise.
///
/// An empty query matches everything with a score of 0 and no indices.
pub fn fuzzy_match(query: &str, text: &str) -> Option<Match> {
    if query.is_empty() {
        return Some(Match { score: 0, indices: Vec::new() });
    }

    let t: Vec<char> = text.chars().collect();
    let q: Vec<char> = query.chars().collect();
    let t_lower: Vec<char> = t.iter().flat_map(|c| c.to_lowercase()).collect();
    // `to_lowercase` can change length for some characters; for matching we want
    // a 1:1 mapping, so fall back to a simple per-char lowercase.
    let t_lower: Vec<char> = if t_lower.len() == t.len() {
        t_lower
    } else {
        t.iter().map(|c| c.to_ascii_lowercase()).collect()
    };
    let q_lower: Vec<char> = q.iter().map(|c| simple_lower(*c)).collect();

    // Fast path: exact substring match. This is both common and the strongest
    // possible signal, so we special-case it for quality and speed.
    if let Some(start) = find_substring(&t_lower, &q_lower) {
        let indices: Vec<usize> = (start..start + q_lower.len()).collect();
        let mut score = SCORE_SUBSTRING + q_lower.len() as i64 * SCORE_MATCH;
        // All-consecutive by definition.
        score += (q_lower.len() as i64 - 1) * SCORE_CONSECUTIVE;
        if start == 0 {
            // A prefix is the strongest possible boundary: word-start + prefix.
            score += SCORE_WORD_START + SCORE_PREFIX;
        } else if is_boundary(&t, start) {
            score += SCORE_WORD_START;
        }
        score -= (start as i64).min(PENALTY_LEADING_MAX) * PENALTY_LEADING / 3;
        return Some(Match { score, indices });
    }

    // General subsequence match. Greedy forward scan is fast and, combined with
    // the substring fast-path above, gives good results for short dev queries.
    let mut indices = Vec::with_capacity(q_lower.len());
    let mut ti = 0usize;
    for &qc in &q_lower {
        let mut found = None;
        while ti < t_lower.len() {
            if t_lower[ti] == qc {
                found = Some(ti);
                ti += 1;
                break;
            }
            ti += 1;
        }
        match found {
            Some(idx) => indices.push(idx),
            None => return None,
        }
    }

    Some(Match { score: score_indices(&t, &indices), indices })
}

/// Lowercase a single char without changing length (ASCII-fast, Unicode-safe).
fn simple_lower(c: char) -> char {
    if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// Is position `i` the start of a word (boundary) within `text`?
fn is_boundary(text: &[char], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let prev = text[i - 1];
    let cur = text[i];
    // Previous char is a separator (space, punctuation, etc.)
    if !is_word_char(prev) {
        return true;
    }
    // camelCase hump: lower/digit -> Upper.
    if cur.is_uppercase() && !prev.is_uppercase() {
        return true;
    }
    false
}

/// Score a set of matched indices for `text` using the boundary/consecutive
/// heuristics.
fn score_indices(text: &[char], indices: &[usize]) -> i64 {
    if indices.is_empty() {
        return 0;
    }
    let mut score = 0i64;
    let first = indices[0];
    // Leading-gap penalty (capped) — earlier matches are better.
    score -= (first as i64).min(PENALTY_LEADING_MAX) * PENALTY_LEADING;

    let mut prev: Option<usize> = None;
    for &idx in indices {
        score += SCORE_MATCH;
        if idx == 0 {
            // A prefix is the strongest possible boundary: word-start + prefix.
            score += SCORE_WORD_START + SCORE_PREFIX;
        } else if is_boundary(text, idx) {
            score += SCORE_WORD_START;
            if text[idx].is_uppercase() && !text[idx - 1].is_uppercase() {
                score += SCORE_CAMEL;
            }
        }
        if let Some(p) = prev {
            if idx == p + 1 {
                score += SCORE_CONSECUTIVE;
            } else {
                score -= ((idx - p - 1) as i64).min(20) * PENALTY_GAP;
            }
        }
        prev = Some(idx);
    }
    score
}

/// Find the first index where `needle` occurs contiguously in `haystack`.
fn find_substring(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    let last = haystack.len() - needle.len();
    'outer: for start in 0..=last {
        for (k, &nc) in needle.iter().enumerate() {
            if haystack[start + k] != nc {
                continue 'outer;
            }
        }
        return Some(start);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(q: &str, t: &str) -> Option<i64> {
        fuzzy_match(q, t).map(|m| m.score)
    }

    #[test]
    fn empty_query_matches_everything() {
        let m = fuzzy_match("", "anything").unwrap();
        assert_eq!(m.score, 0);
        assert!(m.indices.is_empty());
    }

    #[test]
    fn non_subsequence_does_not_match() {
        assert!(fuzzy_match("xyz", "Production migration command").is_none());
        // Right characters but wrong order.
        assert!(fuzzy_match("gim", "migration").is_none());
    }

    #[test]
    fn mig_matches_migration_inside_a_sentence() {
        // The headline requirement: "mig" must find "Production migration command".
        let m = fuzzy_match("mig", "Production migration command").unwrap();
        assert!(m.score > 0);
        // It should match the contiguous "mig" of "migration".
        assert_eq!(m.indices.len(), 3);
    }

    #[test]
    fn substring_beats_scattered() {
        let contiguous = score("mig", "migration").unwrap();
        let scattered = score("mig", "m_i_g").unwrap();
        assert!(contiguous > scattered, "{contiguous} should beat {scattered}");
    }

    #[test]
    fn word_boundary_beats_mid_word() {
        // "migration command" — matching at the start of a word is better than
        // matching the same letters mid-word.
        let at_boundary = score("co", "docker compose").unwrap();
        let mid_word = score("oc", "docker compose").unwrap();
        assert!(at_boundary > mid_word, "{at_boundary} vs {mid_word}");
    }

    #[test]
    fn prefix_is_strong() {
        let prefix = score("prod", "production database").unwrap();
        let middle = score("prod", "the prod box").unwrap();
        assert!(prefix > 0 && middle > 0);
        assert!(prefix >= middle);
    }

    #[test]
    fn camel_case_humps_match() {
        let m = fuzzy_match("gp", "getProductionConfig").unwrap();
        // g (prefix) + P (camel hump)
        assert_eq!(m.indices, vec![0, 3]);
        assert!(m.score > 0);
    }

    #[test]
    fn case_insensitive() {
        assert!(fuzzy_match("DOCKER", "docker compose up").is_some());
        assert!(fuzzy_match("docker", "DOCKER COMPOSE").is_some());
    }

    #[test]
    fn indices_point_at_matched_chars() {
        let m = fuzzy_match("dc", "docker compose").unwrap();
        // d at 0, c at index of "compose"
        assert_eq!(m.indices[0], 0);
        assert_eq!("docker compose".chars().nth(m.indices[1]).unwrap(), 'c');
    }
}
