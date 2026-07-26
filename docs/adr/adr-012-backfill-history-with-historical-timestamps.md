# Backfilled history carries historical timestamps

Status: accepted

## Context

Brownfield repos adopted brain mid-life had empty pasts: churn, growth,
staleness, and `twin at` only reached back to adoption day. Git already
holds the past; the question was how to admit it into a graph whose facts
are sourced, timestamped observations.

## Decision

`brain twin backfill` (crates/brain-observe/src/backfill.rs) replays
`git log --reverse --name-status` and writes each historical fact with
its **commit's timestamp**, sourced `"backfill"`. Because timelines order
by `observed_at_ms`, backfilled facts slot beneath current observations
in every query — `latest()` still answers with the present, while
`files_at`, churn, and association's co-change batches (one commit = one
timestamp) gain the whole past. Deletions become `present=false`,
reappearances restore presence, and every commit lands as a `git_commit`
observation, so `twin at <hash>` resolves any point in history.

Deliberate limits: file-level facts only (no historical symbol
reconstruction — enormous cost, little value; the current refresh covers
the present); blobs over 4 MB are skipped and counted. Idempotence needs
no guards: identical historical facts are content-addressed no-ops, so
re-running backfill writes zero objects.

## Consequences

- The brownfield minute-one is complete: backfill the past, refresh the
  present, hook the future, attend.
- Sleep's delta logic is naturally immune: backfilled observations are
  older than `consolidated_until`, so consolidation never double-counts
  the past as new activity.
- Rewritten history (rebases) backfills the new lineage alongside the
  old observations — both are honest records of what a repo once claimed.
