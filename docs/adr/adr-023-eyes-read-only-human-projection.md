# Eyes is a read-only human projection of the graph

Status: accepted

## Context

The CLI is an efficient interface for agents, but people need a continuous,
spatial view of the same graph: what changed, what deserves attention, how
entities relate, and how the system arrived at its current state. The
`design-draft/` prototype explores that experience, but it also invents
sessions, approvals, features, and health values that are not always present
in the graph.

Eyes must not become a second model of the application. The content-addressed
store and event log remain the system of record; cortex remains the
disposable system of query; attention, staleness, completion, and other
judgments stay computed at query time.

## Decision

`brain eyes` starts a small HTTP server inside the monolithic `brain` binary.
It binds to `127.0.0.1` by default and serves embedded frontend assets plus
read-only JSON projections built directly from `Store` and `Cortex`.

The server is implemented as an internal `brain-eyes` library crate so HTTP
and presentation concerns do not leak into the CLI or graph crates. It still
ships as part of the single `brain` executable.

Eyes v1 has three human views:

- **Now** — the wake delta, attention, failures, staleness, and in-flight
  graph work.
- **Explore** — live entities and typed relations with lenses, search,
  selection, and transitive blast radius.
- **History** — event and observation timelines, with as-of comparison added
  after the first vertical slice.

Every API response identifies the graph view it represents with both the
namespace `HEAD` and event-log cursor. `HEAD` alone is insufficient because
unbound observations and relations also advance the graph.

The API returns purpose-built projection DTOs and never a frontend-owned
status model. An entity body is also a purpose-built projection: text comes
from the entity's latest `content` observation at the identified graph
snapshot. Binary bodies remain file-first (ADR-018) and may be served from a
workspace-relative entity path only after canonical path containment and,
when present, `content_b3` verification. The browser supplies a stable entity
id, never a filesystem path. Raw responses are read-only, size-limited,
non-sniffing, and active text formats are served as plain text.

Live updates are cursor-based: the client can cheaply ask whether the graph
advanced, and Server-Sent Events may stream the same invalidation signal. A
change triggers re-querying derived views rather than incrementally
recreating graph semantics in JavaScript.

Eyes v1 has no mutation endpoints. If later versions expose approvals or
actions, they must enter through the existing capability and
intent/receipt boundary; an HTTP handler never writes around governance.

## Consequences

- Eyes is truthful by construction: restarting it reconstructs the same view
  from graph truth and disposable indexes.
- The UX can grow as new entity kinds appear without pretending absent
  concepts exist. Empty features or intents are honest empty states.
- The binary gains an HTTP/runtime dependency, but distribution remains one
  executable and no Node installation is required.
- Localhost-only and read-only are the initial security boundary. Remote
  binding, authentication, and write capabilities are separate decisions.
- The design draft is a visual reference, not runtime source or an
  authoritative schema.
