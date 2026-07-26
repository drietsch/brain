#!/usr/bin/env bash
# Compatibility wrapper: the watch loop now lives inside the monolithic
# binary. Prefer:  brain watch [dir] [--prefix p] [--interval s] [--docs]
set -euo pipefail
DIR=${1:-.}
PREFIX=${2:-twin/self}
INTERVAL=${3:-60}
BRAIN=${BRAIN_BIN:-target/debug/brain}
exec "$BRAIN" watch "$DIR" --prefix "$PREFIX" --interval "$INTERVAL"
