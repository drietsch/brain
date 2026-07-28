# Features have parts

Status: accepted

## Context

A feature was one entity with four relations: `implemented_by`,
`tested_by`, `decided_by`, `documented_in`. Done meant all four counts
exceeded zero.

That is not how anyone actually works. A real feature has a core, an API,
a user interface, tests and documentation, and each of those is separately
buildable, separately testable, and separately *unfinished*. Flattening
them into one entity means the interesting question — *which part is
holding this up?* — cannot be asked, let alone answered.

The graph had no way to say it. Verified by exhaustive search: zero
occurrences of `part_of`, `parent`, `child_of`, `composed_of` or any
synonym anywhere in the workspace. The only same-kind entity relation in
the entire system was `supersedes` (decision → decision), which is
succession, not containment. The live store held three sibling features in
a flat star around the repo entity.

## Decision

**A part of a feature is a feature, joined by `part_of`.**

1. **Child → parent, not parent → child.** `brain feature link p core
   part_of authentication` reads the way it means, and adding a part never
   rewrites the parent's edges — which is the shape an append-only store
   wants. Children are read as incoming edges, parents as outgoing.

2. **`part_of`, not `contains`.** `contains` already means file → symbol.
   It is swept by the file-lifecycle pass that retracts a deleted file's
   edges, and `say::predicate_phrase` renders it as "defines" / "is
   defined in" — a feature "defining" a sub-feature is nonsense. Reusing
   it would have been free and wrong.

3. **A feature with parts is judged by its parts.** Its own links still
   appear as evidence, and are shown, but they cannot make it ready while
   a part is not. The alternative — letting a parent satisfy its own four
   slots — would mean a feature could be declared done by attaching four
   files to it and ignoring every part underneath. A feature *without*
   parts is judged by its own definition of done, which is exactly the
   previous behaviour, so nothing that existed before changed meaning.

4. **The rollup names what is blocking.** `DoneReport` carries
   `blocked_by`, so a parent says *waiting on UX/UI* rather than merely
   *not done*.

5. **Only trees.** `link` refuses a self-link, a non-feature target, a
   second parent, and any edge that would close a loop — checked by
   walking up from the intended parent. Depth is capped and traversal
   carries a visited set, because nothing walked features before this and
   so nothing protected against a cycle.

6. **A composition edge resolves to a feature.** `resolve_target` tries
   `decision`, `plan`, `skill` and `agent_config` before `feature`, so a
   part named `eyes` would have silently attached to an ADR with the same
   slug. Composition pins the kind; `feature link --kind` exposes the same
   control generally.

## Consequences

- `brain feature tree` shows the rollup; `brain done` explains whether a
  feature is judged by parts or by its own requirements, and names the
  blocker; `feature list` says `3/3 parts` or `4/4 linked` and shows the
  parent.
- Eyes gains a **dimension strip**: one cell per part, or — for a leaf —
  one cell per requirement. The same object is drawn at three scales, and
  each cell is a shape as well as a colour.
- A linked sub-feature must now answer for itself. Before this change,
  `evidence.rs::resolve_link` had no `feature` branch, so a part would
  have reported `good` merely for existing — a parent would have looked
  supported while the thing supporting it was 0/4 done. That is the exact
  failure this product exists to prevent, and it is fixed in the same
  change that made it reachable.
- The proof list no longer silently truncates at six entries. A claim that
  hides half its evidence is not a claim.
- Applying this to the live store immediately produced a true and useful
  result: `eyes` was `done`, gained three parts, and dropped to 0/3 until
  each part was linked to what actually implements it.
- Composition is authored through the CLI only. Eyes stays read-only, and
  no part is ever inferred from a directory name — a part is a claim
  someone made.
