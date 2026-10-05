// FrisyDisk interface. Talks to the Rust core through one `api(cmd, args)`
// call: Tauri's invoke in the desktop app, plain HTTP under `frisyscan serve`.

import { Chart, colorFor } from "./charts.js";
import { prepare } from "./layout.js";
import { escapeHtml, fmtBytes, fmtCount, markdown, plural } from "./util.js";

const T = window.__TAURI__;
const $ = (id) => document.getElementById(id);

async function call(cmd, args = {}) {
  if (T) {
    try {
      return await T.core.invoke("api", { cmd, args });
    } catch (e) {
      throw new Error(typeof e === "string" ? e : e?.message || String(e));
    }
  }
  const res = await fetch(`/api/${cmd}`, { method: "POST", body: JSON.stringify(args) });
  const body = await res.json();
  if (body.error) throw new Error(body.error);
  return body.ok;
}

const state = {
  platform: "macos",
  focus: 0,
  mode: "sunburst",
  tab: "contents",
  view: null,
  tree: null,
  byId: new Map(),
  scanning: false,
  progress: null,
  rootSize: 0,
  largest: [],
  types: [],
  search: "",
  searchRows: [],
  backends: null,
  backend: null,
  advisor: { messages: [], running: false, error: null, risky: [], started: false },
};

const chart = new Chart($("chart"));
const isDark = () => getComputedStyle(document.documentElement).getPropertyValue("--is-dark").trim() === "1";
const trashName = () => (state.platform === "windows" ? "Recycle Bin" : "Trash");

// ---- Small UI pieces ------------------------------------------------------

let toastTimer;
function toast(message, ms = 5200) {
  const el = $("toast");
  el.textContent = message;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), ms);
}

function fail(e) {
  toast(e?.message || String(e));
}

function openModal(html) {
  const modal = $("modal");
  modal.firstElementChild.innerHTML = html;
  modal.hidden = false;
  modal.querySelector("button.primary, button.warn, button")?.focus();
  return modal.firstElementChild;
}

function closeModal() {
  $("modal").hidden = true;
}

function select(groupId, attr, value) {
  for (const b of $(groupId).querySelectorAll("button")) b.setAttribute("aria-selected", String(b.dataset[attr] === value));
}

// ---- Start screen ---------------------------------------------------------

async function showStart() {
  $("scan").hidden = true;
  $("start").hidden = false;
  $("browse").hidden = !T;
  let volumes = [];
  try {
    volumes = await call("volumes");
  } catch (e) {
    fail(e);
  }
  $("volumes").innerHTML = volumes
    .map((v, i) => {
      const used = v.total > 0 ? Math.round(((v.total - v.free) / v.total) * 100) : 0;
      const gauge = used >= 90 ? "var(--warn)" : "var(--accent)";
      const kind = v.network ? `Network share, ${escapeHtml(v.source)}` : `${v.removable ? "Removable, " : ""}${escapeHtml(v.fs.toUpperCase())}`;
      return `<li class="volume">
        <div class="gauge" style="--used:${used};--gauge:${gauge}"><span>${used}%</span></div>
        <div><div class="volume-name">${escapeHtml(v.name)}</div><div class="volume-meta">${kind}</div></div>
        <div class="volume-free"><strong>${fmtBytes(v.free)}</strong> free<br>of ${fmtBytes(v.total)}</div>
        <button class="primary" data-volume="${i}">Scan</button>
      </li>`;
    })
    .join("");
  for (const b of $("volumes").querySelectorAll("button")) b.onclick = () => startScan(volumes[Number(b.dataset.volume)].mount);
}

// ---- Scanning -------------------------------------------------------------

let pollTimer;

async function startScan(path) {
  try {
    await call("start_scan", { path });
  } catch (e) {
    return fail(e);
  }
  Object.assign(state, { focus: 0, scanning: true, largest: [], types: [], search: "", searchRows: [], view: null, tree: null });
  state.advisor = { messages: [], running: false, error: null, risky: [], started: false };
  $("search").value = "";
  $("start").hidden = true;
  $("scan").hidden = false;
  select("modes", "mode", state.mode);
  setTab(state.tab);
  chart.set(null, state.mode, false);
  poll();
}

async function poll() {
  clearTimeout(pollTimer);
  await refresh(false);
  if (state.progress?.scanning) {
    pollTimer = setTimeout(poll, 450);
  } else if (state.scanning) {
    state.scanning = false;
    await refresh(true);
    loadAnalysis();
    window.dispatchEvent(new Event("frisy-scan-done"));
  }
}

async function refresh(animate) {
  let res;
  try {
    res = await call("view", {
      focus: state.focus,
      depth: state.mode === "sankey" ? 4 : 6,
      min_fraction: state.mode === "treemap" ? 0.0015 : 0.0035,
    });
  } catch (e) {
    if (state.focus !== 0) {
      state.focus = 0;
      return refresh(animate);
    }
    return fail(e);
  }
  state.view = res;
  state.progress = res.progress;
  // Hidden space joins the chart as one grey slice, broken into its known parts.
  state.hidden = res.hidden || null;
  if (state.focus === 0) state.rootSize = res.tree.size;
  if (state.hidden && state.focus === 0) {
    const h = state.hidden;
    const parts = h.parts.map((p) => ({ id: null, name: p.name, size: p.bytes, files: 0, dir: false, group: 0, children: [] }));
    if (h.other) parts.push({ id: null, name: "Snapshots, purgeable space and unreadable folders", size: h.other, files: 0, dir: false, group: 0, children: [] });
    res.tree.children.push({ id: null, hidden: true, name: "Hidden space", size: h.bytes, files: 0, dir: false, group: 0, children: parts });
    res.tree.size += h.bytes;
  }
  state.tree = prepare(res.tree);
  state.byId = new Map();
  const index = (n) => {
    if (n.id != null) state.byId.set(n.id, n);
    (n.children || []).forEach(index);
  };
  index(state.tree);
  chart.set(state.tree, state.mode, animate);
  renderCrumbs();
  renderHub();
  renderList();
  renderStatus();
  renderCollector();
  $("empty").hidden = state.tree.size > 0 || state.progress.scanning;
  $("sankey-note").hidden = state.mode !== "sankey" || !state.tree.size;
  $("rescan").textContent = state.progress.scanning ? "Stop" : "Rescan";
}

async function loadAnalysis() {
  if (state.scanning) return;
  const focus = state.focus;
  try {
    const [largest, types] = await Promise.all([call("largest", { focus, limit: 200 }), call("types", { focus })]);
    if (focus !== state.focus) return;
    state.largest = largest;
    state.types = types;
    renderList();
  } catch (e) {
    fail(e);
  }
}

function setFocus(id) {
  if (id == null || id === state.focus) return;
  state.focus = id;
  state.search = "";
  $("search").value = "";
  state.searchRows = [];
  state.largest = [];
  state.types = [];
  refresh(true).then(loadAnalysis);
}

function goUp() {
  const crumbs = state.view?.crumbs || [];
  if (crumbs.length > 1) setFocus(crumbs[crumbs.length - 2].id);
}

function setMode(mode) {
  state.mode = mode;
  select("modes", "mode", mode);
  if (state.view) refresh(true);
}

async function closeScan() {
  clearTimeout(pollTimer);
  state.scanning = false;
  try {
    await call("close_scan");
  } catch {}
  showStart();
}

// ---- Rendering ------------------------------------------------------------

function renderCrumbs() {
  const crumbs = state.view.crumbs;
  $("crumbs").innerHTML = crumbs.map((c) => `<li><button data-id="${c.id}">${escapeHtml(c.name)}</button></li>`).join("");
  for (const b of $("crumbs").querySelectorAll("button")) b.onclick = () => setFocus(Number(b.dataset.id));
  $("up").disabled = crumbs.length < 2;
}

function renderHub() {
  const hub = $("hub");
  hub.hidden = state.mode !== "sunburst" || !state.tree?.size;
  if (hub.hidden) return;
  const node = state.tree;
  hub.style.width = `${Math.round(chart.geo.hole * 1.5)}px`;
  $("hub-name").textContent = node.name;
  $("hub-size").textContent = fmtBytes(node.size);
  $("hub-files").textContent = plural(node.files, "file");
}

function rowHtml(r, total) {
  const node = state.byId.get(r.id);
  const swatch = node && node.depth === 1 ? colorFor(node, { dark: isDark() }) : "";
  const share = total > 0 ? Math.min(1, r.size / total) : 0;
  return `<div class="row" data-id="${r.id}" data-dir="${r.dir}" style="--share:${share.toFixed(4)};${swatch ? `--swatch:${swatch}` : ""}">
    <span class="swatch${r.dir ? "" : " file"}"></span>
    <div><div class="row-name${r.dir ? " dir" : ""}">${escapeHtml(r.name)}${r.inaccessible ? " (no access)" : ""}</div>${
      r.parent_path ? `<div class="row-sub">&lrm;${escapeHtml(r.parent_path)}</div>` : ""
    }</div>
    <span class="row-size">${fmtBytes(r.size)}</span>
  </div>`;
}

function renderList() {
  const list = $("list");
  const advisor = state.tab === "advisor";
  list.hidden = advisor;
  $("advisor").hidden = !advisor;
  $("search-wrap").hidden = advisor || state.tab === "types";
  $("search").disabled = state.scanning;
  $("search").placeholder = state.scanning ? "Search is ready when the scan finishes" : "Search in this folder";
  if (advisor) return renderAdvisor();
  if (!state.view) return;

  const total = state.tree.size;
  const waiting = `<p class="list-note">Ready when the scan finishes.</p>`;
  if (state.tab === "types") {
    if (state.scanning) return void (list.innerHTML = waiting);
    const top = state.types[0]?.bytes || 1;
    list.innerHTML =
      state.types
        .map(
          (b) => `<div class="type"><div class="type-head"><strong>${escapeHtml(b.category)}</strong>
          <span class="count">${plural(b.count, "file")}</span><span class="size">${fmtBytes(b.bytes)}</span></div>
          <div class="type-bar"><i style="width:${Math.max(1, (b.bytes / top) * 100)}%"></i></div></div>`,
        )
        .join("") || `<p class="list-note">No files here.</p>`;
    return;
  }

  let rows;
  let empty;
  if (state.search.length >= 2) {
    rows = state.searchRows;
    empty = "Nothing in this folder matches.";
  } else if (state.tab === "largest") {
    if (state.scanning) return void (list.innerHTML = waiting);
    rows = state.largest;
    empty = "No files here.";
  } else {
    rows = state.view.rows;
    empty = state.scanning ? "Scanning…" : "This folder is empty.";
  }
  let html = rows.map((r) => rowHtml(r, total)).join("");
  if (state.tab === "contents" && state.search.length < 2 && state.hidden && state.focus === 0) {
    const h = state.hidden;
    const what = [...h.parts.map((p) => p.name.toLowerCase()), h.other ? "snapshots, purgeable space and unreadable folders" : ""].filter(Boolean).join(", ");
    html += `<div class="row hidden-row" title="Used on this volume but not visible to the scan: ${escapeHtml(what)}" style="--share:${(h.bytes / total).toFixed(4)}">
      <span class="swatch"></span><div><div class="row-name">Hidden space</div><div class="row-sub">${escapeHtml(what)}</div></div>
      <span class="row-size">${fmtBytes(h.bytes)}</span></div>`;
  }
  list.innerHTML = html || `<p class="list-note">${empty}</p>`;
}

function renderStatus() {
  const el = $("status");
  const h = chart.hover;
  if (h) {
    const share = state.tree.size > 0 ? ((h.size / state.tree.size) * 100).toFixed(1) : "0";
    el.innerHTML = `<span class="swatch" style="background:${colorFor(h, { dark: isDark() })}"></span>
      <span class="name">${escapeHtml(h.name)}</span><strong>${fmtBytes(h.size)}</strong>
      ${h.dir ? `<span>${plural(h.files, "file")}</span>` : ""}<span>${share}% of this view</span>`;
    return;
  }
  const p = state.progress;
  if (!p) return void (el.textContent = "");
  if (p.scanning) {
    el.innerHTML = `<span class="pulse"></span><span>Scanning: <strong>${fmtCount(p.files)}</strong> files, <strong>${fmtBytes(p.bytes)}</strong> so far</span>`;
    return;
  }
  let html = `<span><strong>${fmtBytes(state.rootSize)}</strong> in ${fmtCount(p.files)} files, scanned in ${p.seconds.toFixed(1)} s</span>`;
  if (p.inaccessible > 0) {
    html +=
      state.platform === "macos"
        ? `<button id="privacy" title="Add FrisyDisk under Full Disk Access, then rescan">${plural(p.inaccessible, "folder")} could not be read</button>`
        : state.platform === "windows" && !state.elevated
          ? `<button id="elevate" title="Windows only lets administrators read some system folders">${plural(p.inaccessible, "folder")} could not be read: restart as administrator</button>`
          : `<span>${plural(p.inaccessible, "folder")} could not be read</span>`;
  }
  el.innerHTML = html;
  const elevate = $("elevate");
  if (elevate) elevate.onclick = () => relaunchAsAdmin();
  const privacy = $("privacy");
  if (privacy) privacy.onclick = () => call("open_privacy_settings").catch(fail);
}

function renderCollector() {
  const c = state.view.collector;
  $("collector").hidden = c.items.length === 0;
  $("collector-summary").textContent = `${plural(c.items.length, "item")}, ${fmtBytes(c.size)}`;
  $("collector-trash").textContent = `Move to ${trashName()}…`;
}

// ---- Hover, click, menu ---------------------------------------------------

function highlightRow(node) {
  for (const r of $("list").querySelectorAll(".row.hover")) r.classList.remove("hover");
  if (node?.id != null) $("list").querySelector(`.row[data-id="${node.id}"]`)?.classList.add("hover");
}

function chartPoint(ev) {
  const r = $("chart").getBoundingClientRect();
  return [ev.clientX - r.left, ev.clientY - r.top];
}

$("chart").addEventListener("mousemove", (ev) => {
  const node = chart.hit(...chartPoint(ev));
  const before = chart.hover;
  chart.setHover(node);
  $("chart").style.cursor = node === "center" ? (state.view?.crumbs.length > 1 ? "pointer" : "default") : node ? "pointer" : "default";
  if (chart.hover !== before) {
    renderStatus();
    highlightRow(chart.hover);
  }
});

$("chart").addEventListener("mouseleave", () => {
  chart.setHover(null);
  renderStatus();
  highlightRow(null);
});

$("chart").addEventListener("click", (ev) => {
  const node = chart.hit(...chartPoint(ev));
  if (node === "center") return goUp();
  if (!node) return;
  if (node.dir && node.id != null) setFocus(node.id);
  else if (node.parent && node.parent !== state.tree) setFocus(node.parent.id);
});

$("chart").addEventListener("contextmenu", (ev) => {
  ev.preventDefault();
  const node = chart.hit(...chartPoint(ev));
  if (node && node !== "center" && node.id != null) showMenu(ev.clientX, ev.clientY, node);
});

const listEl = $("list");
// Rows without an id (the hidden-space row) are information only.
const realRow = (ev) => {
  const row = ev.target.closest(".row");
  return row && row.dataset.id ? row : null;
};
listEl.addEventListener("mouseover", (ev) => {
  const row = realRow(ev);
  const node = row ? state.byId.get(Number(row.dataset.id)) : null;
  chart.setHover(node || null);
  renderStatus();
});
listEl.addEventListener("mouseleave", () => {
  chart.setHover(null);
  renderStatus();
});
listEl.addEventListener("click", (ev) => {
  const row = realRow(ev);
  if (row && row.dataset.dir === "true") setFocus(Number(row.dataset.id));
});
listEl.addEventListener("dblclick", (ev) => {
  const row = realRow(ev);
  if (row && row.dataset.dir !== "true") call("open", { id: Number(row.dataset.id) }).catch(fail);
});
listEl.addEventListener("contextmenu", (ev) => {
  const row = realRow(ev);
  if (!row) return;
  ev.preventDefault();
  showMenu(ev.clientX, ev.clientY, { id: Number(row.dataset.id), dir: row.dataset.dir === "true" });
});

function showMenu(x, y, node) {
  const menu = $("menu");
  const mac = state.platform === "macos";
  const items = [
    node.dir && ["Open here", () => setFocus(node.id)],
    [mac ? "Reveal in Finder" : "Show in file manager", () => call("reveal", { id: node.id }).catch(fail)],
    !node.dir && [mac ? "Quick Look" : "Open", () => call("open", { id: node.id }).catch(fail)],
    ["Copy path", () => copyPath(node.id)],
    "hr",
    ["Add to collector", () => stage(node.id), state.scanning || node.id === state.focus],
  ].filter(Boolean);
  menu.innerHTML = "";
  for (const item of items) {
    if (item === "hr") {
      menu.appendChild(document.createElement("hr"));
      continue;
    }
    const b = document.createElement("button");
    b.textContent = item[0];
    b.disabled = Boolean(item[2]);
    b.onclick = () => {
      menu.hidden = true;
      item[1]();
    };
    menu.appendChild(b);
  }
  menu.hidden = false;
  const r = menu.getBoundingClientRect();
  menu.style.left = `${Math.min(x, window.innerWidth - r.width - 8)}px`;
  menu.style.top = `${Math.min(y, window.innerHeight - r.height - 8)}px`;
}

async function copyPath(id) {
  try {
    const path = await call("path", { id });
    await navigator.clipboard.writeText(path);
    toast("Path copied");
  } catch (e) {
    fail(e);
  }
}

// ---- Collector ------------------------------------------------------------

async function stage(id) {
  try {
    await call("stage", { id });
    await refresh(false);
    loadAnalysis();
  } catch (e) {
    fail(e);
  }
}

async function unstage(id) {
  try {
    await call("unstage", id == null ? {} : { id });
    await refresh(false);
    loadAnalysis();
  } catch (e) {
    fail(e);
  }
}

$("collector-review").onclick = () => {
  const c = state.view.collector;
  const card = openModal(`<h2>Collector</h2><p>Nothing here has changed on disk yet.</p>
    ${c.items
      .map(
        (i) => `<div class="staged"><div><div>${escapeHtml(i.name)}</div><small>${escapeHtml(i.path)}</small></div>
        <span class="row-size">${fmtBytes(i.size)}</span><button data-id="${i.id}">Put back</button></div>`,
      )
      .join("")}
    <div class="modal-actions"><button id="put-all">Put everything back</button><button class="primary" id="modal-close">Done</button></div>`);
  for (const b of card.querySelectorAll("button[data-id]"))
    b.onclick = async () => {
      await unstage(Number(b.dataset.id));
      closeModal();
      if (state.view.collector.items.length) $("collector-review").click();
    };
  card.querySelector("#put-all").onclick = async () => {
    await unstage(null);
    closeModal();
  };
  card.querySelector("#modal-close").onclick = closeModal;
};

$("collector-trash").onclick = () => {
  const c = state.view.collector;
  const card = openModal(`<h2>Move ${plural(c.items.length, "item")} (${fmtBytes(c.size)}) to the ${trashName()}?</h2>
    <p>They go to the ${trashName()}, so you can still put them back from there. Space is freed only when you empty it.</p>
    <div class="modal-actions"><button id="modal-cancel">Cancel</button><button class="warn" id="modal-ok">Move to ${trashName()}</button></div>`);
  card.querySelector("#modal-cancel").onclick = closeModal;
  card.querySelector("#modal-cancel").focus();
  card.querySelector("#modal-ok").onclick = async () => {
    closeModal();
    try {
      const r = await call("trash_collector");
      await refresh(false);
      loadAnalysis();
      toast(
        r.failed.length
          ? `Moved ${r.moved}. Could not move: ${r.failed.join("; ")}`
          : `Moved ${plural(r.moved, "item")} (${fmtBytes(r.freed)}) to the ${trashName()}.`,
        r.failed.length ? 12000 : 5200,
      );
    } catch (e) {
      fail(e);
    }
  };
};

// ---- Advisor --------------------------------------------------------------

let advisorTimer;

/** Find what the advisor can talk to. With nothing usable, the Advisor tab stays hidden. */
async function refreshBackends() {
  try {
    const found = await call("backends");
    const keep = state.backend && found.find((b) => b.kind === state.backend.kind && b.model === state.backend.model);
    state.backends = found;
    state.backend = keep || found[0] || null;
  } catch (e) {
    state.backends = [];
    state.backend = null;
  }
  const tab = $("tabs").querySelector('[data-tab="advisor"]');
  tab.hidden = state.backends.length === 0;
  if (tab.hidden && state.tab === "advisor") setTab("contents");
  else if (state.tab === "advisor") renderAdvisor();
  return state.backends;
}

async function openSettings() {
  let cfg;
  try {
    cfg = await call("settings");
  } catch (e) {
    return fail(e);
  }
  const saved = (has) => (has ? "Saved. Leave blank to keep it, or type a new one." : "Not set");
  const remove = (has, field) => (has ? ` <a href="#" data-clear="${field}">Remove key</a>` : "");
  const actions = (kind) =>
    `<div class="provider-actions"><button data-save="${kind}">Save</button><button data-test="${kind}">Test</button><span class="provider-result" id="result-${kind}"></span></div>`;
  const card = openModal(`<h2>Advisor settings</h2>
    <p>The advisor needs a model to talk to. It appears only when one of these is available.</p>
    <label class="check"><input type="checkbox" id="set-enabled" ${cfg.advisor_enabled ? "checked" : ""}> Use the advisor</label>
    <div class="field"><label>GitHub Copilot</label>
      <label class="check"><input type="checkbox" id="set-copilot" ${cfg.copilot_enabled ? "checked" : ""}> Use Copilot through VS Code</label>
      <div class="field-pair"><input type="text" id="set-cp-url" value="${escapeHtml(cfg.copilot_url)}" placeholder="Bridge address (found automatically)" spellcheck="false">
      <input type="password" id="set-cp-token" placeholder="Bridge token" autocomplete="off"></div>
      <small>Found automatically while VS Code runs the agenticode extension. ${cfg.has_copilot_token ? "Token saved." : ""}${remove(cfg.has_copilot_token, "copilot_token")}</small>
      ${actions("copilot")}</div>
    <div class="field"><label for="set-ollama">Ollama address</label>
      <input type="text" id="set-ollama" value="${escapeHtml(cfg.ollama_host)}" spellcheck="false">
      <small>Runs on this computer by default. Point it at another machine to use an Ollama server on your network.</small>
      ${actions("ollama")}</div>
    <div class="field"><label>AgenticWork</label>
      <div class="field-pair"><input type="text" id="set-aw-url" value="${escapeHtml(cfg.agenticwork_url)}" placeholder="https://your-agenticwork-host" spellcheck="false">
      <input type="password" id="set-aw-key" placeholder="API key" autocomplete="off"></div>
      <small>${saved(cfg.has_agenticwork_key)}${remove(cfg.has_agenticwork_key, "agenticwork_key")}</small>
      ${actions("agenticwork")}</div>
    <div class="field"><label for="set-anthropic">Anthropic API key</label>
      <input type="password" id="set-anthropic" placeholder="sk-ant-…" autocomplete="off">
      <small>${saved(cfg.has_anthropic_key)}${remove(cfg.has_anthropic_key, "anthropic_key")}</small>
      ${actions("anthropic")}</div>
    <div class="field"><label for="set-openai">OpenAI API key</label>
      <input type="password" id="set-openai" placeholder="sk-…" autocomplete="off">
      <small>${saved(cfg.has_openai_key)}${remove(cfg.has_openai_key, "openai_key")}</small>
      ${actions("openai")}</div>
    <p class="note">With Copilot, AgenticWork, Anthropic, OpenAI or an Ollama server on another machine, the scan summary (folder and file names with sizes) is sent to that service when you ask for suggestions. Keys are stored on this computer in ${escapeHtml(cfg.file)}.</p>
    <div class="found" id="set-found"></div>
    <div class="modal-actions"><button id="set-cancel">Close</button></div>`);
  const found = card.querySelector("#set-found");
  const describe = (list) => {
    if (!card.querySelector("#set-enabled").checked) return "The advisor is off.";
    if (!list.length) return "Nothing to talk to yet, so the Advisor tab stays hidden.";
    const by = {};
    for (const b of list) by[b.kind] = (by[b.kind] || 0) + 1;
    const names = { copilot: "Copilot", ollama: "Ollama", agenticwork: "AgenticWork", anthropic: "Anthropic", openai: "OpenAI" };
    return "Ready: " + Object.entries(by).map(([k, n]) => `${names[k] || k} (${plural(n, "model")})`).join(", ");
  };
  found.textContent = state.backends ? describe(state.backends) : "";
  const clear = {};
  for (const a of card.querySelectorAll("a[data-clear]"))
    a.onclick = (ev) => {
      ev.preventDefault();
      clear[a.dataset.clear] = "";
      a.parentElement.textContent = "Will be removed when you save.";
    };
  // Each provider's fields: [setting, input id, is a key]. A blank key field means "keep what is saved".
  const fields = {
    copilot: [["copilot_url", "set-cp-url"], ["copilot_token", "set-cp-token", true]],
    ollama: [["ollama_host", "set-ollama"]],
    agenticwork: [["agenticwork_url", "set-aw-url"], ["agenticwork_key", "set-aw-key", true]],
    anthropic: [["anthropic_key", "set-anthropic", true]],
    openai: [["openai_key", "set-openai", true]],
  };
  const changeFor = (kind) => {
    const change = { advisor_enabled: card.querySelector("#set-enabled").checked };
    if (kind === "copilot") change.copilot_enabled = card.querySelector("#set-copilot").checked;
    for (const [field, id, key] of fields[kind]) {
      const v = card.querySelector("#" + id).value.trim();
      if (field in clear) change[field] = "";
      if (v || !key) change[field] = v;
    }
    return change;
  };
  card.querySelector("#set-cancel").onclick = closeModal;
  for (const btn of card.querySelectorAll("[data-save]")) {
    const kind = btn.dataset.save;
    btn.onclick = async () => {
      const result = card.querySelector("#result-" + kind);
      result.textContent = "Saving…";
      try {
        await call("set_settings", changeFor(kind));
        for (const [field, id, key] of fields[kind]) {
          if (key) card.querySelector("#" + id).value = "";
          delete clear[field];
        }
        result.textContent = "Saved.";
        found.textContent = describe(await refreshBackends());
      } catch (e) {
        result.textContent = e.message;
      }
    };
  }
  for (const btn of card.querySelectorAll("[data-test]")) {
    const kind = btn.dataset.test;
    btn.onclick = async () => {
      const result = card.querySelector("#result-" + kind);
      result.textContent = "Testing…";
      btn.disabled = true;
      try {
        const r = await call("test_provider", { kind, settings: changeFor(kind) });
        result.textContent = r.ok ? `Works: ${r.model} answered in ${r.ms} ms (${plural(r.models, "model")} found).` : `Failed: ${r.error}`;
      } catch (e) {
        result.textContent = `Failed: ${e.message}`;
      } finally {
        btn.disabled = false;
      }
    };
  }
}

function renderAdvisor() {
  const a = state.advisor;
  const sel = $("backend");
  if (state.backends) {
    sel.innerHTML = state.backends.length
      ? state.backends.map((b, i) => `<option value="${i}">${escapeHtml(b.label)}</option>`).join("")
      : `<option>No model available</option>`;
    if (state.backend) sel.value = String(state.backends.indexOf(state.backend));
  } else {
    sel.innerHTML = `<option>Looking for local models…</option>`;
  }
  sel.disabled = a.running || !state.backends?.length;
  $("advisor-stop").hidden = !a.running;
  $("advisor-reset").hidden = a.running || !a.started;
  $("advisor-form").hidden = !a.started;
  $("advisor-form").querySelector("button").disabled = a.running;

  const body = $("advisor-body");
  if (!a.started && !a.error) {
    const b = state.backend;
    const where = !b
      ? ""
      : b.remote
        ? `<li><strong>This leaves your computer.</strong> Folder and file names with their sizes are sent to ${escapeHtml(b.label.split(":")[0])}.</li>`
        : `<li>The model runs on this computer. Nothing is sent anywhere else.</li>`;
    body.innerHTML = `<div class="advisor-intro"><h2>Storage advisor</h2>
      <p>Sends this scan and a summary of your volumes, network shares and system disk to the model you pick above, and asks for ways to speed up I/O and make better use of the storage you already have.</p>
      <ul><li>Suggestions only. Nothing is moved, changed or deleted.</li>${where}</ul>
      <div class="io-test">
        <button id="io-run">Test I/O speed</button>
        <label class="io-write"><input type="checkbox" id="io-write"> include write test</label>
        <span id="io-result" class="io-result"></span>
      </div>
      <p><button class="primary" id="advisor-ask" ${state.scanning || !b ? "disabled" : ""}>${
        state.scanning ? "Ready when the scan finishes" : "Ask for suggestions"
      }</button></p></div>`;
    const ask = $("advisor-ask");
    if (ask) ask.onclick = () => askAdvisor(null);
    const io = $("io-run");
    if (io) io.onclick = () => runIoTest();
    return;
  }
  const pinned = body.scrollHeight - body.scrollTop - body.clientHeight < 60;
  let html = a.messages.map((m) => (m.role === "user" ? `<div class="question">${escapeHtml(m.text)}</div>` : markdown(m.text))).join("");
  if (a.running && !(a.messages.at(-1)?.role === "assistant" && a.messages.at(-1).text))
    html += `<p class="note"><span class="pulse" style="display:inline-block"></span> Collecting facts and waiting for the model…</p>`;
  if (a.error) html += `<div class="risky">${escapeHtml(a.error)}</div>`;
  if (a.risky.length)
    html += `<div class="risky"><strong>The model ignored the non-destructive rule in ${plural(a.risky.length, "line")}. Skip these:</strong>${a.risky
      .map((l) => `<code>${escapeHtml(l)}</code>`)
      .join("")}</div>`;
  if (!a.running && a.messages.length) html += `<p class="note">Written by a model. FrisyDisk ran none of this. Read each command before you run it.</p>`;
  body.innerHTML = html;
  if (pinned && a.running) body.scrollTop = body.scrollHeight;
}

async function askAdvisor(followUp) {
  if (!state.backend) return;
  try {
    await call("advisor_ask", { backend: state.backend, follow_up: followUp, focus: state.focus });
    state.advisor.running = true;
    state.advisor.started = true;
    pollAdvisor();
  } catch (e) {
    fail(e);
  }
}

async function runIoTest() {
  const btn = $("io-run");
  const out = $("io-result");
  const write = $("io-write")?.checked ?? false;
  if (!btn || !out) return;
  btn.disabled = true;
  out.textContent = write ? "Testing read and write…" : "Testing…";
  try {
    const home = await call("home");
    const r = await call("benchmark", { path: home, write });
    const parts = [`read ${Math.round(r.read_mbps)} MB/s`];
    if (r.write_mbps != null) parts.push(`write ${Math.round(r.write_mbps)} MB/s`);
    parts.push(`${Math.round(r.random_iops)} IOPS`, `${Math.round(r.random_latency_us)} µs latency`);
    out.textContent = parts.join(" · ") + " — included in the next question.";
  } catch (e) {
    out.textContent = `Failed: ${e.message}`;
  } finally {
    btn.disabled = false;
  }
}

async function pollAdvisor() {
  clearTimeout(advisorTimer);
  try {
    state.advisor = await call("advisor_poll");
  } catch (e) {
    return fail(e);
  }
  if (state.tab === "advisor") renderAdvisor();
  if (state.advisor.running) advisorTimer = setTimeout(pollAdvisor, 350);
  else window.dispatchEvent(new Event("frisy-advisor-done"));
}

$("backend").onchange = (ev) => {
  state.backend = state.backends[Number(ev.target.value)] || null;
  renderAdvisor();
};
$("settings-start").onclick = openSettings;
$("clean-start").onclick = openClean;
$("clean-top").onclick = openClean;

// ---- Cleanup ----------------------------------------------------------------

async function relaunchAsAdmin() {
  try {
    await T.core.invoke("relaunch_as_admin");
  } catch (e) {
    fail(e);
  }
}

async function openClean() {
  const card = openModal(`<h2>Clean up</h2><p>Measuring temporary files, caches and logs…</p>`);
  let r;
  try {
    r = await call("clean_scan", { stale_days: 90 });
  } catch (e) {
    card.innerHTML = `<h2>Clean up</h2><p>${escapeHtml(e.message)}</p><div class="modal-actions"><button class="primary" id="clean-close">Close</button></div>`;
    card.querySelector("#clean-close").onclick = closeModal;
    return;
  }
  const where = trashName();
  const total = r.targets.reduce((s, t) => s + t.bytes, 0);
  const admin = r.targets.some((t) => t.needs_admin);
  card.innerHTML = `<h2>Clean up ${fmtBytes(total)}</h2>
    <p>Apps rebuild caches and temporary files when they need them. Anything changed in the last hour (temporary files: the last day) is left alone, and nothing outside these folders is touched.</p>
    <div class="clean-list">${
      r.targets
        .map(
          (t) => `<label class="clean-item"><input type="checkbox" data-id="${escapeHtml(t.id)}" ${t.group === "dev" || t.group === "trash" ? "" : "checked"}>
          <span><strong>${escapeHtml(t.label)}</strong> <span class="row-size">${fmtBytes(t.bytes)}, ${plural(t.count, "item")}</span>
          ${t.delete_only ? `<em class="warn-text">deleted permanently</em>` : ""}${t.needs_admin ? `<em class="warn-text">partly needs administrator</em>` : ""}
          <small>${escapeHtml(t.detail)}</small>${t.examples[0] ? `<small class="example">${escapeHtml(t.examples[0])}</small>` : ""}</span>
          </label>`,
        )
        .join("") || `<p class="list-note">Nothing to clean right now.</p>`
    }</div>
    ${!r.scan_open ? `<p class="note">Scan your home folder first to also find build folders (node_modules, target) in projects untouched for 90 days.</p>` : ""}
    ${admin && state.platform === "windows" ? `<p class="note"><button id="clean-elevate">Restart as administrator</button> to include system temp files and Windows Update downloads.</p>` : ""}
    <p class="note" id="clean-sum"></p>
    <div class="modal-actions"><button id="clean-cancel">Cancel</button>
      <button id="clean-trash">Move to ${where}</button><button class="warn" id="clean-delete">Delete permanently</button></div>`;
  const boxes = [...card.querySelectorAll("input[type=checkbox]")];
  const picked = () => r.targets.filter((t) => boxes.find((b) => b.dataset.id === t.id)?.checked);
  const update = () => {
    const p = picked();
    const size = p.reduce((s, t) => s + t.bytes, 0);
    card.querySelector("#clean-sum").textContent = p.length
      ? `${p.length === 1 ? "1 category" : `${p.length} categories`} selected, ${fmtBytes(size)}. Moving to the ${where} frees the space only when you empty it; deleting frees it now.`
      : "Select what to clean.";
    card.querySelector("#clean-trash").disabled = !p.length || p.every((t) => t.delete_only);
    card.querySelector("#clean-delete").disabled = !p.length;
  };
  boxes.forEach((b) => (b.onchange = update));
  const wire = () => {
    card.querySelector("#clean-cancel").onclick = closeModal;
    card.querySelector("#clean-trash").onclick = () => run("trash");
    card.querySelector("#clean-delete").onclick = askDelete;
  };
  const elevate = card.querySelector("#clean-elevate");
  if (elevate) elevate.onclick = relaunchAsAdmin;
  const run = async (mode) => {
    const ids = picked().map((t) => t.id);
    card.querySelector(".modal-actions").innerHTML = `<span class="note">Cleaning…</span>`;
    try {
      const rep = await call("clean_run", { ids, mode });
      closeModal();
      const verb = mode === "trash" ? `Moved ${plural(rep.removed, "item")} (${fmtBytes(rep.freed)}) to the ${where}` : `Deleted ${plural(rep.removed, "item")} and freed ${fmtBytes(rep.freed)}`;
      toast(`${verb}.${rep.skipped ? ` Skipped ${fmtCount(rep.skipped)} in use or changed.` : ""}${state.view ? " Rescan to update the chart." : ""}`, 9000);
      if (state.platform && !$("start").hidden) showStart();
    } catch (e) {
      closeModal();
      fail(e);
    }
  };
  // Permanent deletion always takes a second, explicit click.
  function askDelete() {
    const p = picked();
    const size = p.reduce((s, t) => s + t.bytes, 0);
    const n = p.reduce((s, t) => s + t.count, 0);
    const actions = card.querySelector(".modal-actions");
    actions.innerHTML = `<span class="note warn-text">Delete ${plural(n, "item")} (${fmtBytes(size)}) for good? This cannot be undone.</span>
      <button id="clean-back">Back</button><button class="warn" id="clean-confirm">Delete for good</button>`;
    boxes.forEach((b) => (b.disabled = true));
    actions.querySelector("#clean-back").focus();
    actions.querySelector("#clean-back").onclick = () => {
      boxes.forEach((b) => (b.disabled = false));
      actions.innerHTML = `<button id="clean-cancel">Cancel</button><button id="clean-trash">Move to ${where}</button><button class="warn" id="clean-delete">Delete permanently</button>`;
      wire();
      update();
    };
    actions.querySelector("#clean-confirm").onclick = () => run("delete");
  }
  wire();
  update();
}
$("settings-top").onclick = openSettings;
$("advisor-stop").onclick = () => call("advisor_stop").catch(fail);
$("advisor-reset").onclick = async () => {
  await call("advisor_reset").catch(fail);
  state.advisor = { messages: [], running: false, error: null, risky: [], started: false };
  renderAdvisor();
};
$("advisor-form").onsubmit = (ev) => {
  ev.preventDefault();
  const q = $("advisor-input").value.trim();
  if (!q || state.advisor.running) return;
  $("advisor-input").value = "";
  askAdvisor(q);
};

// ---- Tabs, search, top bar, keys -----------------------------------------

function setTab(tab) {
  state.tab = tab;
  select("tabs", "tab", tab);
  renderList();
}

for (const b of $("tabs").querySelectorAll("button")) b.onclick = () => setTab(b.dataset.tab);
for (const b of $("modes").querySelectorAll("button")) b.onclick = () => setMode(b.dataset.mode);

let searchTimer;
$("search").addEventListener("input", (ev) => {
  state.search = ev.target.value.trim();
  clearTimeout(searchTimer);
  if (state.search.length < 2) {
    state.searchRows = [];
    return renderList();
  }
  const query = state.search;
  searchTimer = setTimeout(async () => {
    try {
      const rows = await call("search", { focus: state.focus, query });
      if (query === state.search) {
        state.searchRows = rows;
        renderList();
      }
    } catch (e) {
      fail(e);
    }
  }, 180);
});

$("back").onclick = closeScan;
$("up").onclick = goUp;
$("rescan").onclick = async () => {
  if (state.progress?.scanning) return call("stop_scan").catch(fail);
  const root = state.view?.crumbs[0];
  if (root) startScan(await call("path", { id: root.id }));
};

$("scan-home").onclick = async () => startScan(await call("home"));
$("browse").onclick = async () => {
  try {
    const picked = await T.dialog.open({ directory: true, multiple: false, title: "Choose a folder to scan" });
    if (picked) startScan(picked);
  } catch (e) {
    fail(e);
  }
};
$("path-form").onsubmit = (ev) => {
  ev.preventDefault();
  const path = $("path-input").value.trim();
  if (path) startScan(path);
};

document.addEventListener("mousedown", (ev) => {
  if (!ev.target.closest("#menu")) $("menu").hidden = true;
});
$("modal").addEventListener("mousedown", (ev) => {
  if (ev.target === $("modal")) closeModal();
});
document.addEventListener("keydown", (ev) => {
  if (ev.key === "Escape") {
    $("menu").hidden = true;
    closeModal();
    return;
  }
  if ($("scan").hidden || ev.target.matches("input, select, textarea")) return;
  if ((ev.metaKey || ev.altKey) && ev.key === "ArrowUp") goUp();
  else if (ev.key === "1") setMode("sunburst");
  else if (ev.key === "2") setMode("treemap");
  else if (ev.key === "3") setMode("sankey");
});
document.addEventListener("contextmenu", (ev) => {
  if (!ev.target.closest("#chart, #list, input, .advisor-body")) ev.preventDefault();
});

new ResizeObserver(() => {
  if (!state.tree || $("scan").hidden) return;
  chart.layout();
  chart.draw();
  renderHub();
}).observe($("chart").parentElement);
window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => state.view && refresh(false));

// ---- Start-up -------------------------------------------------------------

async function init() {
  try {
    state.platform = await call("platform");
    state.elevated = await call("elevated");
  } catch {}

  // Scripted runs: launch arguments in the app, query parameters in a browser.
  let opts = Object.fromEntries(new URLSearchParams(location.search));
  if (T) {
    try {
      const args = await T.core.invoke("launch_args");
      for (let i = 0; i < args.length; i++) if (args[i].startsWith("--")) opts[args[i].slice(2)] = args[i + 1] ?? "";
    } catch {}
    T.webview?.getCurrentWebview?.().onDragDropEvent?.((ev) => {
      if (ev.payload.type === "drop" && ev.payload.paths?.length) startScan(ev.payload.paths[0]);
    });
  }
  const ready = refreshBackends();
  if (["sunburst", "treemap", "sankey"].includes(opts.mode)) state.mode = opts.mode;
  if (["contents", "largest", "types"].includes(opts.tab)) state.tab = opts.tab;
  if (opts.tab === "advisor" && (await ready).length) state.tab = "advisor";
  if (opts.scan) {
    if ("advise" in opts) {
      window.addEventListener(
        "frisy-scan-done",
        async () => {
          if (!(state.backends || (await refreshBackends())).length) return;
          setTab("advisor");
          askAdvisor(null);
        },
        { once: true },
      );
    }
    await startScan(opts.scan);
    if ($("scan").hidden) showStart();
  } else {
    showStart();
  }
}

init();
