# Placement policy: where each artifact kind's truth lives

Status: accepted

## Context

Artifacts piled up in repos with no rule for where anything belonged, and
the same information lived twice — once in the graph, once in a file —
with nothing declaring which copy was authoritative. Binary assets
(screenshots, HTML templates) were the worst case: hashed but anonymous,
unable to rot visibly, impossible to tidy when their purpose ended.

## Decision

Placement is a per-kind policy in the registry (ADR-017), three values:

- **file_first** — the file is the authoring surface; the twin captures
  it (code, narrative docs, runbooks, ADRs, assets, prototypes). `home`
  globs say where such files belong; tidy flags strays.
- **graph_first** — the graph is authored directly (`brain artifact
  new|edit`); the file under `project_to` is a read-only render
  (ADR-019). Plans and task lists ship this way.
- **projection** — never authored at all: a rendered query (the
  capability matrix), regenerated on demand.

Projections render under `docs/brain/<kind>/<slug>.md` — git-visible on
purpose: agents grep repo files, reviewers see artifact changes in PRs,
and renders are deterministic so diffs appear only when meaning changed.

Assets (crates/brain-observe/src/assets.rs): bytes stay in files (the
graph's canonical JSON carries no blobs — a `.brain/blobs` sidecar stays
explicitly deferred until a real need appears); the graph holds the typed
entity — subtype, owner via `attached_to`, and declared `depicts` targets,
stated at capture because media cannot be substring-scanned. An asset
whose depicted target changed after its bytes were captured is stale on
every ordinary surface; an asset whose owner concluded is tidy's
`legacy-asset`. Prototypes are a seeded kind (prototypes/**/README.md)
with a lifecycle, so spikes get formally concluded and archived instead
of lingering.

## Consequences

- "Where should this go?" has a queryable answer per kind, and `brain
  tidy` enforces it as governed, revertible moves.
- Screenshots and HTML templates finally have a staleness story — the
  cost is declaring `--depicts` at capture, one flag.
- Graph-first kinds cannot format-drift between agents: the file is a
  render, and the render is deterministic.
