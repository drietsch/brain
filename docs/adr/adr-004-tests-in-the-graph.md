# Tests and test protocols are graph citizens

Status: accepted

## Context

The definition of done requires `tested_by`, but the twin had no notion of
what a test *is*, and test results lived only in terminal scrollback —
unqueryable, unversioned, gone at session end. For agentically-built
software the protocol (what ran, what failed, what flipped) is exactly the
history an agent needs at the start of the next session.

## Decision

Two layers, one identity scheme
(crates/brain-observe/src/testing.rs):

- Static: refresh classifies twinned files by framework (Rust `#[test]`,
  Playwright/Jest specs, pytest, PHPUnit) into `test_framework` /
  `tests_declared` / `file_role` observations, and test files get `covers`
  relations to the files they import.
- Dynamic: `brain testrun import` parses raw `cargo test` output or JUnit
  XML (the interchange format every major framework exports, including
  Playwright). A run is a content-addressed `test_run` entity; each case
  is a `test_case` entity with guarded `result` observations — the
  timeline records transitions, which is the flake/regression history.
  Every run writes Behavioral-level Evidence on the repo entity.

## Consequences

- "Which tests cover this file", "what is failing now", and "which hubs
  have no tests" are graph queries; insights surface untested hubs as a
  concentrated-risk list.
- Re-importing a report is a no-op (content-addressed identity), so CI can
  pipe every run in blindly.
- Parsing is best-effort line scanning, consistent with the twin's
  forgiving-extractor philosophy; a wired-in reporter plugin is the
  upgrade path, same entities and relations.
