---
tags: [adr, bootstrap, agents, skill]
node_type: adr
aliases: [skill-md-bootstrap, thin-agents-md, v0.3.23]
---
# Bootstrap SKILL.md and thin non-clobbering AGENTS.md

## Status

Accepted (v0.3.22 → v0.3.23)

## Context

`rustbrain bootstrap` / `setup` currently writes a **long CLI cookbook** into root `AGENTS.md` (`DEFAULT_AGENTS_MD` in `symbol:rustbrain_core::bootstrap::write_agents_md`). That file is the wrong layer for a flag dump:

- Host repos often **already have** an `AGENTS.md` (org rules, other tools). Today we **skip** it unless `--force`, and `--force` **replaces the whole file**. Both are wrong for a mature host: skip leaves rustbrain invisible; `--force` destroys the host's agent contract.
- Agentic harnesses (Grok, Claude Code, Cursor, Codex, Gemini, …) load **skills** from `.<harness>/skills/<name>/SKILL.md`, not from `AGENTS.md`. A cookbook that never lands in that path is easy for agents to miss.
- This repo now has a real `SKILL.md`: the **agent operating loop plus CLI cookbook**. Root `AGENTS.md` should shrink to a **mandate** (“use rustbrain”), not duplicate the book.

Pre-0.3.23 behavior (must change):

| Existing `AGENTS.md` | `--force` | Result |
|----------------------|-----------|--------|
| missing | n/a | write full builtin cookbook |
| present | no | skip |
| present | yes | **overwrite** with cookbook |

`docs/AGENTS.md` is already a short docs-local mandate. It stays that role.

## Decision

Ship **0.3.23** with three coupled changes.

### 1. Thin builtin `AGENTS.md`

`default_agents_md_template()` becomes a short mandate: what rustbrain is, `PATH`, `context` / `query` / `sync`, `note new` scaffold-then-edit, and a pointer at **SKILL.md** for the cookbook. No CLI flag tables, no multi-brain book.

Marker (unchanged prefix, still how we recognize “ours”):

```text
<!-- rustbrain-agents-md:
```

Header text must say this file is **rustbrain-owned** and will be **refreshed on bootstrap**. Hand-written files must **not** keep this marker (or they will be replaced).

### 2. Never clobber a custom `AGENTS.md` (even with `--force`)

`symbol:write_agents_md` policy:

| Existing file | Detection | Action |
|---------------|-----------|--------|
| missing | — | write the resolved template (builtin thin, or `--agents-template` / env / workspace template) |
| present + starts with / contains `<!-- rustbrain-agents-md:` | rustbrain-owned | **replace** with the resolved template (no `--force` required — migrates fat 0.3.22 cookbooks) |
| present, no rustbrain marker, no append marker | custom | **append** `AGENTS_MD_APPEND_SECTION` |
| present, already has append marker | custom, done | skip (idempotent) |

`--force` does **not** overwrite a custom `AGENTS.md`. It still regenerates `generated: true` files, `.rustbrainignore`, module map, etc.

Append section (idempotent via HTML comments):

```markdown
<!-- rustbrain-agents-section: start -->
## rustbrain

This project uses [rustbrain](https://github.com/shan-alexander/rustbrain).
Before claiming decisions, status, or history:

    rustbrain context "why <decision> / how <feature> works"
    rustbrain query "<topic>" --scores
    rustbrain sync

Cookbook: `SKILL.md` or `.<harness>/skills/rustbrain/SKILL.md`.
<!-- rustbrain-agents-section: end -->
```

Skip append when the file already contains `<!-- rustbrain-agents-section:`.

`--no-agents-md` still skips all root `AGENTS.md` writes. `--agents-template` still supplies **new / rustbrain-owned** body; it is **not** applied on top of a custom file (custom → append only).

Interactive prompt default: **yes** even if `AGENTS.md` exists (append or refresh is safe). Wording: mandate, not “cookbook”.

### 3. Install `SKILL.md` into agent harnesses

Canonical body: repo-root `SKILL.md`, **embedded** in rustbrain-core as `src/templates/SKILL.md` (`include_str!`) so `cargo publish -p rustbrain-core` does not depend on a path outside the crate. Workspace test: embedded bytes == repo-root `SKILL.md` when that file exists.

Harness dirs (directory must **already exist**; we never invent a harness):

`.grok` · `.claude` · `.cursor` · `.agents` · `.codex` · `.gemini` · `.ai`

For **each** existing dir, write:

```text
.<harness>/skills/rustbrain/SKILL.md
```

If **none** of those dirs exist: write **root** `SKILL.md` only (so a repo with no harness still gets a cookbook next to `AGENTS.md`). We do **not** also write root `SKILL.md` when at least one harness was detected.

Per destination:

| Destination | Action |
|-------------|--------|
| missing | create (including `skills/rustbrain/` parents) |
| exists and YAML `name: rustbrain` in the first ~40 lines | **refresh** (ours) |
| exists otherwise | **skip** (unknown skill / hand-written). Not even `--force` |

`--no-skill-md` / `BootstrapOptions.write_skill_md = Some(false)` skips all skill installs. Interactive: ask to install; `--yes` defaults true.

`docs/AGENTS.md`: **no policy change** (still `write_if_allowed` + `--force`). One-line honesty: cookbook pointer may mention `SKILL.md`; do not thin or append-if-custom that file in 0.3.23.

## Alternatives considered

- **Keep fat `AGENTS.md` as cookbook** — rejected. Duplicates `SKILL.md`; harnesses do not load it as a skill; blows the token window of agents that slurp `AGENTS.md`.
- **Skip existing `AGENTS.md` (today)** — rejected. rustbrain stays invisible in repos that already have agent rules.
- **`--force` still overwrites custom `AGENTS.md`** — rejected. Host agent contracts are more valuable than a clean replace. Operators who want a full replace delete the file or strip the custom body themselves.
- **Create `.grok/` when no harness exists** — rejected. Inventing a vendor dir is rude. Root `SKILL.md` is the fallback.
- **Install into one preferred harness only** — rejected. A repo with both `.grok/` and `.claude/` should teach both agents.
- **`--skill-template PATH`** — deferred. One embedded skill is enough for 0.3.23; agents-template already covers org-specific `AGENTS.md`.
- **Doctor finding `no_skill_md`** — deferred. Missing skill is optional; `no_agents_md` already covers a missing mandate.
- **Home-level `~/.grok/skills/…`** — out of scope. Bootstrap is workspace-local.

## Consequences

- **Upgrade:** next `bootstrap` / `setup` on a rustbrain-generated `AGENTS.md` (fat cookbook with the marker) **replaces** it with the thin mandate. Cookbook moves to `SKILL.md`. Repos that edited the generated file **but kept the marker** will lose those edits — documented in the new header (“remove this header to keep a hand-written file”).
- **Custom `AGENTS.md`:** gains a small rustbrain section; never replaced. Re-runs are idempotent.
- **`--force` meaning narrows** for `AGENTS.md` / skill files. Docs and `--help` must stop saying “`--force` overwrites AGENTS.md”.
- **Library:** `BootstrapOptions` grows `write_skill_md: Option<bool>`. Callers using struct literals must set it (or `..Default::default()`).
- **Windows MSVC debug:** clap’s `Parser` plus the embedded skill already exceeded the 1 MiB default stack (`STATUS_STACK_OVERFLOW`). `.cargo/config.toml` sets `/STACK:8MiB` for Windows MSVC targets.
- **Publish:** bump workspace `0.3.22` → `0.3.23`; CHANGELOG section; pin examples in READMEs / `docs/CLI.md`.
- **This repo:** replace root `AGENTS.md` with the thin template; keep root `SKILL.md` as the human-editable cookbook (copied into `crates/rustbrain-core/src/templates/SKILL.md`).

## Implementation notes

- `symbol:rustbrain_core::bootstrap::write_agents_md`
- `symbol:rustbrain_core::bootstrap::default_agents_md_template`
- `symbol:rustbrain_core::bootstrap::BootstrapOptions`
- CLI: `setup` / `bootstrap` `--no-skill-md` next to `--no-agents-md` (`crates/rustbrain-cli/src/main.rs`)
- Tests in `bootstrap.rs`: create / refresh-ours / append / idempotent append / `--force` does not clobber custom / skill into `.grok` and `.claude` / no harness → root / skip foreign skill / `--no-skill-md`
- Do not invent ADR history beyond this note. Do not change `docs/AGENTS.md` policy.

## Related

- `docs/AGENTS.md` (docs-local mandate; policy unchanged in this ADR)
- [[changelog]]
- symbol:write_agents_md
- symbol:default_agents_md_template
- symbol:BootstrapOptions
