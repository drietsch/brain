# A functional brain, not a structural one

Status: accepted

## Context

The system is called brain, and the analogy is productive — but there are
two ways to take it. Structural mimicry (simulated neurons, weights,
spreading activation as storage) would trade away exactly what the
substrate is for: content-addressed identity, provenance, deterministic
queries, replication by set union. The associative, fuzzy, pattern-matching
layer of this system already exists — it is the LLM agent operating the
CLI. Brain's job is to be the part biological memory is bad at.

## Decision

Complete the analogy by *function*, and confine fuzziness to the derived
layer:

| Organ | Mechanism |
|---|---|
| Senses | observers: files, symbols, tests, protocols, git |
| Reflexes | git hooks (fail-open, never a gate) |
| Long-term memory | the immutable graph, timelines |
| Learning | graph-defined capture rules |
| Attention | `brain attend` (crates/brain-observe/src/attention.rs) — salience ranked from churn, blast radius, missing tests, failing protocols, stale/nonconforming docs, incoherent features; computed at query time, never stored |
| Consolidation | `brain sleep` (crates/brain-observe/src/sleep.rs) — distills activity since the last sleep into per-file `memory` digests and a repo `session_summary`; adds summaries, never removes detail |
| Association | `brain related` (crates/brain-observe/src/assoc.rs) — co-change, co-mention, and shared-import signals in a derived, disposable index at the systems-of-query seam |

Neurons are rejected: the agent is the neural layer; the graph is the
ground truth it thinks against.

## Consequences

- Sessions start with `brain attend` (one ranked answer to "what matters
  now") and end with `brain sleep` (the next session orients from the
  consolidated narrative instead of raw history).
- Associative recall can grow richer signals (even embeddings) without
  ever polluting ground truth — soft indexes are rebuildable and
  deletable by contract.
- Salience weights are integers and deliberately simple; tuning them is
  cheap, and because salience is never stored, changing the formula
  rewrites nothing.
