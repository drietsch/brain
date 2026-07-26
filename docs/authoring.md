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

## Run 1 — coding agent in-session (2026-07-26)

The first subject was the coding agent driving this repository (mode 1 — no
API). Emissions in `runs/2026-07-26-coding-agent/`, per-task rows in
`runs.jsonl` there, evidence in the graph under each term's content hash.

| Metric | Result |
|---|---|
| Emission validity | 9/9 |
| Checker pass, first attempt | 9/9 (7 author + 2 edit) |
| Repairs needed | 0 |
| Edit locality | 0.571 (`greet-excited`), 0.737 (`option-default-negative`) |

**Caveats, honestly stated:** the subject also authored the task corpus and
its reference solutions, so this run is contaminated for novelty — it
validates the pipeline end to end and sets an upper-bound baseline, not an
unbiased capability measurement. Where the tasks allowed it, the emissions
were deliberately structured differently from the references (`abs` via
multiply-by-negative-one, `max3` via nested conditionals instead of a helper,
explicit `none` arm in `option-default`), so the run does exercise fresh
composition rather than recall. The discriminating experiment is the same
protocol run by an agent that has never seen this repository, on tasks it has
never seen — and at larger program sizes, where emission validity is likelier
to degrade.

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
