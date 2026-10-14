# Release signing: Windows + macOS universal, in GitHub Actions

Shared across agenticode, frisydisk, goearth and studiorig (all under
`agentic-work/`). Written 2026-10-06 after the build Mac's signing keychain was
lost; everything below reflects the setup that replaced it. Proven end to end
on 2026-10-06 by studiorig (run 37480511845): universal macOS DMGs signed,
notarised ("status: Accepted"), stapled, and accepted by Gatekeeper as
"source=Notarized Developer ID"; Windows installers signed by Trenton White
with a timestamp.

**Backups are local only.** Every credential below is backed up on the
Synology at `/Volumes/volume1/backups/credentials/` (its README lists each
file). Never put credentials in Google Secret Manager or any other cloud store.
GitHub Actions secrets are where CI reads them, and that is the only other copy.

## The two identities

**macOS — Apple Developer ID**
- Identity: `Developer ID Application: Trenton White (T8S7LJANR8)`
- Certificate issued 2026-10-05, **expires 2027-02-01**. Renew in January 2027:
  new CSR → developer.apple.com → Certificates → Developer ID Application, then
  re-export the .p12 and update `APPLE_CERTIFICATE` in every repo.
- Private key backup: Synology, `apple-developer-id-2026-10/devid-2026-10.key`.
- Notarisation uses one **App Store Connect API key** (team-level; the same key
  serves every app). Created 2026-10-06; the `.p8` is on the Synology beside the
  certificate and in `~/.studiorig-signing/` on the build Mac. Apple never lets
  a `.p8` be downloaded twice — if every copy is lost, revoke it in App Store
  Connect (Users and Access → Integrations → Team Keys) and make a new one.
  **Never print the key, its contents or its ID in a log or a chat.**

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
| `APPLE_API_KEY` | App Store Connect **key ID** (what Tauri calls APPLE_API_KEY) — set in all four repos |
| `APPLE_API_KEY_P8` | contents of `AuthKey_<id>.p8` — set in all four repos |
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

## Notarisation — using the key in a pipeline

The secrets hold the key ID and the key's *contents*; every tool wants a *file*.
Write it to the runner's temp dir at the start of the signing step, and nowhere
else:

```bash
key="$RUNNER_TEMP/AuthKey_${APPLE_API_KEY}.p8"
printf '%s' "$APPLE_API_KEY_P8" > "$key"
```

Then, depending on what does the notarising:

- **Tauri** (`tauri build` / tauri-action) notarises and staples the `.app`
  itself when these are in the environment:
  `APPLE_API_ISSUER`, `APPLE_API_KEY` (the ID), `APPLE_API_KEY_PATH="$key"`.
  It does **not** notarise a DMG you make afterwards — do that with notarytool.
- **notarytool directly** (Wails, hand-made DMGs):
  ```bash
  xcrun notarytool submit "$dmg" --key "$key" --key-id "$APPLE_API_KEY" \
    --issuer "$APPLE_API_ISSUER" --wait            # must print "status: Accepted"
  xcrun stapler staple "$app"; xcrun stapler staple "$dmg"
  spctl -a -vvv -t exec "$app"                     # must say source=Notarized Developer ID
  ```
  On failure, `xcrun notarytool log <submission-id> --key … ` prints Apple's
  reasons (usually an unsigned nested binary or a missing hardened runtime).
- **studiorig's `scripts/sign-macos.mjs`** uses its own names: `APPLE_API_KEY`
  is the `.p8` *path* and `APPLE_API_KEY_ID` the ID. Its workflow maps the
  shared secret names onto those; don't rename the secrets to match it.

Order matters: sign everything inside-out → notarise → staple → verify. Signing
anything after notarising invalidates the ticket.

If either secret is missing, build signed-only and say so in the artefact name
(`-UNNOTARIZED` / `-NOT-NOTARIZED`) and the release notes — never ship that.

Locally on the build Mac, `~/.studiorig-signing/macos.env` holds the same key
(path, ID, issuer) next to the build keychain's password.

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
- Notary key: `APPLE_API_KEY`, `APPLE_API_KEY_P8` and `APPLE_API_ISSUER` set
  2026-10-06. Its workflow already writes the .p8 to `APPLE_API_KEY_PATH` for
  Tauri, so the next tagged release should come out notarised — confirm with
  `spctl` on the DMG's app rather than assuming.
- This repo is **public**: secrets are not exposed to fork PRs, but keep
  release workflows on tag pushes only, never `pull_request_target`.
