# Cutting and installing a brain release

Service: brain

## Install (users)

One command, no clone:

```bash
curl -fsSL https://raw.githubusercontent.com/drietsch/brain/main/install.sh | sh
# or: cargo install --locked --git https://github.com/drietsch/brain brain
```

## Release checklist (maintainers)

1. Green suite, protocol imported: `cargo test 2>&1 | brain testrun import - --prefix twin/self`
   (the post-commit hook does this automatically when armed with `--tests`).
2. No rot: `brain twin stale twin/self` is empty, or the flagged docs are
   knowingly historical (executed plans stay flagged — that is correct).
3. Docs are projections: `brain docs generate` and commit `docs/generated/`.
4. Bump `version` in the workspace `Cargo.toml`; commit; push `main`.
5. Verify installability from a clean environment:
   `cargo install --locked --git <repo> brain && brain version`.
6. Sanity: `brain init && brain demo` in a scratch directory, then
   `brain twin refresh . --prefix twin/self && brain twin insights twin/self`
   in the repo.

## Rollback

`cargo install` a previous tag (`--tag vX.Y.Z`); stores are forward-safe —
objects are immutable and unknown observation properties are ignored by
older binaries.
