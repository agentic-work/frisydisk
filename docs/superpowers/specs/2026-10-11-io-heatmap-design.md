# I/O heatmap

Date: 2026-10-11. Approved in conversation after two scoping questions (live I/O
vs. a free recently-changed proxy; platform scope for v1).

## Goal

A fourth chart mode, alongside Sunburst, Treemap and Sankey, that colors the
currently open scan by which files were actually being read or written during
a short sample taken when you ask for it — "what's hot right now," not a
historical or synthetic measure.

## Why this is its own spec

Unlike the existing charts, this one depends on OS-level tracing rather than
the scanner's own data, and that support is genuinely uneven across platforms:
macOS has a ready-to-use tool, Linux needs one that often isn't installed, and
Windows' equivalent (ETW) is a project of its own. The design has to say what
happens on each platform, not just the happy path.

## Platform scope for v1

| Platform | Tool | Behaviour |
| --- | --- | --- |
| macOS | `fs_usage` (built in) | Full live sampling, as described below. |
| Linux | `bpftrace`, if present on `PATH` | Same sampling approach if available; otherwise the tab explains it needs bpftrace instead of showing a chart. |
| Windows | — | Not available in v1. The tab says so. A real ETW-based version is a separate, later piece of work. |

No platform ever shows a fabricated or approximated result in place of a real
one. "Not available" is always an honest, visible state, never a silent
fallback to something that looks like live data but isn't.

## How sampling works (macOS)

1. The person switches to the Heatmap tab and clicks **Sample I/O (8s)**.
2. The backend runs `fs_usage -w -f filesys -t <seconds>` for the requested
   duration (default 8s, clamped server-side to 1–30s before it ever reaches
   a command string), elevated once via `osascript -e 'do shell script
   "<command>" with administrator privileges'` — the standard macOS pattern
   for a single privileged command. The duration is the only part of that
   command built from a caller-supplied value, and it is parsed and clamped
   to a plain integer first — never passed through as a string — so there is
   no way for it to carry shell metacharacters; nothing else in the command
   (no path, no other argument) comes from outside the backend at all.
   Declining the system password prompt is treated as a normal error
   ("I/O sampling needs administrator access"), not a crash.
3. Output is parsed as it streams, line by line, nothing written to disk:
   - Keep lines whose call is a read/write-class syscall (`read`, `write`,
     `pread`, `pwrite`, and their `_nocancel` variants).
   - Take the byte count from the `B=0x…` field.
   - Take the path as the line's last whitespace-delimited token that starts
     with `/` (paths are always absolute in this output; nothing else in the
     format starts with `/`).
   - Add the bytes to a running `HashMap<path, bytes>` for the sample window.
4. The call is synchronous and returns once `fs_usage` exits (at the `-t`
   timeout, or earlier if stopped) — matching the existing `benchmark`
   command's shape, not the Advisor's background-poll one, since the duration
   is short, fixed, and needs no incremental server-pushed progress. The UI
   runs its own countdown locally.
5. Every sampled path is matched against the tree of the **currently open
   scan**. A path outside that tree is dropped — this is a heatmap of what
   you're looking at, not of the whole machine. A matched file's bytes roll
   up to its ancestors the same way size already does, so a folder's heat is
   the sum of everything hot inside it.

## Rendering

Reuses the existing squarified treemap layout and tile geometry unchanged.
The only new thing is the color channel: instead of (or blended with) the
existing size-based hue, a tile's fill reflects its share of the sampled
bytes, on a scale from "no activity" to "hottest file in this view." A tile
with no recorded activity renders with the current dim/inactive look, not as
an error. Hover and click behave exactly as they do today, including seeing
exact byte counts moved during the sample in the detail panel.

## API

- `"heatmap_sample"` — takes an optional `seconds` (defaults to 8, clamped to
  a sane range server-side), requires a scan to be open, and returns the
  per-node heat alongside the usual error cases (`"No scan is open."`, a
  declined-privileges message, a "needs bpftrace" message on Linux, a
  "not available on Windows yet" message on Windows).
- No new persisted state: a heat sample belongs to one chart render, is not
  cached between scans, and does not affect anything else `Api` tracks
  (collector, advisor, hidden space).

## Safety

- Read-only with respect to the filesystem: tracing only ever observes;
  nothing is written, moved or deleted.
- The sample window is short and bounded by construction (`-t`), and the
  subprocess is also hard-killed as a safety net if it somehow outlives that.
- Raw trace output never touches disk and is discarded once aggregated into
  byte-per-path totals.
- Declining the elevation prompt, or running on a platform/toolchain gap, are
  both ordinary, clearly worded outcomes — never a silent empty chart.

## What's out of scope here

- Windows ETW support (ships later, as its own spec).
- A memory-pressure or per-process view; "hottest" here means disk I/O by
  file, which is what was asked for and what `fs_usage` actually measures.
- Any history or trend over time — this is a single point-in-time sample, not
  continuous monitoring.
