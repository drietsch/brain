# The feature is the spine

Status: accepted

## Context

Eyes v4 was organised by kind. Artifacts shelved by kind, Tests grouped by
test-name prefix, Map by directory, MRI by kind cluster, Work listing
sessions, changes and plans side by side. Nothing on a test row, a
decision row or a session row said *which feature this serves*, and the
one surface whose rows were features showed no entity names at all —
`StripCell.id` was hard-coded `None`, so a dimension cell could never be
opened.

That was a truthful rendering of the graph, and that was the problem. Six
features existed. `eyes` linked exactly one file as both its
implementation and its tests, plus a `documented_in` edge to a document
deleted three commits earlier. Against 184 files, 157 test cases, 29
decisions and 1603 relations, the feature layer was 28 edges — 1.7% of
the graph. `brain feature matrix` reported every feature ✓ on every
dimension while 96% of the system belonged to nothing.

A graph in which almost nothing relates to a feature cannot be read
top-down, and a product whose thesis is *claims must show their proof*
cannot leave its own top layer decorative.

## Decision

**The feature registry is the spine: everything in the graph is reachable
from a feature, and the path is checkable.**

1. **A feature declares its files; everything else attaches by
   derivation.** Every other kind already points at files, automatically
   and without anyone linking it: a test file `covers` what it imports, a
   case is `defined_in` one, a document `mentions` the paths it names, a
   session `touched` what it edited, a change `changes` its target. The
   file is therefore the join. `brain-observe/src/spine.rs` walks it, and
   a feature inherits its tests, documents, sessions and changes for the
   price of one authored predicate per file.

2. **Attribution is derived; features and parts never are.** ADR-028 is
   explicit that a part is a claim someone made and is never inferred
   from a directory name. That rule is untouched. What the spine computes
   is *which existing feature an existing record serves* — a different
   act from inventing structure. Nothing in this change reads a path and
   concludes a feature exists.

3. **A derived claim names the file it came through.** Every reached
   record carries the declared file that carries it, and every sentence
   says it: *"it changes crates/brain-eyes/src/http.rs, which this
   feature is built by"*. An attribution that cannot show its join is
   indistinguishable from a guess.

4. **The walk stops at two hops.** Following `imports` transitively would
   let a feature that declares one file claim its entire dependency cone,
   and the spine would smear into a single blur. The only second hop is
   `covers` → `defined_in`: the cases inside a test file that covers a
   declared file.

5. **A document's own file is a narrower join than declared code.**
   Declared code answers to everything that points at it. The file a
   declared *document* lives in answers only to who edited it. Following
   `mentions` there attributed the README, the roadmap and the
   architecture note to whichever feature declared `docs/twin.md`. A
   document that mentions my documentation is not part of my feature.

6. **Coverage is a census, not a finding.** How much of the graph belongs
   to a feature is a readout on the Features surface — per kind, with the
   remainder named rather than rounded away. It is deliberately not
   merged into the proof census on Now: that one asks whether a claim can
   show its proof, this one whether a record is claimed at all. Different
   question, different population.

   Files are quiet rather than red. A repository will never have every
   file under a feature — manifests, scripts and scaffolding belong to
   none — and colouring that as a fault would be a lie.

7. **What the derivation makes newly askable.** The definition of done
   counts links. The automatic edges reach the same files, so each
   declared slot can be checked against observed reality: does the
   document claimed as a feature's documentation `mention` any file that
   feature declares? A slot that is linked but uncorroborated is a claim
   nothing observed supports — strictly stronger than "3 records linked",
   and it needs no new data. Both sides already existed and had never
   been compared.

   It reports as **one** finding, never one per feature. Seventeen rows
   reading "claims something nothing backs up" are one thing to know
   about, seventeen times — the failure ADR-029 §7 names.

8. **Stages are a taught kind, and a stage's state is never derived from
   its features.** `planned_for` (feature → stage) is the delivery axis;
   `part_of` remains composition. A feature has one parent and any number
   of stages, and the two must not compete for the single-parent slot.

   Stage 1 is a research question. Four finished features do not answer
   it. So a stage reports what was recorded about it, its features report
   what they can show, and the wording keeps the subject on the features:
   *"All 4 features planned for it can show their evidence."*

   Stages are **authored, never parsed.** Lifting them out of
   `docs/roadmap.md` with a heading regex would manufacture graph
   structure from prose, which is the move ADR-024 forbids.

9. **A feature's progress is its rollup, everywhere.** `insights.features`
   read a feature's own link count rather than `DoneReport::score`, so
   every parent reported what it happened to be linked to directly. The
   root of the spine said `1/4` while its thirteen parts were all ready,
   and `wake`, `attend`, the tour narration and the capability matrix all
   inherited it. `FeatureProgress` now carries the score, whether it
   counts parts, and `done` — instead of a fraction string four callers
   re-parsed.

## Consequences

- `docs/runbooks/feature-spine.md` holds the authored spine: eighteen
  features under one root, with the files, decisions and documents that
  carry them. Every command is guarded, so re-running the runbook is a
  no-op — which makes it the check that the graph still matches the claim.
- `documented_in` and `decided_by` must pin `--kind`. `resolve_target`
  tries `["file", name]` first, so an unpinned link lands on the file
  entity and resolves through `lifecycle::of` on a file — active whenever
  the file exists. Pinning the artifact is what lets the slot degrade
  when the document rots.
- `brain spine <prefix>` reports the whole thing from the terminal, so
  the CLI and Eyes answer the same question.
- `coherence::check` splits into `check_with` so Eyes can pass the spine
  it already holds; the spine is built once per graph version alongside
  insights, attention and the rest.
- **Readiness will look worse over time, and that is the point.** The
  first day of the spine reports 101 of 412 records claimed, 25 declared
  slots uncorroborated, and 0 of 164 test cases reachable — because cargo
  runs record results without saying where each case lives. None of that
  was less true yesterday.
- What Eyes still cannot say: no session is joined to the change it
  produced (no predicate connects them, and neither carries a principal),
  and `agent_session` cannot be a `feature link` target at all — its
  entity kind is `agent_session` but its sid namespace is `session`.
  Derivation routes around the latter; both are recorded here rather than
  papered over.
