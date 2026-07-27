# Artifact lifecycle is a derived judgment, not a stored fact

Status: accepted

## Context

Artifacts pile up: superseded ADRs kept listing as live decisions, finished
plans stayed stale-flagged forever, and a deleted document's entity survived
in every view. The `supersedes` relation was written but consumed by nothing.
Agents orienting in the repo could not tell the living set from history, so
"what is the real current state" required reading everything.

## Decision

A single derivation (crates/brain-observe/src/lifecycle.rs) answers "is this
artifact current?" for every doc-ish kind. Precedence, first match wins:

1. a live incoming `supersedes` edge → superseded — a structural declaration
   in a document outranks any CLI override (undo it by editing the file);
2. the latest explicit `lifecycle` observation (`active | done | abandoned |
   retired | superseded`, source `agent`, written by `brain plan
   done|abandon|reopen` and `brain artifact set-lifecycle`);
3. the latest `status` observation, when it implies a state (`done`,
   `shipped`, `superseded`, `deprecated`, ...) — plans now parse their
   `Status:` line (crates/brain-observe/src/docs.rs);
4. every `recorded_in` file deleted → retired;
5. active.

Only explicit sets are stored; the judgment itself is computed at query time
and never materialized (ADR-009). Every list, insight, staleness check, and
attention pass filters non-active artifacts by default; `--all` shows
history, tagged.

## Consequences

- A finished plan (`brain plan done`) leaves the lists, the stale report,
  and the wake orientation the moment it is concluded.
- Superseded decisions are history: hidden by default, visible with
  `--all`, and `adr show` names the successor.
- Deleting an artifact's file retires the artifact without destroying its
  timeline — the graph remembers, the views stop reporting.
- An explicit `lifecycle=active` cannot resurrect a superseded document;
  coherence flags an explicitly-active artifact whose home files are gone.
