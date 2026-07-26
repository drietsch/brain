# Capture rules live in the graph

Status: accepted

## Context

Auto-capturing an artifact family used to require a Rust parser in the
binary (crates/brain-observe/src/docs.rs, agents.rs): the conventions were
code. But the data model is schema-open — entity kinds are strings, and
the recording core (record_entity_doc in crates/brain-observe/src/twin.rs)
is generic. The only thing keeping brain from learning a new artifact type
at runtime was that path conventions and field extraction lived in the
binary instead of the graph.

## Decision

Templates may declare, as observations, how their kind is captured
(crates/brain-observe/src/templates.rs):

- `capture` — glob patterns (`*` within a segment, `**` across, `?` one
  character; no dependency) naming the paths that are artifacts of the
  template's `applies_to` kind.
- `fields` — a tiny DSL (`prop=extractor[:arg]`) selecting from a fixed
  extractor vocabulary: `heading`, `line[:Key]`, `frontmatter[:key]`,
  `slug`. The vocabulary is code; which of it applies to what is data.

During refresh, paths not claimed by the built-in detectors (which keep
precedence) are matched against the rules; matches are recorded through
the shared core, so mentions-scan, concerns/recorded_in relations,
conformance against `requires`, staleness, and replication all apply to
taught kinds identically to built-in ones. `brain template set` defines a
type from the CLI; `brain artifact list|show` browses any kind.

## Consequences

- A store can teach itself runbooks, incidents, RFCs — and `brain pull`
  teaches every replica; the ontology travels with the data.
- Extraction stays deliberately shallow (the forgiving-parser
  philosophy): missing fields are conformance findings, not errors.
- The built-in detectors are now merely pre-taught defaults; migrating
  them to seeded capture rules is possible later without changing what
  any store contains.
