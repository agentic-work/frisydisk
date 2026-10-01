# FrisyDisk

See what is using your disks, on macOS and Windows. One Rust scanner, one web
interface, packaged with Tauri.

## What it does

- Scans a volume or folder in parallel and draws the chart while it scans.
- Three views of the same data: sunburst, treemap, and a Sankey flow that opens
  up only the largest folders.
- A synced list, search, a largest-files list and a file-type breakdown for
  whatever folder you are in.
- Collector: stage items, review them, then move them to the Trash (Recycle Bin
  on Windows) after a confirmation. Nothing is deleted permanently.
- Advisor: sends the scan and a summary of your volumes, including NAS shares,
  to a model and asks for non-destructive ways to improve I/O and use existing
  storage better. It only produces text, and any command in an answer that
  would delete data is flagged.

### Advisor providers

The Advisor tab appears only when there is a model to talk to. Under
**Settings** you can use any of:

| Provider | What to set | Where your scan summary goes |
| --- | --- | --- |
| Ollama on this computer | nothing, it is detected at `127.0.0.1:11434` | nowhere, it stays local |
| Ollama on another machine | its address | that machine |
| AgenticWork | base URL and API key | your AgenticWork deployment |
| Anthropic | API key | Anthropic |
| OpenAI | API key | OpenAI |

With nothing configured and no local Ollama, the tab stays hidden. You can also
turn the advisor off. Keys are stored in `settings.json` in your user config
folder (readable only by you on macOS and Linux) and are never sent to the
interface.

## Build

You need Rust, Node 20 or later, and the platform tools Tauri asks for
(Xcode Command Line Tools on macOS; the MSVC build tools and WebView2 on Windows).

```sh
npm ci
npm test            # Rust core tests and the layout tests
npx tauri dev       # run the app from source
npx tauri build     # build an installer for this platform
```

## Layout

| Path | What is there |
| --- | --- |
| `crates/frisy-core` | Scanner, tree, analysis, advisor and the JSON API. No UI. |
| `crates/frisy-core/src/bin/frisyscan.rs` | Command-line scanner and development server. |
| `ui/` | The interface: plain HTML, CSS and JavaScript modules, no bundler. |
| `src-tauri/` | The desktop shell. It forwards one `api` command to the core. |
| `scripts/` | Icon drawing and the signed macOS release. |

## Command line

```sh
cargo run --release --bin frisyscan -- ~/Projects --top 20   # totals and largest children
cargo run --release --bin frisyscan -- --facts               # what the advisor is told about this machine
cargo run --release --bin frisyscan -- ~ --advise            # scan, then ask the first available model
cargo run --release --bin frisyscan -- ~ --advise anthropic  # or name a provider or a model
cargo run --release --bin frisyscan -- serve                 # the UI in a browser at http://127.0.0.1:7878
```

`serve` exposes the same API the app uses, bound to localhost only, so the
interface can be developed and tested in an ordinary browser.
`?scan=/path&mode=sankey&tab=types` in the URL, or `--scan PATH --mode sankey
--tab types` on the app's command line, opens straight into a scan.

## Signed macOS release

```sh
scripts/release-macos.sh install
```

Builds the app, signs it with a Developer ID certificate (hardened runtime,
secure timestamp), notarises and staples it, and wraps it in a signed,
notarised DMG. The header of the script lists the credentials it expects.

## What the numbers mean

- macOS and Linux: bytes allocated on disk; hard links are counted once.
  APFS clones are counted at full size each, as `du` does.
- Windows: logical file sizes; files that exist only in the cloud (OneDrive
  placeholders) count as zero. Junctions and symlinks are not followed.
- A whole-disk total is lower than the system's "used" figure: snapshots, swap
  and folders the app cannot read are not in it. On macOS, grant Full Disk
  Access to see protected folders.

## Licence

MIT. See [LICENSE](LICENSE).
