# The Stage 1 authoring experiment

The founding assumption — agents author programs natively as graph terms, no
files, no text syntax — has one make-or-break empirical test: **can current
models reliably author and *edit* non-trivial terms against
`docs/schema/term.schema.json` via constrained output?** This experiment is
deliberately scheduled before everything else it gates. If it fails, the
calculus gets redesigned while redesign is still cheap.

## Running it

The driver is `scripts/authoring_experiment.py` (requires `ANTHROPIC_API_KEY`
or an `ant auth login` profile; `cargo build -p brain-cli` first):

```bash
python3 scripts/authoring_experiment.py tasks/t0*.json                 # author mode
python3 scripts/authoring_experiment.py --edits tasks/edits/*.json     # edit mode
python3 scripts/authoring_experiment.py --dry-run tasks/t01-increment.json  # inspect prompts
```

Per-run metrics land in `results/authoring-runs.jsonl`; every checked emission
also lands in the graph as `Evidence` attached to its content hash.

**A finding worth recording:** schema-constrained decoding cannot carry the
emission, because the term schema is recursive (a term contains terms) and
structured-outputs features reject recursive schemas. Emissions are therefore
plain JSON and *validity is a measured outcome* — which is what this
experiment exists to measure. If validity turns out poor, a non-recursive
"instruction list" encoding of terms is the first redesign candidate.

## Protocol

1. Give the model a task description (see `tasks/*.json`), the term schema,
   and the builtin foreign-symbol table from `docs/calculus.md`.
2. The model emits a term as plain JSON (see the finding above).
3. Check with `brain task check <task.json> <emitted-term.json>`. The checker
   runs the term against the task's cases in simulation posture (pure,
   fuel-bounded, effects denied) and records the outcome in the graph as
   `Evidence` at the `Behavioral` level, attached to the term's content hash.
4. On failure, the driver feeds the checker output back for one repair
   attempt (the repair-rate metric).
5. For *edit* trials (`tasks/edits/*.json`): the task carries a `base_term`
   and a change request; the driver measures **edit locality** — the fraction
   of the emission's subtrees already present in the base (1.0 = pure reuse,
   near 0 = wholesale regeneration). Heuristic, not a proof.

## Metrics

| Metric | Question |
|---|---|
| Emission validity | Does the JSON parse as a term at all? (Structural level) |
| Case pass rate | Does the term do what the task asked? (Behavioral level) |
| Fuel profile | Is the solution reasonable, or accidentally quadratic? |
| Edit locality | Do edits modify subterms or regenerate wholesale? |
| Hole discipline | When uncertain, does the model emit typed holes rather than guesses? |
| Repair rate | Given a failing case report, does the next emission fix it? |

## Gate

Stage 1 passes when a current-generation model achieves high emission
validity and case pass rates on tasks at this corpus's difficulty, and edits
are predominantly local. Numbers are recorded as evidence in the graph, not
in this document — a result is a fact about a (model, task, term-hash)
triple. If results are poor, the calculus and schema are the suspects, in
that order: too many ops, ambiguous shapes, or missing constructs the tasks
implicitly need.

## Task format

```json
{
  "name": "increment",
  "description": "Return the input integer plus one.",
  "spec": { "input": "int", "output": "int" },
  "cases": [ { "arg": 1, "expect": 2 } ]
}
```

Solutions are unary functions (`lam`). Case `arg`/`expect` values are plain
JSON: integers, strings, booleans, `null` (unit), objects (records), and
`{"$variant": tag, "payload": ...}` for variants. `expect` is compared
against the JSON rendering of the evaluated result. Reference solutions in
`tasks/solutions/` prove each task is solvable and double as regression
fixtures for the checker itself.
