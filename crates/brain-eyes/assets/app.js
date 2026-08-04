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
  return h("span", { class: "cmd-row" },
    h("code", { class: "fix" }, h("span", { text: "run" }), command),
    copyButton(command));
}

/* One toast, reused: the copy promise said out loud, then gone. */
let toastTimer = null;
function showToast(message) {
  document.querySelector(".toast")?.remove();
  clearTimeout(toastTimer);
  const node = h("div", { class: "toast", role: "status", text: message });
  document.body.append(node);
  toastTimer = setTimeout(() => node.remove(), 2400);
}

function copyButton(command) {
  return h("button", {
    class: "copy-btn", title: "Copy the command — you run it; Eyes never writes",
    onclick: (event) => {
      event.stopPropagation();
      navigator.clipboard?.writeText(command);
      showToast("Copied — you run it; Eyes never writes.");
    },
  }, icon("copy", "sm"), "Copy");
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
    class: "ack", title: "Absorbed — hidden here for a week, in this browser only",
    onclick: (event) => {
      event.stopPropagation();
      const acks = loadAcks();
      acks[ackId(item)] = Date.now();
      saveAcks(acks);
      render();
    },
  }, icon("dismiss", "sm"), "not now");
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

/* The old thirteen addresses keep working: every retired hash lands on
   the surface that absorbed it, so bookmarks, dossier links and habit
   all survive the consolidation. */
const LEGACY = {
  next: () => ["now", {}],
  tests: () => ["proof", { tab: "tests" }],
  evidence: () => ["proof", { tab: "evidence" }],
  library: (p) => ["proof", { tab: "artifacts", ...(p.shelf ? { shelf: p.shelf } : {}) }],
  concepts: () => ["proof", { tab: "artifacts", shelf: "concepts" }],
  timeline: () => ["time", {}],
  compare: (p) => ["time", { mode: "compare", ...p }],
  map: (p) => ["structure", p.lens ? { lens: p.lens } : {}],
  mri: () => ["structure", { lens: "mri" }],
};

async function render() {
  let route = readRoute();
  if (LEGACY[route.view]) {
    const [view, params] = LEGACY[route.view](route.params);
    route = { view, params };
  }
  state.view = route.view;
  state.params = route.params;
  const inMri = route.view === "structure" && route.params.lens === "mri";
  for (const button of document.querySelectorAll(".rail button")) {
    const target = button.dataset.go;
    const home = route.view === "thing" ? state.thingHome : null;
    button.classList.toggle("on", target === route.view ||
      (home !== null && target === home));
  }
  // The MRI draws its own dark theatre inside a framed card; the stage
  // around it stays lit like every other surface.
  // On Now the verdict band carries freshness, drift and the promise
  // itself — the topbar saying the same three things a centimetre above
  // would be the page repeating itself.
  document.body.classList.toggle("in-now", route.view === "now");
  if (!inMri && mriHandle) { mriHandle.destroy(); mriHandle = null; }
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


/* -------------------------------------------------------------------- now */

views.now = async () => {
  const seen = previousVisit && Number.isInteger(previousVisit.cursor) ? previousVisit.cursor : null;
  const [data, next] = await Promise.all([
    api("/api/now" + (seen === null ? "" : `?seen=${seen}`)),
    api("/api/next").catch(() => null),
  ]);
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
  // The band wears its reading's tone: fault washes the corner only
  // when a trend is falling or a concern demands a decision now.
  const bandTone = (data.quality ?? []).some((line) => line.tone === "bad")
      || data.needs_you.some((concern) => concern.severity === "act")
    ? "fault" : "calm";

  /* The queue: Now absorbed the standing work list. The rich cards from
     needs_you lead; anything the wider queue holds that they do not
     already say follows, deduplicated by title. Each card wears a
     horizon — decide now, or it can wait. */
  const main = [];
  const said = new Set(data.needs_you.map((c) => c.title));
  const extra = (next?.queue ?? []).filter((c) => !said.has(c.title));
  const inbox = splitAcked([...data.needs_you, ...extra]);
  const waiting = inbox.shown.filter((c) => horizonOf(c) === "can wait").length;
  const quietFold = state.quietOpen !== false;
  const shown = quietFold ? inbox.shown : inbox.shown.filter((c) => horizonOf(c) === "now");

  main.push(h("h2", { class: "section queue-head" },
    "Queue",
    waiting
      ? h("button", {
          class: "row-link quiet-toggle",
          text: quietFold ? `hide the ${waiting} that can wait` : `show the ${waiting} that can wait`,
          onclick: () => { state.quietOpen = !quietFold; render(); },
        })
      : null));
  if (!inbox.shown.length) {
    // Calm rendered as content, not as absence.
    main.push(h("p", { class: "quiet-verdict", text: "Nothing needs you." }));
  }
  if (inbox.shown.length || inbox.acked.length) {
    main.push(h("div", { class: "concerns" }, shown.map(concernCard)));
    const toggle = ackedToggle(inbox.acked);
    if (toggle) main.push(toggle);
  }

  /* The lighter registers keep to the side column: the delta, then the
     pressure as the ranked list it actually is. */
  const side = [];
  side.push(h("section", { class: "side-panel" },
    h("h2", { class: "section", text: data.since.known ? `Since your last session, ${data.since.when}` : "Recently" }),
    h("p", { class: "sub", text: data.since.summary }),
    data.since.episodes.length
      ? h("div", { class: "episodes" }, data.since.episodes.map(episodeRow))
      : null));

  if (data.attention.length) {
    // Churn draws as five ramping bars, filled to the measured count and
    // red past heavy; the test cell is a proof dot, a dashed fault ring,
    // or nothing when the ranking had nothing to say.
    const churnBars = (churn) => {
      if (churn === null || churn === undefined) return h("span", {});
      const filled = Math.min(5, Math.ceil(churn / 5));
      return h("span", { class: "pressure-churn" },
        [0, 1, 2, 3, 4].map((i) =>
          h("i", { "data-on": i < filled ? (churn > 15 ? "hot" : "warm") : null })));
    };
    const testDot = (tested) =>
      h("i", { class: "pressure-test", "data-state":
        tested === true ? "named" : tested === false ? "bare" : "unknown" });
    side.push(h("section", { class: "side-panel" },
      h("h2", { class: "section", text: "Where the pressure is" }),
      h("div", { class: "pressure-head", "aria-hidden": "true" },
        h("span", {}), h("span", {}),
        h("span", { text: "churn" }), h("span", { text: "reach" }), h("span", { text: "test" })),
      h("div", { class: "pressure-list" }, data.attention.map((card, index) =>
        h("button", { class: "pressure-row", title: `a ${card.noun} — ${card.reasons.join(" · ")}`,
            onclick: () => card.id && openThing(card.id) },
          h("span", { class: "pressure-rank", text: String(index + 1) }),
          h("span", { class: "pressure-body" },
            h("span", { class: "pressure-path", text: card.label }),
            h("span", { class: "pressure-why", text: card.reasons.join(" · ") })),
          churnBars(card.churn),
          h("span", { class: "pressure-reach", text: card.reach ?? "" }),
          testDot(card.tested))))));
  }

  stage.replaceChildren(h("div", { class: "page" },
    h("section", { class: "verdict", "data-tone": bandTone }, ...verdict),
    h("div", { class: "now-columns" },
      h("div", { class: "now-main" }, ...main),
      h("aside", { class: "now-side" }, ...side))));
  settleCensus();
};

/* A concern's horizon: acts and watches want a decision now; a note
   can wait. Two axes, one chip. */
function horizonOf(concern) {
  return concern.severity === "note" ? "can wait" : "now";
}

/* One concern, fully worn: severity mark, horizon chip, the unfoldable
   count, the journey with its stuck step, the subjects as chips, the
   command, and the acknowledgement. */
function concernCard(concern) {
  const horizon = horizonOf(concern);
  const node = h("div", { class: `concern ${concern.severity}` },
    h("i", { class: `mark ${({ act: "bad", watch: "watch" })[concern.severity] ?? "quiet"}` }),
    h("div", {},
      h("h3", {},
        concern.title,
        concern.repeats > 1 ? h("span", { class: "repeats", text: `×${concern.repeats}` }) : null,
        h("span", { class: `horizon${horizon === "now" ? " now" : ""}`, text: horizon })),
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
}

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
  const missing = proof.groups.some((group) => group.proven < group.total);
  return h("section", { class: "census" },
    h("p", { class: "census-line", "data-tone": missing ? "fault" : "calm", text: proof.sentence }),
    h("div", { class: "spine" }, proof.groups.map((group) =>
      h("div", { class: "spine-group" },
        h("div", { class: "census-cells" }, group.cells.map((cell) =>
          h("button", {
            class: "census-cell", "data-cell": cell.state, title: cell.text,
            // A mark with no name is unreadable to anyone not looking at it.
            "aria-label": cell.text,
            // Everything navigates: a claim that is an entity opens its
            // dossier; run-hash evidence lands on the Tests register.
            onclick: () => (cell.id.startsWith("sid:")
              ? openThing(cell.id)
              : go("proof", { tab: "tests" })),
          }))),
        h("p", { class: "census-label" },
          h("span", { text: group.label }),
          h("span", { class: "census-count", "data-tone": group.proven < group.total ? "fault" : "calm",
            text: `${group.proven}/${group.total}` }))))),
    // The legend: the state grammar spelled out once, under the spine.
    h("p", { class: "census-legend" },
      h("i", { "data-swatch": "failing" }), " failing",
      h("i", { "data-swatch": "stale" }), " stale",
      h("i", { "data-swatch": "unproven" }), " unproven",
      h("i", { "data-swatch": "proven" }), " proven"));
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
      }, glyph(ref.glyph), h("span", { text: ref.label }),
         h("span", { class: "chip-arrow", "aria-hidden": "true", text: "›" })))));
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
  const W = 118, H = 22;
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
  // The area under the trend, barely there — ground, not data.
  if (pts.length > 1) {
    const area = document.createElementNS("http://www.w3.org/2000/svg", "polygon");
    area.setAttribute("class", "spark-area");
    area.setAttribute("points", `${x(0).toFixed(1)},${H} ` +
      pts.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`).join(" ") +
      ` ${x(pts.length - 1).toFixed(1)},${H}`);
    svg.append(area);
  }
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
  const home = ({
    tests: ["proof", { tab: "tests" }],
    claims: ["proof", { tab: "evidence" }],
    features: ["features", {}],
    docs: ["proof", { tab: "artifacts", shelf: "documents" }],
  })[line.id];
  // The tempo strip: the gaps between readings, drawn as gaps — a
  // series bounded by change, not by time, says so under its trend.
  let tempo = null;
  const at = line.at_ms ?? [];
  if (at.length > 2) {
    const gaps = at.slice(1).map((t, i) => Math.max(1, t - at[i]));
    tempo = h("span", { class: "spark-tempo", "aria-hidden": "true" },
      gaps.map((gap) => h("i", { style: `flex-grow:${gap}` })));
  }
  return h("button", { class: `spark-item ${line.tone}`, title: line.sentence,
      onclick: () => home && go(home[0], home[1]) },
    h("span", { class: "spark-label", text: line.label }),
    h("span", { class: "spark-lane" }, svg, tempo),
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

/* =====================================================================
   Proof — tests, evidence and artifacts: three registers of the same
   question ("what stands behind this?") under one roof.
   ===================================================================== */

views.proof = async (params) => {
  const tab = params.tab || "tests";
  const host = h("div", {});
  const bar = h("div", { class: "tabs proof-tabs" }, [
    ["tests", "Tests", "diamond"],
    ["evidence", "Evidence", "seal"],
    ["artifacts", "Artifacts", "page"],
  ].map(([id, label, mark]) =>
    h("button", { class: tab === id ? "on" : "", onclick: () => go("proof", { tab: id }) },
      icon(mark, "sm"), label,
      h("code", { class: "tab-count", "data-tab": id }))));
  stage.replaceChildren(h("div", { class: "page" },
    h("p", { class: "kicker" }, icon("diamond"), "Prove · Proof"),
    bar, host));
  // The badges say what each register would say before it is opened;
  // the numbers arrive from their own endpoint so no tab loads early.
  api("/api/proof").then((counts) => {
    const set = (id, value, tone) => {
      const node = bar.querySelector(`[data-tab="${id}"]`);
      if (!node) return;
      node.textContent = value ? String(value) : "";
      if (tone) node.dataset.tone = tone; else delete node.dataset.tone;
    };
    set("tests", counts.tests_failing, counts.tests_failing ? "fault" : null);
    set("evidence", counts.claims, null);
    set("artifacts", counts.artifacts, null);
  }).catch(() => {});
  if (tab === "evidence") await evidencePanel(host, params);
  else if (tab === "artifacts") await artifactsPanel(host, params);
  else await testsPanel(host, params);
};

/* ------------------------------------------------------- proof: artifacts */

async function artifactsPanel(host, params) {
  const shelf = params.shelf || "decisions";
  const pills = (shelves, current) => h("nav", { class: "shelves" }, [
    ...shelves.map((s) => ({ ...s, view: s.id === "media" ? "media" : null })),
    { id: "concepts", label: "Concepts", count: null, view: null },
  ].map((s) =>
    h("button", {
      class: s.id === current ? "on" : "",
      onclick: () => (s.view ? go(s.view) : go("proof", { tab: "artifacts", shelf: s.id })),
    }, h("span", { text: s.label }), s.count !== null ? h("em", { text: s.count }) : null)));

  if (shelf === "concepts") {
    const data = await api("/api/concepts");
    state.snapshot = data.snapshot;
    paintChrome(data.snapshot);
    host.replaceChildren(
      pills(await shelvesFor(), "concepts"),
      h("h1", { class: "lede", text: "Concepts" }),
      h("p", { class: "sub",
        text: "The kinds of thing this brain knows about, what each one promises, and what it has learned about whether that promise works." }),
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
          concept.verdicts.map((verdict) => h("p", { class: "verdict", text: verdict }))))));
    return;
  }

  const data = await api(`/api/library?shelf=${encodeURIComponent(shelf)}`);
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  const tableHost = h("div", {});
  host.replaceChildren(
    pills(data.shelves, data.shelf),
    h("h1", { class: "lede", text: data.label }),
    h("p", { class: "sub", text: data.note }),
    tableHost);
  libraryTable(tableHost, data);
}

function libraryTable(host, data) {

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
    onPeek: (row) => openThing(row.id),
    onPush: (row) => openThing(row.id),
    empty: "Nothing on this shelf matches that.",
  });
}

let shelfCache = null;
async function shelvesFor() {
  if (!shelfCache) shelfCache = (await api("/api/library?shelf=decisions")).shelves;
  return shelfCache;
}

/* -------------------------------------------------------------------- map */

/* =====================================================================
   Structure — what the system is made of. The Map is the place; the
   MRI is a lens on the same anatomy, not a separate address.
   ===================================================================== */

views.structure = async (params) => {
  if (params.lens === "mri") return mriBody(params);
  const lens = params.lens || "attention";
  const data = await api(`/api/map?lens=${encodeURIComponent(lens)}`);
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  // A module's dependencies read better as a sentence in its own panel
  // than as an arc across the whole diagram: the anatomy already says
  // what depends on what by putting a block above what carries it.
  const labels = new Map(data.blocks.map((block) => [block.id, block.label]));
  const carries = new Map();
  for (const edge of data.edges) {
    if (!labels.has(edge.from) || !labels.has(edge.to)) continue;
    if (!carries.has(edge.from)) carries.set(edge.from, []);
    carries.get(edge.from).push(labels.get(edge.to));
  }

  const detail = h("div", { class: "map-detail", hidden: true });
  const head = (block) => [
    h("h2", { text: block.label }),
    h("p", { text: `${block.sentence}. ${block.facts.join(". ")}.` }),
    carries.has(block.id)
      ? h("p", { class: "map-carries",
          text: `It rests on ${carries.get(block.id).join(", ")}.` })
      : null,
  ];
  const showBlock = async (block, node) => {
    for (const other of stage.querySelectorAll(".map-block.on")) other.classList.remove("on");
    node.classList.add("on");
    detail.hidden = false;
    detail.replaceChildren(...head(block), h("p", { class: "loading", text: "Reading its files…" }));
    const files = await api(`/api/find?q=${encodeURIComponent(block.path)}&limit=60`);
    const hits = files.hits.filter((hit) => hit.target.kind === "source_file");
    detail.replaceChildren(...head(block),
      hits.length
        ? h("div", { class: "map-files" }, hits.map((hit) =>
            h("button", { class: "chip-ref", onclick: () => openThing(hit.target.id) },
              glyph("block"),
              h("span", { text: hit.target.label.replace(`${block.path}/`, "") }),
              h("span", { class: "chip-arrow", "aria-hidden": "true", text: "›" }))))
        : h("p", { class: "map-hint", text: "Nothing under this path is recorded as a file." }));
  };

  stage.replaceChildren(h("div", { class: "page map-wrap" },
    h("p", { class: "kicker" }, icon("map"), "Explore · Structure"),
    h("h1", { class: "lede", text: data.lens_label }),
    h("p", { class: "page-note", text: data.sentence }),
    h("div", { class: "lenses" },
      ...data.lenses.map(([id, label]) =>
        h("button", { class: id === data.lens ? "on" : "", text: label,
          onclick: () => go("structure", { lens: id }) })),
      h("button", { text: "3D (MRI)",
        onclick: () => go("structure", { lens: "mri" }) })),
    h("p", { class: "map-hint", text: data.lens_note }),
    mapField(data, showBlock),
    detail,
    h("p", { class: "map-hint", text: "Click a block to see what lives in it; its files open their dossiers." })));
};

/* The anatomy, laid out rather than drawn: one row per dependency layer,
   deepest at the bottom, each block sized by how much lives in it and
   tinted only when the lens found something there. No arcs — a block
   sits above what carries it, which is the same fact with less ink. */
function mapField(data, onPick) {
  const layers = new Map();
  for (const block of data.blocks) {
    if (!layers.has(block.layer)) layers.set(block.layer, []);
    layers.get(block.layer).push(block);
  }
  const ordered = [...layers.keys()].sort((a, b) => b - a);
  return h("div", { class: "map-field" }, ordered.map((layer, index) =>
    h("div", { class: "map-layer" },
      h("p", { class: "map-layer-name",
        text: index === ordered.length - 1 ? "foundation" : `layer ${layer}` }),
      h("div", { class: "map-blocks" }, layers.get(layer)
        .slice()
        .sort((a, b) => b.files - a.files)
        .map((block) => {
          const node = h("button", {
            class: "map-block", "data-tone": block.tone,
            style: `flex-grow:${Math.max(1, Math.sqrt(block.files)).toFixed(2)}`,
            onclick: () => onPick(block, node),
          },
            h("strong", { class: "map-block-name", text: block.label }),
            h("code", { class: "map-block-meta",
              // A quiet block says how much it holds; a block the lens
              // marked says what it found instead.
              text: block.tone === "quiet"
                ? `${block.files} files · ${block.symbols} defs`
                : block.sentence }));
          return node;
        })))));
}

/* --------------------------------------------------------------- timeline */

/* =====================================================================
   Time — what happened, and any moment held against the present. The
   timeline is the place; compare is a mode of it, never a second page.
   ===================================================================== */

views.time = async (params) => {
  if (params.from) return compareBody(params);
  const [data, moments] = await Promise.all([
    api("/api/timeline?limit=60"),
    api("/api/moments").catch(() => null),
  ]);
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);
  // Batches cluster by the day they happened on. The day is a record,
  // so it is stamped as one; the row keeps the server's own phrasing of
  // how long ago it was.
  const days = [];
  for (const episode of data.episodes) {
    // Pinned to the language the rest of the cockpit speaks: say.rs
    // writes English, so a weekday in the viewer's own locale would be
    // the one foreign word on the page.
    const day = new Date(episode.at_ms).toLocaleDateString("en-GB",
      { weekday: "long", day: "numeric", month: "long" });
    if (!days.length || days[days.length - 1].day !== day) days.push({ day, episodes: [] });
    days[days.length - 1].episodes.push(episode);
  }
  for (const day of days) day.rows = foldRuns(day.episodes);

  stage.replaceChildren(h("div", { class: "page" },
    h("p", { class: "kicker" }, icon("history"), "Explore · Time"),
    h("h1", { class: "lede", text: "Timeline" }),
    h("p", { class: "page-note",
      text: "Everything the graph recorded, grouped into the batches it actually happened in. Pick a moment below to hold it against the present." }),
    days.length
      ? days.map((day) => h("section", { class: "day" },
          h("p", { class: "day-head", text: day.day }),
          h("div", { class: "panel episodes" }, day.rows.map(timelineRow))))
      : h("p", { class: "empty", text: "Nothing recorded yet." }),
    moments
      ? [
          h("h2", { class: "section", text: "Moments to hold against the present" }),
          h("p", { class: "sub", text: moments.headline }),
          moments.moments.length
            ? h("div", { class: "moments" }, moments.moments.slice(0, 12).map((moment) =>
                h("button", { class: "moment", onclick: () => go("time", { from: moment.value }) },
                  h("span", { class: "moment-label", text: moment.label }),
                  h("span", { class: "moment-when", text: moment.when }))))
            : null,
        ]
      : null));
};

/* What kind of batch this was, as the shape the kind already carries
   everywhere else. Nothing here is a verdict — the mark says which
   register the batch belongs to, not whether it went well. */
const EPISODE_MARK = {
  observation: "block", session: "orbit", review: "shield",
  tests: "diamond", change: "chevron",
};

/* A run of identical batches — six documents each confirmed still
   accurate in the same minute — is one thing that happened six times,
   not six things. Only batches with nothing of their own to show fold,
   so no file, fact or feature is ever collapsed away. */
function foldRuns(episodes) {
  const rows = [];
  for (const episode of episodes) {
    const bare = !episode.items.length && !episode.facts.length && !episode.features.length;
    const previous = rows[rows.length - 1];
    if (bare && previous?.bare
        && previous.episode.kind === episode.kind
        && previous.episode.title === episode.title) {
      previous.repeats += 1;
      continue;
    }
    rows.push({ episode, bare, repeats: 1 });
  }
  return rows;
}

function timelineRow({ episode, repeats }) {
  return h("div", { class: "episode" },
    h("time", { text: episode.when }),
    h("span", { class: "episode-mark", title: episode.kind },
      icon(EPISODE_MARK[episode.kind] ?? "block", "sm")),
    h("div", {},
      h("strong", {},
        episode.title,
        repeats > 1 ? h("span", { class: "repeats", text: `×${repeats}` }) : null),
      episode.facts.map((fact) => h("p", { text: fact })),
      episode.items.length
        ? h("p", {}, episode.items.map((item, index) => [
            index ? ", " : "",
            h("button", { class: "row-link", text: item.label, onclick: () => openThing(item.id) }),
          ]).flat(), episode.more ? ` and ${episode.more} more` : "")
        : null,
      // What that batch of work was on.
      featureTag(episode.features, (f) => openThing(f.id))));
}

async function compareBody(params) {
  const to = params.to || "live";
  const data = await api(
    `/api/compare?from=${encodeURIComponent(params.from)}&to=${encodeURIComponent(to)}`);
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  const parts = [];
  if (data.banner) {
    parts.push(h("div", { class: "asof-banner" },
      h("p", { text: data.banner }),
      h("button", { class: "ghost", text: "Back to live", onclick: () => go("time") })));
  }
  parts.push(h("p", { class: "kicker" }, icon("history"), "Explore · Time"));
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
}

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

  // Around it: compact columns of the neighbourhood, every truncation
  // counted out loud.
  const nb = data.neighborhood;
  const aroundCol = (label, items) => {
    if (!items?.length) return null;
    const shownItems = items.slice(0, 8);
    return h("div", { class: "around-col" },
      h("p", { class: "around-head" }, `${label} · ${items.length}`),
      h("ul", {}, shownItems.map((ref) =>
        h("li", {}, h("button", { class: "linky", text: ref.label,
          onclick: () => openThing(ref.id) })))),
      items.length > shownItems.length
        ? h("p", { class: "around-note", text: `${items.length - shownItems.length} more are not listed` })
        : null);
  };
  const aroundCols = [
    aroundCol("It uses", nb.upstream),
    aroundCol("Depends on it", nb.downstream),
    aroundCol("Tests", nb.tests),
    aroundCol("Described in", [...(nb.docs ?? []), ...(nb.decisions ?? [])]),
  ].filter(Boolean);
  if (aroundCols.length) {
    parts.push(h("h2", { class: "section", text: "Around it" }));
    parts.push(h("div", { class: "around" }, ...aroundCols));
    parts.push(h("p", { class: "page-note", text: nb.sentence }));
  }

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

  /* The sticky sidebar: what a passer-by needs without scrolling — the
     vital signs, who this serves, the one command, and the machine's
     own words last. */
  const side = [];
  const glanceStats = [
    ["State", data.state ?? "—", data.tone],
    ["Changed", data.versions[0]?.when ?? data.history[0]?.when ?? "—", null],
    ["Links", String(data.relations.length), null],
    ["Versions", String(Math.max(data.versions.length, 1)), null],
  ];
  side.push(h("div", { class: "side-card glance" },
    h("h4", { text: "At a glance" }),
    h("div", { class: "glance-grid" }, glanceStats.map(([label, value, tone]) =>
      h("div", { class: "stat" },
        h("code", { class: `stat-value${tone ? ` ${tone}` : ""}`, text: value }),
        h("span", { class: "stat-label", text: label }))))));
  if (data.extras.serves.length) {
    side.push(h("div", { class: "side-card" },
      h("h4", { text: "What it serves" }),
      ...data.extras.serves.map((item) =>
        h("button", { class: "serves-link", onclick: () => openThing(item.target.id) },
          glyph("hexagon"),
          h("span", {}, h("strong", { text: item.target.label }), " — ", item.because)))));
  }
  const command = data.extras.briefing.find((item) => item.fix_command)?.fix_command;
  if (command) {
    side.push(h("div", { class: "side-card side-command" },
      h("h4", { text: "The command" }),
      commandLine(command)));
  }
  side.push(h("div", { class: "side-card machine" },
    h("h4", { text: "Machine detail" }),
    h("dl", {}, data.details.map(([label, value]) =>
      [h("dt", { text: label }), h("dd", { text: value })]).flat())));

  stage.replaceChildren(h("div", { class: "dossier" },
    h("div", { class: "dossier-main" }, ...parts),
    h("aside", { class: "dossier-side" }, ...side)));
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
  const freshnessNode = document.getElementById("freshness");
  freshnessNode.textContent = freshness;
  // The pill's dot takes the working tree's tone: amber when it has
  // moved past the graph, quiet when the two are in step.
  freshnessNode.dataset.tone =
    snapshot.working_tree && snapshot.working_tree.state !== "in_step" ? "signal" : "ok";
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
  return h("span", { class: "cmd-row" },
    h("code", {
      class: "command", text: command, title: "Click to copy",
      onclick: (event) => {
        event.stopPropagation();
        navigator.clipboard?.writeText(command);
        showToast("Copied — you run it; Eyes never writes.");
      },
    }),
    copyButton(command));
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
        cell: (row) => strip(row.strip, () => openThing(row.id)) },
      { key: "score", label: "Ready", width: "62px", class: "dim num",
        cell: (row) => `${row.met}/${row.total}` },
      { key: "verdict", label: "Verdict", width: "minmax(200px, 2fr)", class: "dim nowrap",
        cell: (row) => row.verdict },
      { key: "status", label: "Status", width: "90px", class: "dim",
        cell: (row) => chip(row.status, "quiet") },
      { key: "changed", label: "Changed", width: "88px", class: "dim num",
        cell: (row) => row.when },
    ],
    onPeek: (row) => openThing(row.id),
    onPush: (row) => openThing(row.id),
    empty: "No feature matches that.",
  });
  const _ = params;
};

/* =====================================================================
   Evidence — claim on the left, proof on the right, never the reverse.
   ===================================================================== */

async function evidencePanel(host, params) {
  const data = await api("/api/evidence");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);
  const only = params.category;
  const shown = only ? data.claims.filter((claim) => claim.category === only) : data.claims;

  const filters = h("div", { class: "chips" },
    h("button", { class: `chip${only ? "" : " on"}`, text: "everything",
      onclick: () => go("proof", { tab: "evidence" }) }),
    ...data.categories.map((category) =>
      h("button", {
        class: `chip${only === category.id ? " on" : ""}`,
        title: category.note,
        text: `${category.label} · ${category.unsupported ? `${category.unsupported} unproven` : "all proven"}`,
        onclick: () => go("proof", { tab: "evidence", category: category.id }),
      })));

  host.replaceChildren(
    h("h1", { class: "lede", text: data.headline }),
    h("p", { class: "sub", text: "A claim is never shown stronger than the proof behind it." }),
    filters,
    ...shown.map(claimRow));
}

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

async function testsPanel(host, params) {
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
  // The flag says `suite`, not `group`: every case already carries a
  // `group` naming its suite, and a truthy string there made each case
  // row take the suite branch and read a field it does not have.
  const rows = [...groups.entries()].map(([name, cases]) => ({
    suite: true,
    id: `group:${name}`,
    name,
    cases,
    failing: cases.filter((c) => c.result === "failing").length,
    skipped: cases.filter((c) => c.result === "skipped").length,
    // A case that has flipped verdict repeatedly is not quiet, even
    // while it is green: it is the one that will wake you.
    restless: cases.some((c) => c.flips >= 3),
    frameworks: [...new Set(cases.map((c) => c.framework).filter(Boolean))],
  }));

  const tableHost = h("div", {});
  const parts = [
    h("h1", { class: "lede", text: data.headline }),
    h("p", { class: "sub",
      text: `${data.declared} tests declared across ${data.files} files; ${data.cases.length} have a recorded result.` }),
    // What each framework brought, as records rather than a sentence.
    data.frameworks.length
      ? h("div", { class: "chips ledger" }, data.frameworks.map((f) =>
          h("span", { class: "chip quiet" }, f.label, h("em", { text: String(f.declared) }))))
      : null,
    tableHost,
  ];

  if (data.protocols.length) {
    parts.push(h("h2", { class: "section", text: "Runs" }));
    // Every run carries the same sentence about being run and observed;
    // said once under the section it is a promise, said eight times it
    // is wallpaper.
    const runs = data.protocols.slice(0, 8);
    const shared = runs.every((run) => run.evidence === runs[0].evidence) ? runs[0].evidence : null;
    if (shared) parts.push(h("p", { class: "sub", text: shared }));
    parts.push(h("div", { class: "panel runs" },
      runs.map((run) => protocolRow(run, !shared))));
  }
  if (data.uncovered.length) {
    parts.push(h("h2", { class: "section", text: "Depended on, but no test touches them" }));
    parts.push(h("div", { class: "chips" }, data.uncovered.map((item) =>
      h("button", { class: "chip-ref", onclick: () => openThing(item.id) },
        glyph(item.glyph), h("span", { text: item.label }),
        h("span", { class: "chip-arrow", "aria-hidden": "true", text: "›" })))));
  }
  host.replaceChildren(...parts);

  const verdictOf = (row) => (row.suite
    ? (row.failing ? "failing" : "passing")
    : row.result);

  table(tableHost, {
    rows,
    noun: "suites",
    keyOf: (row) => row.id,
    clickExpands: true,
    childrenOf: (row) => (row.suite ? row.cases : null),
    expandedByDefault: rows.filter((r) => r.failing).map((r) => r.id),
    // When something is failing or restless, the quiet suites wait
    // behind one line so the trouble leads. When nothing is, there is
    // no "more interesting" to lead with, so the whole list stands.
    fold: rows.some((r) => r.failing || r.restless)
      ? {
          when: (row) => row.suite && !row.failing && !row.restless,
          label: (n) => `${n} suite${n === 1 ? "" : "s"} where everything passes — show them`,
          close: "fold the quiet suites away",
        }
      : null,
    // A case opens where it sits: its own detail, not a page you have
    // to come back from.
    detailOf: (row) => (row.suite ? null : caseDetail(row)),
    search: (row) => (row.suite ? row.name : `${row.name} ${row.error ?? ""}`),
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
      { id: "framework", get: (row) => (row.suite ? row.frameworks : row.framework) },
      { id: "feature", get: (row) => (row.suite ? [] : row.features.map((f) => f.label)) },
      {
        id: "history",
        get: (row) => (!row.suite && row.flips >= 3 ? "changed its mind" : null),
        options: [{ value: "changed its mind", label: "changed its mind" }],
      },
    ],
    sort: [
      { key: "name", label: "by name", by: (row) => row.name },
      { key: "result", label: "failing first", by: (row) => (verdictOf(row) === "failing" ? 0 : 1) },
      { key: "size", label: "by size", by: (row) => -(row.cases?.length ?? 0) },
    ],
    columns: [
      { key: "name", label: "Test", width: "minmax(360px, 5fr)",
        cell: (row) => row.suite
          ? h("span", { class: "pill-row" }, kindIcon("diamond"), h("span", { class: "record", text: row.name }))
          : h("span", { class: "case-name", text: row.name.split("::").pop() }) },
      { key: "result", label: "Result", width: "88px",
        cell: (row) => row.suite
          ? h("span", { class: "dim", text: `${row.cases.length} case${row.cases.length === 1 ? "" : "s"}` })
          : h("span", { class: `chip ${row.tone}`, text: row.result }) },
      // One column for what the row has to say, because four columns
      // that are empty on every line say only that the table is wrong.
      { key: "detail", label: "Detail", width: "minmax(200px, 3fr)", class: "dim",
        cell: (row) => {
          if (row.suite) {
            const said = [];
            if (row.failing) said.push(h("span", { class: "case-error", text: `${row.failing} failing` }));
            else said.push("all passing");
            if (row.skipped) said.push(h("span", { class: "case-note", text: `${row.skipped} skipped` }));
            if (row.restless) said.push(h("span", { class: "case-note", text: "one changed its mind" }));
            return h("span", { class: "detail-row" },
              said.flatMap((part, index) => (index ? [" · ", part] : [part])));
          }
          const said = [];
          if (row.error) said.push(h("span", { class: "case-error", text: row.error }));
          if (row.note) said.push(h("span", { class: "case-note", text: row.note }));
          if (row.flips >= 3) said.push(h("span", { class: "case-note", text: `changed its mind ×${row.flips}` }));
          if (row.attachments.length) {
            said.push(h("span", { class: "chips" }, row.attachments.map((a) =>
              h("button", { class: "chip", text: a.noun,
                onclick: (e) => { e.stopPropagation(); openThing(a.id); } }))));
          }
          const serves = featureTag(row.features, (f) => openThing(f.id), 2);
          if (serves) said.push(serves);
          return said.length ? h("span", { class: "detail-row" }, said) : "";
        } },
      { key: "duration", label: "Took", width: "76px", class: "dim num",
        cell: (row) => (row.suite ? "" : row.duration ?? "") },
    ],
    onPeek: (row) => !row.suite && openThing(row.id),
    onPush: (row) => !row.suite && openThing(row.id),
    empty: "No test matches that.",
  });
  const _ = params;
}

/* Everything one case has to show, opened where it sits: what it is
   called in full, how it went and for how long, what it left behind,
   and which claim it serves. */
function caseDetail(row) {
  const facts = [];
  if (row.duration) facts.push(h("span", { text: `took ${row.duration}` }));
  if (row.retries) facts.push(h("span", { text: `retried ${row.retries}×` }));
  if (row.flips >= 3) facts.push(h("span", { class: "case-note", text: `changed its mind ${row.flips}×` }));
  if (row.when) facts.push(h("span", { text: row.when }));

  return h("div", { class: "case-open" },
    h("div", { class: "case-open-head" },
      h("code", { class: "case-full", text: row.name }),
      h("span", { class: `chip ${row.tone}`, text: row.result })),
    row.error ? h("p", { class: "case-error", text: row.error }) : null,
    row.note ? h("p", { class: "case-note", text: row.note }) : null,
    facts.length ? h("p", { class: "pill-row" }, facts) : null,
    // What it left behind, shown rather than named.
    row.attachments.length
      ? h("div", { class: "case-shots" }, row.attachments.map((shot) =>
          h("button", { class: "chip-ref", onclick: () => openThing(shot.id) },
            glyph(shot.glyph ?? "frame"), h("span", { text: shot.noun }),
            h("span", { class: "chip-arrow", "aria-hidden": "true", text: "›" }))))
      : null,
    row.features.length
      ? h("p", { class: "pill-row" }, "serves ", featureTag(row.features, (f) => openThing(f.id)))
      : null,
    h("button", { class: "row-link", text: "open its page",
      onclick: (event) => { event.stopPropagation(); openThing(row.id); } }));
}

function protocolRow(run, sayEvidence = true) {
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
      run.evidence && sayEvidence ? h("p", { class: "session-meta", text: run.evidence }) : null),
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
  const parts = [
    h("p", { class: "kicker" }, icon("frame"),
      h("button", { class: "row-link", text: "Proof · Artifacts",
        onclick: () => go("proof", { tab: "artifacts" }) }),
      " · Tour"),
    h("h1", { class: "lede", text: data.headline }),
  ];

  if (data.tour) parts.push(tourPanel(data.tour));

  if (data.items.length) {
    parts.push(h("h2", { class: "section", text: "Everything captured" }));
    parts.push(h("div", { class: "media-grid" }, data.items.map(mediaCard)));
  }
  stage.replaceChildren(h("div", { class: "page" }, ...parts));
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

async function mriBody(params) {
  const data = await api("/api/mri");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  const host = h("div", { class: "mri-stage" });
  const readout = h("div", { class: "mri-panel mri-readout" });
  const lensPanel = h("div", { class: "mri-panel mri-lenses" },
    h("h3", { text: "Lens" }),
    ...MRI_LENSES.map(([id, label, note]) =>
      h("button", {
        class: id === "anatomy" ? "on" : "",
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
  // The route lens is "mri"; the panel manages the MRI's own lenses.
  const _ = params;
  mriHandle = mount(host, data, {
    lens: "anatomy",
    onPick: (node) => openThing(node.id),
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
function isDark() {
  return document.documentElement.dataset.theme === "dark"
    || (!document.documentElement.dataset.theme
        && matchMedia("(prefers-color-scheme: dark)").matches);
}
/* The button offers the other theme: a moon in the light, a sun in the dark. */
function paintThemeButton() {
  themeButton.replaceChildren(icon(isDark() ? "sun" : "moon", "sm"), isDark() ? "Light" : "Dark");
}
paintThemeButton();
themeButton.addEventListener("click", () => {
  const next = isDark() ? "light" : "dark";
  document.documentElement.dataset.theme = next;
  localStorage.setItem("eyes-theme", next);
  paintThemeButton();
});

/* The register: the same facts in two tellings. Plain view leads with
   the features, the roadmap, and the tour; the operator chrome —
   commands, badges, dense surfaces — recedes. Per-viewer, in this
   browser only, like the theme. */
const registerButton = document.getElementById("register");
const PLAIN_VIEWS = ["roadmap", "features", "media", "thing"];
function applyRegister(plain) {
  document.body.classList.toggle("plain", plain);
  registerButton.textContent = plain ? "Full view" : "Plain view";
}
applyRegister(localStorage.getItem("eyes-register") === "plain");
if (document.body.classList.contains("plain")
    && !PLAIN_VIEWS.includes(readRoute().view)) {
  go("roadmap");
}
registerButton.addEventListener("click", () => {
  const plain = !document.body.classList.contains("plain");
  localStorage.setItem("eyes-register", plain ? "plain" : "full");
  applyRegister(plain);
  if (plain && !PLAIN_VIEWS.includes(state.view)) go("roadmap");
  else if (!plain) render();
  else render();
});

/* Counts on the rail, so the nav says where the trouble is. */
async function paintRail() {
  try {
    const [next, work, tests, evidence, roadmap] = await Promise.all([
      api("/api/next"), api("/api/work"), api("/api/tests"),
      api("/api/evidence"), api("/api/roadmap"),
    ]);
    const set = (key, value, tone) => {
      const node = document.querySelector(`[data-count="${key}"]`);
      if (!node) return;
      node.hidden = !value;
      node.textContent = value;
      if (tone) node.dataset.tone = tone; else delete node.dataset.tone;
    };
    // One rule on every rail item: the count is things needing a
    // decision, tinted by the worst severity among them.
    const acts = next.queue.filter((item) => item.severity === "act").length;
    const watches = next.queue.filter((item) => item.severity === "watch").length;
    set("now", acts + watches, acts ? "bad" : watches ? "watch" : null);
    const workCount = work.signals.length + work.approvals.length;
    set("work", workCount, work.signals.some((s) => s.severity === "act") ? "bad"
      : workCount ? "watch" : null);
    const proofCount = tests.failing.length
      + evidence.claims.filter((c) => !c.supported).length;
    set("proof", proofCount, tests.failing.length ? "bad" : proofCount ? "watch" : null);
    const planned = roadmap.stages.reduce((n, s) => n + s.total - s.ready, 0)
      + roadmap.unplanned.length;
    set("roadmap", planned, planned ? "watch" : null);
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
