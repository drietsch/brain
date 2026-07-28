/* Eyes — the visual layer over the brain.
 *
 * The rule this file lives by: it renders sentences the server wrote and
 * never composes a judgment of its own. If a phrase is missing, the fix
 * belongs in say.rs, not here.
 */
"use strict";

const stage = document.getElementById("stage");
const state = { view: "now", params: {}, snapshot: null, findRows: [], findIndex: 0 };

/* ------------------------------------------------------------------ utils */

function h(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === null || value === undefined || value === false) continue;
    if (key === "class") node.className = value;
    else if (key === "text") node.textContent = value;
    else if (key === "html") node.innerHTML = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else node.setAttribute(key, value);
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

function glyph(shape) {
  return h("i", { class: `glyph ${shape || "block"}`, "aria-hidden": "true" });
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

async function render() {
  const route = readRoute();
  state.view = route.view;
  state.params = route.params;
  for (const button of document.querySelectorAll(".rail button")) {
    const target = button.dataset.go;
    button.classList.toggle("on", target === route.view ||
      (target === "library" && ["concepts", "tests", "thing"].includes(route.view)));
  }
  stage.classList.toggle("dark", route.view === "map");
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
  const data = await api("/api/now");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  const parts = [
    h("h1", { class: "headline", text: data.headline }),
    h("p", { class: "subhead", text: data.subhead }),
  ];

  if (data.needs_you.length) {
    parts.push(h("h2", { class: "section", text: "Needs you" }));
    parts.push(h("div", { class: "concerns" }, data.needs_you.map((concern) => {
      const node = h("div", { class: `concern ${concern.severity}` },
        h("i", { class: "dot" }),
        h("div", {},
          h("h3", { text: concern.title }),
          h("p", { text: concern.reason }),
          fixLine(concern.fix_command)));
      if (concern.target) {
        node.style.cursor = "pointer";
        node.addEventListener("click", () => openThing(concern.target.id));
      }
      return node;
    })));
  }

  parts.push(h("h2", { class: "section", text: data.since.known ? `Since your last session, ${data.since.when}` : "Recently" }));
  parts.push(h("div", { class: "since" },
    h("div", {},
      h("p", { class: "subhead", style: "margin-bottom:14px", text: data.since.summary }),
      data.since.episodes.length
        ? h("div", { class: "episodes" }, data.since.episodes.map(episodeRow))
        : h("p", { class: "empty", text: "No activity recorded since then." })),
    h("dl", { class: "stats" }, data.stats.map((stat) =>
      h("div", { class: "stat" },
        h("dt", { text: stat.label }),
        h("dd", { class: stat.tone },
          stat.value,
          stat.note ? h("small", { text: stat.note }) : null))))));

  if (data.attention.length) {
    parts.push(h("h2", { class: "section", text: "Where the pressure is" }));
    parts.push(h("div", { class: "attention" }, data.attention.map((card) =>
      h("button", { onclick: () => card.id && openThing(card.id) },
        h("div", { class: "who" }, glyph(card.glyph), card.label),
        h("ul", {}, card.reasons.map((reason) => h("li", { text: reason })))))));
  }

  stage.replaceChildren(...parts);
};

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
  const query = params.q || "";
  const data = await api(`/api/library?shelf=${encodeURIComponent(shelf)}&q=${encodeURIComponent(query)}`);
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);

  const search = h("input", {
    class: "shelf-search", type: "search", value: query,
    placeholder: `Search ${data.label.toLowerCase()}…`,
  });
  let timer = null;
  search.addEventListener("input", () => {
    clearTimeout(timer);
    timer = setTimeout(() => go("library", { shelf: data.shelf, q: search.value }), 220);
  });

  const body = data.items.length
    ? h("div", { class: "items" }, data.items.map(shelfItem))
    : h("p", { class: "empty", text: query ? "Nothing here matches that." : "This shelf is empty." });

  stage.replaceChildren(
    h("div", { class: "page-head" }, h("h1", { text: data.label })),
    h("p", { class: "page-note", text: data.note }),
    h("div", { class: "library" }, shelfRail(data.shelves, data.shelf), h("div", {}, search, body)));
};

function shelfRail(shelves, current) {
  const all = [
    ...shelves.map((shelf) => ({ ...shelf, view: "library" })),
    { id: "tests", label: "Tests", count: null, view: "tests" },
    { id: "concepts", label: "Concepts", count: null, view: "concepts" },
  ];
  return h("nav", { class: "shelves" }, all.map((shelf) =>
    h("button", {
      class: shelf.id === current && shelf.view === "library" ? "on" : "",
      onclick: () => (shelf.view === "library" ? go("library", { shelf: shelf.id }) : go(shelf.view)),
    }, h("span", { text: shelf.label }), shelf.count !== null ? h("em", { text: shelf.count }) : null)));
}

function shelfItem(item) {
  return h("button", { class: "item", onclick: () => openThing(item.id) },
    h("div", { class: "item-head" },
      glyph(item.glyph),
      h("h3", { text: item.title }),
      item.coverage ? coverageStrip(item.coverage) : null,
      chip(item.state, item.tone)),
    h("p", { class: "item-sub" },
      [item.noun, item.when, item.state_note].filter(Boolean).join(" · ")),
    item.excerpt ? h("p", { class: "item-excerpt", text: item.excerpt }) : null,
    item.facts.length ? h("ul", { class: "item-facts" }, item.facts.map((fact) => h("li", { text: fact }))) : null);
}

function coverageStrip(cells) {
  return h("span", { class: "coverage", title: cells.map((cell) => `${cell.label}: ${cell.detail}`).join("\n") },
    cells.map((cell) => h("i", { class: cell.met ? "met" : "" })));
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

views.tests = async () => {
  const data = await api("/api/tests");
  state.snapshot = data.snapshot;
  paintChrome(data.snapshot);
  const sections = [
    h("h1", { class: "headline", text: data.headline }),
    h("p", { class: "subhead",
      text: `${data.declared} tests declared across ${data.files} test files.` }),
  ];
  if (data.failing.length) {
    sections.push(h("h2", { class: "section", text: "Failing now" }));
    sections.push(h("div", { class: "items" }, data.failing.map((item) =>
      h("button", { class: "item", onclick: () => openThing(item.id) },
        h("div", { class: "item-head" }, glyph("diamond"), h("h3", { text: item.name }), chip("failing", "bad")),
        h("p", { class: "item-sub", text: item.note })))));
  }
  if (data.flaky.length) {
    sections.push(h("h2", { class: "section", text: "Changed their mind more than once" }));
    sections.push(h("div", { class: "items" }, data.flaky.map((item) =>
      h("button", { class: "item", onclick: () => openThing(item.id) },
        h("div", { class: "item-head" }, glyph("diamond"), h("h3", { text: item.name }),
          chip(item.result, item.result === "pass" ? "good" : "bad")),
        h("p", { class: "item-sub", text: item.note })))));
  }
  if (data.uncovered.length) {
    sections.push(h("h2", { class: "section", text: "Depended on, but no test touches them" }));
    sections.push(h("div", { class: "tests-grid" }, data.uncovered.map((item) =>
      h("button", { class: "item", onclick: () => openThing(item.id) },
        h("div", { class: "who" }, glyph(item.glyph), item.label)))));
  }
  if (data.runs.length) {
    sections.push(h("h2", { class: "section", text: "Recent runs" }));
    sections.push(h("div", { class: "rows" }, data.runs.map((run) =>
      h("div", { class: "row" },
        h("span", { class: "when", text: run.when }),
        h("span", { text: run.failed === 0
          ? `all ${run.total} passed`
          : `${run.failed} of ${run.total} failed` })))));
  }
  stage.replaceChildren(
    h("div", { class: "library" }, shelfRail(await shelvesFor(), "tests"), h("div", {}, ...sections)));
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
                : null))))
      : h("p", { class: "empty", text: "Nothing recorded yet." }));
};

/* ------------------------------------------------------------------ thing */

views.thing = async (params) => {
  if (!params.id) return go("now");
  const data = await api(`/api/thing?id=${encodeURIComponent(params.id)}`);
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

  if (data.extras.coverage.length) {
    parts.push(h("h2", { class: "section", text: "What backs this claim" }));
    parts.push(h("div", { class: "facts" }, data.extras.coverage.map((cell) =>
      h("div", { class: `fact ${cell.met ? "good" : "watch"}` },
        h("i", { class: "mark" }),
        h("span", {}, h("strong", { text: cell.label }), " — ", cell.detail)))));
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
        h("span", { class: "why", text: hit.state || hit.because }))));
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
  document.getElementById("project").textContent = snapshot.prefix;
  const seconds = Math.max(0, Math.round((snapshot.generated_at_ms - snapshot.changed_at_ms) / 1000));
  const freshness = seconds < 90 ? "updated just now"
    : seconds < 5400 ? `updated ${Math.round(seconds / 60)} minutes ago`
    : seconds < 172800 ? `updated ${Math.round(seconds / 3600)} hours ago`
    : `updated ${Math.round(seconds / 86400)} days ago`;
  document.getElementById("freshness").textContent = freshness;
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

render();
