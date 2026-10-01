# FrisyDisk design

Date: 2026-10-01. Revised the same day when Windows support became a requirement.

## Goal

A disk-space visualiser for macOS and Windows, original code, open source (MIT).

## Decisions

- **One codebase on Tauri 2.** The first version was SwiftUI; it could not run on
  Windows, so it was replaced. Rust does the scanning, a web UI does the drawing.
- **The UI talks to the core through one call**, `api(cmd, args) -> JSON`
  (`crates/frisy-core/src/api.rs`). The Tauri shell forwards it; `frisyscan serve`
  exposes the same thing over HTTP on localhost. That keeps behaviour identical in
  both, and lets the interface be tested in a browser against real scans.
- **Everything is polled.** Scan progress and advisor output are read by polling,
  so no event plumbing differs between the app and the dev server.
- **The tree is an arena** (`Vec<Node>` indexed by `u32`) behind one mutex. Totals
  roll up as each directory is read, so a partial scan can be drawn.
- **Platform readers.** macOS uses `getattrlistbulk`; Windows and other systems use
  `std::fs`, which on Windows gets metadata from the directory listing for free.
- **Charts are laid out in JavaScript** from a pruned tree the core sends
  (small items folded into a group). `ui/layout.js` is pure and unit-tested.
- **Sankey shows only the largest folders**: a folder gets a next column only when
  it holds at least 3.5% of the focused folder; the rest merge into "other".

## Safety rules

- The only thing that changes the disk is "Move to Trash" in the collector, after
  a confirmation. Items go to the Trash or Recycle Bin, never deleted outright.
- The advisor only produces text. Its prompt forbids destructive suggestions, and
  because models do not always comply, answers are checked and any deleting
  command is flagged.
- The dev server binds to 127.0.0.1 only.
