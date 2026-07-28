# Agent sessions are first class

Status: accepted

## Context

Everything the brain records answers *what changed*. Nothing answered
*who did it*.

An `Intent` has four fields — action, argument digest, capability,
timestamp — and a principal is not among them. A `Receipt` has four more,
and neither is an actor. An `Observation` carries a `source`, but the
values are mechanism names (`twin`, `govern`, `testrun`, `docsgen`), not
people or programs. Git author is fetched during backfill and
deliberately discarded. `Object::Capability` is defined with a `scope`
map and has never been constructed anywhere in the workspace.

That was defensible while the brain observed a repository. It stopped
being defensible once the repository was written almost entirely by
coding agents. "Which agent touched this, what was it asked to do, and
did it leave the documentation behind?" is the first question a person
asks about a change they did not make — and the graph could not answer any
part of it.

The Work surface made the gap concrete: with no sessions, it could only
list governed changes and open plans, which in a healthy repository is
usually nothing at all.

## Decision

**A coding-agent session is an entity, and it is the graph's record of a
principal.**

1. **Read the transcripts the agents already keep.** Claude Code writes
   `~/.claude/projects/<slug>/<session>.jsonl`; Codex writes
   `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. `brain sessions
   import` reads them into `agent_session` entities with `objective`,
   `started_at`, `ended_at`, `turns`, `tools`, `model`, `files_touched`,
   and `touched` relations to the files they edited.

2. **Scope by working directory.** A session belongs to a prefix only if
   it ran inside that workspace. The check reads the transcript's opening
   records and stops there — most transcripts on a machine belong to other
   projects, and reading all of them in full cost 46 seconds where reading
   headers costs 1.7.

3. **The conversation never enters the graph.** Only the truncated first
   instruction, counts, tool *names* without their arguments, the paths of
   files that were edited, and timings. Not prompts, not responses, not
   tool output, not file contents. A transcript is a private working
   record; the graph gets the shape of the work, not its text. A test
   asserts that a distinctive phrase from the body of an instruction is
   absent from every object in the store.

4. **Reads are not touches.** Only `Edit`, `Write`, `NotebookEdit` and
   `apply_patch` contribute file links. A session that reads two hundred
   files and edits three has a blast radius of three.

5. **Tool results are not turns.** A tool's output comes back as a user
   record in Claude Code's format; counting them reported 740 instructions
   for a session that received 17. Only records carrying human text count,
   and harness-injected context blocks are excluded by shape.

6. **Idempotence is keyed on transcript length *and parser version*.**
   Re-importing an unchanged session writes nothing. Bumping
   `PARSER_VERSION` re-reads everything, which is what let a fix to the
   objective extraction reach sessions that had already been imported —
   the same reason `brain twin refresh --full` exists.

## Consequences

- Work becomes the richest surface instead of the emptiest, and the MRI
  gains a `work` cluster whose nodes pulse while a session is running.
- "What did this agent produce?" is derived, not stored twice: an artifact
  whose file a session edited is an artifact that session produced.
- Historical intents and receipts still have no actor. They will say so
  rather than guess — see ADR-027 on recorded versus reconstructed.
- The graph now depends on two external file formats it does not control.
  Both are parsed defensively: unknown records are skipped, a malformed
  line ends the read, and a transcript that yields no timestamps yields no
  session.
- This is deliberately *not* an authorization model. A session records who
  worked; it grants nothing and gates nothing. Approvals remain unmodelled.
