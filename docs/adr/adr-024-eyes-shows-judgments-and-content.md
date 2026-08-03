# Eyes shows judgments and content, not the graph

Status: accepted

## Context

ADR-023 established what Eyes may do: read-only, loopback, purpose-built
projections, no second model of the application. It did not say what Eyes is
*for*, so the first implementation answered a different question — "does every
surface of `design-draft/` have a graph-backed equivalent?" — recorded as a
parity contract and pursued to completion.

The result covered everything and explained nothing. Measured against the live
`twin/self` store it showed `hub 29`, `churn 2 (0 recent)`, `cursor 5.934`,
`Informational staleness 20`, `content identity e65b66e…`; it dedicated the top
of the home screen to "No approvals are modeled"; it rendered 180 rows of
"file changed" as History; it listed twenty-three architecture decisions as
table rows with a freshness column and no way to read one; and its "MRI" placed
1,297 nodes on a golden-angle spiral that ignored edges entirely, then dropped
75% of them in the browser. Meanwhile the server called none of
`coherence::check`, `features::evaluate`, `lifecycle::of`, `kinds::registry`,
`fitness`, or `sleep::delta_since` — so the graph's actual product was the one
thing not on screen.

## Decision

**Eyes reads judgments and content. It does not browse structure.**

1. **Judgments, in sentences.** Everything the graph concludes reads as
   *thing — judgment — because reason*, with the evidence one click away and,
   since Eyes never writes, the exact command that resolves it. Concerns come
   from `coherence::check`, warn-level staleness with its `changed` list,
   failing protocols, unfinished governed changes, and features claiming more
   than they can show.

2. **Content is read, not tabulated.** A decision is a document with a title, a
   status, what it governs and what replaced it — not a filename in a
   freshness column. Library shelves present each kind in the shape that kind
   deserves: a reading list for decisions, a coverage strip for features,
   results and flake history for tests, the kind registry itself (with
   `fitness` verdicts) for concepts.

3. **Geometry must carry meaning, or there is no drawing.** A picture of a
   thousand nodes communicates only "it is complicated". The Map aggregates
   files into the modules a developer already thinks in, stacks them by
   dependency direction, and colours them by one question at a time. The
   neighbourhood around a single thing is laid out so position is the message:
   what it uses on the left, what depends on it on the right. Both are bounded
   by construction — never more than a few dozen elements.

4. **One voice, server-side.** `brain-eyes/src/say.rs` turns facts into
   sentences; the browser renders text it was given and never composes a
   status model (ADR-023's rule, now enforceable in one place). Every number
   carries its unit *and* its consequence. Machine identity — hashes, stable
   ids, event cursors, relation predicates — lives under a details disclosure,
   never in a headline. A test asserts the deny-list against every human
   surface.

5. **Absence is silence.** Concepts the graph does not model produce no panels,
   no disabled controls and no explanations. The parity contract required the
   opposite and is deleted.

6. **Never re-derive what the workspace already computes.** Every view calls
   the existing `brain_observe` and `cortex` functions through one held index.
   Results that are pure functions of the graph (insights, attention,
   coherence, the kind registry, fitness, the event scan) are computed once per
   graph version and shared; freshness is a `stat` on the append-only event
   log. This is why a page that cost ~9 full passes over the store now answers
   in under 250 ms.

Liveness stays a cheap cursor poll rather than Server-Sent Events: the check is
a `stat` behind a read lock, and holding a streaming connection per tab would
occupy a worker thread for a local tool that changes on commit.

## Consequences

- Eyes is legible to a developer who has never read an ADR — that is now a
  tested property, not an aspiration.
- The design draft keeps its rightful role: a source of *concepts* (body-first
  dossiers, why-panels, proof chains, lifecycle-as-space, glyph vocabulary,
  coverage strips), never an information architecture to reproduce.
- New entity kinds appear in the Library without code changes, because shelves
  come from the kind registry.
- Adding a phrase means editing `say.rs`, not a template — and any jargon that
  escapes there fails the build.
- The one voice lives in `crates/brain-eyes/src/say.rs`; the per-version caches in `crates/brain-eyes/src/state.rs`.
