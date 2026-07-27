# The read-only projection contract

Status: accepted

## Context

`generated=true` only *suppressed* churn and attention — a hand-edit to a
projection was hidden, not detected, which is worse than not tracking it.
If graph-first artifacts render to files that agents can quietly edit,
drift returns through the back door and the graph stops being
authoritative in practice.

## Decision

Three layers, weakest to strongest (crates/brain-observe/src/projection.rs):

1. **Marker** — every projection's first line names the authoring
   command (`Edit via: brain artifact edit <prefix> <kind> <slug>`),
   because that line is what an agent reads first.
2. **Filesystem** — rendered files carry the read-only bit. Best-effort:
   git does not preserve it, so refresh, tidy, and the post-checkout/
   post-merge hooks re-arm it.
3. **Detection (authoritative)** — `expected_b3` on the file entity
   records the exact rendered bytes. Any mismatch is a reported
   violation (`HandEdited`), a vanished file is `Missing`, and a file
   whose artifact moved on is `StaleRender` — each with its fix command.
   The instruction files' managed blocks get the same treatment via
   `instructions_b3`.

Repair re-renders from the graph — the graph always wins — but tidy first
rescues the hand-edit into the artifact's observation timeline
(`hand_edit` + a note): agent work becomes history, never a casualty.
Silently destroying it would teach agents to distrust tidy.

The docs pipeline adopted the same contract: tour.md, narration.txt,
brain.1, and captured media carry `expected_b3` and the read-only bit,
and screenshots keep `rendered_from` provenance in the graph instead of a
temp file that used to be deleted.

## Consequences

- An agent that tries to edit a projection hits a permission error, then
  a first-line marker, then a detected violation naming the right
  command — three chances to do it the graph's way.
- Refresh never re-captures a projection as a second document: bytes
  matching `expected_b3` mark it as a view, not a source.
- Regenerators are the one sanctioned writer: they lower the bit,
  rewrite, re-arm.
