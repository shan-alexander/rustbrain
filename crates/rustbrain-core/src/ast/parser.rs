//! Tree-sitter Rust parser for top-level and nested items.

use crate::ast::symbol::{compute_symbol_hash, AstError, SymbolAnchor};
use tree_sitter::{Node, Parser, TreeCursor};

/// Parses Rust source into [`SymbolAnchor`] records.
pub struct CodeAstParser {
    parser: Parser,
}

impl CodeAstParser {
    /// Create a parser loaded with the tree-sitter Rust grammar.
    pub fn new_rust() -> Result<Self, AstError> {
        let mut parser = Parser::new();
        let language = tree_sitter_rust::LANGUAGE.into();
        parser
            .set_language(&language)
            .map_err(|e| AstError::Message(format!("failed to load tree-sitter rust: {e}")))?;
        Ok(Self { parser })
    }

    /// Extract symbol anchors from Rust source.
    ///
    /// `module_path` should be a logical module path (e.g. `storage::db`), not an
    /// absolute filesystem path. Callers may derive it from a relative file path.
    ///
    /// Impl methods are recorded as `Type::method` for stable, location-independent
    /// identity within the module.
    pub fn parse_symbols(
        &mut self,
        crate_name: &str,
        file_path: &str,
        source_code: &str,
    ) -> Result<Vec<SymbolAnchor>, AstError> {
        let tree = self
            .parser
            .parse(source_code, None)
            .ok_or_else(|| AstError::Message("tree-sitter parse returned None".into()))?;

        let module_path = module_path_from_file(file_path);
        let mut anchors = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();
        walk_items(
            &mut cursor,
            source_code.as_bytes(),
            crate_name,
            &module_path,
            file_path,
            None,
            &mut anchors,
        );
        Ok(anchors)
    }

    /// Recursively scan a directory for `.rs` files.
    pub fn scan_directory(
        &mut self,
        crate_name: &str,
        dir_path: &std::path::Path,
    ) -> Result<Vec<SymbolAnchor>, AstError> {
        let mut all_anchors = Vec::new();
        if !dir_path.exists() {
            return Ok(all_anchors);
        }

        let entries =
            std::fs::read_dir(dir_path).map_err(|e| AstError::Message(e.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|e| AstError::Message(e.to_string()))?;
            let path = entry.path();

            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name == "target" || name.starts_with('.') {
                        continue;
                    }
                }
                all_anchors.extend(self.scan_directory(crate_name, &path)?);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                if let Ok(source) = std::fs::read_to_string(&path) {
                    let rel_path = path.to_string_lossy().to_string();
                    if let Ok(anchors) = self.parse_symbols(crate_name, &rel_path, &source) {
                        all_anchors.extend(anchors);
                    }
                }
            }
        }
        Ok(all_anchors)
    }
}

/// Derive a logical module path from a repository-relative file path.
fn module_path_from_file(file_path: &str) -> String {
    let normalized = file_path.replace('\\', "/");
    let mut parts: Vec<&str> = normalized.split('/').collect();

    if let Some(idx) = parts.iter().position(|p| *p == "src") {
        parts = parts[idx + 1..].to_vec();
    }

    if parts.is_empty() {
        return "crate".to_string();
    }

    if let Some(last) = parts.last_mut() {
        if let Some(stem) = last.strip_suffix(".rs") {
            *last = stem;
        }
    }

    match parts.last().copied() {
        Some("mod") | Some("lib") | Some("main") => {
            parts.pop();
        }
        _ => {}
    }

    if parts.is_empty() {
        "crate".to_string()
    } else {
        parts.join("::")
    }
}

fn walk_items(
    cursor: &mut TreeCursor,
    source: &[u8],
    crate_name: &str,
    module_path: &str,
    file_path: &str,
    impl_type: Option<&str>,
    out: &mut Vec<SymbolAnchor>,
) {
    loop {
        let node = cursor.node();
        let next_impl = if node.kind() == "impl_item" {
            extract_impl_type(&node, source)
        } else {
            None
        };
        let effective_impl = next_impl.as_deref().or(impl_type);

        maybe_record_item(
            &node,
            source,
            crate_name,
            module_path,
            file_path,
            effective_impl,
            out,
        );

        if cursor.goto_first_child() {
            walk_items(
                cursor,
                source,
                crate_name,
                module_path,
                file_path,
                effective_impl,
                out,
            );
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn extract_impl_type(node: &Node, source: &[u8]) -> Option<String> {
    // Prefer type field; fall back to scanning children for type_identifier.
    if let Some(t) = node.child_by_field_name("type") {
        if let Ok(text) = t.utf8_text(source) {
            let name = text
                .split(['<', ' ', ':'])
                .next()
                .unwrap_or(text)
                .trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    let mut c = node.walk();
    for child in node.children(&mut c) {
        if child.kind() == "type_identifier" {
            if let Ok(text) = child.utf8_text(source) {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn maybe_record_item(
    node: &Node,
    source: &[u8],
    crate_name: &str,
    module_path: &str,
    file_path: &str,
    impl_type: Option<&str>,
    out: &mut Vec<SymbolAnchor>,
) {
    let kind = node.kind();
    let interesting = matches!(
        kind,
        "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "mod_item"
            | "type_item"
            | "const_item"
            | "static_item"
            | "function_signature_item"
            | "union_item"
            | "macro_definition"
    );
    if !interesting {
        return;
    }

    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Ok(raw_name) = name_node.utf8_text(source) else {
        return;
    };

    // Qualify methods with their impl type: `StorageEngine::open`
    let symbol_name = if kind == "function_item" || kind == "function_signature_item" {
        if let Some(ty) = impl_type {
            format!("{ty}::{raw_name}")
        } else {
            raw_name.to_string()
        }
    } else {
        raw_name.to_string()
    };

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let symbol_hash = compute_symbol_hash(crate_name, module_path, &symbol_name);
    let doc_comment = collect_doc_comment(node, source);

    out.push(SymbolAnchor {
        symbol_hash,
        crate_name: crate_name.to_string(),
        module_path: module_path.to_string(),
        symbol_name,
        file_path: file_path.to_string(),
        start_line,
        end_line,
        doc_comment,
    });
}

fn collect_doc_comment(node: &Node, source: &[u8]) -> Option<String> {
    let mut prev = node.prev_sibling();
    let mut stack: Vec<String> = Vec::new();
    while let Some(p) = prev {
        if p.kind() == "line_comment" || p.kind() == "block_comment" {
            if let Ok(t) = p.utf8_text(source) {
                let t = t.trim();
                if t.starts_with("///") || t.starts_with("/**") || t.starts_with("//!") {
                    stack.push(t.to_string());
                } else {
                    break;
                }
            }
            prev = p.prev_sibling();
        } else if p.kind() == "attribute_item" {
            prev = p.prev_sibling();
        } else {
            break;
        }
    }
    stack.reverse();
    if stack.is_empty() {
        None
    } else {
        Some(stack.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_impl_methods_qualified() {
        let mut p = CodeAstParser::new_rust().unwrap();
        let src = r#"
            /// Engine
            pub struct StorageEngine;
            impl StorageEngine {
                /// open it
                pub fn open() {}
            }
            pub fn compact_log() {}
        "#;
        let anchors = p.parse_symbols("demo", "src/lib.rs", src).unwrap();
        let names: Vec<_> = anchors.iter().map(|a| a.symbol_name.as_str()).collect();
        assert!(names.contains(&"StorageEngine"));
        assert!(
            names.contains(&"StorageEngine::open"),
            "expected qualified method, got {names:?}"
        );
        assert!(names.contains(&"compact_log"));
        // Method hash differs from free function with same bare name
        let method = anchors
            .iter()
            .find(|a| a.symbol_name == "StorageEngine::open")
            .unwrap();
        let free = compute_symbol_hash("demo", "crate", "open");
        assert_ne!(method.symbol_hash, free);
    }

    #[test]
    fn module_path_from_nested_file() {
        assert_eq!(module_path_from_file("src/storage/db.rs"), "storage::db");
        assert_eq!(module_path_from_file("src/lib.rs"), "crate");
    }
}
