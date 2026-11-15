// Procurator console UI. Read-only: everything comes from /api/model and
// three live SSE streams (state, messages, logs) that the console server
// derives from the services' own logs.
"use strict";

const MAX_MESSAGES = 500;
const MAX_LOGS = 1000;
const MAX_SAMPLES = 120;
const MAX_PULSES_PER_SECOND = 30;
const KINDS = ["publish", "poll", "heartbeat", "status", "rpc", "vmm"];

const app = {
  model: null,
  messages: [], // newest first, since the page opened
  logs: [],
  samples: new Map(), // "w:<id>" | "vm:<worker>/<key>" -> [{t, ...}]
  edges: new Map(), // edge id -> {times: [ms], latency, bad}
  pulseBudget: { second: 0, used: 0 },
  showAll: false,
  feedPaused: false,
  feedKinds: new Set(KINDS),
  errorsOnly: false,
  page: null, // {name, render(), onMessage?(m), onLog?(l)}
};

// ---------- helpers ----------

const el = (tag, attrs = {}, ...children) => {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === undefined || value === null || value === false) continue;
    if (key === "class") node.className = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else node.setAttribute(key, value === true ? "" : value);
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
};
const SVG_NS = "http://www.w3.org/2000/svg";
const svg = (tag, attrs = {}, ...children) => {
  const node = document.createElementNS(SVG_NS, tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === undefined || value === null) continue;
    if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else node.setAttribute(key, value);
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
};

const fmtBytes = (n) => {
  if (n === undefined || n === null) return "–";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let value = n;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1; }
  return `${value < 10 && unit > 0 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
};
const fmtAge = (s) => {
  if (s === undefined || s === null) return "never";
  if (s < 1) return "just now";
  if (s < 60) return `${Math.round(s)} s ago`;
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  return `${(s / 3600).toFixed(1)} h ago`;
};
const fmtDuration = (s) => {
  if (!s && s !== 0) return "–";
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} min`;
  if (s < 86400) return `${(s / 3600).toFixed(1)} h`;
  return `${(s / 86400).toFixed(1)} d`;
};
const fmtTime = (iso) => {
  const date = iso ? new Date(iso) : new Date();
  return date.toTimeString().slice(0, 8);
};
const badge = (text, cls = text) => el("span", { class: `badge ${cls}` }, text);
const pct = (v) => (v === undefined || v === null ? "–" : `${v.toFixed(1)} %`);

const workerIdOf = (endpoint) => (endpoint && endpoint.startsWith("worker:") ? endpoint.slice(7) : null);
const allVms = () => {
  const out = [];
  for (const worker of app.model?.workers ?? []) {
    for (const vm of worker.vms) out.push({ worker, vm });
  }
  return out;
};
const unitOf = (vm) => vm.unit || "ungrouped";

// ---------- routing ----------

function route() {
  const parts = location.pathname.split("/").filter(Boolean).map(decodeURIComponent);
  const [first, second, third] = parts;
  if (!first) return mapPage();
  if (first === "machines" && second) return machinePage(second);
  if (first === "machines") return machinesPage();
  if (first === "units" && second) return unitPage(second);
  if (first === "units") return unitsPage();
  if (first === "vms" && second && third) return vmPage(second, third);
  if (first === "sources") return sourcesPage();
  return notFoundPage();
}

function navigate(path) {
  if (path !== location.pathname) history.pushState(null, "", path);
  show();
}

function show() {
  app.page = route();
  for (const link of document.querySelectorAll("[data-nav]")) {
    link.classList.toggle("active", link.dataset.nav === app.page.name);
  }
  render();
}

function render() {
  const view = document.getElementById("view");
  if (!app.page) return;
  if (!app.model) {
    view.replaceChildren(el("p", { class: "empty" }, "Waiting for the console…"));
    return;
  }
  app.page.render(view);
}

document.addEventListener("click", (event) => {
  const link = event.target.closest("a[data-link]");
  if (!link || event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) return;
  event.preventDefault();
  navigate(link.getAttribute("href"));
});
window.addEventListener("popstate", show);
const link = (path, text) => el("a", { href: path, "data-link": true }, text);
const machinePath = (id) => `/machines/${encodeURIComponent(id)}`;
const vmPath = (worker, key) => `/vms/${encodeURIComponent(worker)}/${encodeURIComponent(key)}`;
const unitPath = (unit) => `/units/${encodeURIComponent(unit)}`;

// ---------- live streams ----------

function setLive(up) {
  const node = document.getElementById("live");
  node.className = `live ${up ? "on" : "off"}`;
  node.textContent = up ? "live" : "reconnecting…";
}

// One connection for everything: browsers allow only six HTTP/1.1
// connections per host, and a page left open must not starve the next one.
const stream = new EventSource("/api/stream");
stream.onopen = () => setLive(true);
stream.onerror = () => setLive(false);
window.addEventListener("pagehide", () => stream.close());
const on = (event, handler) => stream.addEventListener(event, (e) => handler(e.data));

on("state", (data) => {
  app.model = JSON.parse(data);
  recordSamples();
  render();
});
on("message", (data) => {
  const message = JSON.parse(data);
  app.messages.unshift(message);
  if (app.messages.length > MAX_MESSAGES) app.messages.length = MAX_MESSAGES;
  countEdge(message);
  app.page?.onMessage?.(message);
});
on("log", (data) => {
  const log = JSON.parse(data);
  app.logs.unshift(log);
  if (app.logs.length > MAX_LOGS) app.logs.length = MAX_LOGS;
  app.page?.onLog?.(log);
});

// Metric history since the page opened, one point per new sample.
function recordSamples() {
  const now = Date.now();
  const push = (id, sample) => {
    const list = app.samples.get(id) ?? [];
    const last = list[list.length - 1];
    if (last && last.key === sample.key) return;
    list.push({ t: now, ...sample });
    if (list.length > MAX_SAMPLES) list.shift();
    app.samples.set(id, list);
  };
  for (const worker of app.model.workers) {
    const host = worker.host;
    push(`w:${worker.id}`, {
      key: `${worker.age_s !== null ? Math.round(now / 1000 - worker.age_s) : 0}`,
      memUsed: host.mem_total ? (host.mem_total - host.mem_available) : 0,
      load: host.load1,
    });
    for (const vm of worker.vms) {
      if (vm.uptime_s === undefined) continue;
      push(`vm:${worker.id}/${vm.key}`, {
        key: `${vm.uptime_s}`,
        cpu: vm.cpu_pct ?? 0,
        rss: vm.rss_bytes,
        netRx: vm.net_rx,
        netTx: vm.net_tx,
        diskRead: vm.disk_read,
        diskWrite: vm.disk_write,
      });
    }
  }
}

// ---------- messages: shared feed ----------

function describeMessage(m) {
  const parts = [`${m.from} → ${m.to}`, m.method];
  if (m.vm) parts.push(`vm=${m.vm}`);
  if (m.generation !== undefined && m.generation !== null) parts.push(`gen=${m.generation}`);
  if (m.count !== undefined && m.count !== null) parts.push(`n=${m.count}`);
  if (m.unchanged) parts.push("unchanged");
  if (m.latency_ms !== undefined && m.latency_ms !== null) parts.push(`${m.latency_ms.toFixed(1)} ms`);
  return parts.join("  ");
}

function messageRow(m) {
  const bad = m.outcome !== "ok";
  return el("div", { class: "row" },
    el("span", { class: "time" }, fmtTime(m.at)),
    el("span", { class: `what ${bad ? "err" : ""}` },
      el("span", { class: `dot k-${bad ? "bad" : m.kind}` }),
      describeMessage(m),
      bad ? `  ${m.outcome}${m.reason ? `: ${m.reason}` : ""}` : ""));
}

function logRow(l) {
  const text = l.change ? `[${l.change}] ${l.message ?? ""}` : (l.message ?? "");
  return el("div", { class: "row" },
    el("span", { class: "time" }, fmtTime(l.at)),
    el("span", { class: `what lvl-${l.level ?? ""}` },
      `${l.origin}  ${l.level ?? ""}  ${text}${l.vm ? `  vm=${l.vm}` : ""}`));
}

/** A live list filtered by `accept`, newest first. */
function feed(items, accept, row, emptyText) {
  const box = el("div", { class: "feed" });
  const shown = items.filter(accept).slice(0, 200);
  if (shown.length === 0) box.append(el("p", { class: "empty" }, emptyText));
  else box.append(...shown.map(row));
  return {
    node: box,
    add(item) {
      if (!accept(item)) return;
      box.querySelector(".empty")?.remove();
      box.prepend(row(item));
      while (box.childElementCount > 200) box.lastElementChild.remove();
    },
  };
}

const touches = (m, endpoint) => m.from === endpoint || m.to === endpoint || m.origin === endpoint;

// ---------- map ----------

function countEdge(m) {
  const id = edgeId(m.from, m.to, m.origin);
  if (!id) return;
  const edge = app.edges.get(id) ?? { times: [], latency: null, bad: false };
  const now = Date.now();
  edge.times.push(now);
  while (edge.times.length && edge.times[0] < now - 10000) edge.times.shift();
  if (m.latency_ms !== undefined && m.latency_ms !== null) edge.latency = m.latency_ms;
  edge.bad = m.outcome !== "ok";
  app.edges.set(id, edge);
}

/** Which drawn edge a message travels on. */
function edgeId(from, to, origin) {
  const end = (endpoint) => {
    if (!endpoint) return null;
    if (endpoint === "cp") return "cp";
    if (endpoint.startsWith("worker:")) return endpoint;
    if (endpoint.startsWith("client:")) return "clients";
    if (endpoint.startsWith("vmm:")) return `vmm@${origin}`;
    return null;
  };
  const a = end(from);
  const b = end(to);
  if (!a || !b || a === b) return null;
  if (a === "cp" || b === "cp") return `cp|${a === "cp" ? b : a}`;
  if (a.startsWith("vmm@") || b.startsWith("vmm@")) return `vmm|${a.startsWith("vmm@") ? a : b}`;
  return null;
}

function layout(model) {
  const width = 1000;
  const workers = model.workers;
  const boxes = workers.map((w) => {
    const rows = Math.max(1, Math.ceil(w.vms.length / 2));
    return { w, width: 210, height: 56 + rows * 26 + 18 };
  });
  const tallest = Math.max(120, ...boxes.map((b) => b.height));
  const ringX = 360;
  const ringY = Math.max(220, tallest + 40);
  const height = Math.max(520, ringY * 2 + tallest * 0.5 + 40);
  const cp = { x: 400, y: height / 2 };
  const clients = { x: 90, y: height / 2 };
  const placed = new Map();
  boxes.forEach((box, index) => {
    // Spread workers on an arc to the right of the control plane; clients
    // sit on the left.
    const span = Math.min(Math.PI * 1.2, Math.PI * 0.32 * (workers.length - 1));
    const angle = workers.length === 1 ? 0 : -span / 2 + (span * index) / (workers.length - 1);
    const cx = cp.x + Math.cos(angle) * ringX;
    const cy = cp.y + Math.sin(angle) * ringY;
    const x = Math.min(width - box.width - 10, Math.max(cp.x - 40, cx - box.width / 2));
    const y = Math.min(height - box.height - 10, Math.max(10, cy - box.height / 2));
    placed.set(`worker:${box.w.id}`, {
      ...box,
      x, y,
      center: { x: x + box.width / 2, y: y + box.height / 2 },
      vmm: { x: x + box.width / 2, y: y + box.height - 9 },
    });
  });
  return { width, height, cp, clients, placed };
}

function point(endpoint, origin, geo) {
  if (endpoint === "cp") return geo.cp;
  if (endpoint.startsWith("client:")) return geo.clients;
  if (endpoint.startsWith("worker:")) return geo.placed.get(endpoint)?.center ?? null;
  if (endpoint.startsWith("vmm:")) return geo.placed.get(origin)?.vmm ?? null;
  return null;
}

function mapPage() {
  let geo = null;
  let pulses = null;
  let messageFeed = null;
  let drawn = null; // signature of what the SVG currently shows

  const signature = (model) => JSON.stringify([
    model.cluster.known, model.cluster.generation,
    model.workers.map((w) => [w.id, w.healthy, w.generation, w.age_s === null, w.vms.map((vm) => [vm.key, vm.state, vm.unit])]),
  ]);

  const quiet = (m) => m.outcome === "ok" && (m.unchanged === true || m.kind === "heartbeat" || m.method === "vm.counters");

  function pulse(m) {
    if (!geo || !pulses) return;
    if (!app.showAll && quiet(m)) { flashEdge(m); return; }
    const second = Math.floor(Date.now() / 1000);
    if (app.pulseBudget.second !== second) app.pulseBudget = { second, used: 0 };
    if (app.pulseBudget.used >= MAX_PULSES_PER_SECOND) { flashEdge(m); return; }
    app.pulseBudget.used += 1;
    const from = point(m.from, m.origin, geo);
    const to = point(m.to, m.origin, geo);
    if (!from || !to) return;
    flashEdge(m);
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const bad = m.outcome !== "ok";
    const dot = svg("circle", { r: 6, class: `pulse k-${bad ? "bad" : m.kind}`, cx: 0, cy: 0 });
    pulses.append(dot);
    const animation = dot.animate(
      [
        { transform: `translate(${from.x}px, ${from.y}px)`, opacity: 1 },
        { transform: `translate(${to.x}px, ${to.y}px)`, opacity: 1, offset: 0.85 },
        { transform: `translate(${to.x}px, ${to.y}px)`, opacity: 0 },
      ],
      { duration: 900, easing: "ease-in-out" },
    );
    animation.onfinish = () => dot.remove();
  }

  function flashEdge(m) {
    const id = edgeId(m.from, m.to, m.origin);
    const line = id && document.querySelector(`[data-edge="${CSS.escape(id)}"]`);
    if (!line) return;
    line.classList.add(m.outcome === "ok" ? "flash" : "bad");
    setTimeout(() => line.classList.remove("flash", "bad"), 500);
    updateEdgeLabel(id);
  }

  function updateEdgeLabel(id) {
    const label = document.querySelector(`[data-edge-label="${CSS.escape(id)}"]`);
    const edge = app.edges.get(id);
    if (!label || !edge) return;
    const now = Date.now();
    const recent = edge.times.filter((t) => t >= now - 10000).length;
    const rate = (recent / 10).toFixed(1);
    label.textContent = `${rate}/s${edge.latency !== null ? ` · ${edge.latency.toFixed(1)} ms` : ""}`;
  }

  function drawEdge(layer, id, a, b) {
    layer.append(svg("line", { x1: a.x, y1: a.y, x2: b.x, y2: b.y, class: "edge", "data-edge": id }));
    // Closer to the control-plane end, clear of the machine boxes.
    const mx = a.x + (b.x - a.x) * 0.42;
    const my = a.y + (b.y - a.y) * 0.42 - 6;
    layer.append(svg("text", { x: mx, y: my, "text-anchor": "middle", class: "edge-label", "data-edge-label": id }, ""));
  }

  function drawMap(model) {
    geo = layout(model);
    const root = svg("svg", { class: "map", viewBox: `0 0 ${geo.width} ${geo.height}`, role: "img", "aria-label": "Cluster map" });
    const edges = svg("g");
    const nodes = svg("g");
    pulses = svg("g");
    root.append(edges, nodes, pulses);

    // Edges first so boxes sit on top.
    drawEdge(edges, "cp|clients", geo.clients, geo.cp);
    for (const [endpoint, box] of geo.placed) {
      drawEdge(edges, `cp|${endpoint}`, geo.cp, box.center);
    }

    // Clients (CLI / CI callers).
    nodes.append(
      svg("circle", { cx: geo.clients.x, cy: geo.clients.y, r: 34, class: "node" }),
      svg("text", { x: geo.clients.x, y: geo.clients.y + 4, "text-anchor": "middle" }, "clients"),
      svg("text", { x: geo.clients.x, y: geo.clients.y + 52, "text-anchor": "middle", class: "muted" }, "CLI / CI"),
    );

    // Control plane.
    const cluster = model.cluster;
    nodes.append(
      svg("circle", { cx: geo.cp.x, cy: geo.cp.y, r: 46, class: `node cp${cluster.known ? "" : " down"}` }),
      svg("text", { x: geo.cp.x, y: geo.cp.y - 2, "text-anchor": "middle", "font-weight": 600 }, "control plane"),
      svg("text", { x: geo.cp.x, y: geo.cp.y + 15, "text-anchor": "middle", class: "muted" },
        cluster.known ? `gen ${cluster.generation}` : "no state yet"),
    );

    // Machines with their VMs.
    for (const [endpoint, box] of geo.placed) {
      const w = box.w;
      const group = svg("g", { class: "clickable", onclick: () => navigate(machinePath(w.id)) });
      group.append(
        svg("rect", { x: box.x, y: box.y, width: box.width, height: box.height, rx: 10, class: `node${w.healthy ? "" : " down"}` }),
        svg("text", { x: box.x + 12, y: box.y + 20, "font-weight": 600 }, w.id),
        svg("text", { x: box.x + box.width - 12, y: box.y + 20, "text-anchor": "end", class: "muted" },
          w.healthy ? `gen ${w.generation}` : (w.age_s === null ? "no logs" : "stale")),
      );
      w.vms.forEach((vm, index) => {
        const col = index % 2;
        const row = Math.floor(index / 2);
        const x = box.x + 10 + col * 97;
        const y = box.y + 34 + row * 26;
        const chip = svg("g", {
          class: "clickable",
          onclick: (event) => { event.stopPropagation(); navigate(vmPath(w.id, vm.key)); },
        });
        chip.append(
          svg("rect", { x, y, width: 92, height: 21, rx: 5, class: `chip ${vm.state}` }),
          svg("text", { x: x + 6, y: y + 14.5, class: "chip-text" }, vm.key.length > 12 ? `${vm.key.slice(0, 11)}…` : vm.key),
          svg("title", {}, `${vm.key}: ${vm.state}${vm.unit ? ` (unit ${vm.unit})` : ""}`),
        );
        group.append(chip);
      });
      if (w.vms.length === 0) {
        group.append(svg("text", { x: box.x + 12, y: box.y + 48, class: "muted" }, "no VMs"));
      }
      edges.append(svg("line", {
        x1: box.center.x, y1: box.y + 26, x2: box.vmm.x, y2: box.vmm.y,
        class: "edge", "data-edge": `vmm|vmm@${endpoint}`, "stroke-opacity": 0,
      }));
      group.append(
        svg("rect", { x: box.vmm.x - 5, y: box.vmm.y - 5, width: 10, height: 10, transform: `rotate(45 ${box.vmm.x} ${box.vmm.y})`, class: "vmm" }),
        svg("title", {}, `${w.id}: Cloud Hypervisor processes`),
      );
      nodes.append(group);
    }
    return root;
  }

  return {
    name: "map",
    render(view) {
      const model = app.model;
      const total = model.workers.length;
      const converged = model.workers.filter((w) => w.up_to_date).length;
      const healthy = model.workers.filter((w) => w.healthy).length;
      const vmCount = allVms().length;
      const brokenSources = model.sources.filter((s) => !s.connected).length;

      // Keep the message drawer across re-renders; redraw the map only when
      // what it shows changed (redrawing drops pulses in flight).
      const existing = view.querySelector(".map-layout");
      const current = signature(model);
      if (existing && messageFeed) {
        if (current !== drawn) {
          existing.querySelector(".map-svg").replaceChildren(drawMap(model));
          drawn = current;
        }
        existing.querySelector(".stats").replaceWith(statsRow());
        for (const id of app.edges.keys()) updateEdgeLabel(id);
        return;
      }

      function statsRow() {
        return el("div", { class: "stats" },
          stat("Generation", model.cluster.known ? model.cluster.generation : "–"),
          stat("Commit", model.cluster.commit ? model.cluster.commit.slice(0, 10) : "–"),
          stat("Converged", `${converged} / ${total}`),
          stat("Healthy machines", `${healthy} / ${total}`),
          stat("VMs", vmCount),
          stat("Sources", brokenSources ? el("a", { href: "/sources", "data-link": true }, `${brokenSources} down`) : "all up"),
        );
      }

      const kindsLegend = el("span", { class: "legend" },
        ...KINDS.map((kind) => el("span", { class: `k-${kind}` }, kind)),
        el("span", { class: "k-bad" }, "error"));
      const showAll = el("input", {
        type: "checkbox",
        checked: app.showAll,
        onchange: (event) => { app.showAll = event.target.checked; },
      });
      messageFeed = messageDrawer();
      const mapNode = drawMap(model);
      drawn = current;
      view.replaceChildren(
        el("div", { class: "map-layout" },
          el("div", {},
            statsRow(),
            el("div", { class: "panel map-panel" },
              el("div", { class: "map-tools" },
                kindsLegend,
                el("label", {}, showAll, "animate unchanged polls and heartbeats"),
              ),
              el("div", { class: "map-svg" }, mapNode),
            ),
          ),
          el("div", { class: "panel" }, el("h2", {}, "Messages"), messageFeed.node),
        ),
      );
      for (const id of app.edges.keys()) updateEdgeLabel(id);
    },
    onMessage(m) {
      pulse(m);
      messageFeed?.add(m);
    },
  };
}

function stat(label, value) {
  return el("div", { class: "stat" }, el("span", { class: "label" }, label), el("span", { class: "value" }, value));
}

/** Message list with kind filters, pause and errors-only. */
function messageDrawer(scope = () => true) {
  const accept = (m) =>
    scope(m) && app.feedKinds.has(m.kind) && (!app.errorsOnly || m.outcome !== "ok");
  const holder = el("div");
  let list = feed(app.messages, accept, messageRow, "No messages since the page opened.");
  holder.append(list.node);
  const rebuild = () => {
    list = feed(app.messages, accept, messageRow, "No messages since the page opened.");
    holder.replaceChildren(list.node);
  };
  const tools = el("div", { class: "feed-tools" },
    ...KINDS.map((kind) => {
      const button = el("button", {
        class: `chipbtn ${app.feedKinds.has(kind) ? "on" : ""}`,
        onclick: () => {
          if (app.feedKinds.has(kind)) app.feedKinds.delete(kind); else app.feedKinds.add(kind);
          button.classList.toggle("on");
          rebuild();
        },
      }, kind);
      return button;
    }),
    (() => {
      const button = el("button", {
        class: `chipbtn ${app.errorsOnly ? "on" : ""}`,
        onclick: () => { app.errorsOnly = !app.errorsOnly; button.classList.toggle("on"); rebuild(); },
      }, "errors only");
      return button;
    })(),
    (() => {
      const button = el("button", {
        class: `chipbtn ${app.feedPaused ? "on" : ""}`,
        onclick: () => {
          app.feedPaused = !app.feedPaused;
          button.classList.toggle("on");
          button.textContent = app.feedPaused ? "paused" : "pause";
          if (!app.feedPaused) rebuild();
        },
      }, app.feedPaused ? "paused" : "pause");
      return button;
    })(),
  );
  return {
    node: el("div", {}, tools, holder),
    add(m) { if (!app.feedPaused) list.add(m); },
  };
}

// ---------- machines ----------

function machinesPage() {
  return {
    name: "machines",
    render(view) {
      const rows = app.model.workers.map((w) => {
        const used = w.host.mem_total ? w.host.mem_total - w.host.mem_available : null;
        return el("tr", { class: "clickable", onclick: () => navigate(machinePath(w.id)) },
          el("td", {}, link(machinePath(w.id), w.id)),
          el("td", {}, w.healthy ? badge("healthy") : badge(w.age_s === null ? "no logs" : "unhealthy", "unhealthy")),
          el("td", {}, fmtAge(w.age_s)),
          el("td", {}, w.generation),
          el("td", {}, w.vms.length),
          el("td", {}, used === null ? "–" : `${fmtBytes(used)} / ${fmtBytes(w.host.mem_total)}`),
          el("td", {}, w.host.cpus ? `${w.host.load1.toFixed(2)} (${w.host.cpus} CPUs)` : "–"),
        );
      });
      view.replaceChildren(
        el("h1", {}, "Machines"),
        el("p", { class: "sub" }, "Worker hosts, from their state lines."),
        el("div", { class: "panel table-wrap" },
          rows.length
            ? el("table", {},
              el("thead", {}, el("tr", {}, ...["Machine", "Health", "Last state line", "Generation", "VMs", "Memory used", "Load"].map((h) => el("th", {}, h)))),
              el("tbody", {}, rows))
            : el("p", { class: "empty" }, "No workers known yet.")),
      );
    },
  };
}

function vmTable(worker, vms) {
  if (vms.length === 0) return el("p", { class: "empty" }, "No VMs.");
  return el("div", { class: "table-wrap" }, el("table", {},
    el("thead", {}, el("tr", {}, ...["VM", "State", "Unit", "IP", "Uptime", "CPU", "Memory", "Net rx / tx"].map((h) => el("th", {}, h)))),
    el("tbody", {}, vms.map(({ worker: w, vm }) => el("tr", { class: "clickable", onclick: () => navigate(vmPath(w.id, vm.key)) },
      el("td", {}, link(vmPath(w.id, vm.key), vm.key)),
      el("td", {}, badge(vm.state)),
      el("td", {}, link(unitPath(unitOf(vm)), unitOf(vm))),
      el("td", { class: "mono" }, vm.ip || "–"),
      el("td", {}, fmtDuration(vm.uptime_s)),
      el("td", {}, pct(vm.cpu_pct)),
      el("td", {}, fmtBytes(vm.rss_bytes)),
      el("td", {}, vm.net_rx === undefined ? "–" : `${fmtBytes(vm.net_rx)} / ${fmtBytes(vm.net_tx)}`),
    ))),
  ));
}

function sparkline(points, label, format) {
  const values = points.map((p) => p.value);
  const last = values.length ? format(values[values.length - 1]) : "–";
  const box = el("div", { class: "spark-row" }, el("div", { class: "label" }, el("span", {}, label), el("span", {}, last)));
  if (values.length < 2) {
    box.append(el("div", { class: "note" }, "collecting samples since the page opened…"));
    return box;
  }
  const max = Math.max(...values, 1e-9);
  const min = Math.min(...values, 0);
  const step = 200 / (values.length - 1);
  const coords = values.map((v, i) => `${(i * step).toFixed(1)},${(40 - ((v - min) / (max - min || 1)) * 36 - 2).toFixed(1)}`).join(" ");
  box.append(svg("svg", { class: "spark", viewBox: "0 0 200 40", preserveAspectRatio: "none" }, svg("polyline", { points: coords })));
  return box;
}

/** Per-second rate from consecutive cumulative samples. */
function rates(samples, field) {
  const out = [];
  for (let i = 1; i < samples.length; i += 1) {
    const dt = (samples[i].t - samples[i - 1].t) / 1000;
    const dv = samples[i][field] - samples[i - 1][field];
    if (dt > 0 && dv >= 0) out.push({ value: dv / dt });
  }
  return out;
}

function machinePage(id) {
  let logs = null;
  let messages = null;
  return {
    name: "machines",
    render(view) {
      const w = app.model.workers.find((worker) => worker.id === id);
      if (!w) {
        view.replaceChildren(el("h1", {}, id), el("p", { class: "empty" }, "This machine is not in the logs (yet)."));
        return;
      }
      const samples = app.samples.get(`w:${id}`) ?? [];
      const header = el("div", {},
        el("p", { class: "sub" }, link("/machines", "Machines"), " / ", id),
        el("h1", {}, id, " ", w.healthy ? badge("healthy") : badge("unhealthy")),
        el("div", { class: "stats" },
          stat("Last state line", fmtAge(w.age_s)),
          stat("Applied generation", w.generation),
          stat("Up to date", w.up_to_date ? "yes" : "no"),
          stat("VMs", w.vms.length),
          stat("CPUs", w.host.cpus || "–"),
        ));
      const metrics = el("div", { class: "panel" }, el("h2", {}, "Host"),
        sparkline(samples.map((s) => ({ value: s.memUsed })), "Memory used", fmtBytes),
        sparkline(samples.map((s) => ({ value: s.load })), "Load (1 min)", (v) => v.toFixed(2)));
      const vms = el("div", { class: "panel" }, el("h2", {}, "VMs"), vmTable(w, w.vms.map((vm) => ({ worker: w, vm }))));

      const existing = view.querySelector(`[data-machine="${CSS.escape(id)}"]`);
      if (existing && logs && messages) {
        existing.querySelector(".m-header").replaceChildren(header);
        existing.querySelector(".m-metrics").replaceChildren(metrics);
        existing.querySelector(".m-vms").replaceChildren(vms);
        return;
      }
      const endpoint = `worker:${id}`;
      logs = feed(app.logs, (l) => l.origin === endpoint, logRow, "No log lines since the page opened.");
      messages = messageDrawer((m) => touches(m, endpoint));
      view.replaceChildren(el("div", { "data-machine": id },
        el("div", { class: "m-header" }, header),
        el("div", { class: "grid" }, el("div", { class: "m-metrics" }, metrics), el("div", { class: "m-vms" }, vms)),
        el("div", { class: "grid" },
          el("div", { class: "panel" }, el("h2", {}, "Messages"), messages.node),
          el("div", { class: "panel" }, el("h2", {}, "Logs"), logs.node)),
      ));
    },
    onMessage(m) { messages?.add(m); },
    onLog(l) { logs?.add(l); },
  };
}

// ---------- units ----------

function unitsPage() {
  return {
    name: "units",
    render(view) {
      const groups = new Map();
      for (const item of allVms()) {
        const unit = unitOf(item.vm);
        if (!groups.has(unit)) groups.set(unit, []);
        groups.get(unit).push(item);
      }
      const rows = [...groups.entries()].sort(([a], [b]) => a.localeCompare(b)).map(([unit, items]) => {
        const running = items.filter(({ vm }) => vm.state === "running").length;
        const machines = new Set(items.map(({ worker }) => worker.id));
        return el("tr", { class: "clickable", onclick: () => navigate(unitPath(unit)) },
          el("td", {}, link(unitPath(unit), unit)),
          el("td", {}, `${running} / ${items.length}`),
          el("td", {}, [...machines].join(", ")));
      });
      view.replaceChildren(
        el("h1", {}, "Deployment units"),
        el("p", { class: "sub" }, "VMs grouped by the unit their logs carry. Units are not published yet, so every VM is “ungrouped” for now."),
        el("div", { class: "panel table-wrap" },
          rows.length
            ? el("table", {}, el("thead", {}, el("tr", {}, el("th", {}, "Unit"), el("th", {}, "Running"), el("th", {}, "Machines"))), el("tbody", {}, rows))
            : el("p", { class: "empty" }, "No VMs known yet.")),
      );
    },
  };
}

function unitPage(unit) {
  let messages = null;
  return {
    name: "units",
    render(view) {
      const items = allVms().filter(({ vm }) => unitOf(vm) === unit);
      const keys = new Set(items.map(({ vm }) => vm.key));
      const table = el("div", { class: "panel" }, el("h2", {}, "VMs"),
        items.length ? vmTableWithMachine(items) : el("p", { class: "empty" }, "No VMs in this unit."));
      const existing = view.querySelector(`[data-unit="${CSS.escape(unit)}"]`);
      if (existing && messages) {
        existing.querySelector(".u-table").replaceChildren(table);
        return;
      }
      messages = messageDrawer((m) => m.vm && keys.has(m.vm));
      view.replaceChildren(el("div", { "data-unit": unit },
        el("p", { class: "sub" }, link("/units", "Units"), " / ", unit),
        el("h1", {}, unit),
        el("div", { class: "u-table" }, table),
        el("div", { class: "panel" }, el("h2", {}, "Messages about these VMs"), messages.node),
      ));
    },
    onMessage(m) { messages?.add(m); },
  };
}

function vmTableWithMachine(items) {
  return el("div", { class: "table-wrap" }, el("table", {},
    el("thead", {}, el("tr", {}, ...["VM", "Machine", "State", "IP", "Uptime", "CPU", "Memory"].map((h) => el("th", {}, h)))),
    el("tbody", {}, items.map(({ worker, vm }) => el("tr", { class: "clickable", onclick: () => navigate(vmPath(worker.id, vm.key)) },
      el("td", {}, link(vmPath(worker.id, vm.key), vm.key)),
      el("td", {}, link(machinePath(worker.id), worker.id)),
      el("td", {}, badge(vm.state)),
      el("td", { class: "mono" }, vm.ip || "–"),
      el("td", {}, fmtDuration(vm.uptime_s)),
      el("td", {}, pct(vm.cpu_pct)),
      el("td", {}, fmtBytes(vm.rss_bytes)),
    ))),
  ));
}

// ---------- VM ----------

function vmPage(workerId, key) {
  let logs = null;
  let messages = null;
  return {
    name: "machines",
    render(view) {
      const w = app.model.workers.find((worker) => worker.id === workerId);
      const vm = w?.vms.find((candidate) => candidate.key === key);
      const crumbs = el("p", { class: "sub" }, link("/machines", "Machines"), " / ", link(machinePath(workerId), workerId), " / ", key);
      if (!vm) {
        view.replaceChildren(crumbs, el("h1", {}, key), el("p", { class: "empty" }, "This VM is not on this machine (anymore)."));
        logs = null;
        messages = null;
        return;
      }
      const samples = app.samples.get(`vm:${workerId}/${key}`) ?? [];
      const header = el("div", {},
        crumbs,
        el("h1", {}, key, " ", badge(vm.state)),
        el("div", { class: "stats" },
          stat("Machine", link(machinePath(workerId), workerId)),
          stat("Unit", link(unitPath(unitOf(vm)), unitOf(vm))),
          stat("IP", vm.ip || "–"),
          stat("Uptime", fmtDuration(vm.uptime_s)),
          stat("Desired", vm.desired ? "yes" : "no"),
          stat("Applied generation", w.generation),
        ));
      const metrics = el("div", { class: "panel" }, el("h2", {}, "Resources"),
        sparkline(samples.map((s) => ({ value: s.cpu })), "CPU (VMM process, % of one core)", (v) => `${v.toFixed(1)} %`),
        sparkline(samples.map((s) => ({ value: s.rss })), "Memory (VMM resident)", fmtBytes),
        sparkline(rates(samples, "netRx"), "Network in", (v) => `${fmtBytes(v)}/s`),
        sparkline(rates(samples, "netTx"), "Network out", (v) => `${fmtBytes(v)}/s`),
        sparkline(rates(samples, "diskRead"), "Disk read", (v) => `${fmtBytes(v)}/s`),
        sparkline(rates(samples, "diskWrite"), "Disk write", (v) => `${fmtBytes(v)}/s`));

      const existing = view.querySelector(`[data-vm="${CSS.escape(`${workerId}/${key}`)}"]`);
      if (existing && logs && messages) {
        existing.querySelector(".v-header").replaceChildren(header);
        existing.querySelector(".v-metrics").replaceChildren(metrics);
        return;
      }
      logs = feed(app.logs, (l) => l.vm === key, logRow, "No log lines about this VM since the page opened.");
      messages = messageDrawer((m) => m.vm === key);
      view.replaceChildren(el("div", { "data-vm": `${workerId}/${key}` },
        el("div", { class: "v-header" }, header),
        el("div", { class: "grid" },
          el("div", { class: "v-metrics" }, metrics),
          el("div", { class: "panel" }, el("h2", {}, "Worker logs about this VM"), logs.node)),
        el("div", { class: "panel" }, el("h2", {}, "Messages"), messages.node),
      ));
    },
    onMessage(m) { messages?.add(m); },
    onLog(l) { logs?.add(l); },
  };
}

// ---------- sources ----------

function sourcesPage() {
  return {
    name: "sources",
    render(view) {
      const rows = app.model.sources.map((s) => el("tr", {},
        el("td", {}, s.name),
        el("td", { class: "mono" }, s.endpoint),
        el("td", { class: "mono" }, s.kind),
        el("td", {}, s.connected ? badge("ok") : badge(s.error ?? "not connected", "error")),
        el("td", {}, s.lines),
        el("td", {}, s.unparsed),
        el("td", {}, fmtAge(s.last_line_age_s)),
      ));
      const unparsed = app.model.sources.filter((s) => s.recent_unparsed.length).map((s) =>
        el("div", { class: "panel" }, el("h2", {}, `Unparsed lines from ${s.name}`),
          ...s.recent_unparsed.map((line) => el("pre", { class: "unparsed" }, line))));
      view.replaceChildren(
        el("h1", {}, "Sources"),
        el("p", { class: "sub" }, "Where the console reads each service's logs. Unparsed lines usually mean a service logs in the pretty format or drifted from the log-events convention."),
        el("div", { class: "panel table-wrap" }, el("table", {},
          el("thead", {}, el("tr", {}, ...["Name", "Service", "Source", "Status", "Lines", "Unparsed", "Last line"].map((h) => el("th", {}, h)))),
          el("tbody", {}, rows))),
        el("div", { class: "grid", style: "margin-top:16px" }, unparsed),
      );
    },
  };
}

function notFoundPage() {
  return {
    name: null,
    render(view) {
      view.replaceChildren(el("h1", {}, "Not found"), el("p", {}, link("/", "Back to the map")));
    },
  };
}

// Initial snapshot (the state stream also sends one right away).
fetch("/api/model")
  .then((response) => response.json())
  .then((model) => {
    if (!app.model && model.workers) {
      app.model = model;
      recordSamples();
      render();
    }
  })
  .catch(() => {});
show();
