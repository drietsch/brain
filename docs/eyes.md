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

**It shows the shape of the system.** The Map answers where the risk is —
its risk lens composes how often a file changes, how widely it is
imported, and whether any test would catch a break, weighted up when the
documents about it have drifted too; the MRI draws the whole anatomy in
three dimensions, stacked by what depends on what.

**Search** ranks with the graph, not just labels: a symbol's name finds
the file that declares it, a session's note vouches for its subject, and
a widely imported file outranks a leaf — the same answer `brain find`
gives, with every hit saying why it matched.

**Everything relates to a feature, and the path is checkable.** A feature
declares its files; the twin already records that a test covers them, a
document mentions them, a session edited them, a change targets them. So
every test, decision, session and governed change says which feature it
serves and *how it was reached* — "it changes `crates/brain-eyes/src/http.rs`,
which this feature is built by". Nothing infers a feature or a part from
a path (ADR-030).

## The surfaces

Seven addresses, grouped Operate / Plan / Prove / Explore — the old
thirteen still resolve: every retired hash redirects to the surface
that absorbed it.

| | Question it answers |
|---|---|
| **Now** | Should I worry, what changed since I left, and what should happen next? (absorbed the queue — each concern wears a horizon: decide now, or it can wait) |
| **Work** | Who is working here, what became of it, and what is unfinished? |
| **Roadmap** | What is planned, what is moving, and what is done? |
| **Features** | What do we claim, and what actually backs it? |
| **Proof** | What stands behind it? Three registers as tabs: Tests, Evidence, and Artifacts (shelves, including Media→Tour and Concepts) |
| **Time** | What happened, and what was true then? (the Timeline is the place; Compare is a mode of it, entered from any moment) |
| **Structure** | What is this made of, and where is the risk? (the Map is the place; the 3D MRI is a lens on the same anatomy) |
| **Search** (⌘K) | Take me to X |

One rule on every rail badge: the count is things needing a decision,
tinted by the worst severity among them.

**Now** opens with the **verdict band** — one instrument taking one
reading, on its own ground. In it: the verdict sentence; the **claim
spine** — every claim the graph makes as one mark on one line, grouped
by kind, solid where proven, hollow where not, the dimension strip read
at the scale of the whole system; the **sparkrow** — tests passing,
features ready, feature claims, documents in doubt as four trends on a
single baseline, each ending in an arrow, where the derivative is the
alarm (a falling line speaks in fault ink; a rising one is a footnote;
moves too small to mean anything read flat); and the **trust stamp** —
how fresh the reading is, whether the working tree has moved past it,
and the standing read-only promise in one quiet line, so on Now the
topbar's own whisper of those facts stands down. Readings accrue in the
graph only when a refresh or a sleep found the numbers moved, so the
series is bounded by change, not by time.

Below the band the weight decays into two columns: what **needs you**
(identical concerns collapsed and counted; an empty desk says "Nothing
needs you" out loud) carries the wide column, while the delta since
your last `brain sleep` and the **pressure** — a ranked list, because a
ranking should look like one — keep to the side.

**Work** leads with the control room's two warnings when they fire: a
collision (two live sessions converging on the same file) and a stall (a
live session running long with nothing written). Both are derived from
imported transcripts, so each sentence carries its own caveat — the
picture is as fresh as the last `brain sessions import`; watching
transcripts live remains future work. Then the approvals desk when a
governed change is waiting:
the recorded diff (unfolding beneath its one-line summary), the pre-apply
briefing of the target file — what an apply would reach, what covers it,
what past sessions learned there — and the exact `brain change apply`
command to copy. Eyes renders the decision; the CLI executes it, so the
audit trail never forks. Below it: the coding-agent sessions that ran in
this workspace — what each was asked to do, which model, how long, which
files it edited, and what it produced. This is the graph's only record
of a principal (ADR-025); import it with `brain sessions import`.

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

**Compare** puts two moments side by side. The picker is keyed by cause,
not by clock — and when history grows long it trims its commit tail (a
named baseline always survives the trim, and the headline counts what
was dropped): the commits the twin saw as HEAD, and named baselines
(`brain baseline add <prefix> <name>` records one; the surface renders
the command for the moment you are looking at). A past view opens with a
loud banner restating its own moment, and closes with what a past moment
honestly cannot show — the working tree and what needs attention are
only measurable now, so they are left out rather than guessed. The diff
is feature-level, regressions first, a readiness flip always above a
slipped check count; tests, feature readiness, and file counts get
then-and-now rows. The past is judged by today's definition of done.

**MRI** draws every entity, laid out once per graph version on the server
so the anatomy never rearranges while you read it. Height is dependency
depth. Colour is the lens you choose. Motion means something happened.
Nothing is hidden as you zoom — detail resolves (ADR-026).

The **plain register** is the same facts told for a stakeholder: one
topbar toggle and the nav recedes to Roadmap, Features, and the Tour
while operator affordances — commands, badges — disappear. Nothing is
computed differently; it is a telling, not a filter, and it is
per-viewer in the browser like the theme. The cockpit also reads on a
phone: below 700 px the rail becomes a scrollable strip and wide
instruments scroll inside their own frame — the page never pans
sideways.

**Everything navigates.** There is no side-inspector: a census mark, a
chip, a table row, an MRI node — each opens the thing's own dossier
page, and Back is the browser's own history. The **dossier** leads with
what a person about to act needs — the noun, the title and state, then
"Before you edit" — with the vitals held in a sticky sidebar: At a
glance, What it serves, The command, Machine detail.

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
- **It never composes its own wording.** All prose comes from
  `crates/brain-eyes/src/say.rs` on
  the server, so the CLI and Eyes cannot drift apart. A test fails the
  build if machine vocabulary reaches a human surface.

## How it stays fast and truthful

One `Store` and one warm `Cortex` are held (in
`crates/brain-eyes/src/state.rs`) for the life of a graph
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

A past moment costs no second store and no replay: as-of reads are
filters over the same warm index, so Compare recomputes per request and
caches nothing.

The client answers to a browser, not only to the compiler: the suite in
`e2e/eyes.spec.ts` drives the running cockpit through Chrome — every
surface, the search, the plain register, Compare, the approvals desk —
and fails any test whose console was not clean (its first run caught
the missing favicon 404 that had dirtied every session). Run it with
`cd e2e && npm install && npx playwright test`; the report imports with
`brain testrun import e2e/test-results/eyes-report.json`, so the
browser evidence lands in the graph beside every other protocol.

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
