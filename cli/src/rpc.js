// Talks to the Rust scanner (`frisyscan rpc`) over JSON lines.

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { createInterface } from "node:readline";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const exe = process.platform === "win32" ? "frisyscan.exe" : "frisyscan";

/** Find frisyscan: $FRISYSCAN, next to this script, one folder up, then PATH. */
export function findScanner() {
  if (process.env.FRISYSCAN) return process.env.FRISYSCAN;
  const here = dirname(fileURLToPath(import.meta.url));
  for (const dir of [here, join(here, ".."), join(here, "..", "bin"), join(here, "..", "..", "target", "release")]) {
    const p = join(dir, exe);
    if (existsSync(p)) return p;
  }
  if (process.platform === "win32") throw new Error("frisyscan.exe was not found next to frisy. Reinstall frisy.");
  return exe; // search PATH (never the current folder on macOS and Linux)
}

export class Rpc {
  constructor(path = findScanner()) {
    this.next = 1;
    this.pending = new Map();
    this.child = spawn(path, ["rpc"], { stdio: ["pipe", "pipe", "inherit"], windowsHide: true });
    this.failed = null;
    this.child.on("error", (e) => {
      this.failed = new Error(`Could not start ${path}: ${e.message}`);
      for (const p of this.pending.values()) p.reject(this.failed);
      this.pending.clear();
    });
    this.child.on("exit", () => {
      for (const p of this.pending.values()) p.reject(new Error("The scanner stopped."));
      this.pending.clear();
    });
    createInterface({ input: this.child.stdout }).on("line", (line) => {
      let msg;
      try {
        msg = JSON.parse(line);
      } catch {
        return;
      }
      const p = this.pending.get(msg.id);
      if (!p) return;
      this.pending.delete(msg.id);
      if ("error" in msg) p.reject(new Error(msg.error));
      else p.resolve(msg.ok);
    });
  }

  call(cmd, args = {}) {
    if (this.failed) return Promise.reject(this.failed);
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.child.stdin.write(JSON.stringify({ id, cmd, args }) + "\n");
    });
  }

  close() {
    this.child.stdin.end();
    this.child.kill();
  }
}
