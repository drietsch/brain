# Eyes

Eyes is the visual layer over everything the brain knows. It runs on your
machine, reads the graph, and writes nothing.

```bash
brain eyes --prefix twin/self          # prints the local URL
```

The CLI is built for agents: precise, fast, one answer per command. Eyes is
built for people, and it does two things a terminal does badly.

**It reads the graph's judgments out loud.** Everything the brain concludes —
this document drifted, these tests fail, this change never finished, this
feature claims more than it can show — arrives as a sentence with its reason
attached and the command that resolves it. Nothing says `hub 29` or
`cursor 5.934`; hashes and identifiers live under a details disclosure.

**It makes the content readable.** A decision is a document you read, with its
status, what it governs and what replaced it. So are plans, runbooks, agent
rules, test results and assets.

## The six surfaces

| | Question it answers |
|---|---|
| **Now** | Should I worry, and what changed since I left? |
| **Library** | Show me everything the brain knows, so I can read it |
| **Map** | What is this system made of, and where is the risk? |
| **Thing** | What is this, can I trust it, what breaks if I touch it? |
| **Timeline** | What happened? |
| **Find** (⌘K) | Take me to X |

**Now** leads with the worst true thing, then the short list that needs a
person, then what moved since your last `brain sleep`.

**Library** has a shelf per kind, each presented in the shape that kind
deserves: decisions as a reading list, features as coverage strips, tests as
results with flake history, and **Concepts** — the kind registry explaining the
brain's own vocabulary, including what template fitness learned about whether
each contract actually works.

**Map** aggregates files into modules, stacks them by dependency direction, and
colours them by one question: where the pressure is, what is covered, what
moved recently. Click a block to see its files.

**Thing** puts the body first, then the judgments about it, then a
neighbourhood laid out so position is the message — what it uses on the left,
what depends on it on the right, tests and documents around it. Governed
changes show their stage; features show their definition of done; decisions
show what they replaced; tests show every time they changed their mind.

## What Eyes deliberately does not do

- **It never writes.** No approvals, no edits, no mutations — it shows you the
  command and you decide. Governance stays behind the capability and
  intent/receipt boundary.
- **It never draws the whole graph.** A picture of a thousand nodes says only
  "it is complicated". Drawings are aggregated or local, and their geometry
  carries meaning.
- **It never invents.** Concepts the graph does not model produce no panels and
  no explanations of their absence. Numbers it cannot verify, it does not
  claim: a file body served from the workspace says whether the graph had a
  hash to check it against.
- **It never composes its own wording.** All prose comes from `say.rs` on the
  server, so the CLI and Eyes cannot drift apart.

## How it stays fast and truthful

One `Store` and one warm `Cortex` are held for the life of a graph version.
Freshness is a `stat` on the append-only event log. Everything derived —
insights, attention, coherence findings, the kind registry, fitness, the event
scan — is computed once per version through the existing `brain_observe`
functions and shared, never re-implemented. Every response names the snapshot
(`HEAD` plus event cursor) it was computed from.

Eyes binds to `127.0.0.1`, answers only GET, serves file bytes only through a
graph-recorded path that resolves inside the workspace, and sends everything
that is not media as plain text.

## Architecture

ADR-023 sets the read-only boundary. ADR-024 says what Eyes is for and why the
whole-graph view was removed. `design-draft/` remains a visual and conceptual
reference — a source of ideas, never an information architecture to reproduce.
