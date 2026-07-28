# Type encodes epistemology

Status: accepted

## Context

Eyes v3 was correct and plain. Every claim traced to a graph record, the
jargon gate kept the prose honest, and nothing was invented — but it
looked like any other light-mode admin tool, and it read like one. Lists
were tall cards you scrolled; nothing filtered, nothing expanded, and the
only way into a thing was a full page navigation.

The deeper problem was that the interface gave no visual account of the
one distinction the whole product rests on. A screen showing *"tested — 3
records linked"* next to *"crates/brain-eyes/src/tests.rs"* presented an
interpretation and a record in the same voice, at the same weight, as if
they were the same kind of statement. They are not, and the difference is
the product.

## Decision

**Type encodes epistemology.** Three roles, and which one a thing is set
in tells you what kind of statement it is.

| Role | Face | Carries |
|---|---|---|
| judgment | serif — New York, Charter, Iowan Old Style | what the brain **concluded**: headlines, claims, verdicts, narration, document bodies |
| interface | system sans | furniture: labels, navigation, column heads, buttons |
| record | monospace, tabular figures | what was literally **recorded**: paths, hashes, commands, test names, counts, timestamps |

A person can see, before reading a word, which parts of a screen are
interpretation and which are evidence.

All three faces are already on the machine. Nothing is fetched, which the
read-only boundary (ADR-023) requires anyway: a strict CSP and a server
that serves only itself.

Supporting decisions:

1. **One colour for "claimed but not proven."** Violet marks the state
   that has no name in an ordinary dashboard: a requirement is linked, and
   nothing establishes it. It is the most common true state in this
   system — every feature in the live store claims `tested_by` and none
   can show a passing result — and it deserved a colour rather than being
   rounded to green or red.

2. **Shape carries state as well as colour.** Filled circle = fine,
   rotated square = failed, hollow ring = unproven, horizontal dash =
   stale, dashed outline = not settled. Colour is never load-bearing.

3. **Kinds are drawn, not lettered.** One SVG symbol per kind on a common
   34×34 grid — hexagon for a feature, diamond for a test, kite for a
   decision, folded page for a document. The MRI rasterises the same
   geometry into its glyph atlas, so the flat views and the anatomy agree
   about what a thing looks like.

4. **The header rule is near-black; row dividers are hairlines.** That one
   contrast is what makes a dense table read as an instrument rather than
   a spreadsheet. Taken from `design-draft/`, whose visual system this
   builds on throughout.

5. **Three ways down, kept distinct.** *Peek* (click a row) opens the
   inspector beside it and answers *why*. *Push* (Enter, or the name)
   opens the full page and gives *everything*. *Expand* (the chevron)
   shows children in the same grid. Conflating them is what made the
   previous version feel flat.

6. **Filtering is client-side.** Every list in this product is bounded —
   the largest is 150 rows — so a keystroke is instant and no request is
   made. Facet counts are computed against the *other* active filters, so
   a count never promises rows a second filter would remove.

## Consequences

- `assets/list.js` is the single list engine: facets, text filter, sort,
  an honest `N of M` readout, dense rows, tree expansion, and a full
  keyboard model. Every surface uses it, so a habit learned on one works
  on all of them.
- The dimension strip is the signature: a feature's parts, or a leaf's
  requirements, at seven pixels a cell in a row, labelled in a dossier,
  and expanded in a drill-down. Its legend is stated on the page, because
  a colour nobody can decode is decoration.
- Operational surfaces are readable in light or dark; the MRI stays dark
  because it is a different kind of looking. `prefers-reduced-motion` is
  honoured.
- The serif is the one deliberate risk. It is justified rather than
  decorative: it is what makes the claim/proof distinction visible, and it
  is applied with restraint — headlines, verdicts and prose bodies only,
  never to interface furniture.
