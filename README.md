# FrisyDisk

A native macOS app that shows what is using your disks, as an interactive sunburst or treemap.
In the spirit of DaisyDisk, written from scratch in Swift and SwiftUI.

## What it does

- Scans a volume or folder in parallel (about 4 M files in 20 s on an M4 Pro) and draws the chart while it scans.
- Sunburst and treemap views of the same data, with a synced list. Click a folder to drill in.
- Search, a largest-files list and a file-type breakdown for whatever folder you are in.
- Collector: stage items, review them, then move them to the Trash after a confirmation. Nothing is deleted permanently.
- Advisor: sends the scan and a summary of your volumes (including NAS shares) to a model running on your Mac
  and asks for non-destructive ways to improve I/O and use existing storage better. It only produces text.
  Uses Ollama at `127.0.0.1:11434`, or the `agenticode` CLI if it launches.

## Build and install

Needs macOS 14 or later and the Xcode Command Line Tools.

```sh
Scripts/build.sh test      # build and run the unit tests
Scripts/build.sh install   # build build/FrisyDisk.app and copy it to /Applications
```

The app is ad-hoc signed, so it runs on the Mac that built it. It is not notarised.

To let it read protected folders, add FrisyDisk under System Settings → Privacy & Security → Full Disk Access, then rescan.

## Command line

```sh
build/frisyscan ~/Projects --top 20     # totals and the largest children
build/frisyscan --facts                 # the machine report the advisor sees
build/frisyscan ~ --advise gemma4:latest
```

## Scripted runs

The app accepts `--scan PATH`, `--mode treemap`, `--tab largest|types|advisor`, `--advise` and
`--snapshot OUT.png` (save the window and quit). `--test-trash-child NAME` moves that child of
the scan root to the Trash without asking; it exists for testing.

## Limits

- Sizes are allocated bytes. APFS clones are counted at full size each, as `du` does.
- A whole-disk total is lower than `df`: snapshots, swap and folders the app cannot read are not in it.
