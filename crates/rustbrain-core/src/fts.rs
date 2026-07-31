//! FTS5 query helpers: escaping and safe `MATCH` construction.
//!
//! User input must never be passed raw to SQLite FTS5: operators like `AND`,
//! `OR`, `NEAR`, and bare `"` can change query structure or cause errors.

use crate::error::{BrainError, Result};

/// Escape a user query for SQLite FTS5 `MATCH`.
///
/// Each whitespace-separated token is double-quoted so punctuation and FTS
/// operators in user input become literals. Embedded `"` are doubled per the
/// FTS5 quoting rules. Empty input returns [`BrainError::FtsQuery`] rather than
/// matching everything.
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
            // FTS5 escapes embedded double-quotes by doubling them.
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
    fn escapes_and_quotes_tokens() {
        // Input token `"consensus"` doubles internal quotes, then outer-quotes:
        // " + ""consensus"" + "  →  """consensus"""
        let q = escape_fts5_query(r#"raft "consensus""#).unwrap();
        assert_eq!(q, r#""raft" """consensus""""#);
    }

    #[test]
    fn rejects_empty() {
        assert!(escape_fts5_query("   ").is_err());
    }

    #[test]
    fn operators_become_literals() {
        // AND/OR would be operators unquoted; we force literals.
        let q = escape_fts5_query("foo AND bar").unwrap();
        assert_eq!(q, r#""foo" "AND" "bar""#);
    }
}
