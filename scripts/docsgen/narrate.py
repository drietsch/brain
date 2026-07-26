#!/usr/bin/env python3
"""Narration script for the docs screencast, derived from graph queries.

Reads the captured section outputs and speaks the numbers that matter —
so the audio track is as regenerated (and as trustworthy) as the text.
"""
import json
import re
import sys


def main() -> None:
    prefix, sections_path = sys.argv[1], sys.argv[2]
    with open(sections_path) as f:
        sections = {s["id"]: s["text"] for s in json.load(f)}

    lines = []
    lines.append(
        f"This is the live tour of {prefix.replace('/', ' ')}, "
        "generated directly from the semantic graph."
    )

    ins = sections.get("insights", "")
    m = re.search(r"files: (\d+) present.*symbols: (\d+)\s+relations: (\d+)", ins)
    if m:
        lines.append(
            f"The twin currently tracks {m.group(1)} files, "
            f"{m.group(2)} symbols, and {m.group(3)} relations."
        )
    m = re.search(r"tests: (\d+) test file\(s\), (\d+) declared(.*)", ins)
    if m:
        run = re.search(r"(\d+)/(\d+) passed, (\d+) failed", m.group(3))
        if run:
            verdict = "all passing" if run.group(3) == "0" else f"{run.group(3)} failing"
            lines.append(
                f"{m.group(2)} tests are declared; the last imported run had "
                f"{run.group(1)} of {run.group(2)} passing — {verdict}."
            )
        else:
            lines.append(f"{m.group(2)} tests are declared.")

    matrix = sections.get("matrix", "")
    rows = [l for l in matrix.splitlines()[1:] if l.strip()]
    if rows:
        done = sum(1 for l in rows if l.rstrip().endswith("✓"))
        lines.append(
            f"The feature matrix shows {len(rows)} registered features, "
            f"{done} of them meeting the full definition of done."
        )

    adrs = sections.get("decisions", "")
    n_adrs = len([l for l in adrs.splitlines() if l.strip().startswith("[")])
    if n_adrs:
        lines.append(
            f"{n_adrs} architecture decisions are recorded, each linked to "
            "the files it concerns."
        )

    stale = sections.get("stale", "")
    if stale and "no stale docs" not in stale:
        lines.append("Some documents have gone stale and need attention.")
    else:
        lines.append("No documentation is stale: every doc is newer than the files it mentions.")

    lines.append(
        "Everything you just heard was a query, not prose — "
        "regenerate this tour any time with one command."
    )
    print("\n".join(lines))


if __name__ == "__main__":
    main()
