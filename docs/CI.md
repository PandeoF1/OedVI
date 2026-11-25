# CI/CD

Two workflows in `.github/workflows/`:

- **CI** (`ci.yml`): on every push and pull request. Format, clippy, tests,
  frontend type check and translation check, the server and CLI binaries, the
  Docker image, and an unsigned Windows installer as an artifact.
- **Release** (`release.yml`): on a tag `vX.Y.Z`. Builds the signed Windows
  installer, writes `latest.json` for the in-app updater, builds the Linux
  server and CLI, and publishes a GitHub release with all of it.

## Runners

Every job runs on **your own runners** by default: labels `self-hosted` and
`linux` (lint, tests, server, Docker image, release publishing) or `windows`
(the launcher build). GitHub adds the `linux`/`windows` labels itself when you
register a runner.

To use other runners, for one job type or all, set repository variables
(Settings → Secrets and variables → Actions → Variables). Each is a JSON list
of runner labels:

| Variable | Default | Used by |
|---|---|---|
| `RUNNER_LINUX` | `["self-hosted","linux"]` | lint and tests, server, Docker image, release publishing |
| `RUNNER_WINDOWS` | `["self-hosted","windows"]` | the launcher build |

For example `RUNNER_WINDOWS` = `["windows-latest"]` goes back to GitHub's
hosted runner. Nothing in the workflow files needs to change.

Set up a runner with the instructions of Settings → Actions → Runners → *New
self-hosted runner*, then install once, by hand (a GitHub-hosted runner would have them):

**Linux runner**
- Rust (`rustup`), Node.js 22, Docker (for the image check).
- Tauri's system libraries: `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev
  libayatana-appindicator3-dev`. The workflow skips `apt-get` on self-hosted
  runners.

**Windows runner**
- Rust (`rustup`, MSVC toolchain), Node.js 22, Visual Studio Build Tools with
  the “Desktop development with C++” workload, WebView2 (present on Windows
  10/11), and Git for Windows (the steps use `bash`).
- To build the NSIS installer, Tauri downloads NSIS the first time; the runner
  needs internet access.

## Secrets for a release

Repository secrets (Settings → Secrets and variables → Actions → Secrets):

| Secret | What |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | content of the private key made by `npx tauri signer generate -w oedvi.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | its password (empty if none) |
| `OEDVI_UPDATER_PUBKEY` | content of `oedvi.key.pub` |
| `OEDVI_SERVER_URL`, `OEDVI_ADMIN_TOKEN` | optional: also upload the installer to your OedVI server |

The private key never goes into git. Keep a backup: without it, installed
launchers cannot receive updates signed by a new key.

## How updates reach the launchers

- A **standalone** launcher reads
  `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`
  (compiled in by the release workflow as `OEDVI_UPDATE_URL`). A build made
  elsewhere without that variable has no update source.
- A launcher **connected to a server** asks the server
  (`/api/launcher/latest.json`), which serves what you upload to its
  `launcher/` folder.

In both cases the installer is only run if its signature matches the public
key compiled into the launcher.

## Cutting a release

1. Bump `version` in `Cargo.toml` (workspace),
   `launcher/src-tauri/tauri.conf.json` and `launcher/package.json` (and
   `package-lock.json`).
2. Commit, then `git tag vX.Y.Z && git push --tags`.
3. The release appears in the Releases page when the workflow ends.
