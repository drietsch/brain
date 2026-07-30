# Eyes

Eyes is the visual layer over everything the brain knows. It runs on your
machine, reads the graph, and writes nothing.

```bash
brain eyes --prefix twin/self          # prints the local URL
```

The CLI is built for agents: precise, fast, one answer per command. Eyes
is built for people, and it does three things a terminal does badly.

**It reads the graph's judgments out loud.** Everything the brain
concludes — this document drifted, these tests fail, this change never
finished, this feature claims more than it can show — arrives as a
sentence with its reason attached and the command that resolves it.
Nothing says `hub 29` or `cursor 5.934`; hashes and identifiers live under
a details disclosure.

**It makes the content readable, and the evidence visible.** A decision is
a document you read. A failing browser test shows the screenshot it left
behind. A generated screenshot names the command that drew it. The
narrated tour plays, with its chapters, its script, and an honest note
when the recording no longer matches the graph.

**It shows the shape of the system.** The Map answers where the risk is;
the MRI draws the whole anatomy in three dimensions, stacked by what
depends on what.

**Everything relates to a feature, and the path is checkable.** A feature
declares its files; the twin already records that a test covers them, a
document mentions them, a session edited them, a change targets them. So
every test, decision, session and governed change says which feature it
serves and *how it was reached* — "it changes `crates/brain-eyes/src/http.rs`,
which this feature is built by". Nothing infers a feature or a part from
a path (ADR-030).

## The surfaces

| | Question it answers |
|---|---|
| **Now** | Should I worry, and what changed since I left? |
| **Next** | What should happen now, worst first — the same queue agents read? |
| **Work** | Who is working here, what became of it, and what is unfinished? |
| **Roadmap** | What is planned, what is moving, and what is done? |
| **Features** | What do we claim, and what actually backs it? |
| **Tests** | What passed, what failed, and what did the failure look like? |
| **Artifacts** | Show me everything the brain holds, so I can read it |
| **Evidence** | What is claimed, and what stands behind it? |
| **History** | What happened? |
| **Map** | What is this made of, and where is the risk? |
| **MRI** | What shape is this system, and what is moving? |
| **Search** (⌘K) | Take me to X |

**Now** opens with the **census**: every claim the graph makes, one mark
each, grouped by kind. It is the dimension strip read at the scale of the
whole system, and it answers the product's own question in one line —
*how much of what this thing asserts can it actually show?* Below it: what
needs a person (identical concerns collapsed and counted), then what moved
since your last `brain sleep`, then where the pressure is.

**Work** shows the coding-agent sessions that ran in this workspace: what
each was asked to do, which model, how long, which files it edited, and
what it produced. This is the graph's only record of a principal
(ADR-025); import it with `brain sessions import`.

**Roadmap** reads down the spine: each stage, the features planned for
it, and the work in flight against each one. A stage's state is never
derived from its features — Stage 1 is a research question, and four
finished features do not answer it — so the stage says what was recorded
about it and the features say what they can show. Stages are authored,
never parsed out of `docs/roadmap.md`.

**Features** shows features and their parts as a tree. A feature with
parts is judged by its parts — readiness rolls up from the leaves and is
never set by hand (ADR-028) — and the row says which part is holding it
up. Every row carries a **dimension strip**: one cell per part, or one per
requirement for a leaf, at three scales from a seven-pixel row indicator
to a labelled bar in the dossier. Clicking a labelled cell names the
records behind it and what each one currently says.

The page opens with the **coverage census**: how much of the graph belongs
to any feature, per kind, with what nothing claims named rather than
rounded away. It answers a different question from the census on Now —
that one asks whether a claim can show its proof, this one whether a
record is claimed at all.

**Tests** lists every recorded case with its verdict and its history,
every imported run with what it named, and every file the twin classified
as holding tests. A Playwright failure carries its error, its duration,
its retries and its screenshot.

**Artifacts** has a shelf per kind — decisions as a reading list, plans,
documents, agent rules, **Pictures & recordings** for media and the tour,
and **Concepts**, the kind registry explaining the brain's own vocabulary
with what template fitness learned about each contract.

**Evidence** is claim on the left, proof on the right, unsupported first.
A claim is never shown stronger than what backs it.

**MRI** draws every entity, laid out once per graph version on the server
so the anatomy never rearranges while you read it. Height is dependency
depth. Colour is the lens you choose. Motion means something happened.
Nothing is hidden as you zoom — detail resolves (ADR-026).

The **inspector** opens beside any surface without navigating away, and
**Brief me** reads the current screen aloud using the browser's own
speech synthesis — the sentences are the ones `say.rs` wrote, so nothing
is generated and nothing is written.

## How it reads

**Type encodes epistemology** (ADR-029). What the brain concluded is set
in a serif; what was literally recorded — paths, hashes, commands, counts
— is monospace; the interface itself is sans. You can see which parts of a
screen are interpretation and which are evidence before reading a word.

Colour is never load-bearing: state is a shape too. Violet marks the state
that matters most here — *claimed, but nothing establishes it*.

Every list filters in place, with facet counts that respect the other
filters. Three ways down, kept distinct: **peek** a row to open the
inspector beside it, **push** it (Enter) for the full page, **expand** it
to see its parts in the same grid.

## What Eyes deliberately does not do

- **It never writes.** No approvals, no edits, no mutations — it shows you
  the command and you decide. Governance stays behind the capability and
  intent/receipt boundary.
- **It never invents.** Concepts the graph does not model produce no
  panels and no explanations of their absence. There are no approval
  queues, because the graph has none.
- **It never presents a guess as a record.** Anything Eyes worked out
  rather than read — the CLI command equivalent to a past change, for
  instance — is labelled as reconstructed.
- **It never composes its own wording.** All prose comes from `say.rs` on
  the server, so the CLI and Eyes cannot drift apart. A test fails the
  build if machine vocabulary reaches a human surface.

## How it stays fast and truthful

One `Store` and one warm `Cortex` are held for the life of a graph
version. Freshness is a `stat` on the append-only event log. Everything
derived — insights, attention, coherence findings, the kind registry,
fitness, the event scan, the MRI layout — is computed once per version
through the existing `brain_observe` functions and shared, never
re-implemented. Every response names the snapshot it was computed from —
including how the working tree relates to it: uncommitted files the graph
has not seen yet are counted in a topbar badge, re-measured on a short
leash, so no number quietly poses as current.

The personal layer never moves state to the server: the browser keeps the
viewer's last-visit cursor and week-scoped acknowledgements (restorable),
and sends the cursor back as a query parameter so the "since you last
looked" sentence is still composed in the one voice, server-side.

Eyes binds to `127.0.0.1`, answers only GET, serves file bytes only
through a graph-recorded path that resolves inside the workspace, sends
everything that is not media as plain text, and supports byte ranges so
audio and video can be played and seeked.

## Architecture

ADR-023 sets the read-only boundary. ADR-024 says what Eyes is for.
ADR-025 makes agent sessions first class. ADR-026 amends ADR-024's ban on
whole-graph drawing with the conditions under which it is honest.
ADR-027 covers evidence you can look at. ADR-028 gives features parts.
ADR-029 is the design system: type encodes epistemology. ADR-030 makes
the feature the spine. `design-draft/`
remains a visual and conceptual reference whose palette, geometry and
coverage strip this build adopts — a source of ideas, never an
information architecture to reproduce.
