/* The list engine: one dense, filterable, drillable table for every surface.
 *
 * Three ways down, deliberately distinct:
 *
 *   peek    click a row  → the inspector opens beside it. Answers "why?"
 *   push    click the name, or Enter → the full page. Answers "everything".
 *   expand  the chevron  → children appear in the same grid, indented.
 *
 * Filtering is client-side because every list in this product is bounded —
 * the largest is 150 rows — so a keystroke is instant and no request is
 * made. Facet counts are computed against the *other* filters, so a count
 * never promises rows that a second filter would remove.
 */

const CELL = { ready: "ready", stale: "stale", failing: "failing", absent: "absent", unproven: "unproven" };

export function h(tag, props = {}, ...children) {
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

/** A kind's shape, from the sprite. Shape carries kind so colour need not. */
export function icon(name, extra = "") {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", `icon ${extra}`.trim());
  svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#i-${name}`);
  svg.append(use);
  return svg;
}

const GLYPH_ICON = {
  hexagon: "hexagon", diamond: "diamond", kite: "kite", page: "page",
  square: "frame", chevron: "chevron", shield: "shield", orbit: "orbit",
  circle: "circle", block: "block",
};
export function kindIcon(glyph, extra = "") {
  return icon(GLYPH_ICON[glyph] || "block", extra);
}

/** The dimension strip, seven pixels a cell. */
export function strip(cells, onPick) {
  if (!cells || !cells.length) return null;
  const title = cells.map((c) => c.detail).join(" · ");
  const node = h(onPick ? "button" : "span", {
    class: "strip", title,
    ...(onPick ? { onclick: (e) => { e.stopPropagation(); onPick(); } } : {}),
  }, cells.map((cell) => h("i", { "data-cell": CELL[cell.state] || "absent" })));
  return node;
}

/**
 * The features a row serves, as openable tags.
 *
 * Empty renders as nothing rather than as "unclaimed": most of a graph
 * belongs to no feature, and saying so on every row would be noise. The
 * click is stopped from bubbling so it does not also peek the row.
 */
export function featureTag(features, onPick, max = 3) {
  if (!features || !features.length) return null;
  return h("span", { class: "feature-tags" }, features.slice(0, max).map((feature) =>
    h("button", {
      class: "feature-tag", title: `serves ${feature.label}`,
      onclick: (event) => { event.stopPropagation(); onPick?.(feature); },
    }, kindIcon("hexagon"), h("span", { text: feature.label }))),
    features.length > max
      ? h("span", { class: "feature-more", text: `+${features.length - max}` })
      : null);
}

/** The same strip, labelled — for a dossier header. */
export function stripWide(cells, onPick) {
  if (!cells || !cells.length) return null;
  return h("div", { class: "strip-wide" }, cells.map((cell) =>
    h("button", {
      "data-cell": CELL[cell.state] || "absent",
      title: cell.detail,
      onclick: () => onPick?.(cell),
    },
      h("span", { class: "bar" }),
      h("span", { class: "cap", text: cell.label }),
      h("span", { class: "tally", text: cell.state }))));
}

/**
 * Render a filterable table into `host`.
 *
 * Returns a handle with `refresh()` so a caller can re-render after the
 * data changes without rebuilding the whole surface.
 */
export function table(host, spec) {
  const state = {
    text: "",
    facets: {},                 // id -> Set of selected values
    sort: spec.sort?.[0]?.key ?? null,
    dir: 1,
    expanded: new Set(spec.expandedByDefault ?? []),
    active: -1,
  };
  for (const facet of spec.facets ?? []) state.facets[facet.id] = new Set();

  const grid = spec.columns.map((c) => c.width ?? "1fr").join(" ");

  /* ---- filtering ---------------------------------------------------- */
  const matchesText = (row) => {
    if (!state.text) return true;
    const needle = state.text.toLowerCase();
    return (spec.search?.(row) ?? "").toLowerCase().includes(needle);
  };
  const matchesFacets = (row, skip) =>
    (spec.facets ?? []).every((facet) => {
      if (facet.id === skip) return true;
      const chosen = state.facets[facet.id];
      if (!chosen.size) return true;
      const value = facet.get(row);
      return Array.isArray(value) ? value.some((v) => chosen.has(v)) : chosen.has(value);
    });

  // A row survives if it matches, or if any descendant does — otherwise
  // filtering would hide a parent whose child you were looking for.
  const survives = (row) => {
    if (matchesText(row) && matchesFacets(row)) return true;
    return (spec.childrenOf?.(row) ?? []).some(survives);
  };

  const visibleRoots = () => {
    let rows = spec.rows.filter(survives);
    const sort = spec.sort?.find((s) => s.key === state.sort);
    if (sort) {
      rows = [...rows].sort((a, b) => {
        const x = sort.by(a);
        const y = sort.by(b);
        const cmp = typeof x === "string" ? x.localeCompare(y) : (x ?? 0) - (y ?? 0);
        return cmp * state.dir;
      });
    }
    return rows;
  };

  /* ---- flatten, honouring expansion --------------------------------- */
  const flatten = (rows, depth, out, trail) => {
    rows.forEach((row, index) => {
      const children = (spec.childrenOf?.(row) ?? []).filter(survives);
      const key = spec.keyOf(row);
      const last = index === rows.length - 1;
      out.push({ row, depth, children, key, last, trail });
      if (children.length && state.expanded.has(key)) {
        flatten(children, depth + 1, out, [...trail, !last]);
      }
    });
    return out;
  };

  /* ---- rendering ----------------------------------------------------- */
  function draw() {
    const roots = visibleRoots();
    const flat = flatten(roots, 0, [], []);
    const total = spec.rows.length;

    const parts = [];

    // Facet bar: search, chips with live counts, sort, and an honest tally.
    const controls = [];
    if (spec.search) {
      controls.push(h("input", {
        type: "search", placeholder: spec.placeholder ?? "Filter…",
        value: state.text, "aria-label": "Filter this list",
        oninput: (e) => { state.text = e.target.value; draw(); },
      }));
    }
    for (const facet of spec.facets ?? []) {
      if (controls.length) controls.push(h("span", { class: "facet-gap" }));
      const chosen = state.facets[facet.id];
      const counts = new Map();
      for (const row of spec.rows.filter((r) => matchesText(r) && matchesFacets(r, facet.id))) {
        const value = facet.get(row);
        for (const v of Array.isArray(value) ? value : [value]) {
          if (v !== undefined && v !== null && v !== "") counts.set(v, (counts.get(v) ?? 0) + 1);
        }
      }
      for (const option of facet.options ?? [...counts.keys()].sort()) {
        const value = option.value ?? option;
        const label = option.label ?? option;
        const count = counts.get(value) ?? 0;
        if (!count && !chosen.has(value)) continue;
        controls.push(h("button", {
          class: `chip${chosen.has(value) ? " on" : ""}`,
          "aria-pressed": chosen.has(value) ? "true" : "false",
          onclick: () => {
            chosen.has(value) ? chosen.delete(value) : chosen.add(value);
            draw();
          },
        }, label, h("em", { text: String(count) })));
      }
    }
    if (spec.sort?.length > 1) {
      controls.push(h("select", {
        "aria-label": "Sort",
        onchange: (e) => { state.sort = e.target.value; draw(); },
      }, spec.sort.map((s) =>
        h("option", { value: s.key, selected: s.key === state.sort || null, text: s.label }))));
    }
    const shown = flat.length;
    controls.push(h("span", {
      class: "tally-bar", "data-empty": shown === 0 ? "true" : "false",
      // "1 features" is the kind of sloppiness this product notices.
      text: shown === total
        ? `${total} ${total === 1 ? (spec.one ?? (spec.noun ?? "rows").replace(/s$/, "")) : (spec.noun ?? "rows")}`
        : `${shown} of ${total}`,
    }));
    if (controls.length) parts.push(h("div", { class: "filters" }, controls));

    // Header. Sortable columns are buttons; the rest are plain labels.
    const head = h("div", { class: "thead", style: `grid-template-columns:${grid}` },
      spec.columns.map((column) => {
        const sort = spec.sort?.find((s) => s.key === column.key);
        if (!sort) return h("span", { text: column.label });
        return h("button", {
          onclick: () => {
            if (state.sort === column.key) state.dir *= -1;
            else { state.sort = column.key; state.dir = 1; }
            draw();
          },
        }, column.label,
          state.sort === column.key
            ? h("span", { class: "dir", text: state.dir > 0 ? "▲" : "▼" })
            : null);
      }));

    const body = flat.map((entry, position) => row(entry, position));
    parts.push(h("div", { class: "panel" }, head,
      shown
        ? body
        : h("p", { class: "empty", text: spec.empty ?? "Nothing matches that." })));

    host.replaceChildren(...parts);
  }

  function row(entry, position) {
    const { row: data, depth, children, key, last, trail } = entry;
    const expanded = state.expanded.has(key);

    const node = h("div", {
      class: `trow${depth ? " child" : ""}`,
      style: `grid-template-columns:${grid}`,
      role: "row", tabindex: "-1",
      "aria-selected": position === state.active ? "true" : "false",
      "aria-expanded": children.length ? String(expanded) : null,
      onclick: () => { state.active = position; spec.onPeek?.(data); draw(); },
      ondblclick: () => spec.onPush?.(data),
    });

    spec.columns.forEach((column, index) => {
      const content = column.cell(data, { depth, expanded });
      if (index === 0) {
        // The first column carries the tree affordances.
        const lead = [];
        if (depth) {
          lead.push(h("span", {
            class: "branch",
            text: `${trail.map((more) => (more ? "│ " : "  ")).join("")}${last ? "└ " : "├ "}`,
          }));
        }
        lead.push(children.length
          ? h("button", {
              class: "chev", "aria-label": expanded ? "Collapse" : "Expand",
              text: expanded ? "▾" : "▸",
              onclick: (e) => {
                e.stopPropagation();
                expanded ? state.expanded.delete(key) : state.expanded.add(key);
                draw();
              },
            })
          : h("span", { class: "spacer" }));
        // The label rides in its own span: a bare text node in a flex
        // cell can wrap under its icon but can never grow an ellipsis.
        node.append(h("span", { class: "name" }, ...lead,
          h("span", { class: "name-label" }, content)));
      } else {
        node.append(h("span", { class: column.class ?? "dim nowrap" }, content));
      }
    });
    return node;
  }

  /* ---- keyboard ------------------------------------------------------ */
  host.tabIndex = 0;
  host.addEventListener("keydown", (event) => {
    const rows = host.querySelectorAll(".trow");
    if (!rows.length) return;
    const move = (delta) => {
      state.active = Math.max(0, Math.min(rows.length - 1, state.active + delta));
      draw();
      host.querySelectorAll(".trow")[state.active]?.scrollIntoView({ block: "nearest" });
    };
    const current = flatten(visibleRoots(), 0, [], [])[state.active];
    switch (event.key) {
      case "ArrowDown": event.preventDefault(); move(1); break;
      case "ArrowUp": event.preventDefault(); move(-1); break;
      case "ArrowRight":
        if (current?.children.length) { event.preventDefault(); state.expanded.add(current.key); draw(); }
        break;
      case "ArrowLeft":
        if (current) { event.preventDefault(); state.expanded.delete(current.key); draw(); }
        break;
      case "Enter": if (current) { event.preventDefault(); spec.onPush?.(current.row); } break;
      case " ": if (current) { event.preventDefault(); spec.onPeek?.(current.row); } break;
      case "/": {
        const field = host.querySelector('input[type="search"]');
        if (field) { event.preventDefault(); field.focus(); }
        break;
      }
      default: break;
    }
  });

  draw();
  return { refresh: draw, state };
}
