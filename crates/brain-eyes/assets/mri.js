/* The MRI: the living graph, drawn.
 *
 * WebGL2 written directly — no library is fetched, because Eyes serves
 * only itself. Two draw calls carry the whole scene: one instanced quad
 * per node against a glyph atlas painted at runtime, and one line buffer
 * for every edge.
 *
 * The layout arrives finished. The server placed every node once for this
 * graph version, so this file never simulates forces and the anatomy never
 * rearranges under the cursor. That is what makes motion meaningful: if
 * something is moving, it is because something happened.
 *
 * Level of detail is not truncation. Every node is in the buffer; the
 * camera decides which levels are drawn, and the readout always says how
 * many are on screen out of how many exist.
 */

const SHAPES = {
  hexagon: (c, s) => polygon(c, s, 6, -Math.PI / 2),
  diamond: (c, s) => polygon(c, s, 4, 0),
  kite: (c, s) => {
    c.beginPath();
    c.moveTo(s / 2, s * 0.08); c.lineTo(s * 0.86, s * 0.46);
    c.lineTo(s / 2, s * 0.94); c.lineTo(s * 0.14, s * 0.46);
    c.closePath();
  },
  page: (c, s) => {
    c.beginPath();
    c.moveTo(s * 0.2, s * 0.1); c.lineTo(s * 0.66, s * 0.1);
    c.lineTo(s * 0.82, s * 0.3); c.lineTo(s * 0.82, s * 0.9);
    c.lineTo(s * 0.2, s * 0.9); c.closePath();
  },
  square: (c, s) => { c.beginPath(); c.rect(s * 0.18, s * 0.18, s * 0.64, s * 0.64); },
  chevron: (c, s) => {
    c.beginPath();
    c.moveTo(s * 0.2, s * 0.14); c.lineTo(s * 0.8, s * 0.5);
    c.lineTo(s * 0.2, s * 0.86); c.closePath();
  },
  shield: (c, s) => {
    c.beginPath();
    c.moveTo(s / 2, s * 0.1); c.lineTo(s * 0.84, s * 0.28);
    c.lineTo(s * 0.72, s * 0.9); c.lineTo(s / 2, s * 0.94);
    c.lineTo(s * 0.28, s * 0.9); c.lineTo(s * 0.16, s * 0.28);
    c.closePath();
  },
  orbit: (c, s) => {
    c.beginPath(); c.ellipse(s / 2, s / 2, s * 0.4, s * 0.2, -0.5, 0, Math.PI * 2);
  },
  circle: (c, s) => { c.beginPath(); c.arc(s / 2, s / 2, s * 0.34, 0, Math.PI * 2); },
  block: (c, s) => { c.beginPath(); c.arc(s / 2, s / 2, s * 0.3, 0, Math.PI * 2); },
};
const SHAPE_ORDER = Object.keys(SHAPES);
const CELL = 64;

function polygon(context, size, sides, offset) {
  context.beginPath();
  for (let i = 0; i < sides; i += 1) {
    const angle = offset + (i / sides) * Math.PI * 2;
    const x = size / 2 + Math.cos(angle) * size * 0.38;
    const y = size / 2 + Math.sin(angle) * size * 0.38;
    if (i === 0) context.moveTo(x, y); else context.lineTo(x, y);
  }
  context.closePath();
}

/* The glyph atlas: shape carries kind, so colour never has to carry it
   alone. Painted once into a canvas and uploaded as a texture. */
function atlas() {
  const canvas = document.createElement("canvas");
  canvas.width = CELL * SHAPE_ORDER.length;
  canvas.height = CELL;
  const c = canvas.getContext("2d");
  SHAPE_ORDER.forEach((name, index) => {
    c.save();
    c.translate(index * CELL, 0);
    SHAPES[name](c, CELL);
    c.fillStyle = "rgba(255,255,255,0.92)";
    c.fill();
    c.lineWidth = 3;
    c.strokeStyle = "rgba(255,255,255,1)";
    c.stroke();
    c.restore();
  });
  return canvas;
}

const TONE = {
  quiet: [0.62, 0.72, 0.82],
  good: [0.44, 0.78, 0.62],
  watch: [0.88, 0.68, 0.36],
  bad: [0.91, 0.46, 0.44],
};
const PULSE = { changed: 1, failing: 2, working: 3, unfinished: 4 };
const CLUSTER_TINT = {
  implementation: [0.55, 0.68, 0.84],
  features: [0.55, 0.80, 0.68],
  tests: [0.72, 0.66, 0.88],
  decisions: [0.88, 0.76, 0.52],
  documentation: [0.62, 0.76, 0.80],
  artifacts: [0.80, 0.66, 0.72],
  work: [0.86, 0.72, 0.55],
  outside: [0.48, 0.54, 0.62],
};

const VERTEX = `#version 300 es
precision highp float;
layout(location=0) in vec2 corner;
layout(location=1) in vec3 centre;
layout(location=2) in vec3 tint;
layout(location=3) in float size;
layout(location=4) in float shape;
layout(location=5) in float pulse;
layout(location=6) in float level;
uniform mat4 camera;
uniform vec3 eye;
uniform vec3 right;
uniform vec3 up;
uniform float time;
uniform float maxLevel;
uniform float focus;
out vec2 uv;
out vec3 shade;
out float fade;
void main() {
  // Detail finer than the current zoom is drawn smaller and dimmer, never
  // removed. Culling it is what made the first attempt at this view an
  // empty screen that claimed to hold 1,241 things.
  float beyond = step(maxLevel + 0.01, level);
  float beat = pulse > 0.5
    ? 1.0 + 0.18 * sin(time * (pulse == 2.0 ? 6.0 : 2.2) + centre.x)
    : 1.0;
  float scale = size * beat * mix(1.0, 0.62, beyond);
  vec3 world = centre + right * corner.x * scale + up * corner.y * scale;
  gl_Position = camera * vec4(world, 1.0);
  uv = vec2((corner.x + 0.5 + shape) / ${SHAPE_ORDER.length}.0, corner.y + 0.5);
  shade = tint * (pulse > 0.5 ? 1.4 : 1.0);
  float distance = length(centre - eye);
  float depth = clamp(1.0 - (distance - 400.0) / 3200.0, 0.38, 1.0);
  fade = depth * mix(1.0, 0.58, beyond);
}`;

const FRAGMENT = `#version 300 es
precision highp float;
in vec2 uv;
in vec3 shade;
in float fade;
uniform sampler2D glyphs;
out vec4 colour;
void main() {
  float mask = texture(glyphs, uv).a;
  if (mask < 0.04) discard;
  colour = vec4(shade, mask * fade);
}`;

const EDGE_VERTEX = `#version 300 es
precision highp float;
layout(location=0) in vec3 point;
layout(location=1) in float level;
uniform mat4 camera;
uniform float maxLevel;
out float fade;
void main() {
  gl_Position = camera * vec4(point, 1.0);
  fade = step(level, maxLevel + 0.01) * 0.22;
}`;

const EDGE_FRAGMENT = `#version 300 es
precision highp float;
in float fade;
out vec4 colour;
void main() {
  if (fade < 0.01) discard;
  colour = vec4(0.42, 0.56, 0.70, fade);
}`;

function compile(gl, vertexSource, fragmentSource) {
  const make = (type, source) => {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      throw new Error(gl.getShaderInfoLog(shader) || "shader failed to compile");
    }
    return shader;
  };
  const program = gl.createProgram();
  gl.attachShader(program, make(gl.VERTEX_SHADER, vertexSource));
  gl.attachShader(program, make(gl.FRAGMENT_SHADER, fragmentSource));
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new Error(gl.getProgramInfoLog(program) || "program failed to link");
  }
  return program;
}

/* ---- small matrix helpers (column-major, as WebGL wants) ---- */
function multiply(a, b) {
  const out = new Float32Array(16);
  for (let i = 0; i < 4; i += 1) {
    for (let j = 0; j < 4; j += 1) {
      out[i * 4 + j] =
        a[j] * b[i * 4] + a[4 + j] * b[i * 4 + 1] +
        a[8 + j] * b[i * 4 + 2] + a[12 + j] * b[i * 4 + 3];
    }
  }
  return out;
}
function perspective(fov, aspect, near, far) {
  const f = 1 / Math.tan(fov / 2);
  return new Float32Array([
    f / aspect, 0, 0, 0,
    0, f, 0, 0,
    0, 0, (far + near) / (near - far), -1,
    0, 0, (2 * far * near) / (near - far), 0,
  ]);
}
function lookAt(eye, centre, upHint) {
  const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
  const norm = (v) => { const l = Math.hypot(...v) || 1; return [v[0] / l, v[1] / l, v[2] / l]; };
  const cross = (a, b) => [
    a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  const z = norm(sub(eye, centre));
  const x = norm(cross(upHint, z));
  const y = cross(z, x);
  return {
    matrix: new Float32Array([
      x[0], y[0], z[0], 0,
      x[1], y[1], z[1], 0,
      x[2], y[2], z[2], 0,
      -dot(x, eye), -dot(y, eye), -dot(z, eye), 1,
    ]),
    right: x,
    up: y,
  };
}

/* The three distances at which the picture means different things. */
const ZOOM_STEPS = [
  { fraction: 1.00, level: 0, name: "the whole system" },
  { fraction: 0.55, level: 1, name: "modules and neighbourhoods" },
  { fraction: 0.24, level: 2, name: "individual functions and types" },
];

export function mount(host, data, options = {}) {
  const canvas = document.createElement("canvas");
  host.append(canvas);
  const gl = canvas.getContext("webgl2", { antialias: true, alpha: false });
  if (!gl) {
    host.append(Object.assign(document.createElement("p"), {
      className: "mri-panel",
      textContent: "This browser has no WebGL2, so the anatomy cannot be drawn. Every fact in it is also on the Map, the Library and Now.",
    }));
    return { destroy() {} };
  }

  const nodes = data.nodes;
  const count = nodes.length;
  const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

  /* ---- buffers ---- */
  const centre = new Float32Array(count * 3);
  const tint = new Float32Array(count * 3);
  const size = new Float32Array(count);
  const shape = new Float32Array(count);
  const pulse = new Float32Array(count);
  const level = new Float32Array(count);
  let lens = options.lens || "anatomy";

  function paint() {
    nodes.forEach((node, i) => {
      centre[i * 3] = node.x; centre[i * 3 + 1] = node.y; centre[i * 3 + 2] = node.z;
      let colour = CLUSTER_TINT[node.cluster] || TONE.quiet;
      if (lens === "activity") {
        colour = node.pulse ? (node.pulse === "failing" ? TONE.bad : TONE.watch) : [0.32, 0.38, 0.46];
      } else if (lens === "depth") {
        const t = Math.min(node.y / 240, 1);
        colour = [0.35 + t * 0.5, 0.55, 0.85 - t * 0.35];
      }
      tint[i * 3] = colour[0]; tint[i * 3 + 1] = colour[1]; tint[i * 3 + 2] = colour[2];
      size[i] = node.size * 6.0;
      shape[i] = Math.max(0, SHAPE_ORDER.indexOf(node.glyph));
      pulse[i] = reduceMotion ? 0 : (PULSE[node.pulse] || 0);
      level[i] = node.level;
    });
  }
  paint();

  const edgePoints = new Float32Array(data.edges.length * 6);
  const edgeLevel = new Float32Array(data.edges.length * 2);
  data.edges.forEach((edge, i) => {
    const a = nodes[edge.a]; const b = nodes[edge.b];
    if (!a || !b) return;
    edgePoints.set([a.x, a.y, a.z, b.x, b.y, b.z], i * 6);
    edgeLevel[i * 2] = edge.level; edgeLevel[i * 2 + 1] = edge.level;
  });

  const nodeProgram = compile(gl, VERTEX, FRAGMENT);
  const edgeProgram = compile(gl, EDGE_VERTEX, EDGE_FRAGMENT);

  const quad = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, quad);
  gl.bufferData(gl.ARRAY_BUFFER,
    new Float32Array([-0.5, -0.5, 0.5, -0.5, -0.5, 0.5, 0.5, 0.5]), gl.STATIC_DRAW);

  const attribute = (location, source, components, divisor) => {
    const buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, source, gl.DYNAMIC_DRAW);
    gl.enableVertexAttribArray(location);
    gl.vertexAttribPointer(location, components, gl.FLOAT, false, 0, 0);
    if (divisor) gl.vertexAttribDivisor(location, divisor);
    return buffer;
  };

  const nodeArray = gl.createVertexArray();
  gl.bindVertexArray(nodeArray);
  gl.bindBuffer(gl.ARRAY_BUFFER, quad);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
  const tintBuffer = (() => {
    attribute(1, centre, 3, 1);
    const buffer = attribute(2, tint, 3, 1);
    attribute(3, size, 1, 1);
    attribute(4, shape, 1, 1);
    attribute(5, pulse, 1, 1);
    attribute(6, level, 1, 1);
    return buffer;
  })();

  const edgeArray = gl.createVertexArray();
  gl.bindVertexArray(edgeArray);
  attribute(0, edgePoints, 3, 0);
  attribute(1, edgeLevel, 1, 0);

  const texture = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, atlas());
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);

  /* ---- camera ---- */
  // Frame what is there, rather than guessing a distance: a small graph
  // should not open zoomed out to nothing, and a large one should fit.
  const bounds = nodes.reduce((box, node) => ({
    lo: [Math.min(box.lo[0], node.x), Math.min(box.lo[1], node.y), Math.min(box.lo[2], node.z)],
    hi: [Math.max(box.hi[0], node.x), Math.max(box.hi[1], node.y), Math.max(box.hi[2], node.z)],
  }), { lo: [1e9, 1e9, 1e9], hi: [-1e9, -1e9, -1e9] });
  const target = [0, 1, 2].map((i) => (bounds.lo[i] + bounds.hi[i]) / 2);
  const extent = Math.max(...[0, 1, 2].map((i) => bounds.hi[i] - bounds.lo[i]), 40);
  const FOV = 0.85;
  // Fit the widest axis in view, with a margin, instead of guessing.
  const orbit = { yaw: 0.7, pitch: 0.34, distance: (extent / 2) / Math.tan(FOV / 2) * 1.18 };
  let hovered = -1;
  let running = true;

  function resize() {
    const ratio = Math.min(devicePixelRatio || 1, 2);
    canvas.width = Math.max(1, Math.floor(host.clientWidth * ratio));
    canvas.height = Math.max(1, Math.floor(host.clientHeight * ratio));
    gl.viewport(0, 0, canvas.width, canvas.height);
  }
  const observer = new ResizeObserver(resize);
  observer.observe(host);
  resize();

  function eyePosition() {
    return [
      target[0] + Math.cos(orbit.pitch) * Math.sin(orbit.yaw) * orbit.distance,
      target[1] + Math.sin(orbit.pitch) * orbit.distance,
      target[2] + Math.cos(orbit.pitch) * Math.cos(orbit.yaw) * orbit.distance,
    ];
  }

  function currentStep() {
    let step = ZOOM_STEPS[0];
    for (const candidate of ZOOM_STEPS) {
      if (orbit.distance <= extent * candidate.fraction) step = candidate;
    }
    return step;
  }

  /* Screen positions, recomputed per frame for labels and picking. */
  let projected = new Float32Array(count * 3);
  function project(camera) {
    for (let i = 0; i < count; i += 1) {
      const x = centre[i * 3]; const y = centre[i * 3 + 1]; const z = centre[i * 3 + 2];
      const cx = camera[0] * x + camera[4] * y + camera[8] * z + camera[12];
      const cy = camera[1] * x + camera[5] * y + camera[9] * z + camera[13];
      const cw = camera[3] * x + camera[7] * y + camera[11] * z + camera[15];
      projected[i * 3] = cw > 0 ? (cx / cw * 0.5 + 0.5) * host.clientWidth : -1e6;
      projected[i * 3 + 1] = cw > 0 ? (0.5 - cy / cw * 0.5) * host.clientHeight : -1e6;
      projected[i * 3 + 2] = cw;
    }
  }

  const labelLayer = document.createElement("div");
  labelLayer.style.cssText = "position:absolute;inset:0;pointer-events:none;overflow:hidden";
  host.append(labelLayer);

  let lastLabelKey = "";
  function labels(step) {
    // Landmarks are always named; detail is named only when close, and
    // never more than a screenful — a wall of overlapping text is not a
    // label, it is noise.
    const wanted = [];
    if (step.level === 0) {
      for (const cluster of data.clusters) wanted.push({ cluster });
    }
    if (step.level >= 1) {
      for (let i = 0; i < count; i += 1) {
        if (level[i] > step.level) continue;
        if (projected[i * 3 + 2] <= 0) continue;
        if (level[i] > 0 && orbit.distance > extent * 0.4) continue;
        wanted.push({ index: i, depth: projected[i * 3 + 2] });
      }
      wanted.sort((a, b) => (a.depth || 0) - (b.depth || 0));
      wanted.length = Math.min(wanted.length, 48);
    }
    const key = `${step.level}:${wanted.length}:${Math.round(orbit.yaw * 10)}:${Math.round(orbit.distance)}`;
    if (key === lastLabelKey && labelLayer.childElementCount === wanted.length) {
      // Same set: move what is already there rather than rebuild.
      Array.from(labelLayer.children).forEach((node, position) => {
        const item = wanted[position];
        const point = item.cluster
          ? clusterPoint(item.cluster)
          : [projected[item.index * 3], projected[item.index * 3 + 1]];
        node.style.left = `${point[0]}px`;
        node.style.top = `${point[1]}px`;
      });
      return;
    }
    lastLabelKey = key;
    labelLayer.replaceChildren(...wanted.map((item) => {
      const node = document.createElement("div");
      if (item.cluster) {
        node.className = "mri-label mri-cluster-label";
        node.textContent = `${item.cluster.label} · ${item.cluster.count}`;
        const point = clusterPoint(item.cluster);
        node.style.left = `${point[0]}px`;
        node.style.top = `${point[1]}px`;
      } else {
        node.className = "mri-label";
        node.textContent = nodes[item.index].label;
        node.style.left = `${projected[item.index * 3]}px`;
        node.style.top = `${projected[item.index * 3 + 1]}px`;
      }
      return node;
    }));
  }

  let camera = new Float32Array(16);
  function clusterPoint(cluster) {
    const cx = camera[0] * cluster.x + camera[4] * cluster.y + camera[8] * cluster.z + camera[12];
    const cy = camera[1] * cluster.x + camera[5] * cluster.y + camera[9] * cluster.z + camera[13];
    const cw = camera[3] * cluster.x + camera[7] * cluster.y + camera[11] * cluster.z + camera[15];
    if (cw <= 0) return [-9999, -9999];
    return [(cx / cw * 0.5 + 0.5) * host.clientWidth, (0.5 - cy / cw * 0.5) * host.clientHeight];
  }

  let start = performance.now();
  function frame() {
    if (!running) return;
    const eye = eyePosition();
    const view = lookAt(eye, target, [0, 1, 0]);
    const aspect = Math.max(host.clientWidth, 1) / Math.max(host.clientHeight, 1);
    camera = multiply(perspective(FOV, aspect, 1, 8000), view.matrix);
    const step = currentStep();

    gl.clearColor(0.027, 0.043, 0.063, 1);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.disable(gl.DEPTH_TEST);

    gl.useProgram(edgeProgram);
    gl.uniformMatrix4fv(gl.getUniformLocation(edgeProgram, "camera"), false, camera);
    gl.uniform1f(gl.getUniformLocation(edgeProgram, "maxLevel"), step.level);
    gl.bindVertexArray(edgeArray);
    gl.drawArrays(gl.LINES, 0, data.edges.length * 2);

    gl.useProgram(nodeProgram);
    gl.uniformMatrix4fv(gl.getUniformLocation(nodeProgram, "camera"), false, camera);
    gl.uniform3fv(gl.getUniformLocation(nodeProgram, "eye"), eye);
    gl.uniform3fv(gl.getUniformLocation(nodeProgram, "right"), view.right);
    gl.uniform3fv(gl.getUniformLocation(nodeProgram, "up"), view.up);
    gl.uniform1f(gl.getUniformLocation(nodeProgram, "time"), (performance.now() - start) / 1000);
    gl.uniform1f(gl.getUniformLocation(nodeProgram, "maxLevel"), step.level);
    gl.uniform1f(gl.getUniformLocation(nodeProgram, "focus"), hovered >= 0 ? 1 : 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.uniform1i(gl.getUniformLocation(nodeProgram, "glyphs"), 0);
    gl.bindVertexArray(nodeArray);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, count);

    project(camera);
    labels(step);
    options.onFrame?.({ step, inFocus: countInFocus(step.level), total: count });
    requestAnimationFrame(frame);
  }

  /* Everything is drawn; this is how much of it is at full detail. */
  function countInFocus(maxLevel) {
    let shown = 0;
    for (let i = 0; i < count; i += 1) if (level[i] <= maxLevel) shown += 1;
    return shown;
  }

  /* ---- interaction ---- */
  let dragging = null;
  canvas.addEventListener("pointerdown", (event) => {
    dragging = { x: event.clientX, y: event.clientY, moved: false };
    canvas.setPointerCapture(event.pointerId);
  });
  canvas.addEventListener("pointermove", (event) => {
    if (dragging) {
      const dx = event.clientX - dragging.x;
      const dy = event.clientY - dragging.y;
      if (Math.abs(dx) + Math.abs(dy) > 3) dragging.moved = true;
      orbit.yaw -= dx * 0.006;
      orbit.pitch = Math.max(-1.2, Math.min(1.35, orbit.pitch + dy * 0.005));
      dragging.x = event.clientX;
      dragging.y = event.clientY;
      return;
    }
    hovered = pick(event);
    canvas.title = hovered >= 0 ? nodes[hovered].label : "";
  });
  canvas.addEventListener("pointerup", (event) => {
    const wasDrag = dragging?.moved;
    dragging = null;
    if (wasDrag) return;
    const index = pick(event);
    if (index >= 0) options.onPick?.(nodes[index]);
  });
  canvas.addEventListener("wheel", (event) => {
    event.preventDefault();
    orbit.distance = Math.max(extent * 0.04, Math.min(extent * 3, orbit.distance * (1 + event.deltaY * 0.0012)));
  }, { passive: false });

  function pick(event) {
    const bounds = canvas.getBoundingClientRect();
    const x = event.clientX - bounds.left;
    const y = event.clientY - bounds.top;
    const maxLevel = currentStep().level;
    let best = -1;
    let bestDistance = 18;
    for (let i = 0; i < count; i += 1) {
      if (level[i] > maxLevel || projected[i * 3 + 2] <= 0) continue;
      const distance = Math.hypot(projected[i * 3] - x, projected[i * 3 + 1] - y);
      if (distance < bestDistance) { bestDistance = distance; best = i; }
    }
    return best;
  }

  canvas.tabIndex = 0;
  canvas.setAttribute("aria-label",
    `The graph as a spatial anatomy: ${count} things. Every fact here is also available as a list on the Map, Artifacts and Now.`);
  canvas.addEventListener("keydown", (event) => {
    const step = (factor) => {
      orbit.distance = Math.max(extent * 0.04, Math.min(extent * 3, orbit.distance * factor));
    };
    const keys = {
      "+": () => step(0.82), "=": () => step(0.82), "-": () => step(1.22),
      ArrowLeft: () => { orbit.yaw -= 0.12; },
      ArrowRight: () => { orbit.yaw += 0.12; },
      ArrowUp: () => { orbit.pitch = Math.min(1.35, orbit.pitch + 0.08); },
      ArrowDown: () => { orbit.pitch = Math.max(-1.2, orbit.pitch - 0.08); },
    };
    const action = keys[event.key];
    if (!action) return;
    event.preventDefault();
    action();
  });

  requestAnimationFrame(frame);

  return {
    setLens(next) {
      lens = next;
      paint();
      gl.bindBuffer(gl.ARRAY_BUFFER, tintBuffer);
      gl.bufferSubData(gl.ARRAY_BUFFER, 0, tint);
    },
    focusOn(clusterId) {
      const cluster = data.clusters.find((c) => c.id === clusterId);
      if (!cluster) return;
      target[0] = cluster.x; target[1] = cluster.y; target[2] = cluster.z;
      orbit.distance = Math.max(140, cluster.radius * 3.2);
    },
    destroy() {
      running = false;
      observer.disconnect();
      labelLayer.remove();
      canvas.remove();
    },
  };
}
