# Authoring the feature spine

Service: brain

The feature registry is the top layer of the graph: every test, decision,
document, session and governed change is reachable from a feature through
the files it declares. A feature is a claim someone made (ADR-028), so the
spine is authored here rather than inferred from directory names.

Every command below is guarded. `feature add` prints `unchanged` and
`feature link` prints `already linked` when the graph already agrees, so
re-running this runbook is a no-op and is the way to check the spine still
matches the tree. A full run takes about twenty seconds.

`documented_in` and `decided_by` pin `--kind` on purpose. `resolve_target`
tries `["file", name]` first, so an unpinned link lands on the *file*
entity and its evidence resolves through `lifecycle::of` on a file — active
whenever the file is present. Pinning the kind attaches the artifact, so the
slot degrades when the document rots.

## Steps

### 1. The root

```bash
brain feature add twin/self brain \
  --title "brain — an agent-native semantic substrate" --status active
brain feature link twin/self brain documented_in readme --kind doc
brain feature link twin/self brain documented_in architecture --kind doc
```

### 2. The kernel

```bash
brain feature add twin/self kernel \
  --title "Kernel — objects, identity, canonical bytes" --status active --part-of brain
brain feature link twin/self kernel implemented_by crates/brain-core/src/object.rs
brain feature link twin/self kernel implemented_by crates/brain-core/src/canonical.rs
brain feature link twin/self kernel implemented_by crates/brain-core/src/ids.rs
brain feature link twin/self kernel implemented_by crates/brain-core/src/lib.rs
brain feature link twin/self kernel tested_by crates/brain-core/src/object.rs
brain feature link twin/self kernel tested_by crates/brain-core/src/canonical.rs
brain feature link twin/self kernel tested_by crates/brain-core/src/ids.rs
brain feature link twin/self kernel decided_by adr-001-relation-predicate-field --kind decision
brain feature link twin/self kernel decided_by adr-002-alpha-normalization-at-store-boundary --kind decision
brain feature link twin/self kernel documented_in architecture --kind doc
```

### 3. The store

```bash
brain feature add twin/self store \
  --title "Store — content-addressed objects, namespaces, replication" --status active --part-of brain
brain feature link twin/self store implemented_by crates/brain-store/src/lib.rs
brain feature link twin/self store implemented_by crates/brain-store/src/intents.rs
brain feature link twin/self store implemented_by crates/brain-store/src/sync.rs
brain feature link twin/self store tested_by crates/brain-store/src/lib.rs
brain feature link twin/self store tested_by crates/brain-store/src/intents.rs
brain feature link twin/self store tested_by crates/brain-store/src/sync.rs
brain feature link twin/self store decided_by adr-002-alpha-normalization-at-store-boundary --kind decision
brain feature link twin/self semantic-twin decided_by adr-014-relation-currency-via-edge-tombstones --kind decision
brain feature link twin/self store documented_in architecture --kind doc
```

### 4. Index and cortex

```bash
brain feature add twin/self index-and-cortex \
  --title "Index and cortex — replay, and reads that stay warm" --status active --part-of brain
brain feature link twin/self index-and-cortex implemented_by crates/brain-index/src/lib.rs
brain feature link twin/self index-and-cortex implemented_by crates/brain-cortex/src/lib.rs
brain feature link twin/self index-and-cortex tested_by crates/brain-index/src/lib.rs
brain feature link twin/self index-and-cortex tested_by crates/brain-cortex/src/lib.rs
brain feature link twin/self index-and-cortex decided_by adr-011-cortex --kind decision
brain feature link twin/self index-and-cortex documented_in architecture --kind doc
```

### 5. The semantic twin (already registered)

```bash
brain feature add twin/self semantic-twin --title "Semantic twin" --status active --part-of brain
brain feature link twin/self semantic-twin implemented_by crates/brain-observe/src/twin/refresh.rs
brain feature link twin/self semantic-twin implemented_by crates/brain-observe/src/twin/reads.rs
brain feature link twin/self semantic-twin implemented_by crates/brain-observe/src/docs.rs
brain feature link twin/self semantic-twin implemented_by crates/brain-observe/src/symbols.rs
brain feature link twin/self semantic-twin implemented_by crates/brain-observe/src/backfill.rs
brain feature link twin/self semantic-twin implemented_by crates/brain-observe/src/baseline.rs
brain feature link twin/self semantic-twin tested_by crates/brain-observe/src/twin/tests.rs
brain feature link twin/self semantic-twin tested_by crates/brain-observe/src/baseline.rs
brain feature link twin/self semantic-twin tested_by crates/brain-observe/src/docs.rs
brain feature link twin/self semantic-twin tested_by crates/brain-observe/src/symbols.rs
brain feature link twin/self semantic-twin tested_by crates/brain-observe/src/backfill.rs
brain feature link twin/self semantic-twin decided_by adr-008-capture-rules-in-the-graph --kind decision
brain feature link twin/self semantic-twin decided_by adr-012-backfill-history-with-historical-timestamps --kind decision
brain feature link twin/self semantic-twin documented_in twin --kind doc
```

### 6. The brain functions

```bash
brain feature add twin/self brain-functions \
  --title "Brain functions — attention, sleep, wake, association, coherence" --status active --part-of brain
brain feature link twin/self brain-functions implemented_by crates/brain-observe/src/attention.rs
brain feature link twin/self brain-functions implemented_by crates/brain-observe/src/sleep.rs
brain feature link twin/self brain-functions implemented_by crates/brain-observe/src/wake.rs
brain feature link twin/self brain-functions implemented_by crates/brain-observe/src/assoc.rs
brain feature link twin/self brain-functions implemented_by crates/brain-observe/src/coherence.rs
brain feature link twin/self brain-functions implemented_by crates/brain-observe/src/lifecycle.rs
brain feature link twin/self brain-functions tested_by crates/brain-observe/src/attention.rs
brain feature link twin/self brain-functions tested_by crates/brain-observe/src/sleep.rs
brain feature link twin/self brain-functions tested_by crates/brain-observe/src/wake.rs
brain feature link twin/self brain-functions tested_by crates/brain-observe/src/assoc.rs
brain feature link twin/self brain-functions tested_by crates/brain-observe/src/coherence.rs
brain feature link twin/self brain-functions tested_by crates/brain-observe/src/lifecycle.rs
brain feature link twin/self brain-functions decided_by adr-009-functional-brain-not-structural --kind decision
brain feature link twin/self brain-functions decided_by adr-013-lifecycle-as-derived-judgment --kind decision
brain feature link twin/self brain-functions decided_by adr-015-staleness-severity-and-acknowledgement --kind decision
brain feature link twin/self brain-functions decided_by adr-016-wake-and-the-sleep-window --kind decision
brain feature link twin/self brain-functions documented_in twin --kind doc
```

### 7. Artifacts — the kind registry, authoring, projections

```bash
brain feature add twin/self artifact-registry \
  --title "Artifacts — the kind registry, authoring, and projections" --status active --part-of brain
brain feature link twin/self artifact-registry implemented_by crates/brain-observe/src/templates.rs
brain feature link twin/self artifact-registry implemented_by crates/brain-observe/src/kinds.rs
brain feature link twin/self artifact-registry implemented_by crates/brain-observe/src/projection.rs
brain feature link twin/self artifact-registry implemented_by crates/brain-observe/src/instructions.rs
brain feature link twin/self artifact-registry implemented_by crates/brain-observe/src/fitness.rs
brain feature link twin/self artifact-registry implemented_by crates/brain-observe/src/tidy.rs
brain feature link twin/self artifact-registry tested_by crates/brain-observe/src/templates.rs
brain feature link twin/self artifact-registry tested_by crates/brain-observe/src/kinds.rs
brain feature link twin/self artifact-registry tested_by crates/brain-observe/src/projection.rs
brain feature link twin/self artifact-registry tested_by crates/brain-observe/src/instructions.rs
brain feature link twin/self artifact-registry tested_by crates/brain-observe/src/fitness.rs
brain feature link twin/self artifact-registry tested_by crates/brain-observe/src/tidy.rs
brain feature link twin/self artifact-registry decided_by adr-003-templates-in-the-graph --kind decision
brain feature link twin/self artifact-registry decided_by adr-017-artifact-kind-registry --kind decision
brain feature link twin/self artifact-registry decided_by adr-018-placement-policy-and-assets --kind decision
brain feature link twin/self artifact-registry decided_by adr-019-read-only-projection-contract --kind decision
brain feature link twin/self artifact-registry decided_by adr-020-opt-in-enforcement-gates --kind decision
brain feature link twin/self artifact-registry decided_by adr-022-template-fitness --kind decision
brain feature link twin/self artifact-registry documented_in twin --kind doc
```

### 8. Governed changes (already registered)

```bash
brain feature add twin/self governed-changes --title "Governed changes" --status active --part-of brain
brain feature link twin/self governed-changes implemented_by crates/brain-observe/src/govern.rs
brain feature link twin/self governed-changes implemented_by crates/brain-store/src/intents.rs
brain feature link twin/self governed-changes tested_by crates/brain-observe/src/govern.rs
brain feature link twin/self governed-changes tested_by crates/brain-store/src/intents.rs
brain feature link twin/self governed-changes decided_by adr-010-governed-mode --kind decision
brain feature link twin/self governed-changes decided_by adr-021-tidy-through-governed-changes --kind decision
brain feature link twin/self governed-changes documented_in architecture --kind doc
```

### 9. Tests in the graph

```bash
brain feature add twin/self tests-in-the-graph \
  --title "Tests in the graph — protocols, cases, and their history" --status active --part-of brain
brain feature link twin/self tests-in-the-graph implemented_by crates/brain-observe/src/testing.rs
brain feature link twin/self tests-in-the-graph tested_by crates/brain-observe/src/testing.rs
brain feature link twin/self tests-in-the-graph decided_by adr-004-tests-in-the-graph --kind decision
brain feature link twin/self tests-in-the-graph documented_in twin --kind doc
```

### 10. Agent sessions

```bash
brain feature add twin/self agent-sessions \
  --title "Agent sessions — who worked here, and on what" --status active --part-of brain
brain feature link twin/self agent-sessions implemented_by crates/brain-observe/src/sessions.rs
brain feature link twin/self agent-sessions implemented_by crates/brain-observe/src/agents.rs
brain feature link twin/self agent-sessions tested_by crates/brain-observe/src/sessions.rs
brain feature link twin/self agent-sessions tested_by crates/brain-observe/src/agents.rs
brain feature link twin/self agent-sessions decided_by adr-025-agent-sessions-are-first-class --kind decision
brain feature link twin/self agent-sessions documented_in twin --kind doc
```

### 11. Evidence and assets

```bash
brain feature add twin/self evidence-and-assets \
  --title "Evidence and assets — screenshots, recordings, the tour" --status active --part-of brain
brain feature link twin/self evidence-and-assets implemented_by crates/brain-observe/src/assets.rs
brain feature link twin/self evidence-and-assets implemented_by crates/brain-observe/src/tour.rs
brain feature link twin/self evidence-and-assets implemented_by crates/brain-cli/src/docsgen.rs
brain feature link twin/self evidence-and-assets tested_by crates/brain-observe/src/assets.rs
brain feature link twin/self evidence-and-assets tested_by crates/brain-observe/src/tour.rs
brain feature link twin/self evidence-and-assets tested_by crates/brain-cli/src/docsgen.rs
brain feature link twin/self evidence-and-assets decided_by adr-005-docs-as-projections --kind decision
brain feature link twin/self evidence-and-assets decided_by adr-027-evidence-you-can-look-at --kind decision
brain feature link twin/self evidence-and-assets documented_in twin --kind doc
```

### 12. The command line

```bash
brain feature add twin/self the-cli \
  --title "The command line — one binary, and git as its trigger" --status active --part-of brain
brain feature link twin/self the-cli implemented_by crates/brain-cli/src/main.rs
brain feature link twin/self the-cli implemented_by crates/brain-cli/src/manual.rs
brain feature link twin/self the-cli implemented_by crates/brain-cli/src/hooks.rs
brain feature link twin/self the-cli tested_by crates/brain-cli/src/manual.rs
brain feature link twin/self the-cli tested_by crates/brain-cli/src/hooks.rs
brain feature link twin/self the-cli decided_by adr-006-monolithic-binary --kind decision
brain feature link twin/self the-cli decided_by adr-007-git-triggers-the-brain --kind decision
brain feature link twin/self the-cli documented_in readme --kind doc
```

### 13. Native code — the calculus

```bash
brain feature add twin/self native-code \
  --title "Native code — the calculus, and programs that live in the graph" --status active --part-of brain
brain feature link twin/self native-code implemented_by crates/brain-runtime/src/lib.rs
brain feature link twin/self native-code implemented_by crates/brain-cli/src/notation.rs
brain feature link twin/self native-code implemented_by crates/brain-cli/src/tasks.rs
brain feature link twin/self native-code tested_by crates/brain-runtime/src/lib.rs
brain feature link twin/self native-code tested_by crates/brain-cli/src/notation.rs
brain feature link twin/self native-code tested_by crates/brain-cli/src/tasks.rs
brain feature link twin/self native-code decided_by adr-002-alpha-normalization-at-store-boundary --kind decision
brain feature link twin/self native-code documented_in calculus --kind doc
```

### 14. Eyes, and its three parts (already registered)

```bash
brain feature add twin/self eyes --title "Eyes human projection" --status active --part-of brain
brain feature link twin/self eyes decided_by adr-023-eyes-read-only-human-projection --kind decision
brain feature link twin/self eyes documented_in eyes --kind doc

brain feature link twin/self eyes-core implemented_by crates/brain-eyes/src/state.rs
brain feature link twin/self eyes-core implemented_by crates/brain-eyes/src/query/mod.rs
brain feature link twin/self eyes-core implemented_by crates/brain-eyes/src/body.rs
brain feature link twin/self eyes-core implemented_by crates/brain-eyes/src/lib.rs
brain feature link twin/self eyes-core tested_by crates/brain-eyes/src/tests.rs
brain feature link twin/self eyes-core tested_by crates/brain-eyes/src/lib.rs
brain feature link twin/self eyes-core decided_by adr-024-eyes-shows-judgments-and-content --kind decision
brain feature link twin/self eyes-core documented_in eyes --kind doc

brain feature link twin/self eyes-http implemented_by crates/brain-eyes/src/http.rs
brain feature link twin/self eyes-http tested_by crates/brain-eyes/src/tests.rs
brain feature link twin/self eyes-http tested_by e2e/eyes.spec.ts
brain feature link twin/self eyes-http decided_by adr-023-eyes-read-only-human-projection --kind decision
brain feature link twin/self eyes-http documented_in eyes --kind doc

brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/assets/app.js
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/assets/list.js
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/assets/mri.js
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/say.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/dto.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/now.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/features.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/evidence.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/thing.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/library.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/map.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/mri.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/tests.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/timeline.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/work.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/find.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/media.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/next.rs
brain feature link twin/self eyes-ux implemented_by crates/brain-eyes/src/query/compare.rs
brain feature link twin/self eyes-ux tested_by crates/brain-eyes/src/tests.rs
brain feature link twin/self eyes-ux tested_by crates/brain-eyes/src/say.rs
brain feature link twin/self eyes-ux tested_by e2e/eyes.spec.ts
brain feature link twin/self eyes-ux decided_by adr-024-eyes-shows-judgments-and-content --kind decision
brain feature link twin/self eyes-ux decided_by adr-026-the-anatomy-is-drawn-in-full --kind decision
brain feature link twin/self eyes-ux decided_by adr-029-type-encodes-epistemology --kind decision
brain feature link twin/self eyes-ux documented_in eyes --kind doc
```

### 15. Check it

```bash
brain feature tree twin/self          # the spine, with readiness rolled up
brain feature matrix twin/self        # the definition of done, per feature
```

Readiness is expected to *fall* when the spine is first authored: a feature
with four real slots and one missing document is honestly incomplete where
a feature with one link per slot was not.
