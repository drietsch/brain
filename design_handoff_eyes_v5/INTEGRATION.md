# Integration — the v5 visual system into brain-eyes

Drop-in files for `crates/brain-eyes/assets/`. Same class names, same DOM
contract, no build step. Copy all four over the originals:

- `assets/styles.css` — token-level reskin + appended v5 block
- `assets/index.html` — rail groups + honest finder placeholder
- `assets/app.js` — three one-line fixes (details below)
- `assets/icons.svg` — the shape vocabulary + new interface marks
  (i-copy, i-voice, i-stop, i-dismiss, i-undo, i-sun, i-moon, i-bolt)

## What styles.css now does

1. **oklch ramps.** All neutrals are a stepped hue-265 ramp; proof/signal/
   fault/trace/unproven sit at equal lightness+chroma (L .55 light, ~.72
   dark) so no severity shouts over another. Dark mode is an elevation
   model: surfaces lighten as they rise.
2. **Depth token.** `--shadow` on every card class; cards fall
   paper→sunk via a subtle gradient. `--grad-accent` on `.primary`.
3. **Motion tokens.** `--t-settle` / `--t-nudge`; sparklines draw in once
   (`.spark-path`, paths already carry `pathLength=1`); `button:active`
   presses 0.5px. All inside `prefers-reduced-motion` guards you already
   have at the bottom of the file.
4. **LAW FIXES (ship these even if nothing else):**
   - `.census-cell[data-cell="failing"|"stale"|"absent"]` — previously all
     three fell through to proven green. The spine could not show a
     failing claim.
   - `.speakable` no longer permanently ringed; only `.speaking` lights.
   - `.bar` alias for the test-run split bar (app.js also fixed properly).
5. **Rail groups** (`.rail-group`) — hidden in plain view and on phones.
6. **Display serif.** `--judgment` now leads with "Source Serif 4";
   a commented `@font-face` block at the end shows how to self-host the
   one woff2 (no CDN — Eyes serves only itself). Without the file it
   falls back to the current system stack, unchanged.

## app.js — the three fixes

- **Brief me** selector list now includes `.hero, .verdict-sub,
  .census-line, .concern h3, .concern p` — before this, Now spoke one
  side-column sentence and skipped the verdict.
- **protocolRow** builds `.bar-split` (was `.bar`, which had no rule —
  the pass/fail/skip bar was invisible).
- **settleCensus** uses fill-mode `backwards` (was `both`, whose retained
  transform killed the census hover).

## index.html

- Rail grouped: Operate / Plan / Prove / Explore.
- Finder placeholder no longer promises question-answering the endpoint
  does not do.

## Not ported on purpose (needs Rust/routing work, not assets)

- Now+Next merge, Proof/Time/Structure consolidation (13 → 7 nav items)
- Navigate-only drill model (retiring the reflowing inspector column)
- Dossier reranking (briefing-first + At-a-glance sidebar)
- Markdown body excerpt/expand on document dossiers
- Tempo strip under sparklines; labeled facet groups in list.js

The working reference for all of these is `Eyes Rework.dc.html` in this
project — every interaction is clickable there.
