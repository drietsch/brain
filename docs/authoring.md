# The Stage 1 authoring experiment

The founding assumption — agents author programs natively as graph terms, no
files, no text syntax — has one make-or-break empirical test: **can current
models reliably author and *edit* non-trivial terms against
`docs/schema/term.schema.json` via constrained output?** This experiment is
deliberately scheduled before everything else it gates. If it fails, the
calculus gets redesigned while redesign is still cheap.

## Protocol

1. Give the model a task description (see `tasks/*.json`), the term schema,
   and the builtin foreign-symbol table from `docs/calculus.md`.
2. The model emits a term as JSON — with schema-constrained/structured output
   where the serving stack supports it, plain JSON mode otherwise.
3. Check with `brain task check <task.json> <emitted-term.json>`. The checker
   runs the term against the task's cases in simulation posture (pure,
   fuel-bounded, effects denied) and records the outcome in the graph as
   `Evidence` at the `Behavioral` level, attached to the term's content hash.
4. For *edit* trials: give the model an existing passing term plus a change
   request ("also handle the `none` case"); measure whether the emission is a
   local modification (small term diff) or a from-scratch rewrite.

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
