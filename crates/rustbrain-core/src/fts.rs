//! FTS5 query helpers: escaping, stopword stripping, and safe `MATCH` construction.
//!
//! User input must never be passed raw to SQLite FTS5: operators like `AND`,
//! `OR`, `NEAR`, and bare `"` can change query structure or cause errors.
//! Natural-language prompts also contain stopwords (`why`, `not`, `the`) that
//! would force AND-matches to fail when those tokens are absent from notes.

use crate::error::{BrainError, Result};

/// English stopwords + question fillers stripped for retrieval (not for display).
const STOPWORDS: &[&str] = &[
    "a", "about", "above", "after", "again", "against", "all", "also", "an", "and",
    "are", "as", "at", "be", "been", "before", "being", "below", "between", "both",
    "but", "by", "can", "could", "describe", "did", "do", "does", "down", "during",
    "each", "else", "explain", "few", "find", "for", "from", "further", "get", "give",
    "had", "has", "have", "help", "here", "how", "i", "if", "in", "into", "is", "it",
    "its", "just", "look", "looking", "make", "may", "me", "might", "more", "most",
    "must", "my", "need", "needs", "no", "nor", "not", "of", "off", "on", "once",
    "only", "or", "other", "our", "out", "over", "own", "per", "please", "same",
    "shall", "should", "show", "so", "some", "such", "summarize", "tell", "than",
    "that", "the", "their", "them", "then", "there", "these", "they", "this", "those",
    "through", "to", "too", "under", "up", "use", "used", "using", "versus", "very",
    "via", "vs", "want", "wants", "was", "we", "were", "what", "when", "where",
    "which", "who", "whom", "whose", "why", "will", "with", "would", "you", "your",
];

/// Prepared FTS query plus the significant tokens used for ranking boosts.
#[derive(Debug, Clone)]
pub struct PreparedQuery {
    /// Escaped FTS5 `MATCH` expression (quoted tokens, optional OR).
    pub fts_match: String,
    /// Lowercase significant tokens (stopwords removed when possible).
    pub tokens: Vec<String>,
    /// True when multiple significant tokens were OR-joined for recall.
    pub used_or: bool,
}

/// Tokenize raw user text: split on whitespace/punctuation, lowercase, keep alnum/_/-/:/.
pub fn tokenize_query(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in raw.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == ':' || ch == '.' {
            cur.push(ch.to_ascii_lowercase());
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.retain(|t| !t.is_empty() && t != "-" && t != "_" && t != ":" && t != ".");
    out
}

fn is_stopword(t: &str) -> bool {
    STOPWORDS.binary_search(&t).is_ok()
}

/// Strip stopwords; if nothing remains, fall back to tokens longer than 1 char.
pub fn significant_tokens(tokens: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut kept: Vec<String> = tokens
        .iter()
        .filter(|t| !is_stopword(t) && t.len() > 1)
        .filter(|t| seen.insert((*t).clone()))
        .cloned()
        .collect();
    if kept.is_empty() {
        seen.clear();
        kept = tokens
            .iter()
            .filter(|t| t.len() > 1)
            .filter(|t| seen.insert((*t).clone()))
            .cloned()
            .collect();
    }
    if kept.is_empty() {
        kept = tokens.to_vec();
    }
    kept
}

fn quote_fts_token(t: &str) -> String {
    let q = t.replace('"', "\"\"");
    format!("\"{q}\"")
}

/// Tokens that are real English words but too generic to retrieve alone.
const GENERIC_TOPIC_TOKENS: &[&str] = &[
    "architecture", "code", "codebase", "design", "overview", "project", "repo",
    "repository", "software", "stack", "structure", "system", "tool", "tools",
];

/// True when every token is a generic overview word (or the list is empty).
pub fn is_generic_topic(tokens: &[String]) -> bool {
    if tokens.is_empty() {
        return true;
    }
    tokens.iter().all(|t| {
        GENERIC_TOPIC_TOKENS.binary_search(&t.as_str()).is_ok()
    })
}

/// Build a safe FTS5 MATCH string and ranking tokens from raw user input.
///
/// Strategy:
/// 1. Tokenize (punctuation-safe)
/// 2. Prefer significant (non-stopword) tokens
/// 3. Single token → exact quoted match
/// 4. Multiple tokens → `OR` between them for natural-language recall
///    (documents rarely contain every filler word from a full question)
///
/// # Errors
///
/// Returns [`BrainError::FtsQuery`] when no usable token remains.
pub fn prepare_search_query(raw: &str) -> Result<PreparedQuery> {
    let all = tokenize_query(raw);
    if all.is_empty() {
        return Err(BrainError::FtsQuery(
            "query must contain at least one alphanumeric token".into(),
        ));
    }
    let tokens = significant_tokens(&all);
    if tokens.is_empty() {
        return Err(BrainError::FtsQuery(
            "query must contain at least one non-stopword token".into(),
        ));
    }

    let used_or = tokens.len() > 1;
    let fts_match = if used_or {
        tokens
            .iter()
            .map(|t| quote_fts_token(t))
            .collect::<Vec<_>>()
            .join(" OR ")
    } else {
        quote_fts_token(&tokens[0])
    };

    Ok(PreparedQuery {
        fts_match,
        tokens,
        used_or,
    })
}

/// Escape a user query for SQLite FTS5 `MATCH` (legacy helper).
///
/// Prefer [`prepare_search_query`] for search/context paths — this keeps
/// whitespace-split quoting without stopword handling for callers that want
/// literal AND of every token.
pub fn escape_fts5_query(raw: &str) -> Result<String> {
    let tokens: Vec<&str> = raw.split_whitespace().filter(|t| !t.is_empty()).collect();
    if tokens.is_empty() {
        return Err(BrainError::FtsQuery(
            "query must contain at least one non-whitespace token".into(),
        ));
    }

    let escaped = tokens
        .into_iter()
        .map(|t| {
            let q = t.replace('"', "\"\"");
            format!("\"{q}\"")
        })
        .collect::<Vec<_>>()
        .join(" ");

    Ok(escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopwords_sorted() {
        let mut sorted = STOPWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            STOPWORDS,
            sorted.as_slice(),
            "STOPWORDS must be sorted for binary_search"
        );
    }

    #[test]
    fn escapes_and_quotes_tokens() {
        let q = escape_fts5_query(r#"raft "consensus""#).unwrap();
        assert_eq!(q, r#""raft" """consensus""""#);
    }

    #[test]
    fn rejects_empty() {
        assert!(escape_fts5_query("   ").is_err());
        assert!(prepare_search_query("   ").is_err());
        assert!(prepare_search_query("!!!").is_err());
    }

    #[test]
    fn operators_become_literals() {
        let q = escape_fts5_query("foo AND bar").unwrap();
        assert_eq!(q, r#""foo" "AND" "bar""#);
    }

    #[test]
    fn prepare_strips_stopwords_and_ors() {
        let p = prepare_search_query("why egui not tauri").unwrap();
        assert_eq!(p.tokens, vec!["egui".to_string(), "tauri".to_string()]);
        assert!(p.used_or);
        assert_eq!(p.fts_match, r#""egui" OR "tauri""#);
    }

    #[test]
    fn prepare_single_significant() {
        let p = prepare_search_query("explain raft").unwrap();
        assert_eq!(p.tokens, vec!["raft".to_string()]);
        assert!(!p.used_or);
        assert_eq!(p.fts_match, r#""raft""#);
    }

    #[test]
    fn tokenize_strips_punctuation() {
        let t = tokenize_query("egui/tauri + duckdb?");
        assert!(t.contains(&"egui".to_string()));
        assert!(t.contains(&"tauri".to_string()));
        assert!(t.contains(&"duckdb".to_string()));
    }

    #[test]
    fn generic_topic_detection() {
        assert!(is_generic_topic(&["architecture".into(), "overview".into()]));
        assert!(!is_generic_topic(&["egui".into(), "architecture".into()]));
        let p = prepare_search_query("summarize architecture").unwrap();
        // "summarize" is a stopword → only architecture remains → generic
        assert!(is_generic_topic(&p.tokens), "tokens={:?}", p.tokens);
    }

    #[test]
    fn generic_tokens_sorted() {
        let mut s = GENERIC_TOPIC_TOKENS.to_vec();
        s.sort_unstable();
        assert_eq!(GENERIC_TOPIC_TOKENS, s.as_slice());
    }
}
