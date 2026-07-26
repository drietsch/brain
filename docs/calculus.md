# The core calculus

Deliberately tiny: 12 operations. Every enrichment must be paid for by
demonstrated authoring pain, not anticipated elegance. Programs are authored
as JSON against `docs/schema/term.schema.json` (agents emit it via
schema-constrained output; nobody hand-writes it — that is the point).

## Operations

| `op` | Fields | Meaning |
|---|---|---|
| `lit` | `value` | Literal: `int` (i64), `str`, `bool`, `unit`. No floats. |
| `var` | `name` | Variable reference. |
| `lam` | `param`, `body` | Function abstraction (closure over environment). |
| `app` | `func`, `arg` | Application. |
| `let` | `name`, `value`, `body` | Local binding. |
| `record` | `fields` | Record construction; field order canonicalized. |
| `field` | `record`, `field` | Field projection. |
| `variant` | `tag`, `payload` | Tagged value (sum type constructor). |
| `match` | `scrutinee`, `arms`, `default?` | Case analysis; each arm binds the payload. |
| `ref` | `node` | Reference to another `Code` object by content hash. Evaluates in an empty environment (top-level definitions are closed). Replaces imports/linking. |
| `foreign` | `symbol`, `arg` | The only gate to the world. The runtime registry declares each symbol's effect class (`pure`/`external`) and required capability. |
| `hole` | `id`, `expected?` | Typed hole. Evaluation suspends with `Incomplete`; partial programs are first-class. |

## Semantics

- Eager, deterministic, environment-based tree-walking evaluation.
- Fuel-metered: every step costs 1 fuel; exhaustion halts with `FuelExhausted`.
  Non-termination is a budget question, not a hang.
- No exceptions: errors are values of the evaluation (`EvalError`), and
  domain-level alternatives should be modeled with `variant`/`match`.
- Effects: `foreign` with class `external` is wrapped in
  `EffectPort::begin` (durable intent, before) / `commit` (receipt, after).
  Capability checks precede the boundary. Pure symbols never touch it.

## Builtin foreign symbols (scaffold set)

| Symbol | Class | Requires | Meaning |
|---|---|---|---|
| `core/add` | pure | — | `{a: int, b: int} -> int` (checked overflow). |
| `core/concat` | pure | — | `{a: str, b: str} -> str`. |
| `core/eq` | pure | — | `{a, b} -> bool` (structural). |
| `io/echo` | external | `io` | Identity with a declared external effect; exists so the full intent/receipt path is exercisable end to end. |

## Known limitations (deliberate, documented)

- **No type checker yet.** `expected` on holes and `Spec` types are opaque
  strings awaiting a real type system.
- **Hashing is not alpha-equivalent.** `\x.x` and `\y.y` are different nodes.
  Canonicalizing binders (de Bruijn indices in the encoding) should land
  before a large code corpus accumulates.
- **`match` arms bind exactly one payload**; no nested patterns. Compose
  matches instead.
- **Recursion** is expressible only via self-application (and will burn fuel);
  a `fix` construct or recursive `ref` bindings are future work, gated on
  authoring evidence.
