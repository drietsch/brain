---
name: twin
description: Orient in this codebase via the semantic twin instead of re-reading files — refresh, query structure, read past sessions' notes, record decisions and plans.
---

# The twin skill

Use the graph as persistent memory across sessions.

## Start of session

```bash
cargo build -p brain-cli -q
target/debug/brain twin refresh . --prefix twin/self   # record drift since last session
target/debug/brain twin insights twin/self             # churn, hubs, decisions, plans, notes
target/debug/brain notes twin/self                     # repo-level breadcrumbs
```

## While working

- Structure queries beat grepping for orientation:
  `brain twin symbols|imports|rdeps twin/self/<path>`.
- Which decisions cover a file: `brain adr list twin/self`, then
  `brain adr show twin/self <slug>` for the rationale and mentions.

## End of session

- `brain note twin/self "<what you learned>"` — repo-level; or note a
  specific file entity.
- Approved plan? `brain plan add <plan-file> --prefix twin/self`.
- Significant decision? Write `docs/adr/adr-NNN-<slug>.md` with a
  `Status:` line; the next refresh captures and links it.
- `brain twin refresh . --prefix twin/self` once more so the twin's last
  state matches what you leave behind.
