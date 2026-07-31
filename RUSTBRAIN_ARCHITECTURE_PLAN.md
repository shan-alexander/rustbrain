# `rustbrain`: Project Architecture & Implementation Plan

> **Project Name**: `rustbrain`  
> **Vision**: A lightweight, project-scoped, Rust-first 2nd-brain library and CLI engine that equips software repositories with a queryable, graph-connected, AST-aware knowledge mesh for software engineers and AI agents.

---

## 1. Executive Summary

`rustbrain` solves a critical pain point in modern software engineering: documentation in repositories is often scattered across informal `docs/` directories, disconnected from code symbols, difficult to query, and invisible to AI agent mental maps.

`rustbrain` provides:
1. **Zero Lock-In Markdown Core**: Human engineers write and edit standard Markdown notes with YAML frontmatter and WikiLinks (`[[Note]]`).
2. **Code AST Anchoring**: Notes can anchor directly to codebase entities (`symbol:crate::module::Struct`) parsed via `tree-sitter`.
3. **Sub-Millisecond AI Agent Queries**: High-speed zero-copy `mmap` binary cache (`memmap2`) with SIMD-accelerated vector search and Compressed Sparse Row (CSR) graph traversal ($< 1\text{ ms}$ context generation).
4. **Zero-Copy JSON Mutations (`jshift`)**: Ultra-fast in-place JSON field updates without `serde_json` allocation/re-serialization cycles.
5. **Obsidian Interoperability**: Native, two-way Obsidian vault import/export without third-party crate vulnerabilities.
6. **Decoupled Portability**: Extract and transfer knowledge brains across repositories without breaking AST links.

---

## 2. Systems Architecture & Tiered Engine Design

`rustbrain` uses a **4-tier architecture** balancing human readability, transactional data integrity, and extreme agent query performance:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                     TIER 1: HUMAN EDITING LAYER                         │
│   Plain Markdown Files (.md) + YAML Frontmatter + Obsidian WikiLinks    │
└────────────────────────────────────┬────────────────────────────────────┘
                                     │ (notify watcher / sync)
                                     ▼
┌─────────────────────────────────────────────────────────────────────────┐
│              TIER 2: TRANSACTIONAL MASTER & AST ENGINE                  │
│   - SQLite Master Database (`.brain/db.sqlite`)                         │
│   - SQLite FTS5 Full-Text Keyword Index (BM25)                          │
│   - Tree-Sitter AST Incremental Code Parser                             │
└──────────────────┬──────────────────────────────────┬───────────────────┘
                   │                                  │
                   │ (jshift zero-copy mutation)      │ (mmap CSR compiler)
                   ▼                                  ▼
┌──────────────────────────────────────┐  ┌───────────────────────────────┐
│   TIER 3: STREAM & IPC MUTATION      │  │  TIER 4: ZERO-COPY MMAP CACHE │
│   - `jshift` In-Place JSONL Mutator  │  │  - `memmap2` CSR Graph Matrix │
│   - Fast Export Manifest Assembly    │  │  - SIMD Vector Dot Products   │
└──────────────────────────────────────┘  └───────────────┬───────────────┘
                                                          │
                                                          ▼
                                          ┌───────────────────────────────┐
                                          │      AI AGENT / CLI READ      │
                                          │   Sub-millisecond Context     │
                                          └───────────────────────────────┘
```

---

## 3. Data Model & Storage Specifications

### 3.1 SQLite Master Database Schema (`.brain/db.sqlite`)

```sql
-- Nodes (Concept Notes, Code Symbols, External Crates)
CREATE TABLE nodes (
    id TEXT PRIMARY KEY,               -- UUID or normalized string slug
    node_type TEXT NOT NULL,           -- 'goal', 'adr', 'alternative', 'concept', 'symbol', 'reference', 'edge_case'
    title TEXT NOT NULL,
    file_path TEXT,                    -- Relative file path (if concept/doc)
    symbol_hash INTEGER,               -- 64-bit BLAKE3 signature (if code symbol)
    summary TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Weighted Directed Relationship Graph
CREATE TABLE edges (
    source_id TEXT NOT NULL,
    target_id TEXT NOT NULL,
    relation_type TEXT NOT NULL,       -- 'implements', 'depends_on', 'relates_to', 'blocks'
    weight REAL NOT NULL DEFAULT 1.0,  -- Weight in range [0.0, 1.0]
    decay_rate REAL DEFAULT 0.0,       -- Optional temporal decay
    created_at INTEGER NOT NULL,
    PRIMARY KEY (source_id, target_id, relation_type),
    FOREIGN KEY (source_id) REFERENCES nodes(id) ON DELETE CASCADE,
    FOREIGN KEY (target_id) REFERENCES nodes(id) ON DELETE CASCADE
);

-- AST Code Symbol Anchors
CREATE TABLE symbol_anchors (
    symbol_hash INTEGER PRIMARY KEY,   -- BLAKE3(crate + module + signature)
    crate_name TEXT NOT NULL,
    module_path TEXT NOT NULL,
    symbol_name TEXT NOT NULL,
    file_path TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    doc_comment TEXT
);

-- Node Tags
CREATE TABLE node_tags (
    node_id TEXT NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (node_id, tag),
    FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
);

-- SQLite FTS5 BM25 Search Table
CREATE VIRTUAL TABLE node_fts USING fts5(
    node_id UNINDEXED,
    title,
    content,
    tags
);
```

### 3.2 Compressed Sparse Row (CSR) Zero-Copy Mmap Layout (`.brain/graph.mmap`)

The `.brain/graph.mmap` file is compiled from SQLite and mapped via `memmap2`. It enables $O(1)$ graph traversal and SIMD vector dot products with **zero allocations**:

```
[Header: 64 Bytes]
  ├─ Magic Bytes: "RUSTBRAIN" (8B)
  ├─ Version: u32 (4B)
  ├─ Node Count N: u32 (4B)
  ├─ Edge Count E: u32 (4B)
  ├─ Vector Dim D: u32 (4B)
  └─ Padding/Reserved (40B)

[Node Symbol Hash Index: N * 8 Bytes]
  └─ [u64, u64, ...] sorted symbol hashes for binary search lookup

[CSR Row Offsets: (N + 1) * 4 Bytes]
  └─ [u32, u32, ...] start index into Target Array for node i

[CSR Edge Targets: E * 4 Bytes]
  └─ [u32, u32, ...] target node indices

[CSR Edge Weights: E * 4 Bytes]
  └─ [f32, f32, ...] pre-computed edge weight values

[SIMD Vector Matrix: N * D * 4 Bytes]
  └─ 64-byte aligned f32 array for SIMD (AVX-512 / ARM NEON) cosine similarity
```

---

## 4. Decoupled Knowledge Portability & Obsidian Interoperability

### 4.1 Decoupled Export Layer
To transfer a "brain" between projects without breaking AST dependencies, `rustbrain` separates data into two layers:
* **Layer A (Portable Concept Core)**: Note text, concept nodes, weighted graph relationships, vector embeddings, and tags.
* **Layer B (Repo-Local AST Overlay)**: Local source code line anchors (`src/main.rs#L42`) and specific crate bindings (`symbol:crate::foo::Bar`).

During export (`rustbrain export --decouple-ast`), Layer B is stripped or placed in an optional sidecar manifest. The resulting `.brainbundle` (or Markdown directory) can be imported cleanly into any project or Obsidian vault.

### 4.2 Two-Way Obsidian Integration Engine
Using `pulldown-cmark`, `rustbrain` implements native bidirectional Obsidian sync:
* **WikiLink Extraction**: Parses `[[Note Name#Section|Display Text]]` into weighted edges (`relates_to`).
* **Frontmatter Parsing**: Reads and updates standard Obsidian YAML frontmatter (`tags: [...]`, `aliases: [...]`).
* **Obsidian Canvas Support**: Reads `.canvas` JSON structures, translating node layouts into graph relationships.

---

## 5. Crate Architecture & Module Hierarchy

`rustbrain` is structured as a modular Cargo workspace:

```
rustbrain/
├── Cargo.toml                   # Workspace manifest
├── crates/
│   ├── rustbrain-core/          # Core graph, storage, SQLite & mmap engines
│   │   ├── src/
│   │   │   ├── storage/         # SQLite rusqlite driver & migrations
│   │   │   ├── mmap/            # memmap2 CSR graph & SIMD vector engine
│   │   │   ├── graph/           # Node, edge, and weighting logic
│   │   │   ├── mutator/         # jshift in-place zero-copy JSON mutations
│   │   │   └── lib.rs
│   ├── rustbrain-ast/           # Tree-Sitter code parsing & symbol hashing
│   │   ├── src/
│   │   │   ├── parser.rs        # Tree-Sitter CST parser
│   │   │   ├── symbol.rs        # BLAKE3 symbol signature generator
│   │   │   └── lib.rs
│   ├── rustbrain-obsidian/      # Native Markdown & Obsidian WikiLink engine
│   │   ├── src/
│   │   │   ├── wikilink.rs      # pulldown-cmark WikiLink parser & emitter
│   │   │   ├── frontmatter.rs   # YAML frontmatter sync
│   │   │   └── lib.rs
│   └── rustbrain-cli/           # Binary CLI & AI Agent prompt context injector
│       ├── src/
│       │   ├── main.rs
│       │   ├── commands/        # init, query, sync, export, context
│       │   └── agent_fmt.rs     # AI prompt context formatters (XML/Markdown)
```

---

## 6. Key Crate Dependencies

| Dependency | Purpose |
| :--- | :--- |
| **`rusqlite`** | Embedded transactional database with bundled SQLite + FTS5. |
| **`memmap2`** | Cross-platform zero-copy memory-mapping for sub-millisecond graph/vector reads. |
| **`tree-sitter` / `tree-sitter-rust`** | Incremental AST parsing of codebase files. |
| **`jshift`** | Zero-copy in-place JSON/JSONL byte mutations without serde allocations. |
| **`pulldown-cmark`** | High-performance Markdown and WikiLink parsing for Obsidian. |
| **`blake3`** | Ultra-fast SIMD hashing for AST symbol signatures. |
| **`clap`** | CLI argument parsing. |
| **`notify`** | File watcher for live workspace auto-syncing. |

---

## 7. API Design & Usage Examples

### 7.1 Rust Library API

```rust
use rustbrain_core::{Brain, BrainOptions, QueryFilter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Open project brain
    let brain = Brain::open("./.brain")?;

    // Sub-millisecond context retrieval for AI Agent
    let context = brain.context_for_prompt(
        "How does our double-entry accounting engine handle raft consensus?",
        /* max_tokens */ 2048,
    )?;

    println!("AI Context:\n{}", context.to_markdown());
    Ok(())
}
```

### 7.2 CLI Commands

```bash
# Initialize rustbrain in a repository
rustbrain init

# Query notes and code symbols (Hybrid BM25 + Vector + Graph)
rustbrain query "raft consensus"

# Inject fast context for AI Agent prompts
rustbrain context --for-prompt "explain log compaction" --max-tokens 1500

# Export brain into a portable, decoupled bundle
rustbrain export --out ./my_brain.brainbundle --decouple-ast

# Sync bidirectional changes with an Obsidian vault
rustbrain sync --obsidian-vault ~/Obsidian/MyProject
```

---

## 8. Phased Development Roadmap

- [x] **Phase 1: Workspace & Core Storage Engine**
  - Scaffold Cargo workspace (`rustbrain-core`, `rustbrain-ast`, `rustbrain-obsidian`, `rustbrain-cli`).
  - Implement SQLite migrations, node/edge schema, and FTS5 search.
  - Implement `jshift` integration for JSONL streaming mutations.
- [x] **Phase 2: Tree-Sitter AST & Symbol Hashing**
  - Implement `rustbrain-ast` using `tree-sitter-rust`.
  - Build BLAKE3 symbol signature hashing for location-independent symbol anchors.
- [x] **Phase 3: Zero-Copy Mmap CSR & SIMD Engine**
  - Build `.brain/graph.mmap` compiler (`CsrCompiler`).
  - Implement zero-copy CSR traversal and SIMD vector dot products using `memmap2`.
- [x] **Phase 4: Obsidian Interoperability**
  - Implement `rustbrain-obsidian` using `pulldown-cmark`.
  - Build bidirectional WikiLink (`[[...]]`), YAML frontmatter sync, and Obsidian Canvas (`.canvas`) JSON parser.
- [x] **Phase 5: CLI Engine & AI Context Injector**
  - Build `rustbrain-cli` with `init`, `query`, `context`, `export`, and `sync` commands.
  - Implement global developer registry (`~/.config/rustbrain/registry.json`), `--all-workspaces` queries, portable `.brainbundle` exporter, and prompt context injectors (XML/Markdown).

---

## 9. Evaluated Alternatives & Methodological Design Decisions

### 9.1 `.kg` File Format Assessment
* **Analysis**: `.kg` is a custom line-oriented text DSL for knowledge graphs. While human-readable and Git-diffable, adopting `.kg` creates **custom format lock-in** and requires custom text parsing on every query.
* **Decision**: **REJECTED as primary storage.** `rustbrain` retains standard Markdown (`.md`) with YAML frontmatter and WikiLinks (`[[Note]]`). This delivers 100% of `.kg`'s readability with **zero ecosystem lock-in** (fully compatible with Obsidian, VS Code, and GitHub). `rustbrain`'s internal `.brain/graph.mmap` cache provides $O(1)$ zero-copy reads that text-based `.kg` parsers cannot match.

### 9.2 Zettelkasten Method Integration Strategy
* **Analysis**: The Zettelkasten framework promotes atomic notes, dense bidirectional links, unique immutable IDs, and hub/index notes.
  * **Formal Integration (Rigid Zettelkasten)**: *Rejected.* Forcing strict academic timestamp IDs or mandatory atomic note splitting on software developers creates unnecessary friction.
  * **Informal / Pragmatic Integration**: *Adopted.* `rustbrain` incorporates key Zettelkasten mechanics directly into the core engine without constraining developer workflow:
    1. **Automatic Bidirectional Backlinks**: If Note A links to Note B (`[[Note B]]`), the graph engine automatically computes the implicit backlink and boosts edge centrality.
    2. **Atomic Concept Nodes**: Encourages short, single-topic concept notes (`docs/concepts/*.md`) alongside broader decision records.
    3. **First-Class 7-Node Taxonomy**: Standardizes notes into 7 first-class domain types:
       - `goal`: Overarching project goals, non-goals, SLA targets, and scope boundaries.
       - `adr`: Architectural Decision Records (decisions made to achieve goals).
       - `alternative`: Alternatives considered (in-depth benchmark comparisons, discarded architectural options, competitor tools, and tradeoffs).
       - `concept`: Pure atomic technical/domain concepts (Zettelkasten notes).
       - `symbol`: Code AST entities (functions, structs, traits, modules) extracted via Tree-Sitter.
       - `reference`: External dependency nuances, crate quirks, API caveats, and doc links.
       - `edge_case`: Known bugs, concurrency traps, memory aliasing quirks, and platform gotchas.
    4. **Hub Node Auto-Discovery**: Automatically computes and ranks high-degree hub/index nodes for high-level AI context summaries.

---

## 10. Universal Multi-Brain Topology: MainBrain & Sibling SubBrains

### 10.1 Two-Tier Domain-Neutral Hierarchy
To ensure `rustbrain` is universally applicable—whether for Rust monorepos, TypeScript projects, Go microservices, or personal multi-topic knowledge vaults—we adopt a clean **2-Tier Hierarchy**:

```
                              ┌────────────────────────┐
                              │       MainBrain        │
                              │   (Workspace Root)     │
                              └───────────┬────────────┘
                                          │
                  ┌───────────────────────┼───────────────────────┐
                  ▼                       ▼                       ▼
          ┌──────────────┐        ┌──────────────┐        ┌──────────────┐
          │  SubBrain A  │        │  SubBrain B  │        │  SubBrain C  │
          │   ("core")   │        │  ("biology") │        │   ("math")   │
          └──────────────┘        └──────────────┘        └──────────────┘
```

1. **`MainBrain` (Workspace Root)**:
   - Primary knowledge store for global project goals, high-level ADRs, non-goals, and system-wide architecture.
2. **`SubBrain` (Flat Sibling Scopes)**:
   - Named domain scopes mapped to subdirectories containing a `docs/` folder or registered subbrain manifest (e.g. `subbrain: core`, `subbrain: biology`).
   - Every node (Goal, ADR, Concept, Symbol, EdgeCase) has **one canonical owner scope** (`mainbrain` or a named `subbrain`).

### 10.2 Architectural Rules & Design Decisions

#### **Rule 1: Flat 1-Tier Sibling SubBrains (No Arbitrary Deep Nesting)**
* Arbitrary deeply nested subbrains (`main/biology/cellular/mitosis/docs/`) introduce high user cognitive friction and complex query path resolution.
* **Decision**: All SubBrains are flat siblings under the `MainBrain`. Deep physical folder structures (e.g. `biology/cellular/docs/`) map to a clean flat subbrain identifier (e.g. `subbrain: cellular`).

#### **Rule 2: Cross-Scope Graph Links**
* Notes have one primary owner scope, but **WikiLinks (`[[Link]]`) cross subbrain boundaries seamlessly**.
* If Note A in `subbrain: biology` links to Note B in `subbrain: chemistry`, the weighted graph edge ($A \rightarrow B$) spans across subbrains.
* When querying a specific scope (`rustbrain query --scope biology`), `rustbrain` retrieves `biology` nodes plus any directly linked `MainBrain` or cross-scope neighbor nodes.

### 10.3 Multi-Level CLI Query Scoping

```bash
# 1. Query entire project workspace (MainBrain + all sibling SubBrains)
rustbrain query "mitosis ATP"

# 2. Scope query to a specific SubBrain
rustbrain query "mitosis ATP" --scope biology

# 3. Query across ALL registered project workspaces on developer machine
rustbrain query "mitosis ATP" --all-workspaces
```

### 10.4 Registry Hierarchy
1. **Workspace Manifest (`.brain/workspace.json`)**: Auto-discovers sub-directories and local SubBrain scopes.
2. **Global Developer Registry (`~/.config/rustbrain/registry.json`)**: Registers all workspaces on the machine for `--all-workspaces` queries.

### 10.5 Obsidian Vault Integration & Remote Mobile Workflow
* **Vault Mapping**:
  - **Single Vault per Workspace (Default)**: Open the project root `docs/` folder in Obsidian.
  - **Master Vault**: Open `~/Obsidian/DeveloperBrain` containing project subdirectories.
* **Mobile Workflow**: Notes created/edited on mobile in Obsidian Mobile sync via **Git**. Upon `git pull` or file change, `rustbrain`'s watcher (`notify`) automatically re-indexes Markdown files and re-bakes the `.mmap` cache in **< 50ms**.

---

## 11. Phase 1 Release Objective (v0.1.0)

The primary goal of **Phase 1** is to deliver a robust, fully working **v0.1.0 Rust crate (`rustbrain`)** and CLI binary that software engineers can immediately integrate into their repositories to build project-scoped 2nd brains.




