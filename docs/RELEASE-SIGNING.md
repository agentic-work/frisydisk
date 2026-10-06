# Release signing: Windows + macOS universal, in GitHub Actions

Shared across agenticode, frisydisk, goearth and studiorig (all under
`agentic-work/`). Written 2026-10-06 after the build Mac's signing keychain was
lost; everything below reflects the setup that replaced it.

## The two identities

**macOS — Apple Developer ID**
- Identity: `Developer ID Application: Trenton White (T8S7LJANR8)`
- Certificate issued 2026-10-05, **expires 2027-02-01**. Renew in January 2027:
  new CSR → developer.apple.com → Certificates → Developer ID Application, then
  re-export the .p12 and update `APPLE_CERTIFICATE` in every repo.
- Private key backup: Secret Manager `macos-devid-p12` (project
  `agenticwork-dev`, us-east1 — the org policy allows nothing else).
- Notarisation uses an **App Store Connect API key** (team-level, one key serves
  every app). As of this writing **none exists**: the previous one was lost with
  the keychain. Until `APPLE_API_KEY` and `APPLE_API_KEY_P8` are set, macOS
  builds are signed but NOT notarised — Gatekeeper on anyone else's Mac will
  refuse them. Do not distribute those.

**Windows — Azure Artifact Signing (Trusted Signing)**
- Endpoint `https://eus.codesigning.azure.net/`, account `patchbaysigning`,
  certificate profile `patchbay` (Public Trust, active).
- Each repo has its **own** service principal (`<repo>-github-signing`) holding
  only *Artifact Signing Certificate Profile Signer* on that one profile, so one
  can be revoked without touching the others. Secrets expire October 2028.
- **A client secret that begins with `-` cannot be used**: the signing CLI runs
  `az login ... -p <secret>` and az reads it as a flag ("expected one
  argument"). Entra issues one roughly 1 time in 60. If it happens:
  `az ad sp credential reset --id <AZURE_CLIENT_ID> --years 2` until it does not.

## Secret names (identical in every repo)

| Name | What |
|---|---|
| `APPLE_CERTIFICATE` | base64 of the Developer ID .p12 (legacy PKCS#12 encryption — macOS `security import` rejects OpenSSL 3's default) |
| `APPLE_CERTIFICATE_PASSWORD` | its password |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Trenton White (T8S7LJANR8)` |
| `APPLE_TEAM_ID` | `T8S7LJANR8` |
| `APPLE_API_ISSUER` | App Store Connect issuer UUID (set) |
| `APPLE_API_KEY` | App Store Connect **key ID** (what Tauri calls APPLE_API_KEY) — *pending* |
| `APPLE_API_KEY_P8` | contents of `AuthKey_<id>.p8` — *pending* |
| `AZURE_TENANT_ID` / `AZURE_CLIENT_ID` / `AZURE_CLIENT_SECRET` | the repo's own service principal |
| `AZURE_SIGNING_ENDPOINT` / `_ACCOUNT` / `_PROFILE` | as above (vars or secrets, per the repo's workflow) |

## macOS universal — the rules that bite

1. **Build one universal binary**, not two DMGs.
   - Tauri: `rustup target add aarch64-apple-darwin x86_64-apple-darwin`, then
     `tauri build --target universal-apple-darwin`. Output lands in
     `target/universal-apple-darwin/release/bundle/`.
   - Wails: `wails build -platform darwin/universal`.
   - A hosted `macos-14` (arm64) runner builds both slices. Do not use
     `macos-13` (Intel): GitHub is retiring it and jobs sit queued for hours.
2. **Every native thing you bundle must be universal too** — sidecars, a vendored
   Node, `.node` addons, dylibs. Check with `lipo -archs <file>`; merge two
   slices with `lipo -create a b -output c`. A single-arch sidecar in a
   universal app either fails the bundle or crashes on the other architecture.
3. **Sign inside-out, with each binary's own entitlements.** `codesign` REPLACES
   entitlements rather than merging them. Re-signing a vendored Node (or any
   JIT runtime) with the hardened runtime and no entitlements strips
   allow-jit / allow-unsigned-executable-memory, and the process dies with
   SIGTRAP the first time it compiles — the signature still verifies. Read each
   nested binary's entitlements (`codesign -d --entitlements :- <bin>`) and give
   them back when re-signing; then *run* the signed binary as a smoke test.
4. **Make the DMG with `hdiutil`**, not Tauri's DMG bundler or anything that
   scripts Finder: AppleScript hangs on a headless runner.
5. **Notarise, then staple both the .app and the .dmg**
   (`xcrun notarytool submit --wait`, `xcrun stapler staple`), then prove it:
   `spctl -a -vvv -t exec <app>` must say `source=Notarized Developer ID`.
6. Import the certificate into a **throwaway keychain** per job
   (`security create-keychain` with a random password, `set-key-partition-list
   -S apple-tool:,apple:,codesign:`), and delete it in an `if: always()` step.

## Windows — the rules that bite

1. **Sign each binary during bundling**, via `bundle.windows.signCommand`
   (Tauri) or before packing the installer (Wails/NSIS). A post-build signing
   action only signs the outer installer and leaves the executable users run
   unsigned.
2. Install the signer at a **pinned version**:
   `cargo install artifact-signing-cli --version 0.11.0 --locked`
   (`trusted-signing-cli` is the deprecated name).
3. **Verify** afterwards: `signtool verify /pa /v <setup.exe>` must show
   `Issued to: Trenton White` and a timestamp.
4. Tauri hides a custom sign command's output when it fails ("failed to run
   node"). Sign a scratch copy of any .exe *before* the long compile so a
   credential fault fails in a minute with its real error.

## Workflow hygiene

- Pin third-party actions to a commit SHA (first-party `actions/*` may use tags).
- Give signing secrets to the steps that sign, not to the whole job.
- Never pass `${{ inputs.* }}` or event fields straight into `run:`; go through `env:`.
- Private repo ⇒ Releases and run artifacts are visible only to repo members.
  That is the point for Pro builds — and it also means customers **cannot**
  download from them. Public distribution needs a public release channel.

## frisydisk — status and what is left

- `release.yml` already does it right: universal (`--target
  universal-apple-darwin --bundles dmg`), Windows NSIS signed per binary, draft
  Release on tag. Release 0.5.0 (2026-10-05) passed.
- Its `APPLE_CERTIFICATE` is the **previous** Developer ID (pre-2026-10-05),
  valid until 2027-02-01. Its private key now exists only inside that secret.
  Leave it until the January renewal, then switch to the new one.
- Azure: its own service principal and `vars.AZURE_SIGNING_*` are set.
- **Missing:** `APPLE_API_KEY` + `APPLE_API_KEY_P8`. Without them the DMG is
  signed, not notarised.
- This repo is **public**: secrets are not exposed to fork PRs, but keep
  release workflows on tag pushes only, never `pull_request_target`.
