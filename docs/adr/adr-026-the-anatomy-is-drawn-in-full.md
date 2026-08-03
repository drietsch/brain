# The anatomy is drawn in full

Status: accepted
Amends: adr-024

## Context

ADR-024 clause 3 says: *"Geometry must carry meaning, or there is no
drawing. A picture of a thousand nodes communicates only 'it is
complicated'."* It removed the whole-graph view outright.

That rule was written against a specific object, and the ADR describes it
exactly: 1,297 nodes placed on a golden-angle spiral that **ignored edges
entirely**, of which the browser then **silently dropped 75%**. Both
failures were real, and neither was caused by the node count.

- The spiral carried no information. Position was an index, not a fact.
- The truncation was a lie. The screen said "the graph"; it showed a
  quarter of it and never mentioned the rest.

Reading clause 3 as "never draw everything" would forbid a class of view
on the evidence of one bad implementation. Reading it as written —
geometry must carry meaning — permits a whole-graph view and constrains
how it must be built.

## Decision

**The whole graph may be drawn, in three dimensions, provided position
means something and nothing is hidden.** This amends ADR-024 clause 3;
the rest of ADR-024 stands unchanged.

Four constraints, each answering one of the original failures:

1. **Layout is computed on the server, once per graph version, and
   cached.** The browser orbits a fixed anatomy and never runs a force
   simulation. This is what makes the briefing's "stable anatomy, moving
   activity" achievable: nothing drifts while you are reading it, so
   anything that moves on screen is a fact — a file that changed, a test
   that is failing, a session that is running.

2. **Position carries three claims, and they are checkable.** Height is
   dependency depth, taken from the same layering the Map uses, so what
   everything rests on sits at the bottom. Clusters are the categories a
   developer already thinks in and never move between versions. Within a
   module, things that reference each other are pulled together by their
   real edges. A test asserts that the crate which uses another sits
   above it.

3. **Detail resolves; it is never dropped.** Every node is in the payload
   and every node is drawn. Approaching the graph makes finer things
   larger, brighter and labelled — it does not make them exist. The
   readout says "all 1,432 drawn, 35 in focus", never "35 of 1,432 on
   screen". A test asserts that every twinned file appears in the payload
   and that the level counts sum to the node count.

4. **Labels are bounded; nodes are not.** At most a screenful of labels
   is placed, nearest first. A wall of overlapping text is not a label,
   it is noise — but suppressing a *label* is honest in a way that
   suppressing a *node* is not.

The renderer is WebGL2 written directly against the API: two draw calls,
instanced billboards against a glyph atlas painted at runtime, one line
buffer for edges. No library is fetched, because Eyes serves only itself
and ADR-023 keeps it offline.

## Consequences

- ADR-024's clause 3 now reads: geometry must carry meaning, drawings are
  aggregated *or complete*, and completeness is a promise that must be
  kept rather than quietly broken.
- The Map keeps its job. It answers "what is this made of and where is the
  risk" with a bounded, labelled, accessible diagram; the MRI answers
  "what shape is this system, and what is moving". Both read the same
  layering, so they cannot disagree about what depends on what.
- Every fact in the MRI is reachable without it — the same nodes appear in
  the Map, Artifacts, Tests and Find. The 3D view is additive, and the
  canvas says so in its accessible label.
- The layout costs about two seconds on a 1,400-node graph. It is computed
  during warm-up and shared, so no request pays for it.
- A browser without WebGL2 gets a sentence naming where the same
  information lives, not a blank canvas.
- The renderer lives in `crates/brain-eyes/assets/mri.js`, laid out server-side in `crates/brain-eyes/src/query/mri.rs`.
