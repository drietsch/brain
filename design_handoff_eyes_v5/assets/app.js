/* Eyes — the visual layer over the brain.
 *
 * The rule this file lives by: it renders sentences the server wrote and
 * never composes a judgment of its own. If a phrase is missing, the fix
 * belongs in say.rs, not here.
 */
"use strict";

import { featureTag, h, icon, kindIcon, strip, stripWide, table } from "/assets/list.js";

const stage = document.getElementById("stage");
const state = { view: "now", params: {}, snapshot: null, findRows: [], findIndex: 0, thingHome: null };
/* Declared here because render() runs at load, before the sections below. */
let mriHandle = null;
let speaking = false;

/* ------------------------------------------------------------------ utils */

function glyph(shape) {
  return kindIcon(shape);
}

function chip(label, tone) {
  return label ? h("span", { class: `chip ${tone || "quiet"}`, text: label }) : null;
}

function fixLine(command) {
  if (!command) return null;
  return h("code", { class: "fix" }, h("span", { text: "run" }), command);
}

async function api(path) {
  const response = await fetch(path, { headers: { accept: "application/json" } });
  const payload = await response.json().catch(() => ({ error: "unreadable response" }));
  if (!response.ok) throw new Error(payload.error || `request failed (${response.status})`);
  return payload;
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (character) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[character]);
}

/* Markdown, small and safe: everything is escaped before any tag is added. */
function markdown(source) {
  const lines = escapeHtml(source).split("\n");
  const out = [];
  let inCode = false;
  let listType = null;
  let paragraph = [];

  const inline = (text) => text
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[\s(])\*([^*\n]+)\*/g, "$1<em>$2</em>")
    .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, '<a href="$2" rel="noreferrer">$1</a>');

  const flushParagraph = () => {
    if (!paragraph.length) return;
    out.push(`<p>${inline(paragraph.join(" "))}</p>`);
    paragraph = [];
  };
  const closeList = () => { if (listType) { out.push(`</${listType}>`); listType = null; } };
  const flush = () => { flushParagraph(); closeList(); };

  for (const raw of lines) {
    if (raw.trim().startsWith("```")) {
      flush();
      out.push(inCode ? "</code></pre>" : "<pre><code>");
      inCode = !inCode;
      continue;
    }
    if (inCode) { out.push(raw); continue; }

    if (raw.trim() === "") { flush(); continue; }

    const heading = raw.match(/^(#{1,4})\s+(.*)$/);
    if (heading) {
      flush();
      const level = Math.min(heading[1].length, 4);
      out.push(`<h${level}>${inline(heading[2])}</h${level}>`);
      continue;
    }
    if (/^\s*[-*]\s+/.test(raw)) {
      flushParagraph();
      if (listType !== "ul") { closeList(); out.push("<ul>"); listType = "ul"; }
      out.push(`<li>${inline(raw.replace(/^\s*[-*]\s+/, ""))}</li>`);
      continue;
    }
    if (/^\s*\d+\.\s+/.test(raw)) {
      flushParagraph();
      if (listType !== "ol") { closeList(); out.push("<ol>"); listType = "ol"; }
      out.push(`<li>${inline(raw.replace(/^\s*\d+\.\s+/, ""))}</li>`);
      continue;
    }
    if (/^\s*&gt;\s?/.test(raw)) {
      flush();
      out.push(`<blockquote>${inline(raw.replace(/^\s*&gt;\s?/, ""))}</blockquote>`);
      continue;
    }
    if (/^\s*\|.*\|\s*$/.test(raw)) { flush(); out.push(`<p>${inline(raw)}</p>`); continue; }
    // An ordinary line: a continuation of the bullet above it, or part
    // of the paragraph being built.
    if (listType && !paragraph.length) {
      out[out.length - 1] = out[out.length - 1].replace(/<\/li>$/, ` ${inline(raw.trim())}</li>`);
      continue;
    }
    closeList();
    paragraph.push(raw.trim());
  }
  flush();
  if (inCode) out.push("</code></pre>");
  return out.join("\n");
}

/* ---------------------------------------------------------------- routing */

function go(view, params = {}) {
  const query = new URLSearchParams(params).toString();
  location.hash = `#${view}${query ? `?${query}` : ""}`;
}

function openThing(id) { go("thing", { id }); }

function readRoute() {
  const raw = location.hash.replace(/^#/, "") || "now";
  const [view, query] = raw.split("?");
  return { view: view || "now", params: Object.fromEntries(new URLSearchParams(query || "")) };
}

const views = {};

/* Per-viewer memory, in this browser only: the previous visit's cursor
   and acknowledged concerns. The server never stores who looked — the
   marker rides back as a query parameter so the sentence about it is
   still composed server-side, in the one voice. */
const previousVisit = (() => {
  try { return JSON.parse(localStorage.getItem("eyes-visit")); } catch { return null; }
})();
function rememberVisit(snapshot) {
  if (!snapshot || rememberVisit.done) return;
  rememberVisit.done = true;
  localStorage.setItem("eyes-visit", JSON.stringify({ cursor: snapshot.cursor, at_ms: Date.now() }));
}
function loadAcks() {
  let acks;
  try { acks = JSON.parse(localStorage.getItem("eyes-acks")) ?? {}; } catch { acks = {}; }
  /* Acknowledgements expire after a week: a concern that persists that
     long deserves to be seen again. */
  const cutoff = Date.now() - 7 * 86400 * 1000;
  for (const key of Object.keys(acks)) if (acks[key] < cutoff) delete acks[key];
  return acks;
}
function saveAcks(acks) { localStorage.setItem("eyes-acks", JSON.stringify(acks)); }
function ackId(item) { return `${item.severity}|${item.title}|${item.reason}`; }
function ackButton(item) {
  return h("button", {
    class: "ack", text: "acknowledge", title: "Absorbed — hidden here for a week, in this browser only",
    onclick: (event) => {
      event.stopPropagation();
      const acks = loadAcks();
      acks[ackId(item)] = Date.now();
      saveAcks(acks);
      render();
    },
  });
}
function splitAcked(items) {
  const acks = loadAcks();
  const shown = [], acked = [];
  for (const item of items) (acks[ackId(item)] ? acked : shown).push(item);
  return { shown, acked };
}
function ackedToggle(acked) {
  if (!acked.length) return null;
  return h("details", { class: "also" },
    h("summary", { text: `${acked.length} acknowledged — show` }),
    h("ul", {}, acked.map((item) => h("li", {
      class: "restore", title: "Click to bring it back",
      text: `${item.title} — ${item.reason}`,
      onclick: () => {
        const acks = loadAcks();
        delete acks[ackId(item)];
        saveAcks(acks);
        render();
      },
    }))));
}

async function render() {
  const route = readRoute();
  state.view = route.view;
  state.params = route.params;
  for (const button of document.querySelectorAll(".rail button")) {
    const target = button.dataset.go;
    const home = route.view === "thing" ? state.thingHome : null;
    button.classList.toggle("on", target === route.view ||
      (target === "library" && ["concepts", "media"].includes(route.view)) ||
      (home !== null && target === home));
  }
  // The full-bleed dark canvas belongs to the MRI, which draws its own
  // background and wants no stage padding. Keyed to "map" it stripped the
  // Map of its padding and put a night backdrop behind it: the heading
  // ran under the topbar, the lens buttons overflowed, and a light-theme
  // panel sat on near-black.
  stage.classList.toggle("dark", route.view === "mri");
  document.body.classList.toggle("in-mri", route.view === "mri");
  // On Now the verdict band carries freshness, drift and the promise
  // itself — the topbar saying the same three things a centimetre above
  // would be the page repeating itself.
  document.body.classList.toggle("in-now", route.view === "now");
  if (route.view !== "mri" && mriHandle) { mriHandle.destroy(); mriHandle = null; }
  stopSpeaking();
  stage.replaceChildren(h("p", { class: "loading", text: "Reading the graph…" }));
  try {
    const renderer = views[route.view] || views.now;
    await renderer(route.params);
  } catch (error) {
    stage.replaceChildren(
      h("h1", { class: "headline", text: "Eyes could not read that." }),
      h("p", { class: "subhead", text: error.message })
    );
  }
  stage.scrollTop = 0;
}

/* ------------------------------------------------------------------- next */

views.next = async () => {
  const data = await api("/api/next");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  const parts = [
    h("h1", { class: "hero", text: data.headline }),
    h("p", { class: "hero-sub", text: data.subhead }),
  ];
  const inbox = splitAcked(data.queue);
  if (inbox.shown.length) {
    parts.push(h("div", { class: "concerns" }, inbox.shown.map((item) =>
      h("div", { class: `concern ${item.severity}` },
        h("i", { class: `mark ${({ act: "bad", watch: "watch" })[item.severity] ?? "quiet"}` }),
        h("div", {},
          h("h3", { text: item.title }),
          h("p", { text: item.reason }),
          item.fix_command ? commandLine(item.fix_command) : null,
          ackButton(item))))));
  }
  const toggle = ackedToggle(inbox.acked);
  if (toggle) parts.push(toggle);
  stage.replaceChildren(h("div", { class: "page" }, ...parts));
};

/* -------------------------------------------------------------------- now */

views.now = async () => {
  const seen = previousVisit && Number.isInteger(previousVisit.cursor) ? previousVisit.cursor : null;
  const data = await api("/api/now" + (seen === null ? "" : `?seen=${seen}`));
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  /* The verdict band: one instrument, one reading. The sentence, the
     claim spine, the trend row and the trust stamp share one ground —
     everything below it decays in weight. */
  const verdict = [
    h("h1", { class: "hero", text: data.headline }),
    h("p", { class: "verdict-sub" },
      data.subhead,
      data.since_you_looked
        ? h("span", { class: "visit-inline", text: ` · ${data.since_you_looked}` })
        : null),
    census(data.proof),
    sparkrow(data.quality),
    h("p", { class: "stamp" }, ...stampLine(data.snapshot)),
  ];

  const main = [];
  const inbox = splitAcked(data.needs_you);
  main.push(h("h2", { class: "section", text: "Needs you" }));
  if (!inbox.shown.length) {
    // Calm rendered as content, not as absence.
    main.push(h("p", { class: "quiet-verdict", text: "Nothing needs you." }));
  }
  if (inbox.shown.length || inbox.acked.length) {
    main.push(h("div", { class: "concerns" }, inbox.shown.map((concern) => {
      const node = h("div", { class: `concern ${concern.severity}` },
        h("i", { class: `mark ${({ act: "bad", watch: "watch" })[concern.severity] ?? "quiet"}` }),
        h("div", {},
          h("h3", {},
            concern.title,
            concern.repeats > 1 ? h("span", { class: "repeats", text: `×${concern.repeats}` }) : null),
          h("p", { text: concern.reason }),
          // A count you cannot unfold is a count you cannot check. The card
          // itself opens the thing, so unfolding must not also navigate.
          concern.also.length
            ? h("details", { class: "also", onclick: (event) => event.stopPropagation() },
                h("summary", { text: `and ${concern.repeats - 1} more like it` }),
                h("ul", {},
                  concern.also.map((line) => h("li", { text: line })),
                  // Never let a shown list pass for the whole list.
                  concern.repeats - 1 > concern.also.length
                    ? h("li", { class: "faint", text: `${concern.repeats - 1 - concern.also.length} more are not listed here` })
                    : null))
            : null,
          // The journey: which step the subjects are stuck on.
          concern.steps.length
            ? h("div", { class: "journey" }, concern.steps.map((step) =>
                h("span", { class: `journey-step ${step.state}` },
                  h("i", { class: "journey-dot", "aria-hidden": "true" }),
                  step.when ? `${step.label} ${step.when}` : step.label)))
            : null,
          // The subjects themselves, each openable — the card shows what
          // it is about instead of asking the reader to imagine it. A
          // long shelf folds; a short one stands open.
          concern.chips.length ? chipFold(concern.chips) : null,
          fixLine(concern.fix_command),
          ackButton(concern)));
      if (concern.target) {
        node.style.cursor = "pointer";
        node.addEventListener("click", () => openThing(concern.target.id));
      }
      return node;
    })));
    const toggle = ackedToggle(inbox.acked);
    if (toggle) main.push(toggle);
  }

  /* The lighter registers keep to the side column: the delta, then the
     pressure as the ranked list it actually is. */
  const side = [];
  side.push(h("h2", { class: "section", text: data.since.known ? `Since your last session, ${data.since.when}` : "Recently" }));
  side.push(h("p", { class: "sub", text: data.since.summary }));
  if (data.since.episodes.length) {
    side.push(h("div", { class: "episodes" }, data.since.episodes.map(episodeRow)));
  }

  if (data.attention.length) {
    side.push(h("h2", { class: "section", text: "Where the pressure is" }));
    side.push(h("div", { class: "pressure-list" }, data.attention.map((card, index) =>
      h("button", { class: "pressure-row", title: `a ${card.noun}`, onclick: () => card.id && openThing(card.id) },
        h("span", { class: "pressure-rank", text: String(index + 1) }),
        h("span", { class: "pressure-body" },
          h("span", { class: "pressure-path", text: card.label }),
          h("span", { class: "pressure-why", text: card.reasons.join(" · ") }))))));
  }

  stage.replaceChildren(
    h("section", { class: "verdict" }, ...verdict),
    h("div", { class: "now-columns" },
      h("div", { class: "now-main" }, ...main),
      h("aside", { class: "now-side" }, ...side)));
  settleCensus();
};

/* The trust stamp: how fresh the reading is, whether the tree has moved
   past it, and the standing promise — one quiet line under the verdict.
   On Now the topbar's own whisper of the same facts stands down. */
function stampLine(snapshot) {
  const parts = [];
  const freshness = document.getElementById("freshness")?.textContent;
  if (freshness) parts.push(h("span", { text: freshness }));
  if (snapshot.working_tree) {
    const ahead = snapshot.working_tree.state === "ahead";
    parts.push(h("span", { class: ahead ? "stamp-drift" : "", text: snapshot.working_tree.sentence }));
  }
  parts.push(h("span", { text: "read only" }));
  return parts.flatMap((part, index) => (index ? [" · ", part] : [part]));
}

/**
 * The census: every claim the system makes, one mark each.
 *
 * The same device as a feature's dimension strip, read at the scale of the
 * whole graph. It is the product's thesis in one object — everything in
 * here is a claim, and each one either can or cannot show its proof.
 */
function census(proof) {
  if (!proof || !proof.total) return null;
  return h("section", { class: "census" },
    h("p", { class: "census-line", text: proof.sentence }),
    h("div", { class: "spine" }, proof.groups.map((group) =>
      h("div", { class: "spine-group" },
        h("div", { class: "census-cells" }, group.cells.map((cell) =>
          h("button", {
            class: "census-cell", "data-cell": cell.state, title: cell.text,
            // A mark with no name is unreadable to anyone not looking at it.
            "aria-label": cell.text,
            onclick: () => showInspector(cell.id),
          }))),
        h("p", { class: "census-label" },
          h("span", { text: group.label }),
          h("span", { class: "census-count", text: `${group.proven}/${group.total}` }))))));
}

/* A concern's subjects as a foldable shelf of openable chips. Short
   shelves stand open; long ones fold behind an honest count. */
function chipFold(chips) {
  const summary = h("summary", {});
  const fold = h("details", { class: "chip-fold", onclick: (event) => event.stopPropagation() },
    summary,
    h("div", { class: "concern-chips" }, chips.map((ref) =>
      h("button", {
        class: "chip-ref", title: `a ${ref.noun}`,
        onclick: (event) => { event.stopPropagation(); openThing(ref.id); },
      }, glyph(ref.glyph), h("span", { text: ref.label })))));
  const speak = () => {
    summary.textContent = fold.open ? "fold them away" : `show all ${chips.length}`;
  };
  fold.open = chips.length <= 6;
  fold.addEventListener("toggle", speak);
  speak();
  return fold;
}

/* The sparkrow: the four trends as one instrument line under the spine.
   The stroke stays quiet; the arrow carries the alarm — a falling line
   is loud, a rising one is a footnote, and every item opens the surface
   that holds its evidence: a trend is never the end of the trail. */
function sparkrow(lines) {
  if (!lines || !lines.length) return null;
  return h("div", { class: "sparkrow" }, lines.map(sparkItem));
}

function sparkItem(line) {
  const W = 92, H = 22;
  const pts = line.points;
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
  svg.setAttribute("class", "spark");
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", line.sentence);
  const min = Math.min(...pts);
  const span = (Math.max(...pts) - min) || 1;
  const x = (i) => (pts.length === 1 ? W / 2 : 3 + (i / (pts.length - 1)) * (W - 6));
  const y = (v) => H - 4 - ((v - min) / span) * (H - 8);
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("class", "spark-path");
  path.setAttribute("pathLength", "1");
  path.setAttribute("d", pts.map((v, i) => `${i ? "L" : "M"}${x(i).toFixed(1)} ${y(v).toFixed(1)}`).join(" "));
  svg.append(path);
  const dot = document.createElementNS("http://www.w3.org/2000/svg", "circle");
  dot.setAttribute("class", "spark-dot");
  dot.setAttribute("cx", x(pts.length - 1).toFixed(1));
  dot.setAttribute("cy", y(pts[pts.length - 1]).toFixed(1));
  dot.setAttribute("r", "1.8");
  svg.append(dot);
  const arrow = { rising: "↗", falling: "↘", flat: "→" }[line.trend] ?? "→";
  const home = ({ tests: "tests", claims: "evidence", features: "features", docs: "library" })[line.id];
  return h("button", { class: `spark-item ${line.tone}`, title: line.sentence,
      onclick: () => home && go(home) },
    h("span", { class: "spark-label", text: line.label }),
    svg,
    pts.length > 1 ? h("span", { class: "spark-arrow", text: arrow, "aria-hidden": "true" }) : null,
    h("span", { class: "spark-now", text: line.current }));
}

/* The one orchestrated moment: the census resolves like an instrument
   taking a reading, left to right, then settles. It runs once per page
   and never while a person is reading. */
function settleCensus() {
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  const cells = stage.querySelectorAll(".census-cell");
  cells.forEach((cell, index) => {
    cell.style.animation = `settle .34s ease-out ${Math.min(index * 22, 700)}ms backwards`;
  });
}

function episodeRow(episode) {
  return h("button", { class: "episode", onclick: () => go("timeline") },
    h("time", { text: episode.when }),
    h("div", {},
      h("strong", { text: episode.title }),
      episode.facts.map((fact) => h("p", { text: fact })),
      episode.items.length
        ? h("p", { text: episode.items.slice(0, 4).map((item) => item.label).join(", ") +
            (episode.more ? ` and ${episode.more} more` : "") })
        : null));
}

/* ---------------------------------------------------------------- library */

views.library = async (params) => {
  const shelf = params.shelf || "decisions";
  const data = await api(`/api/library?shelf=${encodeURIComponent(shelf)}`);
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  const host = h("div", {});
  stage.replaceChildren(h("div", { class: "library" },
    shelfRail(data.shelves, data.shelf),
    h("div", {},
      h("h1", { class: "lede", text: data.label }),
      h("p", { class: "sub", text: data.note }),
      host)));

  table(host, {
    rows: data.items,
    noun: "records",
    keyOf: (row) => row.id,
    search: (row) => `${row.title} ${row.label} ${row.excerpt ?? ""} ${row.facts.join(" ")}`,
    placeholder: `Filter ${data.label.toLowerCase()}…`,
    facets: [
      { id: "state", get: (row) => row.state },
      { id: "kind", get: (row) => row.noun },
      { id: "feature", get: (row) => row.features.map((f) => f.label) },
    ],
    sort: [
      { key: "changed", label: "by last change", by: (row) => -row.at_ms },
      { key: "title", label: "by name", by: (row) => row.title },
      { key: "state", label: "by state", by: (row) => row.state ?? "" },
    ],
    columns: [
      { key: "title", label: data.label, width: "minmax(260px, 3fr)",
        cell: (row) => h("span", { class: "pill-row" },
          kindIcon(row.glyph), h("span", { text: row.title })) },
      { key: "state", label: "State", width: "116px",
        cell: (row) => (row.state ? chip(row.state, row.tone) : "") },
      { key: "why", label: "Because", width: "minmax(200px, 3fr)", class: "dim nowrap",
        cell: (row) => row.state_note ?? row.excerpt ?? "" },
      { key: "serves", label: "Serves", width: "minmax(150px, 1.6fr)", class: "nowrap",
        cell: (row) => featureTag(row.features, (f) => openThing(f.id), 2) ?? "" },
      { key: "changed", label: "Changed", width: "92px", class: "dim num",
        cell: (row) => row.when ?? "" },
    ],
    onPeek: (row) => showInspector(row.id),
    onPush: (row) => openThing(row.id),
    empty: "Nothing on this shelf matches that.",
  });
};

function shelfRail(shelves, current) {
  // A media shelf opens the surface built for looking at things; the
  // rest are lists.
  const all = [
    ...shelves.map((shelf) => ({ ...shelf, view: shelf.id === "media" ? "media" : "library" })),
    { id: "concepts", label: "Concepts", count: null, view: "concepts" },
  ];
  return h("nav", { class: "shelves" }, all.map((shelf) =>
    h("button", {
      class: shelf.id === current ? "on" : "",
      onclick: () => (shelf.view === "library" ? go("library", { shelf: shelf.id }) : go(shelf.view)),
    }, h("span", { text: shelf.label }), shelf.count !== null ? h("em", { text: shelf.count }) : null)));
}

views.concepts = async () => {
  const data = await api("/api/concepts");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);
  stage.replaceChildren(
    h("div", { class: "page-head" }, h("h1", { text: "Concepts" })),
    h("p", { class: "page-note",
      text: "The kinds of thing this brain knows about, what each one promises, and what it has learned about whether that promise works." }),
    h("div", { class: "library" },
      shelfRail(await shelvesFor(), "concepts"),
      h("div", { class: "concepts" }, data.concepts.map((concept) =>
        h("div", { class: "concept" },
          h("h3", {}, glyph(concept.glyph), concept.label,
            concept.count ? h("span", { class: "chip quiet", text: concept.count }) : null),
          h("p", { class: "purpose", text: concept.purpose }),
          h("dl", {},
            h("dt", { text: "Lives" }), h("dd", { text: concept.placement_note }),
            concept.home.length ? h("dt", { text: "At" }) : null,
            concept.home.length ? h("dd", { text: concept.home.join(", ") }) : null,
            concept.requires.length ? h("dt", { text: "Must have" }) : null,
            concept.requires.length ? h("dd", { text: concept.requires.join(", ") }) : null,
            h("dt", { text: "Rules" }), h("dd", { text: concept.enforcement_note }),
            h("dt", { text: "Ageing" }), h("dd", { text: concept.rot_note })),
          concept.verdicts.map((verdict) => h("p", { class: "verdict", text: verdict })))))));
};

let shelfCache = null;
async function shelvesFor() {
  if (!shelfCache) shelfCache = (await api("/api/library?shelf=decisions")).shelves;
  return shelfCache;
}

/* -------------------------------------------------------------------- map */

views.map = async (params) => {
  const lens = params.lens || "attention";
  const data = await api(`/api/map?lens=${encodeURIComponent(lens)}`);
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  const detail = h("div", { class: "map-detail", hidden: true });
  const showBlock = async (block) => {
    detail.hidden = false;
    detail.replaceChildren(
      h("h2", { text: block.label }),
      h("p", { text: `${block.sentence}. ${block.facts.join(". ")}.` }),
      h("p", { class: "loading", text: "Reading its files…" }));
    const files = await api(`/api/find?q=${encodeURIComponent(block.path)}&limit=60`);
    detail.replaceChildren(
      h("h2", { text: block.label }),
      h("p", { text: `${block.sentence}. ${block.facts.join(". ")}.` }),
      h("ul", {}, files.hits
        .filter((hit) => hit.target.kind === "source_file")
        .map((hit) => h("li", {}, h("button", {
          text: hit.target.label.replace(`${block.path}/`, ""),
          onclick: () => openThing(hit.target.id),
        })))));
  };

  stage.replaceChildren(h("div", { class: "map-wrap" },
    h("div", { class: "map-head" },
      h("div", {},
        h("h1", { text: data.lens_label }),
        h("p", { text: `${data.lens_note} ${data.sentence}` })),
      h("div", { class: "lenses" }, data.lenses.map(([id, label]) =>
        h("button", { class: id === data.lens ? "on" : "", text: label,
          onclick: () => go("map", { lens: id }) })))),
    h("div", { class: "map-field" }, mapSvg(data, showBlock)),
    detail));
};

function mapSvg(data, onPick) {
  const layers = new Map();
  for (const block of data.blocks) {
    if (!layers.has(block.layer)) layers.set(block.layer, []);
    layers.get(block.layer).push(block);
  }
  const ordered = [...layers.keys()].sort((a, b) => b - a); // deepest at the bottom
  const width = 1000;
  const rowHeight = 118;
  const height = Math.max(220, ordered.length * rowHeight + 40);
  const positions = new Map();

  ordered.forEach((layer, rowIndex) => {
    const row = layers.get(layer).slice().sort((a, b) => b.files - a.files);
    const total = row.reduce((sum, block) => sum + Math.max(1, Math.sqrt(block.files)), 0);
    let x = 40;
    const usable = width - 80 - (row.length - 1) * 14;
    for (const block of row) {
      const w = Math.max(120, (Math.max(1, Math.sqrt(block.files)) / total) * usable);
      const y = rowIndex * rowHeight + 34;
      positions.set(block.id, { x, y, w, h: 74, block });
      x += w + 14;
    }
  });

  const tone = (block) => {
    const base = { good: [46, 125, 91], watch: [168, 106, 18], bad: [176, 58, 52], quiet: [70, 92, 116] }[block.tone] ||
      [70, 92, 116];
    const weight = 0.18 + (block.value / 100) * 0.5;
    return `rgba(${base[0]}, ${base[1]}, ${base[2]}, ${weight.toFixed(2)})`;
  };

  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", data.sentence);

  const add = (tag, attrs, textContent) => {
    const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
    if (textContent !== undefined) node.textContent = textContent;
    return node;
  };

  for (const edge of data.edges) {
    const from = positions.get(edge.from);
    const to = positions.get(edge.to);
    if (!from || !to) continue;
    const x1 = from.x + from.w / 2;
    const y1 = from.y + from.h;
    const x2 = to.x + to.w / 2;
    const y2 = to.y;
    const mid = (y1 + y2) / 2;
    svg.append(add("path", {
      class: "mb-edge",
      d: `M ${x1} ${y1} C ${x1} ${mid}, ${x2} ${mid}, ${x2} ${y2}`,
      "stroke-width": Math.min(3, 0.6 + edge.weight * 0.25),
    }));
  }

  ordered.forEach((layer, rowIndex) => {
    svg.append(add("text", {
      class: "map-layer-label", x: 6, y: rowIndex * rowHeight + 26,
    }, rowIndex === ordered.length - 1 ? "foundation" : `layer ${layer}`));
  });

  for (const { x, y, w, h: boxHeight, block } of positions.values()) {
    const group = add("g", { class: "mb-block", tabindex: "0", role: "button" });
    group.append(add("rect", { x, y, width: w, height: boxHeight, rx: 8, fill: tone(block) }));
    group.append(add("text", { class: "name", x: x + 14, y: y + 27 }, block.label));
    group.append(add("text", { class: "meta", x: x + 14, y: y + 46 },
      `${block.files} files · ${block.symbols} defs`));
    group.append(add("text", { class: "meta", x: x + 14, y: y + 62 },
      block.sentence.length > Math.floor(w / 6.2)
        ? `${block.sentence.slice(0, Math.floor(w / 6.2))}…`
        : block.sentence));
    const pick = () => onPick(block);
    group.addEventListener("click", pick);
    group.addEventListener("keydown", (event) => { if (event.key === "Enter") pick(); });
    svg.append(group);
  }
  return svg;
}

/* --------------------------------------------------------------- timeline */

views.timeline = async () => {
  const data = await api("/api/timeline?limit=60");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);
  stage.replaceChildren(
    h("div", { class: "page-head" }, h("h1", { text: "Timeline" })),
    h("p", { class: "page-note",
      text: "Everything the graph recorded, grouped into the batches it actually happened in." }),
    data.episodes.length
      ? h("div", { class: "episodes" }, data.episodes.map((episode) =>
          h("div", { class: "episode" },
            h("time", { text: episode.when }),
            h("div", {},
              h("strong", { text: episode.title }),
              episode.facts.map((fact) => h("p", { text: fact })),
              episode.items.length
                ? h("p", {}, episode.items.map((item, index) => [
                    index ? ", " : "",
                    h("button", { class: "row-link", text: item.label, onclick: () => openThing(item.id) }),
                  ]).flat(), episode.more ? ` and ${episode.more} more` : "")
                : null,
              // What that batch of work was on.
              featureTag(episode.features, (f) => openThing(f.id))))))
      : h("p", { class: "empty", text: "Nothing recorded yet." }));
};

/* ---------------------------------------------------------------- compare */

views.compare = async (params) => {
  if (!params.from) {
    const data = await api("/api/moments");
    state.snapshot = data.snapshot;
    paintChrome(data.snapshot);
    stage.replaceChildren(
      h("div", { class: "page-head" }, h("h1", { text: "Compare" })),
      h("p", { class: "page-note", text: data.headline }),
      data.moments.length
        ? h("div", { class: "moments" }, data.moments.map((moment) =>
            h("button", { class: "moment", onclick: () => go("compare", { from: moment.value }) },
              h("span", { class: "moment-label", text: moment.label }),
              h("span", { class: "moment-when", text: moment.when }))))
        : null);
    return;
  }
  const to = params.to || "live";
  const data = await api(
    `/api/compare?from=${encodeURIComponent(params.from)}&to=${encodeURIComponent(to)}`);
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  const parts = [];
  if (data.banner) {
    parts.push(h("div", { class: "asof-banner" },
      h("p", { text: data.banner }),
      h("button", { class: "ghost", text: "Back to live", onclick: () => go("now") })));
  }
  parts.push(h("h1", { class: "hero", text: data.headline }));
  parts.push(h("p", { class: "hero-sub",
    text: `${data.then_moment.label}, ${data.then_moment.when} — against ${data.vs_moment.label}.` }));

  parts.push(h("div", { class: "delta-strip" }, data.metrics.map((metric) =>
    h("div", { class: `delta ${metric.tone}`, title: metric.sentence, "aria-label": metric.sentence },
      h("span", { class: "delta-label", text: metric.label }),
      h("span", { class: "delta-values" },
        h("span", { text: metric.then_value }),
        h("span", { class: "delta-arrow", "aria-hidden": "true", text: "→" }),
        h("span", { text: metric.now_value }))))));

  // Regressions physically first — the loudest thing leads.
  const section = (label, rows) => {
    if (!rows.length) return;
    parts.push(h("h2", { class: "section", text: label }));
    parts.push(h("div", { class: "diff-rows" }, rows.map((row) =>
      h("div", { class: "diff-row" },
        h("i", { class: `mark ${({ bad: "bad", good: "good" })[row.tone] ?? "quiet"}` }),
        h("div", {},
          h("h3", { text: row.title }),
          h("p", { text: row.sentence }))))));
  };
  section("Regressions", data.regressions);
  section("Improvements", data.improvements);
  section("Appeared since", data.appeared);
  section("No longer present", data.removed);

  parts.push(h("p", { class: "omissions", text: data.omissions }));
  if (data.baseline_command) {
    parts.push(h("h2", { class: "section", text: "Name this moment" }));
    parts.push(commandLine(data.baseline_command));
  }
  stage.replaceChildren(h("div", { class: "page" }, ...parts));
};

/* ------------------------------------------------------------------ thing */

views.thing = async (params) => {
  if (!params.id) return go("now");
  const data = await api(`/api/thing?id=${encodeURIComponent(params.id)}`);
  // Light the section this thing belongs to, so the rail never claims you
  // are somewhere you are not.
  state.thingHome = ({
    feature: "features", test_case: "tests", test_run: "tests",
    agent_session: "work", change: "work", asset: "library",
  })[data.kind] ?? "library";
  for (const button of document.querySelectorAll(".rail button")) {
    button.classList.toggle("on", button.dataset.go === state.thingHome);
  }
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  const parts = [
    h("div", { class: "crumb" },
      h("button", { text: "← Back", onclick: () => history.back() }),
      "·", data.noun),
    h("div", { class: "thing-head" },
      glyph(data.glyph),
      h("h1", { text: data.title }),
      chip(data.state, data.tone)),
    h("p", { class: "thing-kind",
      text: [data.label !== data.title ? data.label : null, data.state_note].filter(Boolean).join(" · ") }),
  ];

  if (data.extras.stages.length) {
    parts.push(h("div", { class: "stages" }, data.extras.stages.map((step) =>
      h("div", { class: `stage-step ${step.state}` },
        h("strong", { text: step.label }),
        h("span", { text: step.when ? `${step.when} — ${step.note}` : step.note })))));
  }

  // A governed change: the recorded diff, exactly as proposed.
  if (data.extras.diff.length) {
    parts.push(h("h2", { class: "section", text: "What changes" }));
    if (data.extras.diff_summary) parts.push(h("p", { class: "sub", text: data.extras.diff_summary }));
    parts.push(diffBlock(data.extras.diff, data.extras.diff_note));
  }

  // A feature: the strip at its third and largest scale, then the parts
  // themselves, each answering for itself.
  const feature = data.extras.feature;
  if (feature) {
    parts.push(h("p", { class: "lede", text: feature.verdict }));
    // A cell that stands for a part opens it. A cell that stands for a
    // requirement is not an entity, so it opens through the records
    // behind it instead — which is the honest way to make it openable.
    const behind = h("div", { class: "behind" });
    parts.push(stripWide(feature.strip, (cell) => {
      if (cell.id) return openThing(cell.id);
      behind.replaceChildren(
        h("p", { class: "page-note", text: `${cell.label} — ${cell.detail}` }),
        h("ul", { class: "proof" }, (cell.records || []).map((record) =>
          h("li", {},
            h("span", { class: `mark ${record.tone}`, text: "·" }),
            h("button", { class: "linky", text: record.target.label,
                          onclick: () => openThing(record.target.id) }),
            h("span", {}, ` — ${record.text}`),
            record.basis ? h("span", { class: "basis", text: ` (${record.basis})` }) : null))));
    }));
    parts.push(behind);
    if (feature.parts.length) {
      parts.push(h("h2", { class: "section", text: "Its parts" }));
      parts.push(h("div", { class: "facts" }, feature.parts.map((part) =>
        h("button", { class: `fact ${part.done ? "good" : "watch"}`, onclick: () => openThing(part.id) },
          h("i", { class: `mark ${part.done ? "good" : "unproven"}` }),
          h("span", {},
            h("strong", { text: part.title }), " — ", part.verdict),
          strip(part.strip)))));
    }
    if (feature.blocked_by) {
      parts.push(h("p", { class: "sub", text: `Waiting on ${feature.blocked_by}.` }));
    }
  } else if (data.extras.coverage.length) {
    parts.push(h("h2", { class: "section", text: "What backs this claim" }));
    parts.push(h("div", { class: "facts" }, data.extras.coverage.map((cell) =>
      h("div", { class: `fact ${cell.met ? "good" : "watch"}` },
        h("i", { class: `mark ${cell.met ? "good" : "watch"}` }),
        h("span", {}, h("strong", { text: cell.label }), " — ", cell.detail)))));
  }

  // What this feature reaches: its claim first, then what the graph
  // already pointed at those files without anyone linking it.
  const reach = data.extras.reach;
  if (reach && reach.groups.length) {
    parts.push(h("h2", { class: "section", text: "What it reaches" }));
    parts.push(h("p", { class: "sub", text: reach.sentence }));
    parts.push(h("div", { class: "reach" }, reach.groups.map((group) =>
      h("div", { class: "reach-group" },
        h("p", { class: "reach-head" },
          h("span", { text: group.label }),
          h("span", { class: "reach-basis",
                      text: group.declared ? "declared" : "reached through its files" })),
        h("ul", {}, group.items.map((item) =>
          h("li", {},
            h("button", { class: "linky", text: item.target.label,
                          onclick: () => openThing(item.target.id) }),
            item.through
              ? h("span", { class: "basis", text: ` via ${item.through.label}` })
              : null)))))));
  }

  // Which features this thing serves — on every kind of page, not just
  // features. This is the line that makes the spine readable from below.
  if (data.extras.serves.length) {
    parts.push(h("h2", { class: "section", text: "What it serves" }));
    parts.push(h("div", { class: "facts" }, data.extras.serves.map((item) =>
      h("button", { class: "fact quiet", onclick: () => openThing(item.target.id) },
        glyph("hexagon"),
        h("span", {}, h("strong", { text: item.target.label }), " — ", item.because)))));
  }

  if (data.extras.briefing.length) {
    parts.push(h("h2", { class: "section", text: "Before you edit" }));
    parts.push(h("div", { class: "concerns" }, data.extras.briefing.map((item) =>
      h("div", { class: `concern ${item.severity}` },
        h("i", { class: `mark ${({ act: "bad", watch: "watch" })[item.severity] ?? "quiet"}` }),
        h("div", {},
          h("h3", { text: item.title }),
          item.reason ? h("p", { text: item.reason }) : null,
          item.fix_command ? commandLine(item.fix_command) : null)))));
  }

  if (data.facts.length) {
    parts.push(h("h2", { class: "section", text: "What Eyes can tell you" }));
    parts.push(h("div", { class: "facts" }, data.facts.map((fact) =>
      h("div", { class: `fact ${fact.tone}` },
        h("i", { class: "mark" }),
        h("span", {}, fact.text, fact.reason ? h("small", { text: ` — ${fact.reason}` }) : null),
        fact.target ? h("button", { text: "open", onclick: () => openThing(fact.target.id) }) : null))));
  }

  if (data.body) {
    parts.push(h("h2", { class: "section", text: "The thing itself" }));
    parts.push(bodyView(data));
  } else if (data.body_error) {
    parts.push(h("h2", { class: "section", text: "The thing itself" }));
    parts.push(h("p", { class: "empty", text: data.body_error }));
  }

  parts.push(h("h2", { class: "section", text: "Around it" }));
  parts.push(h("div", { class: "neighbourhood" }, neighbourhoodSvg(data.neighborhood)));
  parts.push(h("p", { class: "page-note", style: "margin-top:10px", text: data.neighborhood.sentence }));

  if (data.extras.superseded_by.length || data.extras.supersedes.length) {
    parts.push(h("h2", { class: "section", text: "Replacement" }));
    parts.push(h("div", { class: "rows" }, [
      ...data.extras.superseded_by.map((item) => relationRow("was replaced by", item)),
      ...data.extras.supersedes.map((item) => relationRow("replaces", item)),
    ]));
  }

  if (data.extras.flips.length) {
    parts.push(h("h2", { class: "section", text: "Result history" }));
    parts.push(h("div", { class: "rows" }, data.extras.flips.map((entry) =>
      h("div", { class: "row" }, h("span", { class: "when", text: entry.when }), h("span", { text: entry.text })))));
  }

  if (data.relations.length) {
    parts.push(h("h2", { class: "section", text: "Links" }));
    parts.push(h("div", { class: "rows" }, data.relations.map((relation) =>
      relationRow(relation.phrase, relation.other))));
  }

  if (data.versions.length > 1) {
    parts.push(h("h2", { class: "section", text: "Versions" }));
    parts.push(h("div", { class: "rows" }, data.versions.map((version) =>
      h("div", { class: "row" },
        h("span", { class: "when", text: version.when }),
        h("span", {}, version.note, h("span", { class: "who", text: version.hash }))))));
  }

  if (data.history.length) {
    parts.push(h("h2", { class: "section", text: "History" }));
    parts.push(h("div", { class: "rows" }, data.history.map((entry) =>
      h("div", { class: "row" },
        h("span", { class: "when", text: entry.when }),
        h("span", {}, entry.text,
          h("span", { class: "who", text: entry.source }),
          entry.detail ? h("span", { class: "who", text: entry.detail }) : null)))));
  }

  parts.push(h("details", { class: "details" },
    h("summary", { text: "Machine detail" }),
    h("dl", {}, data.details.map(([label, value]) => [h("dt", { text: label }), h("dd", { text: value })]).flat())));

  stage.replaceChildren(...parts);
};

function relationRow(phrase, target) {
  return h("button", { class: "row", onclick: () => openThing(target.id) },
    h("span", { class: "when", text: phrase }),
    h("span", {}, glyph(target.glyph), " ", target.label,
      h("span", { class: "who", text: target.noun })));
}

function bodyView(data) {
  const body = data.body;
  if (body.format === "image") {
    return h("div", { class: "body-view" },
      h("img", { src: `/api/body?id=${encodeURIComponent(data.id)}`, alt: data.title }),
      h("p", { class: "body-origin", text: body.origin }));
  }
  if (["audio", "video"].includes(body.format)) {
    return h("div", { class: "body-view" },
      h(body.format, { src: `/api/body?id=${encodeURIComponent(data.id)}`, controls: "controls" }),
      h("p", { class: "body-origin", text: body.origin }));
  }
  const rendered = body.format === "markdown"
    ? h("div", { html: markdown(body.text || "") })
    : h("pre", {}, h("code", { text: body.text || "" }));
  return h("div", { class: "body-view" }, rendered,
    body.truncated ? h("p", { class: "body-origin", text: "Shown in part — this is a long file." }) : null,
    h("p", { class: "body-origin", text: body.origin }));
}

function neighbourhoodSvg(nb) {
  const columns = [
    { key: "upstream", label: "it uses", items: nb.upstream, total: nb.upstream_total },
    { key: "center", label: "", items: [nb.center], total: 1 },
    { key: "downstream", label: "depends on it", items: nb.downstream, total: nb.downstream_total },
  ];
  const extras = [
    { label: "tested by", items: nb.tests },
    { label: "described in", items: nb.docs.concat(nb.decisions) },
  ].filter((group) => group.items.length);

  const rowH = 30;
  const colW = 300;
  const width = 960;
  const tallest = Math.max(1, ...columns.map((column) => column.items.length));
  const extraRows = extras.reduce((sum, group) => sum + group.items.length + 1, 0);
  const height = Math.max(140, 46 + tallest * rowH + (extraRows ? extraRows * rowH + 20 : 0));

  const add = (tag, attrs, textContent) => {
    const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
    if (textContent !== undefined) node.textContent = textContent;
    return node;
  };
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", nb.sentence);

  const place = (item, x, y, w, isCenter) => {
    const group = add("g", { class: `nb-node${isCenter ? " center" : ""}`, tabindex: "0", role: "button" });
    group.append(add("rect", { x, y, width: w, height: 22, rx: 5 }));
    const label = item.label.length > Math.floor(w / 6.6)
      ? `…${item.label.slice(-Math.floor(w / 6.6) + 1)}`
      : item.label;
    group.append(add("text", { x: x + 9, y: y + 15 }, label));
    const open = () => openThing(item.id);
    group.addEventListener("click", open);
    group.addEventListener("keydown", (event) => { if (event.key === "Enter") open(); });
    svg.append(group);
    return { x, y, w };
  };

  const centerBox = { x: (width - colW) / 2, y: 30, w: colW };
  columns.forEach((column, columnIndex) => {
    const x = columnIndex === 0 ? 10 : columnIndex === 1 ? centerBox.x : width - colW - 10;
    if (column.label) {
      svg.append(add("text", { class: "nb-col", x: x + 2, y: 18 },
        `${column.label}${column.total > column.items.length ? ` (${column.items.length} of ${column.total})` : ""}`));
    }
    column.items.forEach((item, rowIndex) => {
      const y = 30 + rowIndex * rowH;
      const box = place(item, x, y, colW, columnIndex === 1);
      if (columnIndex === 0) {
        svg.append(add("path", { class: "nb-edge",
          d: `M ${box.x + colW} ${y + 11} C ${box.x + colW + 40} ${y + 11}, ${centerBox.x - 40} ${41}, ${centerBox.x} ${41}` }));
      }
      if (columnIndex === 2) {
        svg.append(add("path", { class: "nb-edge",
          d: `M ${centerBox.x + colW} ${41} C ${centerBox.x + colW + 40} ${41}, ${box.x - 40} ${y + 11}, ${box.x} ${y + 11}` }));
      }
    });
  });

  let y = 46 + tallest * rowH;
  for (const group of extras) {
    svg.append(add("text", { class: "nb-col", x: centerBox.x + 2, y: y + 10 }, group.label));
    y += 18;
    for (const item of group.items) {
      place(item, centerBox.x, y, colW, false);
      y += rowH;
    }
  }
  return svg;
}

/* ------------------------------------------------------------------- find */

const overlay = document.getElementById("find-overlay");
const findInput = document.getElementById("find-input");
const findResults = document.getElementById("find-results");

function openFind() {
  overlay.hidden = false;
  findInput.value = "";
  findResults.replaceChildren(h("p", { class: "finder-empty", text: "Type to search names, paths and titles." }));
  findInput.focus();
}
function closeFind() { overlay.hidden = true; }

let findTimer = null;
findInput.addEventListener("input", () => {
  clearTimeout(findTimer);
  findTimer = setTimeout(async () => {
    const query = findInput.value.trim();
    if (!query) return findResults.replaceChildren();
    const data = await api(`/api/find?q=${encodeURIComponent(query)}&limit=20`);
    state.findRows = data.hits;
    state.findIndex = 0;
    if (!data.hits.length) {
      findResults.replaceChildren(h("p", { class: "finder-empty", text: "Nothing by that name." }));
      return;
    }
    findResults.replaceChildren(...data.hits.map((hit, index) =>
      h("button", { class: index === 0 ? "on" : "", onclick: () => { closeFind(); openThing(hit.target.id); } },
        glyph(hit.target.glyph),
        h("span", { text: hit.target.label }),
        h("span", { class: "why", text: hit.features.length
          ? `${hit.state || hit.because} · serves ${hit.features.map((f) => f.label).join(", ")}`
          : hit.state || hit.because }))));
    if (data.note) findResults.append(h("p", { class: "finder-empty", text: data.note }));
  }, 160);
});

findInput.addEventListener("keydown", (event) => {
  const buttons = [...findResults.querySelectorAll("button")];
  if (event.key === "Escape") return closeFind();
  if (!buttons.length) return;
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    state.findIndex = (state.findIndex + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length;
    buttons.forEach((button, index) => button.classList.toggle("on", index === state.findIndex));
    buttons[state.findIndex].scrollIntoView({ block: "nearest" });
  }
  if (event.key === "Enter") { event.preventDefault(); buttons[state.findIndex].click(); }
});

overlay.addEventListener("click", (event) => { if (event.target === overlay) closeFind(); });
document.getElementById("open-find").addEventListener("click", openFind);
document.addEventListener("keydown", (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
    event.preventDefault();
    openFind();
  }
});

/* ----------------------------------------------------------------- chrome */

function paintChrome(snapshot) {
  if (!snapshot) return;
  rememberVisit(snapshot);
  document.getElementById("project").textContent = snapshot.prefix;
  const seconds = Math.max(0, Math.round((snapshot.generated_at_ms - snapshot.changed_at_ms) / 1000));
  const freshness = seconds < 90 ? "updated just now"
    : seconds < 5400 ? `updated ${Math.round(seconds / 60)} minutes ago`
    : seconds < 172800 ? `updated ${Math.round(seconds / 3600)} hours ago`
    : `updated ${Math.round(seconds / 86400)} days ago`;
  document.getElementById("freshness").textContent = freshness;
  const drift = document.getElementById("drift");
  if (drift) {
    const tree = snapshot.working_tree;
    if (tree && tree.state !== "in_step") {
      drift.textContent = tree.sentence;
      drift.hidden = false;
    } else {
      drift.hidden = true;
      drift.textContent = "";
    }
  }
}

for (const button of document.querySelectorAll("[data-go]")) {
  button.addEventListener("click", () => go(button.dataset.go));
}

window.addEventListener("hashchange", render);

/* Cheap liveness: the cursor only moves when the graph does. */
setInterval(async () => {
  try {
    const snapshot = await api("/api/snapshot");
    if (state.snapshot && snapshot.cursor !== state.snapshot.cursor) {
      shelfCache = null;
      render();
    } else {
      paintChrome(snapshot);
    }
  } catch (_) { /* the server went away; keep showing the last view */ }
}, 6000);

/* =====================================================================
   Work — who did something, and what is unfinished.
   ===================================================================== */

views.work = async () => {
  const data = await api("/api/work");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);
  const parts = [
    h("h1", { class: "lede", text: data.headline }),
    h("p", { class: "sub", text: "Agent sessions, governed changes, and plans still open." }),
  ];

  // The live warnings lead everything: intervene or trust is decided
  // here, in seconds.
  if (data.signals.length) {
    parts.push(h("h2", { class: "section", text: "Live now" }));
    parts.push(h("div", { class: "concerns" }, data.signals.map((item) => {
      const node = h("div", { class: `concern ${item.severity}` },
        h("i", { class: `mark ${({ act: "bad", watch: "watch" })[item.severity] ?? "quiet"}` }),
        h("div", {},
          h("h3", { text: item.title }),
          h("p", { text: item.reason }),
          fixLine(item.fix_command)));
      if (item.target) {
        node.style.cursor = "pointer";
        node.addEventListener("click", () => openThing(item.target.id));
      }
      return node;
    })));
  }

  // The desk leads: a decision that is waiting outranks history.
  if (data.approvals.length) {
    parts.push(h("h2", { class: "section", text: "Waiting for your decision" }));
    parts.push(h("div", { class: "approvals" }, data.approvals.map(approvalCard)));
  }

  if (data.sessions_hint) {
    parts.push(h("p", { class: "empty", text: data.sessions_hint }));
    if (data.sessions_hint_command) parts.push(commandLine(data.sessions_hint_command));
  }
  for (const session of data.sessions) {
    parts.push(sessionCard(session));
  }

  if (data.rework.length) {
    parts.push(h("h2", { class: "section", text: "Handed back and forth" }));
    parts.push(h("div", { class: "facts" }, data.rework.map((fact) =>
      h("button", { class: "fact watch", onclick: () => fact.target && openThing(fact.target.id) },
        h("i", { class: "mark" }),
        h("span", {}, h("strong", { text: fact.text }), fact.reason ? ` — ${fact.reason}` : "")))));
  }

  if (data.changes.length) {
    parts.push(h("h2", { class: "section", text: "Governed changes" }));
    parts.push(h("div", { class: "items" }, data.changes.map(workItem)));
  }
  if (data.plans.length) {
    parts.push(h("h2", { class: "section", text: "Plans still open" }));
    parts.push(h("div", { class: "items" }, data.plans.map(workItem)));
  }
  stage.replaceChildren(h("div", { class: "page" }, ...parts));
};

/* One proposed change: what it does, what applying it would reach, and
   the command that applies it. The diff unfolds — a summary you cannot
   unfold is a summary you cannot check. */
function approvalCard(approval) {
  const head = h("div", { class: "approval-head" },
    h("button", { class: "row-link", text: approval.target, onclick: () => openThing(approval.id) }),
    h("span", { class: "approval-when", text: `proposed ${approval.when}` }));
  const body = [
    h("p", { class: "approval-reason", text: approval.reason }),
    approval.diff.length
      ? h("details", { class: "approval-diff" },
          h("summary", { text: approval.summary }),
          diffBlock(approval.diff, approval.diff_note))
      : h("p", { class: "approval-reason", text: approval.summary }),
  ];
  if (approval.briefing.length) {
    body.push(h("div", { class: "concerns" }, approval.briefing.map((item) =>
      h("div", { class: `concern ${item.severity}` },
        h("i", { class: `mark ${({ act: "bad", watch: "watch" })[item.severity] ?? "quiet"}` }),
        h("div", {},
          h("h3", { text: item.title }),
          item.reason ? h("p", { text: item.reason }) : null)))));
  }
  body.push(commandLine(approval.apply_command));
  body.push(featureTag(approval.features, (f) => openThing(f.id)));
  return h("div", { class: "approval" }, head, ...body);
}

/* The recorded diff, line by line: removed above added, context dim. */
function diffBlock(rows, note) {
  return h("div", { class: "diff" },
    h("pre", { class: "diff-lines" }, rows.map((row) =>
      h("span", { class: `diff-line ${row.kind}`,
        text: `${({ gone: "- ", new: "+ " })[row.kind] ?? "  "}${row.text}\n` }))),
    note ? h("p", { class: "diff-hidden", text: note }) : null);
}

function sessionCard(session) {
  const head = h("div", { class: "session-head" },
    h("span", { class: "session-actor", text: session.agent_label }),
    session.model ? h("span", { class: "session-meta", text: session.model }) : null,
    h("span", { class: "session-meta", text: "·" }),
    session.live
      ? h("span", { class: "session-meta" }, h("i", { class: "session-live-dot" }), session.state)
      : h("span", { class: "session-meta", text: session.state }));

  const meta = h("p", { class: "pill-row" },
    h("span", { text: `${session.turns} instruction${session.turns === 1 ? "" : "s"}` }),
    h("span", { text: `spanned ${session.ran_for}` }),
    h("span", { text: `${session.touched.length + session.more_touched} file${
      session.touched.length + session.more_touched === 1 ? "" : "s"} edited` }));

  const body = [head, h("p", { class: "session-objective", text: session.objective }), meta];
  if (session.outcome) {
    body.push(h("p", { class: "pill-row" }, h("span", { text: session.outcome })));
  }

  if (session.tools.length) {
    body.push(h("div", { class: "chips" }, session.tools.map((tool) =>
      h("span", { class: "chip", text: `${tool.label} ${tool.count}` }))));
  }
  if (session.touched.length) {
    body.push(h("div", { class: "touched" },
      session.touched.map((ref) =>
        h("a", { href: `#thing?id=${encodeURIComponent(ref.id)}`, text: ref.label })),
      session.more_touched
        ? h("span", { class: "session-meta", text: `and ${session.more_touched} more` })
        : null));
  }
  if (session.produced.length) {
    body.push(h("p", { class: "pill-row" }, h("span", { text: "produced" }),
      ...session.produced.map((ref) =>
        h("a", { href: `#thing?id=${encodeURIComponent(ref.id)}`, text: ref.label }))));
  }
  // What the work was on, derived from the files it edited.
  if (session.features.length) {
    body.push(h("p", { class: "pill-row" }, h("span", { text: "worked on" }),
      featureTag(session.features, (f) => openThing(f.id))));
  }
  return h("article", { class: `session${session.live ? " live" : ""}` }, ...body);
}

function workItem(item) {
  return h("button", { class: "item", onclick: () => openThing(item.id) },
    h("div", { class: "item-head" }, glyph(item.glyph), h("h3", { text: item.title }),
      chip(item.stage, item.tone)),
    h("p", { class: "item-sub", text: item.note }),
    item.features.length
      ? h("p", { class: "pill-row" }, featureTag(item.features, (f) => openThing(f.id)))
      : null,
    item.fix_command ? commandLine(item.fix_command) : null);
}

/* A command you can copy. Eyes never runs it. */
function commandLine(command) {
  return h("code", {
    class: "command", text: command, title: "Click to copy",
    onclick: (event) => {
      event.stopPropagation();
      navigator.clipboard?.writeText(command);
      const node = event.currentTarget;
      const was = node.textContent;
      node.textContent = "copied";
      setTimeout(() => { node.textContent = was; }, 900);
    },
  });
}

/* =====================================================================
   Features — the definition of done, with its evidence resolved.
   ===================================================================== */

/**
 * How much of the graph belongs to a feature at all.
 *
 * A different question from the census on Now, asked of a different
 * population: that one asks whether a claim can show its proof, this one
 * whether a record is claimed by anything. Mixing them would count two
 * things as one.
 */
function coverageView(coverage) {
  if (!coverage) return null;
  return h("section", { class: "coverage" },
    h("p", { class: "coverage-line", text: coverage.sentence }),
    h("div", { class: "coverage-rows" }, coverage.rows.map((row) => {
      const share = row.total ? Math.round((row.claimed / row.total) * 100) : 0;
      return h("div", { class: "coverage-row" },
        h("p", { class: "coverage-head" },
          h("span", { text: row.label }),
          h("span", { class: "coverage-count", text: `${row.claimed}/${row.total}` })),
        h("div", { class: "coverage-bar", title: `${share}%` },
          h("i", { "data-tone": row.tone, style: `width:${share}%` })),
        row.note ? h("p", { class: "coverage-note", text: row.note }) : null,
        // Never a bare count: what is not claimed can be opened.
        row.unclaimed.length
          ? h("details", { class: "also" },
              h("summary", { text: `${row.unclaimed_total} not claimed` }),
              h("ul", {}, row.unclaimed.map((ref) =>
                h("li", {}, h("button", { class: "linky", text: ref.label,
                                          onclick: () => openThing(ref.id) }))),
                row.unclaimed_total > row.unclaimed.length
                  ? h("li", { class: "faint",
                      text: `${row.unclaimed_total - row.unclaimed.length} more are not listed here` })
                  : null))
          : null);
    })));
}

/* =====================================================================
   Roadmap — what is planned, what is moving, what is done.
   ===================================================================== */

views.roadmap = async () => {
  const data = await api("/api/roadmap");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  const inflightRow = (item) =>
    h("div", { class: `inflight ${item.tone}` },
      h("button", { class: "inflight-head", onclick: () => openThing(item.id) },
        glyph(item.glyph), h("strong", { text: item.title }), chip(item.stage, item.tone)),
      h("p", { class: "item-sub", text: item.note }),
      // A derived attribution always shows the join that justifies it.
      item.because ? h("p", { class: "inflight-why", text: item.because }) : null,
      item.fix_command ? commandLine(item.fix_command) : null);

  const featureRow = (row) =>
    h("div", { class: "road-feature" },
      h("button", { class: "road-feature-head", onclick: () => openThing(row.id) },
        glyph("hexagon"),
        h("strong", { text: row.title }),
        strip(row.strip),
        h("span", { class: "road-verdict", text: row.verdict })),
      row.last_touched
        ? h("p", { class: "road-when" }, `last moved ${row.last_touched}`,
            row.last_touched_what
              ? h("button", { class: "linky", text: row.last_touched_what.label,
                              onclick: () => openThing(row.last_touched_what.id) })
              : null)
        : null,
      row.inflight.length
        ? h("div", { class: "inflights" }, row.inflight.map(inflightRow))
        : null);

  const parts = [
    h("h1", { class: "lede", text: data.headline }),
    h("p", { class: "sub", text: data.note }),
  ];

  for (const phase of data.stages) {
    parts.push(h("section", { class: "road-stage" },
      h("div", { class: "road-stage-head" },
        h("h2", { class: "road-stage-title", text: phase.title }),
        phase.state ? chip(phase.state, phase.tone) : null),
      phase.summary ? h("p", { class: "road-stage-summary", text: phase.summary }) : null,
      h("p", { class: "road-stage-verdict", text: phase.verdict }),
      phase.features.length
        ? h("div", { class: "road-features" }, phase.features.map(featureRow))
        : null));
  }

  if (data.unplanned.length) {
    parts.push(h("h2", { class: "section", text: "Not planned for any stage" }));
    parts.push(h("div", { class: "road-features" }, data.unplanned.map(featureRow)));
  }
  if (data.unattributed.length) {
    parts.push(h("h2", { class: "section", text: "In flight, belonging to nothing yet" }));
    parts.push(h("div", { class: "inflights" }, data.unattributed.map(inflightRow)));
  }

  stage.replaceChildren(h("div", { class: "page" }, ...parts));
};

views.features = async (params) => {
  const data = await api("/api/features");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  if (!data.roots.length) {
    stage.replaceChildren(h("div", { class: "page" },
      h("h1", { class: "lede", text: data.headline }),
      h("p", { class: "sub", text: "A feature is what the system claims to do. The graph holds none yet." }),
      commandLine("brain feature add <prefix> <slug> --title \"…\"")));
    return;
  }

  // A colour nobody can decode is decoration. The strip says what its
  // cells mean, once, next to the strips themselves.
  const legend = h("p", { class: "legend" }, [
    ["ready", "proven"],
    ["unproven", "linked, but nothing establishes it"],
    ["failing", "contradicted"],
    ["absent", "nothing linked"],
  ].map(([cell, meaning]) =>
    h("span", {}, h("i", { class: "strip" }, h("i", { "data-cell": cell })), meaning)));

  const host = h("div", {});
  stage.replaceChildren(h("div", { class: "page" },
    h("h1", { class: "lede", text: data.headline }),
    h("p", { class: "sub", text: data.note }),
    legend,
    coverageView(data.coverage),
    host));

  table(host, {
    rows: data.roots,
    noun: "features",
    keyOf: (row) => row.id,
    childrenOf: (row) => row.parts,
    // Unfinished roots open themselves. When every root is finished the
    // page would otherwise be one collapsed line, so open them all — a
    // tree with a single root is not a list.
    expandedByDefault: (data.roots.some((r) => !r.done)
      ? data.roots.filter((r) => !r.done)
      : data.roots).map((r) => r.id),
    search: (row) => `${row.title} ${row.slug} ${row.status} ${row.verdict}`,
    placeholder: "Filter features…",
    facets: [
      {
        id: "state",
        get: (row) => (row.done ? "ready" : row.met === 0 ? "not started" : "in progress"),
        options: [
          { value: "in progress", label: "in progress" },
          { value: "not started", label: "not started" },
          { value: "ready", label: "ready" },
        ],
      },
      { id: "status", get: (row) => row.status },
      {
        id: "shape",
        get: (row) => (row.by_parts ? "has parts" : "a single claim"),
        options: [{ value: "has parts", label: "has parts" }, { value: "a single claim", label: "a single claim" }],
      },
    ],
    sort: [
      { key: "title", label: "by name", by: (row) => row.title },
      { key: "ready", label: "by readiness", by: (row) => row.met / Math.max(row.total, 1) },
      { key: "changed", label: "by last change", by: (row) => -row.at_ms },
    ],
    columns: [
      { key: "title", label: "Feature", width: "minmax(220px, 2fr)",
        cell: (row) => h("span", { class: "pill-row" },
          kindIcon("hexagon"),
          h("span", { text: row.title })) },
      { key: "dims", label: "Parts", width: "70px", class: "dim",
        cell: (row) => strip(row.strip, () => showInspector(row.id)) },
      { key: "score", label: "Ready", width: "62px", class: "dim num",
        cell: (row) => `${row.met}/${row.total}` },
      { key: "verdict", label: "Verdict", width: "minmax(200px, 2fr)", class: "dim nowrap",
        cell: (row) => row.verdict },
      { key: "status", label: "Status", width: "90px", class: "dim",
        cell: (row) => chip(row.status, "quiet") },
      { key: "changed", label: "Changed", width: "88px", class: "dim num",
        cell: (row) => row.when },
    ],
    onPeek: (row) => showInspector(row.id),
    onPush: (row) => openThing(row.id),
    empty: "No feature matches that.",
  });
  const _ = params;
};

/* =====================================================================
   Evidence — claim on the left, proof on the right, never the reverse.
   ===================================================================== */

views.evidence = async (params) => {
  const data = await api("/api/evidence");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);
  const only = params.category;
  const shown = only ? data.claims.filter((claim) => claim.category === only) : data.claims;

  const filters = h("div", { class: "chips" },
    h("button", { class: `chip${only ? "" : " on"}`, text: "everything",
      onclick: () => go("evidence") }),
    ...data.categories.map((category) =>
      h("button", {
        class: `chip${only === category.id ? " on" : ""}`,
        title: category.note,
        text: `${category.label} · ${category.unsupported ? `${category.unsupported} unproven` : "all proven"}`,
        onclick: () => go("evidence", { category: category.id }),
      })));

  stage.replaceChildren(h("div", { class: "page" },
    h("h1", { class: "lede", text: data.headline }),
    h("p", { class: "sub", text: "A claim is never shown stronger than the proof behind it." }),
    filters,
    ...shown.map(claimRow)));
};

function claimRow(claim) {
  const mark = (tone) => ({ good: "✓", bad: "✗", watch: "!" }[tone] || "·");
  return h("div", { class: `claim${claim.supported ? "" : " unsupported"}` },
    h("div", { class: "claim-side" },
      h("h3", { text: claim.claim }),
      h("p", { class: "claim-verdict", text: claim.verdict }),
      claim.subject
        ? h("p", {}, h("a", {
            href: `#thing?id=${encodeURIComponent(claim.subject.id)}`,
            text: claim.subject.label }))
        : null,
      claim.fix_command ? commandLine(claim.fix_command) : null),
    h("ul", { class: "proof" }, claim.proof.map((proof) =>
      h("li", {},
        h("span", { class: `mark ${proof.tone}`, text: mark(proof.tone) }),
        h("span", {}, proof.target
          ? h("a", { href: `#thing?id=${encodeURIComponent(proof.target.id)}`, text: proof.text })
          : proof.text,
          proof.basis ? h("span", { class: "basis", text: ` — ${proof.basis}` }) : null)))));
}

/* =====================================================================
   Tests — every case, every run, and the evidence a failure left behind.
   ===================================================================== */

views.tests = async (params) => {
  const data = await api("/api/tests");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  // Cases are grouped by the file or module that defines them, so a
  // suite reads as a suite rather than 150 loose rows.
  const groups = new Map();
  for (const row of data.cases) {
    if (!groups.has(row.group)) groups.set(row.group, []);
    groups.get(row.group).push(row);
  }
  const rows = [...groups.entries()].map(([name, cases]) => ({
    group: true,
    id: `group:${name}`,
    name,
    cases,
    failing: cases.filter((c) => c.result === "failing").length,
    frameworks: [...new Set(cases.map((c) => c.framework).filter(Boolean))],
  }));

  const host = h("div", {});
  const parts = [
    h("h1", { class: "lede", text: data.headline }),
    h("p", { class: "sub",
      text: `${data.declared} tests declared across ${data.files} files; ${data.cases.length} have a recorded result.` }),
    host,
  ];

  if (data.protocols.length) {
    parts.push(h("h2", { class: "section", text: "Runs" }));
    for (const run of data.protocols.slice(0, 8)) parts.push(protocolRow(run));
  }
  if (data.uncovered.length) {
    parts.push(h("h2", { class: "section", text: "Depended on, but no test touches them" }));
    parts.push(h("div", { class: "chips" }, data.uncovered.map((item) =>
      h("button", { class: "chip", text: item.label, onclick: () => openThing(item.id) }))));
  }
  stage.replaceChildren(h("div", { class: "page" }, ...parts));

  const verdictOf = (row) => (row.group
    ? (row.failing ? "failing" : "passing")
    : row.result);

  table(host, {
    rows,
    noun: "suites",
    keyOf: (row) => row.id,
    childrenOf: (row) => (row.group ? row.cases : null),
    expandedByDefault: rows.filter((r) => r.failing).map((r) => r.id),
    search: (row) => (row.group ? row.name : `${row.name} ${row.error ?? ""}`),
    placeholder: "Filter tests…",
    facets: [
      {
        id: "result",
        get: verdictOf,
        options: [
          { value: "failing", label: "failing" },
          { value: "skipped", label: "skipped" },
          { value: "passing", label: "passing" },
        ],
      },
      { id: "framework", get: (row) => (row.group ? row.frameworks : row.framework) },
      { id: "feature", get: (row) => (row.group ? [] : row.features.map((f) => f.label)) },
      {
        id: "history",
        get: (row) => (!row.group && row.flips >= 3 ? "changed its mind" : null),
        options: [{ value: "changed its mind", label: "changed its mind" }],
      },
    ],
    sort: [
      { key: "name", label: "by name", by: (row) => row.name },
      { key: "result", label: "failing first", by: (row) => (verdictOf(row) === "failing" ? 0 : 1) },
      { key: "size", label: "by size", by: (row) => -(row.cases?.length ?? 0) },
    ],
    columns: [
      { key: "name", label: "Test", width: "minmax(320px, 3fr)",
        cell: (row) => row.group
          ? h("span", { class: "pill-row" }, kindIcon("diamond"), h("span", { class: "record", text: row.name }))
          : h("span", { class: "case-name", text: row.name.split("::").pop() }) },
      { key: "result", label: "Result", width: "88px",
        cell: (row) => row.group
          ? h("span", { class: "dim", text: `${row.cases.length} case${row.cases.length === 1 ? "" : "s"}` })
          : h("span", { class: `chip ${row.tone}`, text: row.result }) },
      { key: "detail", label: "Detail", width: "minmax(180px, 2fr)", class: "dim nowrap",
        cell: (row) => {
          if (row.group) {
            if (row.failing) return h("span", { class: "case-error", text: `${row.failing} failing` });
            const extra = row.frameworks.join(", ");
            return extra ? `all passing · ${extra}` : "all passing";
          }
          if (row.error) return h("span", { class: "case-error", text: row.error });
          if (row.note) return h("span", { class: "case-note", text: row.note });
          return "";
        } },
      { key: "evidence", label: "Evidence", width: "110px", class: "dim",
        cell: (row) => {
          if (row.group || !row.attachments.length) return "";
          return h("span", { class: "chips" }, row.attachments.map((a) =>
            h("button", { class: "chip", text: a.noun,
              onclick: (e) => { e.stopPropagation(); openThing(a.id); } })));
        } },
      { key: "serves", label: "Serves", width: "minmax(120px, 1.4fr)", class: "nowrap",
        cell: (row) => (row.group ? "" : featureTag(row.features, (f) => openThing(f.id), 2) ?? "") },
      { key: "duration", label: "Took", width: "76px", class: "dim num",
        cell: (row) => (row.group ? "" : row.duration ?? "") },
    ],
    onPeek: (row) => !row.group && showInspector(row.id),
    onPush: (row) => !row.group && openThing(row.id),
    empty: "No test matches that.",
  });
  const _ = params;
};

function protocolRow(run) {
  const width = (n) => `${(n / Math.max(run.total, 1)) * 100}%`;
  return h("button", { class: "protocol", onclick: () => openThing(run.id) },
    h("span", { class: "when", text: run.when }),
    h("div", {},
      h("p", { class: "item-sub" },
        h("b", { text: run.verdict }), " ", run.source,
        run.duration ? ` · ${run.duration}` : ""),
      h("div", { class: "bar-split" },
        h("span", { class: "pass", style: `width:${width(run.passed)}` }),
        h("span", { class: "fail", style: `width:${width(run.failed)}` }),
        h("span", { class: "skip", style: `width:${width(run.skipped)}` })),
      run.named.length
        ? h("p", { class: "session-meta",
            text: `named: ${run.named.slice(0, 6).map((c) => c.name).join(", ")}` })
        : null,
      run.evidence ? h("p", { class: "session-meta", text: run.evidence }) : null),
    run.verified_change
      ? h("span", { class: "chip", text: `verified ${run.verified_change.label}` })
      : null);
}

/* =====================================================================
   Media — the narrated tour and everything else that was captured.
   ===================================================================== */

views.media = async () => {
  const data = await api("/api/media");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);
  const parts = [h("h1", { class: "lede", text: data.headline })];

  if (data.tour) parts.push(tourPanel(data.tour));

  if (data.items.length) {
    parts.push(h("h2", { class: "section", text: "Everything captured" }));
    parts.push(h("div", { class: "media-grid" }, data.items.map(mediaCard)));
  }
  stage.replaceChildren(h("div", { class: "library" },
    shelfRail(await shelvesFor(), "media"), h("div", { class: "page" }, ...parts)));
};

function tourPanel(tour) {
  const body = [
    h("div", { class: "pill-row" },
      chip(tour.state, tour.tone),
      h("span", { text: tour.state_note })),
  ];

  if (tour.drift.length) {
    body.push(h("div", { class: "drift" },
      h("p", { text: "The recording still says things the graph has moved on from:" }),
      ...tour.drift.map((entry) => h("p", {},
        entry.recorded ? h("del", { text: entry.recorded }) : null,
        entry.recorded && entry.current ? " → " : null,
        entry.current ? h("ins", { text: entry.current }) : null)),
      commandLine(tour.regenerate_command)));
  }

  body.push(h("h3", { class: "section", text: "Chapters" }));
  for (const chapter of tour.chapters) {
    body.push(h("button", { class: "chapter",
      onclick: () => chapter.image && openThing(chapter.image.id) },
      chapter.image
        ? h("img", { src: `/api/body?id=${encodeURIComponent(chapter.image.id)}`,
                     alt: chapter.title, loading: "lazy" })
        : h("span", { class: "session-meta", text: "no picture" }),
      h("div", {},
        h("h4", { text: chapter.title.replace(/\s*\(`[^`]*`\)/, "") }),
        commandLine(chapter.command),
        chapter.narration
          ? h("p", { class: "chapter-narration speakable", text: chapter.narration })
          : h("p", { class: "session-meta", text: "the script says nothing about this chapter" }))));
  }

  return h("section", { class: "tour" },
    tour.video
      ? h("video", { controls: "controls", preload: "metadata",
                     src: `/api/body?id=${encodeURIComponent(tour.video.id)}` })
      : null,
    h("div", { class: "tour-body" }, ...body));
}

function mediaCard(item) {
  return h("button", { class: "shot", onclick: () => openThing(item.id) },
    item.subtype === "image"
      ? h("img", { src: `/api/body?id=${encodeURIComponent(item.id)}`, alt: item.label, loading: "lazy" })
      : null,
    h("div", { class: "shot-body" },
      h("h4", { text: item.label }),
      h("p", { class: "pill-row" }, chip(item.state, item.tone), h("span", { text: item.when })),
      h("p", { class: "session-meta", text: item.state_note }),
      item.rendered_from ? commandLine(item.rendered_from) : null));
}

/* =====================================================================
   MRI — the living graph.
   ===================================================================== */

const MRI_LENSES = [
  ["anatomy", "Anatomy", "Colour by what kind of thing it is."],
  ["activity", "Activity", "Only what moved since your last session."],
  ["depth", "Dependency depth", "How far up the stack it sits."],
];

views.mri = async (params) => {
  const data = await api("/api/mri");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  const host = h("div", { class: "mri-stage" });
  const readout = h("div", { class: "mri-panel mri-readout" });
  const lensPanel = h("div", { class: "mri-panel mri-lenses" },
    h("h3", { text: "Lens" }),
    ...MRI_LENSES.map(([id, label, note]) =>
      h("button", {
        class: params.lens === id || (!params.lens && id === "anatomy") ? "on" : "",
        title: note, text: label,
        onclick: (event) => {
          for (const button of lensPanel.querySelectorAll("button")) button.classList.remove("on");
          event.currentTarget.classList.add("on");
          mriHandle?.setLens(id);
        },
      })),
    h("h3", { text: "Regions" }),
    ...data.clusters.map((cluster) =>
      h("button", {
        text: `${cluster.label} · ${cluster.count}`,
        onclick: () => mriHandle?.focusOn(cluster.id),
      })));

  const hint = h("div", { class: "mri-panel mri-hint" },
    h("p", { text: "Drag to turn. Scroll to approach — detail resolves as you get closer, nothing is hidden. Click anything to inspect it." }));

  stage.replaceChildren(host);
  host.append(lensPanel, hint, readout);

  const { mount } = await import("/assets/mri.js");
  mriHandle?.destroy();
  mriHandle = mount(host, data, {
    lens: params.lens || "anatomy",
    onPick: (node) => showInspector(node.id),
    // Nothing is ever hidden, so the readout says how much is at full
    // detail — not how much survived a cull.
    onFrame: ({ step, inFocus, total }) => {
      readout.replaceChildren(
        h("p", {}, h("b", { text: step.name }), " — ",
          `all ${total} drawn, ${inFocus} in focus`),
        h("p", { text: data.headline }),
        h("p", { text: "+ and − to approach, arrows to turn" }));
    },
  });
};

/* =====================================================================
   The inspector: one component, every surface.
   ===================================================================== */

const inspector = document.getElementById("inspector");
const inspectorBody = document.getElementById("inspector-body");

function closeInspector() {
  inspector.hidden = true;
  document.body.classList.remove("with-inspector");
}
document.getElementById("inspector-close").addEventListener("click", closeInspector);

async function showInspector(id) {
  document.body.classList.add("with-inspector");
  inspector.hidden = false;
  inspectorBody.replaceChildren(h("p", { class: "loading", text: "Reading…" }));
  let data;
  try {
    data = await api(`/api/thing?id=${encodeURIComponent(id)}`);
  } catch (error) {
    inspectorBody.replaceChildren(h("p", { class: "empty", text: error.message }));
    return;
  }
  document.getElementById("inspector-glyph").className = `glyph ${data.glyph || "block"}`;
  document.getElementById("inspector-kind").textContent = data.noun || "";
  document.getElementById("inspector-title").textContent = data.title || data.label;

  const section = (title, ...children) =>
    children.filter(Boolean).length
      ? h("section", { class: "inspector-section" }, h("h3", { text: title }), ...children)
      : null;

  const parts = [];
  if (data.facts?.length) {
    parts.push(section("State", h("ul", { class: "proof" }, data.facts.map((fact) =>
      h("li", {},
        h("span", { class: `mark ${fact.tone}`, text: "·" }),
        h("span", {}, fact.text,
          fact.reason ? h("span", { class: "basis", text: ` — ${fact.reason}` }) : null))))));
  }
  if (data.extras?.audit?.length) {
    parts.push(section("What happened", h("ul", { class: "proof" },
      data.extras.audit.map((entry) => h("li", {},
        h("span", { class: "mark", text: entry.recorded ? "·" : "≈" }),
        h("span", {},
          h("b", { text: `${entry.label}: ` }), entry.value,
          entry.note ? h("span", { class: "basis", text: ` — ${entry.note}` }) : null))))));
  }
  if (data.extras?.attachments?.length) {
    parts.push(section("What the run left behind",
      h("div", { class: "case-shots" }, data.extras.attachments.map((attachment) =>
        attachment.subtype === "image"
          ? h("img", { src: `/api/body?id=${encodeURIComponent(attachment.id)}`, alt: attachment.label })
          : h("button", { class: "chip", text: attachment.noun,
                          onclick: () => openThing(attachment.id) })))));
  }
  if (data.neighborhood) {
    const nb = data.neighborhood;
    const list = (label, items) => items?.length
      ? h("p", { class: "pill-row" }, h("span", { text: label }),
          ...items.slice(0, 8).map((ref) =>
            h("a", { href: `#thing?id=${encodeURIComponent(ref.id)}`, text: ref.label })))
      : null;
    // These arrive as references already; treating them as edges is what
    // left this section blank, and then threw on the first thing that had
    // a neighbour at all.
    parts.push(section("Around it",
      list("uses", nb.upstream),
      list("used by", nb.downstream),
      list("tests", nb.tests),
      list("documents", nb.docs?.concat(nb.decisions ?? []))));
  }
  if (data.body?.origin) {
    parts.push(section("Where these bytes came from",
      h("p", { class: "session-meta", text: data.body.origin })));
  }
  parts.push(h("p", {}, h("a", { href: `#thing?id=${encodeURIComponent(id)}`,
    text: "Open the full page →" })));

  inspectorBody.replaceChildren(...parts.filter(Boolean));
}

/* =====================================================================
   Brief me — the current screen, spoken.

   The sentences are the ones say.rs wrote; the browser only reads them
   aloud. No audio is generated and nothing is written, which is how a
   read-only tool gets a voice.
   ===================================================================== */

function stopSpeaking() {
  window.speechSynthesis?.cancel();
  speaking = false;
  document.getElementById("brief-me")?.setAttribute("aria-pressed", "false");
  for (const node of document.querySelectorAll(".speaking")) node.classList.remove("speaking");
}

const briefButton = document.getElementById("brief-me");

briefButton.addEventListener("click", () => {
  if (!window.speechSynthesis) {
    briefButton.textContent = "no voice in this browser";
    return;
  }
  if (speaking) return stopSpeaking();

  const spoken = Array.from(stage.querySelectorAll(
    ".hero, .verdict-sub, .census-line, .concern h3, .concern p, .lede, .headline, .subhead, .sub, .item-sub, .claim-side h3, .claim-verdict, .chapter-narration, .session-objective"
  )).filter((node) => node.textContent.trim().length > 12).slice(0, 24);

  if (!spoken.length) return;
  speaking = true;
  briefButton.setAttribute("aria-pressed", "true");

  spoken.forEach((node, index) => {
    const utterance = new SpeechSynthesisUtterance(node.textContent.trim());
    utterance.rate = 1.02;
    utterance.onstart = () => node.classList.add("speaking");
    utterance.onend = () => {
      node.classList.remove("speaking");
      if (index === spoken.length - 1) stopSpeaking();
    };
    speechSynthesis.speak(utterance);
  });
});

/* =====================================================================
   Theme — the operational screens are readable in either.
   ===================================================================== */

const themeButton = document.getElementById("theme");
const savedTheme = localStorage.getItem("eyes-theme");
if (savedTheme) document.documentElement.dataset.theme = savedTheme;
themeButton.addEventListener("click", () => {
  const dark = document.documentElement.dataset.theme === "dark"
    || (!document.documentElement.dataset.theme
        && matchMedia("(prefers-color-scheme: dark)").matches);
  const next = dark ? "light" : "dark";
  document.documentElement.dataset.theme = next;
  localStorage.setItem("eyes-theme", next);
});

/* The register: the same facts in two tellings. Plain view leads with
   the features, the roadmap, and the tour; the operator chrome —
   commands, badges, dense surfaces — recedes. Per-viewer, in this
   browser only, like the theme. */
const registerButton = document.getElementById("register");
function applyRegister(plain) {
  document.body.classList.toggle("plain", plain);
  registerButton.textContent = plain ? "Full view" : "Plain view";
}
applyRegister(localStorage.getItem("eyes-register") === "plain");
if (document.body.classList.contains("plain")
    && ["", "#", "#now"].includes(location.hash)) {
  go("features");
}
registerButton.addEventListener("click", () => {
  const plain = !document.body.classList.contains("plain");
  localStorage.setItem("eyes-register", plain ? "plain" : "full");
  applyRegister(plain);
  go(plain ? "features" : "now");
});

/* Counts on the rail, so the nav says where the trouble is. */
async function paintRail() {
  try {
    const [now, next, work, tests, evidence, roadmap] = await Promise.all([
      api("/api/now"), api("/api/next"), api("/api/work"), api("/api/tests"),
      api("/api/evidence"), api("/api/roadmap"),
    ]);
    const set = (key, value, tone) => {
      const node = document.querySelector(`[data-count="${key}"]`);
      if (!node) return;
      node.hidden = !value;
      node.textContent = value;
      if (tone) node.dataset.tone = tone; else delete node.dataset.tone;
    };
    // Occurrences, not rows: a row that collapsed four identical concerns
    // still stands for four, and the headline counts it that way.
    const notes = now.needs_you.reduce((sum, concern) => sum + concern.repeats, 0);
    set("now", notes, notes ? "watch" : null);
    const urgent = next.queue.filter((item) => item.severity === "act").length;
    set("next", next.queue.length, urgent ? "bad" : next.queue.length ? "watch" : null);
    set("work", work.sessions.filter((s) => s.live).length + work.changes.length,
      work.changes.length ? "watch" : null);
    set("tests", tests.failing.length, tests.failing.length ? "bad" : null);
    const planned = roadmap.stages.reduce((n, s) => n + s.total - s.ready, 0)
      + roadmap.unplanned.length;
    set("roadmap", planned, planned ? "watch" : null);
    set("evidence", evidence.claims.filter((c) => !c.supported).length,
      evidence.claims.some((c) => !c.supported) ? "watch" : null);
    const live = document.querySelector("[data-live]");
    if (live) live.hidden = !work.sessions.some((s) => s.live);
  } catch {
    /* The rail is decoration; a failure here must not blank the page. */
  }
}
paintRail();
setInterval(paintRail, 30000);

/* The shape vocabulary is inlined once so every icon is a local
   reference and nothing is fetched while drawing. */
function mountSprite() {
  for (const button of document.querySelectorAll(".rail button[data-icon]")) {
    const mark = button.querySelector(".rail-mark");
    if (mark) mark.replaceChildren(icon(button.dataset.icon));
  }
}

/* The first paint happens last: every view must be registered before a
   route is resolved, or a deep link falls back to Now. */
mountSprite();
render();
