# Every git commit and push triggers the brain

Status: accepted

## Context

The twin is only as good as its freshness, and remembering to run
`brain twin refresh` is exactly the kind of discipline that fails. Git
commits and pushes are the natural heartbeat of agentically-built
software — the moments when reality changes and the graph should notice.

## Decision

`brain hook install` (crates/brain-cli/src/hooks.rs) writes minimal
`post-commit` and `pre-push` hooks that call back into
`brain hook run <event>` — the behavior lives in the monolithic binary, so
improving it never requires reinstalling hooks. On each event the twin
refreshes and prints one line, plus warnings an author should see right
now: stale docs, template violations, failing tests from the last
protocol. `pre-push --docs` optionally regenerates the doc projections.

Hooks are **fail-open by design**: the twin is a sense organ, never a
gate. `hook run` swallows its own errors and the hook script ends in
`|| true` — a brain failure can never block a commit or a push. Foreign
hooks are respected: install refuses to overwrite them without `--force`,
uninstall removes only hooks carrying the brain marker.

## Amendment: opt-in test protocols per commit (`--tests`)

`brain hook install --tests [--test-cmd "<cmd>"]` extends post-commit:
the repo's test command — inferred from its manifest (Cargo.toml → `cargo
test`, package.json → `npm test`, pyproject → `pytest`, phpunit.xml) or
given explicitly — runs after each commit and its output is imported as a
protocol automatically. The command is stored as a `test_command`
observation on the repo entity, not in the hook file: change it any time
without reinstalling, and it replicates with the graph. Still fail-open:
failing tests are recorded and reported, never a reason to block; runs
stay content-addressed, so an unchanged suite re-imports as a no-op.

## Consequences

- The growth series gains a point per meaningful commit; git_commit
  observations track the twin against history automatically.
- Feedback arrives at the moment of change ("2 docs went stale with this
  commit"), not at the next manual refresh.
- Enforcement (blocking a push on failing checks) remains deliberately
  out of scope for reflective mode; that is governed-mode territory.
