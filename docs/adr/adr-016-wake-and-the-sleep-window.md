# Wake, and the sleep watermark as the universal recency window

Status: accepted

## Context

Orientation was fragmented across `attend`, `twin insights` (which alone
printed the session summary), `notes`, and `twin stale` — and attention had
no recency at all: churn was a lifetime count, so README.md's 41 historical
edits permanently outranked the file being worked on today. Meanwhile
insights truncated every list to five and the narration reported those
truncated counts as totals ("5 architecture decisions" in a repo with 12).

## Decision

1. **`brain wake <prefix>`** (crates/brain-observe/src/wake.rs) composes
   the whole present in one token-budgeted command: last session summary
   and its age, the delta since (shared with sleep via
   `sleep::delta_since`), failing tests, top attention, warn-stale, work
   in flight (active plans, unsettled governed changes, features short of
   their DoD), notes since sleep, and coherence findings. Nothing it shows
   is stored; everything is a query.
2. **`consolidated_until` — the sleep watermark — is the recency window
   everywhere.** Attention's churn becomes `recent*4 + lifetime.min(10)`
   where recent counts edits after the last sleep
   (crates/brain-observe/src/attention.rs): the present dominates, history
   is capped, and a store that never slept scores every edit as recent,
   which matches its reality. No clock parameter, no decay constant to
   tune — the session rhythm itself defines "now". Sleep is likewise the
   acknowledgement boundary for notes: wake shows notes newer than the
   watermark.
3. **Truncation honesty**: `Insights` returns full lists; rendering
   truncates with an explicit "showing 5 of 12" line. A truncated count
   must never pose as a total.

## Consequences

- A fresh session — any agent, any vendor — runs one command and knows
  the real current state in under forty lines.
- The attend → work → sleep rhythm becomes load-bearing: sleeping is what
  moves the recency window forward, so consolidation is rewarded with
  sharper attention the next morning.
- The narration bug class ("truncated list reported as total") is gone at
  the source: totals come from full lists.
