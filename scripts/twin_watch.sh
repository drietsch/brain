#!/usr/bin/env bash
# Continuous twin loop: refresh the twin and print insights on an interval.
# The sense organ for agentically-built software - run it beside your agent
# sessions and the graph keeps a live, queryable picture of what they build.
#
# Usage: scripts/twin_watch.sh [dir] [prefix] [interval-seconds]
set -euo pipefail
DIR=${1:-.}
PREFIX=${2:-twin/self}
INTERVAL=${3:-60}
BRAIN=${BRAIN_BIN:-target/debug/brain}

while true; do
  echo "--- $(date -u +%H:%M:%SZ) refresh ---"
  "$BRAIN" twin refresh "$DIR" --prefix "$PREFIX"
  "$BRAIN" twin insights "$PREFIX"
  sleep "$INTERVAL"
done
