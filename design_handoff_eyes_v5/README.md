# Handoff: Eyes v5 — enterprise UX/UI rework

Target codebase: **drietsch/brain**, `crates/brain-eyes` (branch
`claude/aztechement-detailed-review-upo8tb`). One Rust binary, GET-only
server, vanilla-JS client (`assets/app.js` + `assets/list.js`), one
stylesheet, no framework, no build step. All work stays inside those
constraints.

## About the design files

- `Eyes Rework.dc.html` is a **design reference / clickable prototype in
  HTML** — it shows intended look and behavior for every surface. Do NOT
  ship it. Recreate its decisions inside the existing vanilla-JS client
  and, where noted, the Rust server (`say.rs` owns all people-facing
  sentences; the client never composes wording).
- `assets/` here is different: these four files ARE production drop-ins
  for `crates/brain-eyes/assets/` (Phase 1 below). They keep every class
  name and DOM contract of the current client.
- `INTEGRATION.md` documents exactly what those drop-ins change.

## Fidelity

High-fidelity. Colors, type, spacing, radii, shadows, motion and copy in
the prototype are final intent. Where the prototype and the drop-in
`styles.css` disagree on a token value, `styles.css` wins (it is mapped
onto the codebase's real variable names).

## Phased plan (one PR per phase, in order)

### Phase 1 — assets drop-in (no logic changes; ~minutes)
Copy `assets/{styles.css,index.html,app.js,icons.svg}` over
`crates/brain-eyes/assets/`. Contains four bug fixes that must not be
lost even if styling is debated:
1. `.census-cell` states failing/stale/absent (previously rendered as
   proven green — the spine could not show a failing claim);
2. Brief-me selector list now includes `.hero, .verdict-sub,
   .census-line, .concern h3, .concern p`;
3. `protocolRow` uses `.bar-split` (the pass/fail/skip bar was invisible);
4. `.speakable` no longer permanently outlined; `settleCensus` fill-mode
   `backwards` so census hover works.
Acceptance: existing Playwright suite green; both themes; plain view.

### Phase 2 — affordances + components (app.js/list.js + styles.css)
Reference: prototype's Now / Proof / Work views.
- Link affordance system: navigating rows/chips get hover wash + a `›`
  that nudges 2px right (`--t-nudge`); fold controls get a quieter hover;
  text toggles underline on hover.
- Sparkrow: soft area fill under each spark (opacity .07 of line ink);
  tempo strip under the line — one block per interval, width ∝ real
  elapsed time between points (`QualityLine.points` neighbors), 3-4px
  tall; title text: "each block is the real time between two moves".
- Pressure list → 3-column mini-table: rank · path · churn bars (5 steps,
  fault ink past 15 changes) / reach count / hollow dashed fault ring for
  "no test names it". Prose reasons move to title/aria-label.
- list.js facet bar: label each facet group (RESULT, FRAMEWORK…) in
  10px/700 tracked caps; groups separated by a 1px divider; counts inside
  the chip.
- Episode rows get kind glyphs (session=dot, change=rotated hollow
  square, tests=fault square, doc=dashed square) + a narrow time gutter.
- Document dossiers: markdown body renders through the existing
  `markdown()` into `.body-view`, with an excerpt/"Show the whole
  document" fold (server sends `truncated`; add a `?full=1` body fetch or
  ship full text and clamp client-side).

### Phase 3 — IA consolidation (http.rs routes + say.rs + app.js views)
Reference: prototype rail + Now/Proof/Time/Structure views.
- Rail 13 → 7: Now (absorbs Next's queue — each concern carries a
  horizon: "now" / "can wait"), Work, Roadmap, Features,
  Proof (Tests·Evidence·Artifacts as tabs; Artifacts keeps shelves incl.
  Media→Tour and Concepts), Time (Timeline + Compare: any moment row
  enters compare mode with an as-of banner + "Back to live"),
  Structure (Map default + MRI as a lens, not a place).
- Rail groups Operate/Plan/Prove/Explore (already in Phase 1 index.html).
- Verdict band = top of queue: when needs_you[0] is severity `act` and
  matches the headline, render it expanded in the band; queue starts at
  the second card. Sentences come from say.rs.
- Badge semantics: one rule — the count is "things needing a decision",
  tinted by worst severity; identical meaning on every rail item.

### Phase 4 — drill model + dossier (thing.rs payload order + app.js)
Reference: prototype Dossier view.
- Retire the reflowing inspector column: everything navigates
  (`#thing?id=`); peeks become the dossier page. Keep back = history.
- Dossier reranks: noun eyebrow → title + state chip → one-sentence lede
  → "Before you edit" briefing → kind-specific sections (stages+diff for
  changes; parts for features; result history + attachments for tests;
  markdown body for documents) → Around it as three compact columns →
  History; sticky right sidebar: At a glance (4 stats), What it serves,
  The command, Machine detail. Cap content at 760px.

## Design tokens (authoritative; already in assets/styles.css)

- Neutrals: one oklch ramp, hue 265 — light: ink .215/.02, body .39/.022,
  meta .55/.022, muted .68/.018, faint .78/.014, line .912/.009,
  page .944/.007; dark is an elevation model: page .185 → paper .235 →
  hover .27 (surfaces lighten as they rise).
- Severity at equal L/C: proof oklch(.55 .13 156), signal (.55 .13 80),
  fault (.55 .16 27), trace/accent (.55 .17 275), unproven (.55 .15 300);
  dark L≈.70-.75, C≈.12-.14. Fills at L .955, edges at L .88.
- Accent gradient: linear-gradient(120deg, oklch(.6 .17 275),
  oklch(.62 .17 302)) — primary buttons + brand mark only.
- Shadow: 0 0 0 1px rgb(22 25 35/.02), 0 1px 2px /.06, 0 5px 14px /.045.
- Motion: --t-settle .24s cubic-bezier(.2,.7,.3,1) for enter;
  --t-nudge .12s ease for hover; breathe 2.4s for live. Nothing else
  moves; all inside prefers-reduced-motion guards.
- Type: --judgment "Source Serif 4" (self-host one variable woff2; the
  @font-face stub is commented at the end of styles.css), --ui system
  sans 13.5px base, --record ui-monospace with tabular-nums. Verdict
  clamp(30px, 2.6vw, 38px); page titles 26px serif; section labels
  10.5px/700 tracked caps.

## Assets

`assets/icons.svg` = the repo's own 34-grid stroke vocabulary plus new
interface marks: i-copy, i-voice, i-stop, i-dismiss, i-undo, i-sun,
i-moon, i-bolt. Icons ≤13px render at stroke-width 3, else 2.4.

## Suggested Claude Code prompt (run in the repo root)

> Read design_handoff_eyes_v5/README.md and INTEGRATION.md, then open
> Eyes Rework.dc.html in a browser as the visual spec. Execute Phase 1
> exactly (copy the four asset files over crates/brain-eyes/assets/),
> run the Playwright suite and the vocabulary/CSS-class build tests, and
> stop for review. Then propose a diff plan for Phase 2 before writing
> code. Never compose user-facing sentences in the client — new wording
> goes through say.rs. Keep both themes, plain view, keyboard focus and
> prefers-reduced-motion working in every phase.

## Files in this bundle

- `Eyes Rework.dc.html` — the clickable prototype (all 9 surfaces)
- `assets/` — Phase-1 production drop-ins
- `INTEGRATION.md` — line-level notes for the drop-ins
