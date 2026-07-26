#!/usr/bin/env python3
"""Stage 1 authoring experiment driver.

Asks a Claude model to author (or edit) programs in the brain core calculus,
checks each emission with `brain task check` (which records Evidence in the
graph), and tallies the metrics from docs/authoring.md:

  emission validity  - did the emission parse as JSON / as a term?
  case pass rate     - did the term satisfy the task's cases?
  repair rate        - given the failure report, did the next emission fix it?
  edit locality      - for edit tasks, how much of the base term survived?

Two ways to supply the model:

1. **Agent-in-the-loop (preferred, no API):** the coding agent driving this
   repository IS the authoring model. It reads the task, writes the emission
   to a file, and scores it:

     python3 scripts/authoring_experiment.py --score emission.json \
         --model coding-agent tasks/t04-abs.json
     python3 scripts/authoring_experiment.py --score emission.json \
         --model coding-agent --edits tasks/edits/e01-greet-excited.json

2. **API mode:** calls the Anthropic API for each task (requires
   ANTHROPIC_API_KEY or an `ant auth login` profile):

     python3 scripts/authoring_experiment.py tasks/t0*.json
     python3 scripts/authoring_experiment.py --edits tasks/edits/*.json

  `--dry-run` prints the prompts and exits (also useful as the exact prompt
  to hand an agent in mode 1).

Build the checker first: `cargo build -p brain-cli` (or set BRAIN_BIN).

Note: schema-constrained decoding is deliberately NOT used. The term schema is
recursive (a term contains terms), and the structured-outputs feature does not
support recursive schemas — so emissions are plain JSON and validity is a
measured outcome, which is what this experiment is for.
"""

import argparse
import json
import os
import pathlib
import subprocess
import sys
import time
from collections import Counter

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCHEMA_PATH = ROOT / "docs" / "schema" / "term.schema.json"
RESULTS_DIR = ROOT / "results"

DEFAULT_MODEL = "claude-opus-5"

BUILTINS = """\
Available foreign symbols (called via {"op": "foreign", "symbol": ..., "arg": ...};
arguments are records with the listed fields):
  core/add    {a: int, b: int} -> int
  core/sub    {a: int, b: int} -> int
  core/mul    {a: int, b: int} -> int
  core/lt     {a: int, b: int} -> bool        (a < b)
  core/eq     {a, b} -> bool                  (structural equality)
  core/if     {cond: bool, then: T, else: T} -> T   (eager: both branches evaluate)
  core/concat {a: str, b: str} -> str
There is no recursion and there are no lists. Solutions must be a single unary
function: a term with op "lam" whose body computes the result from the parameter.
"""

SYSTEM_PROMPT = """\
You author programs for an agent-native substrate. Programs are terms of a tiny
calculus, written as JSON against the provided schema. Respond with ONLY the
JSON term - no prose, no markdown fences, no explanation. If you are uncertain
about a subterm, you may emit a typed hole: {"op": "hole", "id": "...", "expected": "..."}.
"""


def author_prompt(task: dict, schema_text: str) -> str:
    parts = [
        f"Task: {task['description']}",
        f"Spec: {json.dumps(task.get('spec', {}))}",
        f"Example cases (arg -> expected result): {json.dumps(task['cases'])}",
        BUILTINS,
        "Term JSON Schema:",
        schema_text,
        "Emit the solution term now (JSON only).",
    ]
    return "\n\n".join(parts)


def edit_prompt(task: dict, schema_text: str) -> str:
    parts = [
        "You are EDITING an existing program. Change request: " + task["description"],
        "Base program (a term of the calculus):",
        json.dumps(task["base_term"], indent=2),
        f"After the edit, these cases must hold (arg -> expected): {json.dumps(task['cases'])}",
        "Make the smallest modification that satisfies the change request - "
        "preserve every subterm you do not need to touch.",
        BUILTINS,
        "Term JSON Schema:",
        schema_text,
        "Emit the full edited term now (JSON only).",
    ]
    return "\n\n".join(parts)


def repair_prompt(previous_emission: str, failure: str) -> str:
    return (
        "Your previous emission failed checking.\n\n"
        f"Previous emission:\n{previous_emission}\n\n"
        f"Checker output:\n{failure}\n\n"
        "Emit a corrected term now (JSON only)."
    )


def extract_json(text: str) -> str:
    """Tolerate accidental code fences; anything further is an invalid emission."""
    t = text.strip()
    if t.startswith("```"):
        t = t.split("\n", 1)[1] if "\n" in t else t
        if t.rstrip().endswith("```"):
            t = t.rstrip()[: -3]
    return t.strip()


def brain_cmd() -> list[str]:
    override = os.environ.get("BRAIN_BIN")
    if override:
        return [override]
    built = ROOT / "target" / "debug" / "brain"
    if built.exists():
        return [str(built)]
    return ["cargo", "run", "-q", "-p", "brain-cli", "--"]


def check(task_path: pathlib.Path, term_path: pathlib.Path) -> tuple[bool, str]:
    proc = subprocess.run(
        brain_cmd() + ["task", "check", str(task_path), str(term_path)],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    return proc.returncode == 0, (proc.stdout + proc.stderr).strip()


def term_nodes(term) -> int:
    """Number of calculus nodes (dicts with an "op") in a term — the size ladder."""
    n = 0
    if isinstance(term, dict):
        if "op" in term:
            n += 1
        for v in term.values():
            n += term_nodes(v)
    elif isinstance(term, list):
        for v in term:
            n += term_nodes(v)
    return n


def subtree_multiset(term) -> Counter:
    """Multiset of canonicalized subtrees, for the edit-locality heuristic."""
    out: Counter = Counter()

    def walk(node):
        out[json.dumps(node, sort_keys=True)] += 1
        if isinstance(node, dict):
            for v in node.values():
                walk(v)
        elif isinstance(node, list):
            for v in node:
                walk(v)

    walk(term)
    return out


def edit_locality(base, emission) -> float:
    """Fraction of the emission's subtrees already present in the base.

    1.0 = pure reuse; near 0 = wholesale regeneration. Heuristic, not a proof.
    """
    base_trees = subtree_multiset(base)
    emit_trees = subtree_multiset(emission)
    total = sum(emit_trees.values())
    if total == 0:
        return 0.0
    shared = sum(min(count, base_trees.get(tree, 0)) for tree, count in emit_trees.items())
    return shared / total


def call_model(client, model: str, prompt: str) -> str:
    response = client.messages.create(
        model=model,
        max_tokens=16000,
        system=SYSTEM_PROMPT,
        messages=[{"role": "user", "content": prompt}],
    )
    if response.stop_reason == "refusal":
        raise RuntimeError("model refused the request")
    return "".join(b.text for b in response.content if b.type == "text")


def run_task(client, model: str, task_path: pathlib.Path, is_edit: bool,
             schema_text: str, max_repairs: int) -> dict:
    task = json.loads(task_path.read_text())
    prompt = edit_prompt(task, schema_text) if is_edit else author_prompt(task, schema_text)

    row = {
        "at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "model": model,
        "task": task["name"],
        "mode": "edit" if is_edit else "author",
        "valid_json": False,
        "checker_passed": False,
        "repairs_used": 0,
        "edit_locality": None,
        "detail": "",
    }

    emission_text = call_model(client, model, prompt)
    for attempt in range(max_repairs + 1):
        candidate = extract_json(emission_text)
        try:
            parsed = json.loads(candidate)
            row["valid_json"] = True
        except json.JSONDecodeError as e:
            row["detail"] = f"invalid JSON: {e}"
            parsed = None

        if parsed is not None:
            out_path = RESULTS_DIR / f"{task['name']}-{model}-attempt{attempt}.json"
            out_path.write_text(json.dumps(parsed, indent=2))
            passed, output = check(task_path, out_path)
            row["detail"] = output.splitlines()[-1] if output else ""
            if passed:
                row["checker_passed"] = True
                if is_edit:
                    row["edit_locality"] = round(edit_locality(task["base_term"], parsed), 3)
                break
            failure = output
        else:
            failure = row["detail"]

        if attempt < max_repairs:
            row["repairs_used"] = attempt + 1
            emission_text = call_model(client, model, repair_prompt(candidate, failure))

    return row


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tasks", nargs="*", help="author-mode task files")
    parser.add_argument("--edits", nargs="*", default=[], help="edit-mode task files")
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--max-repairs", type=int, default=1)
    parser.add_argument("--dry-run", action="store_true",
                        help="print the prompts and exit; no API key needed")
    parser.add_argument("--score", metavar="EMISSION",
                        help="score a pre-authored emission file against exactly "
                             "one task (agent-in-the-loop mode; no API)")
    args = parser.parse_args()

    if not args.tasks and not args.edits:
        parser.error("no task files given")
    schema_text = SCHEMA_PATH.read_text()

    jobs = [(pathlib.Path(t), False) for t in args.tasks] + \
           [(pathlib.Path(t), True) for t in args.edits]

    if args.score:
        if len(jobs) != 1:
            parser.error("--score takes exactly one task file")
        path, is_edit = jobs[0]
        task = json.loads(path.read_text())
        raw = pathlib.Path(args.score).read_text()
        is_notation = args.score.endswith(".term")
        if is_notation:
            # Canonicalize via the CLI; validity = the notation parsed.
            proc = subprocess.run(
                brain_cmd() + ["notation", args.score],
                cwd=ROOT, capture_output=True, text=True,
            )
            text = proc.stdout if proc.returncode == 0 else ""
            notation_error = proc.stderr.strip()
        else:
            text = raw
            notation_error = ""
        row = {
            "at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "model": args.model,
            "task": task["name"],
            "mode": "edit" if is_edit else "author",
            "valid_json": False,
            "checker_passed": False,
            "repairs_used": 0,
            "edit_locality": None,
            "detail": "",
            "encoding": "term" if is_notation else "json",
            "emission_bytes": len(raw),
        }
        try:
            parsed = json.loads(extract_json(text)) if text else None
            if parsed is not None:
                row["valid_json"] = True
                row["term_nodes"] = term_nodes(parsed)
            else:
                row["detail"] = f"invalid notation: {notation_error}"
        except json.JSONDecodeError as e:
            parsed = None
            row["detail"] = f"invalid JSON: {e}"
        if parsed is not None:
            passed, output = check(path, pathlib.Path(args.score))
            row["checker_passed"] = passed
            row["detail"] = output.splitlines()[-1] if output else ""
            if passed and is_edit:
                row["edit_locality"] = round(edit_locality(task["base_term"], parsed), 3)
        RESULTS_DIR.mkdir(exist_ok=True)
        with (RESULTS_DIR / "authoring-runs.jsonl").open("a") as f:
            f.write(json.dumps(row) + "\n")
        print(json.dumps(row))
        return 0 if row["checker_passed"] else 1

    if args.dry_run:
        for path, is_edit in jobs:
            task = json.loads(path.read_text())
            prompt = edit_prompt(task, schema_text) if is_edit else author_prompt(task, schema_text)
            print(f"===== {task['name']} ({'edit' if is_edit else 'author'}) =====")
            print(f"[system]\n{SYSTEM_PROMPT}")
            print(f"[user]\n{prompt}\n")
        return 0

    import anthropic  # deferred so --dry-run works without the SDK installed
    client = anthropic.Anthropic()

    RESULTS_DIR.mkdir(exist_ok=True)
    log_path = RESULTS_DIR / "authoring-runs.jsonl"
    rows = []
    for path, is_edit in jobs:
        row = run_task(client, args.model, path, is_edit, schema_text, args.max_repairs)
        rows.append(row)
        with log_path.open("a") as f:
            f.write(json.dumps(row) + "\n")
        status = "PASS" if row["checker_passed"] else "FAIL"
        loc = f"  locality={row['edit_locality']}" if row["edit_locality"] is not None else ""
        print(f"{status}  {row['task']:<24} repairs={row['repairs_used']}{loc}  {row['detail']}")

    total = len(rows)
    print(f"\n{sum(r['valid_json'] for r in rows)}/{total} valid emissions, "
          f"{sum(r['checker_passed'] for r in rows)}/{total} passed, "
          f"{sum(r['repairs_used'] > 0 and r['checker_passed'] for r in rows)} repaired; "
          f"log: {log_path}")
    return 0 if all(r["checker_passed"] for r in rows) else 1


if __name__ == "__main__":
    sys.exit(main())
