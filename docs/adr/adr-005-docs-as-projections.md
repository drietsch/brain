# Documentation is a projection of the graph

Status: accepted

## Context

Hand-written docs rot: the moment a mentioned file changes, prose silently
lies. Screenshots and screencasts rot faster. "Always up-to-date" cannot be
a discipline; it has to be a property of how docs are produced.

## Decision

Two mechanisms (crates/brain-observe/src/twin.rs, scripts/docsgen/):

- Hand-written docs get **staleness detection**: a doc whose `mentions`
  targets have newer `content_b3` observations than the doc's own `content`
  is surfaced by `brain twin stale` and in insights. Staleness is derived
  at query time, never written — stale is a judgment about now.
- Generated docs are **projections**: `scripts/docsgen/generate.sh` renders
  tour.md, terminal screenshots and a typed screencast (Playwright +
  bundled Chromium), and a TTS narration whose sentences are computed from
  graph queries — so the audio track is as regenerated as the text. The
  artifacts are twinned like any file, so their freshness is itself
  queryable.

TTS backend is pluggable: Qwen3-TTS-12Hz-0.6B-Base when its stack is
installed (scripts/docsgen/tts.py documents the setup), espeak-ng as the
offline fallback.

## Consequences

- Generated docs cannot rot: regeneration is one command, and the twin
  records exactly when it last happened.
- Hand-written docs can still rot, but rot is now visible the moment it
  happens instead of when a reader trips over it.
- Media artifacts (png, webm, wav) are twinned; docs/generated/ carries a
  do-not-edit contract — the graph is the source, files are output.
- The projection pipeline lives in `crates/brain-cli/src/docsgen.rs`.
