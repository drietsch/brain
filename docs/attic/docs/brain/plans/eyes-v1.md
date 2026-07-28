<!-- brain:projection kind=plan slug=eyes-v1 — GENERATED, READ-ONLY. Edit via: brain artifact edit twin/self plan eyes-v1 --file <md> -->

# Eyes v1: Human projection of the graph

Status: active

## Outcome

Running `brain eyes --prefix twin/self` opens a read-only local application
whose Now and Explore views are populated from the current graph rather than
demo fixtures.

## Invariants

- The store and event log are the only systems of record.
- Cortex is the query backend and remains disposable.
- No raw store paths, object files, or frontend-maintained truth cross the
  HTTP boundary.
- Every snapshot carries namespace HEAD and the put-history cursor.
- V1 binds to loopback and exposes no mutation endpoint.
- Retracted relations and absent entities do not appear as live structure.
- One installed `brain` binary contains the server and frontend.

## Phase 1 — Projection API

1. Add an internal `brain-eyes` crate with serializable DTOs for snapshot
   identity, summary counts, attention, entities, and relations.
2. Build a fresh Cortex view per API request so event-log delta replay keeps
   responses current without a second cache.
3. Expose read-only endpoints for overview, graph exploration, entity detail,
   and cursor status.
4. Cover projection behavior with temporary-store tests, including relation
   retraction and run-twice read-only guarantees.

## Phase 2 — Embedded human interface

1. Serve HTML, CSS, and JavaScript through `include_str!`; require no external
   CDN or Node build at runtime.
2. Implement three-view navigation with Now and Explore functional first and
   History as an honest next-slice empty state.
3. Use the design draft's IBM Plex instrument-panel vocabulary. The signature
   element is an SVG attention field: real graph nodes sized by relation
   degree and lit by salience.
4. Add search, kind lenses, selection, a universal inspector, keyboard focus,
   responsive behavior, and reduced-motion support.
5. Poll the lightweight cursor endpoint and refresh derived views only when
   the graph advances.

## Phase 3 — CLI and verification

1. Add `brain eyes [--prefix P] [--bind IP] [--port N]` to the command parser
   and manual. Defaults: `twin/self`, `127.0.0.1`, and an available port.
2. Print the exact URL and snapshot identity on startup; do not launch a
   browser unless explicitly requested by a future flag.
3. Run the full Rust suite, start Eyes against this repository's `.brain`
   store, exercise the API, and inspect the rendered UI at desktop and narrow
   widths.
4. Refresh the twin so the ADR, crate, symbols, relations, and plan projection
   become graph citizens.

## Deferred

- Server-Sent Events beyond cursor invalidation.
- Historical comparison and replay controls.
- Approval, action, or reconciliation writes.
- Remote binding, authentication, and multi-store selection.
- Agent-session views until sessions are first-class graph data.
