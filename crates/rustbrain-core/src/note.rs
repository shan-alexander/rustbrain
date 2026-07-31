//! Create structured Markdown notes for the brain (`note new`).

use crate::error::{BrainError, Result};
use crate::id::node_id_from_rel_path;
use crate::types::NodeType;
use std::path::{Path, PathBuf};

/// Options for [`create_note`].
#[derive(Debug, Clone)]
pub struct NoteNewOptions {
    /// Node type (`goal`, `adr`, `concept`, …).
    pub node_type: NodeType,
    /// Human title (also used for H1).
    pub title: String,
    /// Optional body text after the title (agents use `--note`).
    pub note: Option<String>,
    /// Optional extra tags.
    pub tags: Vec<String>,
    /// Optional aliases.
    pub aliases: Vec<String>,
    /// Subdirectory under workspace (default chosen from type).
    pub dir: Option<PathBuf>,
    /// If true, overwrite an existing file at the target path.
    pub force: bool,
}

/// Result of creating a note on disk.
#[derive(Debug, Clone)]
pub struct NoteCreated {
    /// Absolute path written.
    pub path: PathBuf,
    /// Relative path from workspace.
    pub rel_path: PathBuf,
    /// Stable node id after next sync.
    pub node_id: String,
}

/// Default docs subdirectory for a node type.
pub fn default_dir_for_type(ty: &NodeType) -> &'static str {
    match ty {
        NodeType::Goal => "docs/goals",
        NodeType::Adr => "docs/adr",
        NodeType::Alternative => "docs/adr",
        NodeType::Concept => "docs/concepts",
        NodeType::Symbol => "docs/concepts",
        NodeType::Reference => "docs/concepts",
        NodeType::EdgeCase => "docs/edge_cases",
    }
}

/// Slugify a title into a filename stem.
pub fn slugify_title(title: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "untitled".into()
    } else {
        out
    }
}

/// Create a Markdown note under the workspace.
///
/// Does **not** open SQLite — call [`crate::Brain::sync`] afterward to index.
pub fn create_note(workspace: &Path, opts: &NoteNewOptions) -> Result<NoteCreated> {
    if opts.title.trim().is_empty() {
        return Err(BrainError::other("note title must not be empty"));
    }

    let dir_rel = opts
        .dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(default_dir_for_type(&opts.node_type)));
    let abs_dir = workspace.join(&dir_rel);
    std::fs::create_dir_all(&abs_dir)?;

    let stem = slugify_title(&opts.title);
    let filename = format!("{stem}.md");
    let abs_path = abs_dir.join(&filename);
    let rel_path = dir_rel.join(&filename);

    if abs_path.exists() && !opts.force {
        return Err(BrainError::other(format!(
            "note already exists: {} (use --force to overwrite)",
            rel_path.display()
        )));
    }

    let mut tags = opts.tags.clone();
    if tags.is_empty() {
        tags.push(opts.node_type.as_str().to_string());
    }

    let tags_yaml = format_yaml_list(&tags);
    let aliases_yaml = if opts.aliases.is_empty() {
        String::new()
    } else {
        format!("aliases: {}\n", format_yaml_list(&opts.aliases))
    };

    let body = opts.note.as_deref().unwrap_or("").trim();
    let body_section = if body.is_empty() {
        match opts.node_type {
            NodeType::Adr => {
                "\n## Status\n\nProposed\n\n## Context\n\n<!-- why this decision is needed -->\n\n## Decision\n\n<!-- what we decided -->\n\n## Consequences\n\n<!-- trade-offs -->\n"
                    .to_string()
            }
            NodeType::Goal => {
                "\n## Goals\n\n- \n\n## Non-goals\n\n- \n".to_string()
            }
            _ => "\n".to_string(),
        }
    } else {
        format!("\n{body}\n")
    };

    let content = format!(
        "---\n\
         tags: {tags_yaml}\n\
         node_type: {}\n\
         {aliases_yaml}\
         ---\n\
         # {}\n\
         {body_section}",
        opts.node_type.as_str(),
        opts.title.trim(),
    );

    std::fs::write(&abs_path, content)?;

    let node_id = node_id_from_rel_path(&rel_path);
    Ok(NoteCreated {
        path: abs_path,
        rel_path,
        node_id,
    })
}

fn format_yaml_list(items: &[String]) -> String {
    if items.is_empty() {
        return "[]".into();
    }
    let inner = items
        .iter()
        .map(|t| {
            if t.chars().any(|c| c.is_whitespace() || c == ':' || c == '#') {
                format!("\"{}\"", t.replace('"', "\\\""))
            } else {
                t.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_adr_with_body() {
        let dir = tempdir().unwrap();
        let created = create_note(
            dir.path(),
            &NoteNewOptions {
                node_type: NodeType::Adr,
                title: "Use SQLite".into(),
                note: Some("We need embedded storage.".into()),
                tags: vec!["storage".into()],
                aliases: vec![],
                dir: None,
                force: false,
            },
        )
        .unwrap();
        assert!(created.path.exists());
        let text = std::fs::read_to_string(&created.path).unwrap();
        assert!(text.contains("node_type: adr"));
        assert!(text.contains("# Use SQLite"));
        assert!(text.contains("We need embedded storage."));
        assert_eq!(created.node_id, "docs/adr/use-sqlite");
    }

    #[test]
    fn refuses_overwrite_without_force() {
        let dir = tempdir().unwrap();
        let opts = NoteNewOptions {
            node_type: NodeType::Concept,
            title: "Dup".into(),
            note: None,
            tags: vec![],
            aliases: vec![],
            dir: None,
            force: false,
        };
        create_note(dir.path(), &opts).unwrap();
        assert!(create_note(dir.path(), &opts).is_err());
    }
}
