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
| `core/sub` | pure | — | `{a: int, b: int} -> int` (checked overflow). |
| `core/mul` | pure | — | `{a: int, b: int} -> int` (checked overflow). |
| `core/lt` | pure | — | `{a: int, b: int} -> bool` (`a < b`). |
| `core/if` | pure | — | `{cond: bool, then: T, else: T} -> T`. **Eager**: both branches evaluate before selection. A lazy `if` in the calculus is future work, gated on authoring evidence. |
| `core/concat` | pure | — | `{a: str, b: str} -> str`. |
| `core/eq` | pure | — | `{a, b} -> bool` (structural). |
| `io/echo` | external | `io` | Identity with a declared external effect; exists so the full intent/receipt path is exercisable end to end. |

`core/sub`, `core/mul`, `core/lt` and `core/if` were added when the Stage 1
task corpus demonstrated the gap (no way to branch on a comparison) — the
"enrichment must be paid for by authoring pain" rule working as intended. Note
the calculus itself did not change: the registry is the extension point.

## Compact term notation

Run 2 of the authoring experiment showed JSON emission validity holds to ~90
nodes but the encoding costs 10–20x the bytes of a text form. The compact
notation (`.term` files, parser/printer in `brain-cli/src/notation.rs`) is
the response — an S-expression authoring/projection surface over the *same*
canonical Term:

```text
(lam n (if (lt n 0) (mul n -1) n))          ; abs
{h (get clock h) m m2}                      ; record
(match e (case tick _ (tag green unit)) (else (tag red unit)))
(add a b) (sub a b) (lt a b) (if c t e)     ; sugar for core/* foreign calls
(io/echo x)                                 ; any symbol with '/' is foreign
(hole h0 int)  (ref b3:<hash>)              ; holes and hash references
```

Identity is untouched: a program authored in notation and the same program
authored in JSON parse to the identical Term, hash to the identical NodeId,
and deduplicate to one node in the graph (verified in Run 3 across four
programs). `brain notation <file>` converts either direction; `put-code` and
`task check` accept both encodings by extension.

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
