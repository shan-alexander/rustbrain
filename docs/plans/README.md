---
node_type: plan
tags: [plan, index]
status: backlog
---
# Plans / roadmaps / tasklists

Use **`node_type: plan`** (aliases: roadmap, backlog, todo, tasklist) for prioritization
and work queues — not ship history (that is root **`CHANGELOG.md`** → hub `changelog`).

**All of this is optional.** If you only use concepts/ADRs, rustbrain still works.

### Status (indexed densely on `sync`)

| Token | Meaning |
|-------|---------|
| `backlog` | not started |
| `in_progress` | active work |
| `qa` | review / testing |
| `done` | finished |
| `cancelled` | abandoned |
| `blocked` | stuck / on hold / reopened (`undone` still accepted as alias) |

Set overall with frontmatter `status: in_progress` and/or checkboxes:

- `- [ ]` backlog · `- [/]` in progress · `- [x]` done · `- [~]` cancelled · `- [?]` qa · `- [!]` blocked

```bash
rustbrain note new --type plan --title "Q3 platform roadmap"
# edit checkboxes / status, then:
rustbrain sync
rustbrain query "status:in_progress" --type plan --scores
rustbrain context "open plan tasks"
```

Root hubs (if present): `ROADMAP.md` → id `roadmap`, `BACKLOG.md` → id `backlog`.
