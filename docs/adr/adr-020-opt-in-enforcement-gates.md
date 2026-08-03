# Opt-in enforcement gates, and exit code 3

Status: accepted

## Context

"Sense organ, never a gate" (ADR-007) is the right default — but prose
conventions are followed with different reliability by different agents,
and some contracts deserve teeth. The boundary between "the brain broke"
and "the brain refused" needed to be machine-readable, or hooks could
not fail open on errors while still blocking on refusals.

## Decision

Enforcement is per-kind graph data: `enforce = advisory` (default) |
`enforced`, set with `brain template set <slug> --enforce`. Three
enforcement points, all speaking the same protocol:

- **Write time** — `brain artifact new|edit` validates against the
  kind's `requires`; an enforced kind refuses with the missing fields
  and the scaffold command, writes nothing, and exits **3**.
- **Commit time** — `brain hook install --gate` (a second, repo-level
  opt-in) adds a pre-commit hook. Its script maps exit 3 to a blocked
  commit and anything else to success; `hook run pre-commit` checks only
  staged content — hand-edited projections and enforced-kind
  conformance, judged on `git show :path`, not the working tree — and
  fails open on every internal error. The refusal names each violation's
  fix and mentions `--no-verify` for when the brain is wrong.
- **Check time** — `brain artifact render --check` and `brain
  instructions generate --check` for CI.

Exit codes: 0 ok, 1 error, 2 usage, **3 deliberate refusal**. The
`refused:` error prefix carries the distinction through the CLI.

## Consequences

- Strictness is dialed per kind as trust grows, and the dial is data —
  it replicates with `brain pull` and appears in the generated
  instruction block, so every agent family reads the same rules.
- post-commit/pre-push/post-checkout/post-merge remain pure sense
  organs; the gate is the only fail-closed path and it is doubly opt-in.
- Advisory kinds still get warnings everywhere a violation is visible —
  enforcement changes the consequence, never the observation.
- The gate consults the registry in `crates/brain-observe/src/kinds.rs`; the pre-commit gate itself lives in `crates/brain-cli/src/hooks.rs`.
