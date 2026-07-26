#!/bin/sh
# brain — one-command install of the monolithic binary:
#
#   curl -fsSL https://raw.githubusercontent.com/drietsch/brain/main/install.sh | sh
#
# Builds from source via cargo (the binary is a single static-ish executable;
# media capture/TTS at runtime use node+playwright / python3 when present,
# and are skipped gracefully otherwise). Override the source with BRAIN_REPO.
set -eu

REPO="${BRAIN_REPO:-https://github.com/drietsch/brain}"

if ! command -v cargo >/dev/null 2>&1; then
  echo "brain needs a Rust toolchain (cargo not found)."
  echo "Install one first:  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  exit 1
fi

echo "installing brain from $REPO ..."
cargo install --locked --git "$REPO" brain

BIN="$(command -v brain || echo "$HOME/.cargo/bin/brain")"
echo
echo "installed: $BIN ($("$BIN" version))"
echo
echo "get started:"
echo "  brain init                          # store + deliverable templates"
echo "  brain twin refresh . --prefix twin/self"
echo "  brain twin insights twin/self"
echo "  brain docs generate                 # always-up-to-date docs"
