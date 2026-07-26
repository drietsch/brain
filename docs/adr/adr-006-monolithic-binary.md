# brain ships as one monolithic binary, installed with one command

Status: accepted

## Context

The substrate had grown a shell-and-python pipeline around the binary:
docs generation lived in scripts, the watch loop in bash, and installation
meant cloning a workspace and knowing cargo. A tool that wants to be an
agent's persistent memory must cost one command to acquire and one file to
run — anything more is friction that keeps the graph empty.

## Decision

- The CLI package is named `brain` (crates/brain-cli), so
  `cargo install --locked --git <repo> brain` installs the binary, and
  install.sh wraps that for `curl | sh`.
- The docs pipeline moved into the binary: `brain docs generate`
  (crates/brain-cli/src/docsgen.rs) produces tour.md and the narration
  natively — the narration is computed from graph queries, not parsed from
  text — and carries the capture/TTS helpers embedded
  (crates/brain-cli/assets/capture.mjs, crates/brain-cli/assets/tts.py),
  writing them to a temp dir at run time. Media steps use node+playwright,
  python3, and ffmpeg when present and are skipped gracefully when not:
  the core projection never fails for want of a browser.
- The continuous loop is built in: `brain watch [dir] [--prefix p]
  [--interval s] [--docs]`. scripts/twin_watch.sh remains as a thin
  compatibility wrapper.
- Release builds are lean: thin LTO, one codegen unit, stripped symbols.

## Consequences

- One artifact to distribute, version, and reason about; `brain version`
  reports it.
- Optional runtime tools are honest dependencies of *media*, not of the
  substrate: a bare machine still gets the full graph, tour.md, and
  narration text.
- The embedded helpers version with the binary — no skew between the
  installed tool and its pipeline.
