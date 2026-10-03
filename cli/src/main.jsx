// frisy [PATH]          explore a folder or volume (current folder by default)
// frisy clean           find and remove temp files, caches and unused build folders
// frisy du [-c] PATH... totals like `du -sh`, straight from the scanner
// frisy --print [PATH]  print the largest items and exit (also used when output is not a terminal)

import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { render } from "ink";
import App from "./App.jsx";
import { bar, bytes, count, hue } from "./format.js";
import { findScanner, Rpc } from "./rpc.js";

const args = process.argv.slice(2);
const help = `FrisyDisk in the terminal

  frisy [PATH]           explore PATH (default: this folder)
  frisy clean            clean temp files, caches, logs and unused build folders
  frisy du [-c] PATH...  quick totals, like du -sh but much faster
  frisy --print [PATH]   print the largest items and exit

Keys: arrows move, enter opens, backspace goes up, tab switches view,
/ searches, space marks, x moves to the Trash, c cleans up, q quits.

Folders you cannot read are skipped. To include them, run with sudo
(macOS and Linux) or from PowerShell opened as Administrator (Windows).`;

if (args.includes("-h") || args.includes("--help")) {
  console.log(help);
  process.exit(0);
}

if (args[0] === "du") {
  const r = spawnSync(findScanner(), args, { stdio: "inherit" });
  process.exit(r.status ?? 1);
}

const print = args.includes("--print") || !process.stdout.isTTY || !process.stdin.isTTY;
const rest = args.filter((a) => a !== "--print");
const clean = rest[0] === "clean";
const path = resolve(clean ? process.cwd() : rest[0] || ".");

if (print) {
  await printReport(path, clean);
} else {
  const rpc = new Rpc();
  const app = render(<App rpc={rpc} path={path} startInClean={clean} />, { exitOnCtrlC: false });
  await app.waitUntilExit();
  rpc.close();
}

async function printReport(path, clean) {
  const rpc = new Rpc();
  const out = (s = "") => process.stdout.write(s + "\n");
  const color = process.stdout.isTTY ? (hex, s) => `\x1b[38;2;${parseInt(hex.slice(1, 3), 16)};${parseInt(hex.slice(3, 5), 16)};${parseInt(hex.slice(5), 16)}m${s}\x1b[0m` : (_, s) => s;
  try {
    if (clean) {
      const r = await rpc.call("clean_scan");
      out("Could be cleaned (nothing has been removed):");
      for (const t of r.targets) out(`${bytes(t.bytes).padStart(10)}  ${t.label}  (${count(t.count)} items)${t.needs_admin ? "  needs administrator" : ""}`);
      if (!r.targets.length) out("  nothing");
      return;
    }
    await rpc.call("start_scan", { path });
    let v;
    do {
      await new Promise((r) => setTimeout(r, 150));
      v = await rpc.call("view", { focus: 0, depth: 1, min_fraction: 0, rows: 25 });
    } while (v.progress.scanning);
    const total = v.tree.size;
    out(`${v.path}  ${bytes(total)} in ${count(v.tree.files)} files (${v.progress.seconds.toFixed(1)} s)`);
    let at = 0;
    for (const r of v.rows) {
      const share = total ? r.size / total : 0;
      out(`${color(hue(at + share / 2), bar(share, 20))} ${(share * 100).toFixed(1).padStart(5)}% ${bytes(r.size).padStart(9)}  ${r.name}${r.dir ? "/" : ""}`);
      at += share;
    }
    if (v.hidden) out(`${" ".repeat(28)}${bytes(v.hidden.bytes).padStart(9)}  (hidden space: system, snapshots, swap, unreadable)`);
    if (v.progress.inaccessible) out(`\n${count(v.progress.inaccessible)} folders could not be read.`);
  } catch (e) {
    console.error(`frisy: ${e.message}`);
    process.exitCode = 1;
  } finally {
    rpc.close();
  }
}
