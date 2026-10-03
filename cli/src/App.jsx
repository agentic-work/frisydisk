import { Box, Text, useApp, useInput, useStdout } from "ink";
import TextInput from "ink-text-input";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { bar, bytes, count, fit, hue, plural } from "./format.js";

const TABS = ["contents", "largest", "types"];
const TAB_NAMES = { contents: "Contents", largest: "Largest files", types: "File types" };
const FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

function useSize() {
  const { stdout } = useStdout();
  const [size, setSize] = useState({ cols: stdout.columns || 100, rows: stdout.rows || 30 });
  useEffect(() => {
    const on = () => setSize({ cols: stdout.columns || 100, rows: stdout.rows || 30 });
    stdout.on("resize", on);
    return () => stdout.off("resize", on);
  }, [stdout]);
  return size;
}

function useSpinner(active) {
  const [i, setI] = useState(0);
  useEffect(() => {
    if (!active) return;
    const t = setInterval(() => setI((x) => (x + 1) % FRAMES.length), 90);
    return () => clearInterval(t);
  }, [active]);
  return FRAMES[i];
}

/** Keep the cursor inside a scrolling window of `height` rows. */
function windowFor(cursor, height, total) {
  const top = Math.max(0, Math.min(cursor - Math.floor(height / 2), total - height));
  return [top, Math.min(total, top + height)];
}

export default function App({ rpc, path, startInClean }) {
  const { exit } = useApp();
  const { cols, rows } = useSize();
  const [screen, setScreen] = useState(startInClean ? "clean" : "browse");
  const [platform, setPlatform] = useState("");
  const [message, setMessage] = useState(null);
  const quit = useCallback(() => {
    rpc.close();
    exit();
  }, [rpc, exit]);

  useEffect(() => {
    rpc.call("platform").then(setPlatform, () => {});
  }, [rpc]);

  const height = Math.max(5, rows - 6);
  const common = { rpc, cols, height, platform, setMessage, quit };
  return (
    <Box flexDirection="column" width={cols}>
      {screen === "browse" ? (
        <Browse {...common} path={path} openClean={() => setScreen("clean")} />
      ) : (
        <Clean {...common} back={startInClean ? quit : () => setScreen("browse")} />
      )}
      {message ? (
        <Text color={message.error ? "red" : "green"} wrap="truncate-end">
          {message.text}
        </Text>
      ) : (
        <Text> </Text>
      )}
    </Box>
  );
}

// ---- Browsing a scan ------------------------------------------------------

function Browse({ rpc, path, cols, height, platform, setMessage, quit, openClean }) {
  const [focus, setFocus] = useState(0);
  const [view, setView] = useState(null);
  const [tab, setTab] = useState("contents");
  const [largest, setLargest] = useState([]);
  const [types, setTypes] = useState([]);
  const [cursor, setCursor] = useState(0);
  const [marked, setMarked] = useState(new Set());
  const [searching, setSearching] = useState(false);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState(null);
  const [confirm, setConfirm] = useState(null);
  const [generation, setGeneration] = useState(0);
  const [started, setStarted] = useState(0);
  const scanning = !view || view.progress.scanning;
  const spin = useSpinner(scanning);
  const timer = useRef(null);

  // Start (or restart) the scan.
  useEffect(() => {
    let live = true;
    rpc.call("start_scan", { path }).then(
      () => live && setStarted((n) => n + 1),
      (e) => {
        setMessage({ error: true, text: e.message });
        setTimeout(quit, 50);
      },
    );
    return () => {
      live = false;
    };
  }, [rpc, path, generation]);

  // Poll while scanning; load once when the focus changes after that.
  useEffect(() => {
    if (!started) return;
    let live = true;
    const load = async () => {
      try {
        const v = await rpc.call("view", { focus, depth: 1, min_fraction: 0, rows: 5000 });
        if (!live) return;
        setView(v);
        if (v.progress.scanning) timer.current = setTimeout(load, 350);
      } catch (e) {
        if (!live) return;
        if (focus !== 0) setFocus(0);
        else setMessage({ error: true, text: e.message });
      }
    };
    load();
    return () => {
      live = false;
      clearTimeout(timer.current);
    };
  }, [rpc, focus, started]);

  // Analysis for the other tabs once the scan is done.
  useEffect(() => {
    if (scanning) return;
    rpc.call("largest", { focus, limit: 300 }).then(setLargest, () => {});
    rpc.call("types", { focus }).then(setTypes, () => {});
  }, [rpc, focus, scanning, view?.tree?.size]);

  const total = view?.tree?.size || 0;
  const hidden = focus === 0 && view?.hidden ? view.hidden : null;

  // The rows on screen for the current tab.
  const list = useMemo(() => {
    if (!view) return [];
    if (results) return results.map((r) => ({ ...r, sub: r.parent_path }));
    if (tab === "largest") return largest.map((r) => ({ ...r, sub: r.parent_path }));
    if (tab === "types") return types.map((t) => ({ type: true, name: t.category, size: t.bytes, files: t.count }));
    let cursorShare = 0;
    const out = view.rows.map((r) => {
      const share = total > 0 ? r.size / total : 0;
      const row = { ...r, hue: cursorShare + share / 2 };
      cursorShare += share;
      return row;
    });
    if (hidden) out.push({ hiddenRow: true, name: "Hidden space", size: hidden.bytes });
    return out;
  }, [view, tab, largest, types, results, total, hidden]);

  useEffect(() => setCursor((c) => Math.min(c, Math.max(0, list.length - 1))), [list.length]);
  const current = list[cursor];

  const runSearch = async (q) => {
    setSearching(false);
    if (q.trim().length < 2) {
      setResults(null);
      return;
    }
    try {
      setResults(await rpc.call("search", { focus, query: q.trim() }));
      setCursor(0);
    } catch (e) {
      setMessage({ error: true, text: e.message });
    }
  };

  const trashTargets = () => {
    const ids = marked.size ? [...marked] : current && current.id != null && !current.type ? [current.id] : [];
    const items = ids.map((id) => list.find((r) => r.id === id) || view.rows.find((r) => r.id === id)).filter(Boolean);
    return items;
  };

  const doTrash = async (items) => {
    setConfirm(null);
    try {
      for (const it of items) await rpc.call("stage", { id: it.id });
      const r = await rpc.call("trash_collector");
      setMarked(new Set());
      const where = platform === "windows" ? "Recycle Bin" : "Trash";
      setMessage(
        r.failed.length
          ? { error: true, text: `Moved ${r.moved}; could not move ${r.failed.length}: ${r.failed[0]}` }
          : { text: `Moved ${plural(r.moved, "item")} (${bytes(r.freed)}) to the ${where}.` },
      );
      const v = await rpc.call("view", { focus, depth: 1, min_fraction: 0, rows: 5000 });
      setView(v);
    } catch (e) {
      setMessage({ error: true, text: e.message });
    }
  };

  useInput((input, key) => {
    if (searching) {
      if (key.escape) setSearching(false);
      return;
    }
    if (confirm) {
      if (input === "y") doTrash(confirm.items);
      else setConfirm(null);
      return;
    }
    setMessage(null);
    const move = (to) => setCursor(Math.max(0, Math.min(list.length - 1, to)));
    if (input === "q" || (key.ctrl && input === "c")) return quit();
    if (key.upArrow || input === "k") return move(cursor - 1);
    if (key.downArrow || input === "j") return move(cursor + 1);
    if (key.pageUp) return move(cursor - height);
    if (key.pageDown) return move(cursor + height);
    if (input === "g") return move(0);
    if (input === "G") return move(list.length - 1);
    if (key.tab) {
      setResults(null);
      setTab(TABS[(TABS.indexOf(tab) + (key.shift ? TABS.length - 1 : 1)) % TABS.length]);
      return setCursor(0);
    }
    if (input === "/") return setSearching(true);
    if (key.escape && results) return setResults(null);
    if (key.return || key.rightArrow || input === "l") {
      if (current?.dir && current.id != null) {
        setResults(null);
        setTab("contents");
        setFocus(current.id);
        setCursor(0);
      }
      return;
    }
    if (key.leftArrow || key.backspace || key.delete || input === "h") {
      const crumbs = view?.crumbs || [];
      if (crumbs.length > 1) {
        setFocus(crumbs[crumbs.length - 2].id);
        setTab("contents");
        setResults(null);
        setCursor(0);
      }
      return;
    }
    if (input === " " && current?.id != null && !current.type) {
      const next = new Set(marked);
      next.has(current.id) ? next.delete(current.id) : next.add(current.id);
      setMarked(next);
      return move(cursor + 1);
    }
    if (input === "x") {
      if (scanning) return setMessage({ error: true, text: "Wait for the scan to finish." });
      const items = trashTargets();
      if (items.length) setConfirm({ items });
      return;
    }
    if (input === "o" && current?.id != null) {
      rpc.call("reveal", { id: current.id }).catch((e) => setMessage({ error: true, text: e.message }));
      return;
    }
    if (input === "c") return openClean();
    if (input === "r") {
      setView(null);
      setFocus(0);
      setGeneration((g) => g + 1);
    }
  });

  // ---- Drawing ----
  const crumbs = view?.crumbs?.map((c) => c.name) || [path];
  const where = fit(crumbs.join(" › "), Math.max(10, cols - 44));
  const p = view?.progress;
  const summary = !p
    ? "starting…"
    : p.scanning
      ? `${spin} ${count(p.files)} files, ${bytes(p.bytes)} so far`
      : `${bytes(total)} · ${plural(view.tree.files, "file")} · ${p.seconds.toFixed(1)} s`;

  return (
    <Box flexDirection="column">
      <Box justifyContent="space-between">
        <Text>
          <Text bold color="#ffc83d">
            FrisyDisk{" "}
          </Text>
          <Text bold>{where}</Text>
        </Text>
        <Text dimColor={!p?.scanning} color={p?.scanning ? "#ffc83d" : undefined}>
          {summary}
        </Text>
      </Box>
      <Strip rows={tab === "contents" && !results ? list : []} total={total + (hidden ? 0 : 0)} width={cols} />
      <Box>
        {TABS.map((t) => (
          <Text key={t} inverse={t === tab && !results} color={t === tab && !results ? "#ffc83d" : undefined} dimColor={t !== tab || !!results}>
            {` ${TAB_NAMES[t]} `}
          </Text>
        ))}
        {results ? <Text color="#7cc4ff">{`  Search: ${query} (${results.length})`}</Text> : null}
        {marked.size ? <Text color="#ff8f6b">{`  ${marked.size} marked`}</Text> : null}
      </Box>
      <List rows={list} cursor={cursor} height={height} cols={cols} total={total} marked={marked} tab={results ? "search" : tab} scanning={scanning} />
      <Footer
        cols={cols}
        current={current}
        hidden={hidden}
        searching={searching}
        query={query}
        setQuery={setQuery}
        runSearch={runSearch}
        confirm={confirm}
        platform={platform}
        inaccessible={p && !p.scanning ? p.inaccessible : 0}
      />
    </Box>
  );
}

/** One line showing how the folder splits, like a flattened treemap. */
function Strip({ rows, total, width }) {
  if (!rows.length || !total) return <Text dimColor>{"─".repeat(width)}</Text>;
  const cells = [];
  let used = 0;
  for (const r of rows) {
    const n = Math.round((r.size / total) * width);
    if (n < 1) break;
    cells.push(
      <Text key={r.id ?? r.name} color={r.hiddenRow ? "#5b6b78" : hue(r.hue ?? 0)}>
        {"█".repeat(Math.min(n, width - used))}
      </Text>,
    );
    used += n;
    if (used >= width) break;
  }
  if (used < width) cells.push(<Text key="rest" color="#3a4a58">{"█".repeat(width - used)}</Text>);
  return <Text>{cells}</Text>;
}

function List({ rows, cursor, height, cols, total, marked, tab, scanning }) {
  if (!rows.length) {
    const why = scanning ? "Scanning…" : tab === "search" ? "Nothing here matches." : "Nothing here.";
    return (
      <Box height={height}>
        <Text dimColor>{`  ${why}`}</Text>
      </Box>
    );
  }
  const [from, to] = windowFor(cursor, height, rows.length);
  const topSize = Math.max(...rows.map((r) => r.size), 1);
  const barW = Math.max(8, Math.min(24, Math.floor(cols / 6)));
  const nameW = Math.max(10, cols - barW - 26);
  const lines = [];
  for (let i = from; i < to; i++) {
    const r = rows[i];
    const on = i === cursor;
    const share = r.type ? r.size / topSize : total ? r.size / total : 0;
    const color = r.hiddenRow ? "#7b8b98" : r.type ? "#ffc83d" : r.hue != null ? hue(r.hue) : "#7cc4ff";
    const name = r.type
      ? `${r.name}  ${plural(r.files, "file")}`
      : r.hiddenRow
        ? "Hidden space (system, snapshots, swap, unreadable)"
        : r.name + (r.dir ? "/" : "") + (r.inaccessible ? "  (no access)" : "");
    const sub = r.sub ? `  ${r.sub}` : "";
    lines.push(
      <Box key={`${r.id ?? r.name}-${i}`}>
        <Text color={on ? "#ffc83d" : undefined}>{on ? "❯" : " "}</Text>
        <Text color="#ff8f6b">{marked.has(r.id) ? "●" : " "}</Text>
        <Text color={color}>{bar(share, barW)}</Text>
        <Text dimColor>{` ${(share * 100).toFixed(share >= 0.1 ? 0 : 1).padStart(4)}%`}</Text>
        <Text bold={on}>{` ${bytes(r.size).padStart(9)}  `}</Text>
        <Text bold={r.dir || on} italic={r.hiddenRow} color={r.dir ? color : undefined} inverse={on && false} wrap="truncate-end">
          {fit(name, nameW)}
          <Text dimColor>{fit(sub, Math.max(0, nameW - name.length))}</Text>
        </Text>
      </Box>,
    );
  }
  return (
    <Box flexDirection="column" height={height}>
      {lines}
    </Box>
  );
}

function Footer({ cols, current, hidden, searching, query, setQuery, runSearch, confirm, platform, inaccessible }) {
  if (searching) {
    return (
      <Box>
        <Text color="#7cc4ff">Search: </Text>
        <TextInput value={query} onChange={setQuery} onSubmit={runSearch} placeholder="name contains…" />
      </Box>
    );
  }
  if (confirm) {
    const n = confirm.items.length;
    const size = confirm.items.reduce((s, i) => s + i.size, 0);
    const where = platform === "windows" ? "Recycle Bin" : "Trash";
    return <Text color="#ff8f6b" bold>{`Move ${plural(n, "item")} (${bytes(size)}) to the ${where}?  y = yes, any other key = no`}</Text>;
  }
  let info = "";
  if (current?.hiddenRow && hidden) {
    const parts = hidden.parts.map((p) => `${p.name} ${bytes(p.bytes)}`);
    if (hidden.other) parts.push(`snapshots, purgeable and unreadable ${bytes(hidden.other)}`);
    info = `Hidden: ${parts.join(", ")}`;
  } else if (inaccessible) {
    info = `${plural(inaccessible, "folder")} could not be read. ${
      platform === "windows" ? "Run PowerShell as Administrator" : platform === "macos" ? "Give your terminal Full Disk Access or use sudo" : "Use sudo"
    } to see everything.`;
  }
  const keys = "↑↓ move  ⏎ open  ⌫ up  tab view  / search  space mark  x trash  o reveal  c clean  r rescan  q quit";
  return (
    <Box flexDirection="column">
      <Text dimColor wrap="truncate-end">
        {fit(info, cols) || " "}
      </Text>
      <Text dimColor wrap="truncate-end">
        {fit(keys, cols)}
      </Text>
    </Box>
  );
}

// ---- Cleanup --------------------------------------------------------------

function Clean({ rpc, cols, height, platform, setMessage, back }) {
  const [state, setState] = useState({ loading: true });
  const [cursor, setCursor] = useState(0);
  const [chosen, setChosen] = useState(new Set());
  const [asking, setAsking] = useState(false);
  const [running, setRunning] = useState(false);
  const spin = useSpinner(state.loading || running);
  const where = platform === "windows" ? "Recycle Bin" : "Trash";

  const measure = useCallback(() => {
    setState({ loading: true });
    rpc.call("clean_scan").then(
      (r) => {
        setState({ loading: false, ...r });
        setChosen(new Set(r.targets.filter((t) => t.group !== "trash" && t.group !== "dev").map((t) => t.id)));
        setCursor(0);
      },
      (e) => setState({ loading: false, targets: [], error: e.message }),
    );
  }, [rpc]);
  useEffect(measure, [measure]);

  const targets = state.targets || [];
  const picked = targets.filter((t) => chosen.has(t.id));
  const pickedSize = picked.reduce((s, t) => s + t.bytes, 0);
  const onlyDelete = picked.length > 0 && picked.every((t) => t.delete_only);

  const run = async (mode) => {
    setAsking(false);
    setRunning(true);
    try {
      const r = await rpc.call("clean_run", { ids: picked.map((t) => t.id), mode });
      const what = mode === "trash" ? `Moved ${plural(r.removed, "item")} (${bytes(r.freed)}) to the ${where}` : `Deleted ${plural(r.removed, "item")}, freed ${bytes(r.freed)}`;
      const extra = r.skipped ? `; skipped ${count(r.skipped)} in use or changed${r.errors[0] ? ` (${r.errors[0]})` : ""}` : "";
      setMessage({ text: what + extra + "." });
    } catch (e) {
      setMessage({ error: true, text: e.message });
    }
    setRunning(false);
    measure();
  };

  useInput((input, key) => {
    if (running || state.loading) {
      if (input === "q" || key.escape) back();
      return;
    }
    if (asking) {
      if (input === "t" && !onlyDelete) run("trash");
      else if (input === "d") run("delete");
      else setAsking(false);
      return;
    }
    setMessage(null);
    if (input === "q" || key.escape || key.backspace || key.leftArrow) return back();
    if (key.upArrow || input === "k") setCursor((c) => Math.max(0, c - 1));
    if (key.downArrow || input === "j") setCursor((c) => Math.min(targets.length - 1, c + 1));
    if (input === " " && targets[cursor]) {
      const next = new Set(chosen);
      const id = targets[cursor].id;
      next.has(id) ? next.delete(id) : next.add(id);
      setChosen(next);
    }
    if (input === "a") setChosen(chosen.size === targets.length ? new Set() : new Set(targets.map((t) => t.id)));
    if (key.return && picked.length) setAsking(true);
    if (input === "r") measure();
  });

  const nameW = Math.max(16, Math.min(30, Math.floor(cols / 3)));
  return (
    <Box flexDirection="column">
      <Box justifyContent="space-between">
        <Text>
          <Text bold color="#ffc83d">
            FrisyDisk{" "}
          </Text>
          <Text bold>Clean up</Text>
        </Text>
        <Text dimColor>{state.loading ? `${spin} measuring…` : `${bytes(targets.reduce((s, t) => s + t.bytes, 0))} found`}</Text>
      </Box>
      <Text dimColor wrap="truncate-end">
        {fit("Temporary files, caches and logs are rebuilt by apps when needed. Items changed in the last hour (temp: day) are left alone.", cols)}
      </Text>
      <Box flexDirection="column" height={height}>
        {state.error ? <Text color="red">{state.error}</Text> : null}
        {!state.loading && !targets.length && !state.error ? <Text dimColor>  Nothing to clean right now.</Text> : null}
        {targets.map((t, i) => {
          const on = i === cursor;
          return (
            <Box key={t.id} flexDirection="column">
              <Box>
                <Text color={on ? "#ffc83d" : undefined}>{on ? "❯ " : "  "}</Text>
                <Text color={chosen.has(t.id) ? "#ffc83d" : undefined}>{chosen.has(t.id) ? "[x] " : "[ ] "}</Text>
                <Text bold>{fit(t.label, nameW).padEnd(nameW)}</Text>
                <Text bold>{bytes(t.bytes).padStart(10)}</Text>
                <Text dimColor>{`  ${plural(t.count, "item")}`}</Text>
                {t.needs_admin ? <Text color="#ff8f6b">  needs administrator</Text> : null}
                {t.delete_only ? <Text color="#ff8f6b">  deleted permanently</Text> : null}
              </Box>
              {on ? (
                <Text dimColor wrap="truncate-end">
                  {fit(`      ${t.detail}${t.examples[0] ? `  e.g. ${t.examples[0]}` : ""}`, cols)}
                </Text>
              ) : null}
            </Box>
          );
        })}
        {!state.loading && !state.scan_open ? <Text dimColor>{"\n  Scan your home folder first (frisy ~, then press c) to also find unused build folders."}</Text> : null}
      </Box>
      {asking ? (
        <Text color="#ff8f6b" bold>
          {onlyDelete
            ? `Delete ${picked.length === 1 ? "1 category" : `${picked.length} categories`} (${bytes(pickedSize)}) permanently?  d = delete, any other key = cancel`
            : `Clean ${bytes(pickedSize)}:  t = move to the ${where}   d = delete permanently (frees space now)   other key = cancel`}
        </Text>
      ) : running ? (
        <Text color="#ffc83d">{`${spin} cleaning…`}</Text>
      ) : (
        <Text dimColor wrap="truncate-end">
          {fit(`${picked.length} selected, ${bytes(pickedSize)}   ↑↓ move  space select  a all  ⏎ clean  r measure again  esc back`, cols)}
        </Text>
      )}
    </Box>
  );
}
