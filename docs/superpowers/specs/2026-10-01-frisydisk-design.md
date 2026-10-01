# FrisyDisk design

Date: 2026-10-01. Approved in conversation; built straight through at the owner's request.

## Goal

A native macOS disk-space visualiser in the DaisyDisk style, original code (DaisyDisk is closed source),
with everything unlocked and a few things DaisyDisk does not do.

## Parts

- **FrisyCore** (library, no UI)
  - `DiskScanner`: parallel walk with `getattrlistbulk`. Allocated sizes, hard links counted once,
    stays on the scanned volume (for `/` it also follows firmlinks into the Data volume),
    never materialises cloud-only folders.
  - `ScanTree` / `FileNode`: the tree and the one lock that guards it. Totals roll up as each
    directory is read, so the UI can draw a partial scan.
  - `SunburstLayout`, `TreemapLayout`: pure layout and hit testing.
  - `Analysis`: largest files, search, file-type breakdown.
  - `SystemFacts`, `Advisor`: read-only machine and scan reports, and the client for a local model.
- **FrisyDisk** (SwiftUI app): volume picker, sunburst and treemap, synced list, search,
  largest files, types, collector, advisor.
- **frisyscan** (CLI): scan totals and the advisor from a terminal.
- **frisytests**: self-contained test runner.

## Safety rules

- The only thing that changes the disk is "Move to Trash" in the collector, after a confirmation.
  Items go to the Trash, never permanently deleted.
- The advisor only produces text. It runs nothing. Its prompt forbids destructive suggestions,
  and the UI says the output is unreviewed model text.

## Build

Only the Command Line Tools are assumed. `Scripts/build.sh` calls `swiftc` directly because
SwiftPM is broken in the installed tools; `Package.swift` describes the same targets.
View state avoids `@State` because the tools lack the SwiftUI macro plugin.
