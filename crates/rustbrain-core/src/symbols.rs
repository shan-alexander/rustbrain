//! Symbol reference parsing and stable symbol node IDs.
//!
//! Notes may reference code with `symbol:…` anchors. This module parses those
//! refs, builds canonical `symbol/…` node ids, and resolves them against the set
//! of indexed symbol nodes.

use crate::id::node_id_from_rel_path;
use std::path::Path;

/// Build the canonical node id for a code symbol.
///
/// Format: `symbol/{crate}/{module_path_with_dots}/{symbol_name}`
/// where `symbol_name` may be `Type::method` for impl methods.
pub fn symbol_node_id(crate_name: &str, module_path: &str, symbol_name: &str) -> String {
    let module = module_path.replace("::", ".").replace('/', ".");
    format!(
        "symbol/{}/{}/{}",
        sanitize_seg(crate_name),
        sanitize_seg(&module),
        sanitize_seg(symbol_name)
    )
}

fn sanitize_seg(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == ':' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A reference from a Markdown note to a code symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRef {
    /// Raw text as written (e.g. `symbol:crate::foo::Bar` or `Bar`).
    pub raw: String,
    /// Optional crate qualifier.
    pub crate_name: Option<String>,
    /// Optional module path.
    pub module_path: Option<String>,
    /// Symbol name (may include `Type::method`).
    pub symbol_name: String,
}

/// Extract `symbol:…` references from Markdown body (outside code fences).
///
/// Supported forms:
/// - `symbol:Name`
/// - `symbol:module::Name`
/// - `symbol:crate::module::Name`
/// - WikiLinks `[[symbol:…]]` (target already extracted by wikilink parser — also scanned raw)
pub fn extract_symbol_refs(markdown: &str) -> Vec<SymbolRef> {
    let mut refs = Vec::new();
    let mut in_fence = false;
    let bytes = markdown.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if is_line_start(bytes, i) && i + 2 < bytes.len() && &bytes[i..i + 3] == b"```" {
            in_fence = !in_fence;
            i += 3;
            continue;
        }
        if in_fence {
            i += 1;
            continue;
        }
        if bytes[i] == b'`' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'`' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
            continue;
        }

        // Match "symbol:" via bytes (ASCII) so multi-byte UTF-8 never hits
        // a mid-char str index (e.g. '→' after `i += 1` through text).
        if i + 7 <= bytes.len() && &bytes[i..i + 7] == b"symbol:" {
            // `i` is a char boundary if we only entered here after ASCII
            // advances or from a boundary; enforce for safety.
            if !markdown.is_char_boundary(i) {
                i += 1;
                continue;
            }
            let start = i + 7;
            if !markdown.is_char_boundary(start) {
                i += 1;
                continue;
            }
            let rest = &markdown[start..];
            let end_rel = rest
                .find(|c: char| {
                    c.is_whitespace()
                        || c == ']'
                        || c == ')'
                        || c == ','
                        || c == ';'
                        || c == '"'
                        || c == '\''
                })
                .unwrap_or(rest.len());
            // end_rel is a char boundary by construction of str::find.
            let raw_path = rest[..end_rel].trim_end_matches(['.', '!', '?', ':']);
            if !raw_path.is_empty() {
                if let Some(sym) = parse_symbol_path(raw_path) {
                    refs.push(sym);
                }
            }
            i = start + end_rel;
            continue;
        }
        i += 1;
    }
    refs
}

/// Parse `crate::module::Name` or `Name` into a [`SymbolRef`].
pub fn parse_symbol_path(raw: &str) -> Option<SymbolRef> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let parts: Vec<&str> = raw.split("::").collect();
    match parts.len() {
        1 => Some(SymbolRef {
            raw: raw.to_string(),
            crate_name: None,
            module_path: None,
            symbol_name: parts[0].to_string(),
        }),
        2 => Some(SymbolRef {
            raw: raw.to_string(),
            crate_name: None,
            module_path: Some(parts[0].to_string()),
            symbol_name: parts[1].to_string(),
        }),
        n if n >= 3 => {
            let crate_name = parts[0].to_string();
            let symbol_name = parts[n - 1].to_string();
            let module_path = parts[1..n - 1].join("::");
            Some(SymbolRef {
                raw: raw.to_string(),
                crate_name: Some(crate_name),
                module_path: Some(module_path),
                symbol_name,
            })
        }
        _ => None,
    }
}

/// Resolve a symbol ref against known symbol node ids.
///
/// Matching priority:
/// 1. Exact full id if caller built one
/// 2. Exact `symbol_name` suffix unique match
/// 3. Unique `module::name` suffix
pub fn resolve_symbol_ref(
    sym: &SymbolRef,
    symbol_ids: &std::collections::HashSet<String>,
) -> Option<String> {
    // Direct constructed id attempts
    if let (Some(c), Some(m)) = (&sym.crate_name, &sym.module_path) {
        let id = symbol_node_id(c, m, &sym.symbol_name);
        if symbol_ids.contains(&id) {
            return Some(id);
        }
    }

    let name = sym.symbol_name.to_lowercase();
    let mut hits: Vec<&String> = symbol_ids
        .iter()
        .filter(|id| {
            let last = id.rsplit('/').next().unwrap_or("");
            last.eq_ignore_ascii_case(&sym.symbol_name)
                || last.to_lowercase().ends_with(&format!("::{name}"))
                || last.to_lowercase() == name
        })
        .collect();

    if let Some(m) = &sym.module_path {
        let m_norm = m.replace("::", ".");
        hits.retain(|id| id.contains(&m_norm) || id.contains(m));
    }
    if let Some(c) = &sym.crate_name {
        hits.retain(|id| id.contains(&format!("symbol/{c}/")) || id.contains(c));
    }

    hits.sort();
    hits.dedup();
    if hits.len() == 1 {
        Some(hits[0].clone())
    } else {
        None
    }
}

/// Helper for docs path ids (re-export convenience).
pub fn note_id_from_path(workspace_rel: &Path) -> String {
    node_id_from_rel_path(workspace_rel)
}

fn is_line_start(bytes: &[u8], i: usize) -> bool {
    i == 0 || bytes[i - 1] == b'\n'
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn extract_symbol_refs_basic() {
        let md = "See symbol:demo::storage::Engine and symbol:compact_log.\n";
        let refs = extract_symbol_refs(md);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].symbol_name, "Engine");
        assert_eq!(refs[0].crate_name.as_deref(), Some("demo"));
        assert_eq!(refs[1].symbol_name, "compact_log");
    }

    #[test]
    fn ignores_fenced_symbol() {
        let md = "```\nsymbol:Hidden\n```\nsymbol:Visible\n";
        let refs = extract_symbol_refs(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].symbol_name, "Visible");
    }

    #[test]
    fn unicode_arrows_do_not_panic() {
        // Real docs often use '→' (3-byte UTF-8). Byte-stepping must not slice mid-char.
        let md = "Flow: UI → backend. See symbol:query_json for details.\n";
        let refs = extract_symbol_refs(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].symbol_name, "query_json");
    }

    #[test]
    fn resolve_unique_suffix() {
        let mut ids = HashSet::new();
        ids.insert("symbol/demo/crate/StorageEngine".into());
        ids.insert("symbol/demo/crate/compact_log".into());
        let r = parse_symbol_path("StorageEngine").unwrap();
        assert_eq!(
            resolve_symbol_ref(&r, &ids).as_deref(),
            Some("symbol/demo/crate/StorageEngine")
        );
    }
}
