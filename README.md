# OedVI

A personal game launcher. It fetches **builders** (git repositories), builds
them and runs the game, with live performance charts. It works **standalone**
out of the box, and can connect to an OedVI **server** that publishes the game
files, the builders and a performance comparison.

```
STANDALONE (default)                          REMOTE (optional)
PC (Tauri launcher, Windows)                  SERVER (oedvi-server, Linux/Docker)
├── your game folder  ← you point to it       ├── game/       ← the files it publishes
├── builders/         ← TOML files, editable  ├── builders/   ← one TOML file per builder
├── templates/        ← shared bases          ├── templates/  ← shared bases (extends)
├── repos/            ← git clones            ├── launcher/   ← signed installers (updates)
├── cache/, data/     ← caches and saves      ├── stats/      ← performance reports
└── logs/, state/     ← logs, local state     └── manifest    ← BLAKE3 chunks, published on change
```

## Modes

| | Standalone (default) | Connected to a server |
|---|---|---|
| Game files | the folder you point to; **nothing is downloaded** | downloaded, verified and kept in sync |
| Builders | files on your PC, editable in the *Editor* tab | published by the server |
| Updates | GitHub releases | the server |
| Stats | your own sessions | compared with everyone's |

Switch in **Settings → Mode**. A fresh install is standalone: no address, no
token, nothing to configure.

## What it does

**Builders**
- A builder is a git repository + a ref + build commands + a run command, with
  placeholders (`{game_dir}`, `{repo_dir}`…).
- To manage dozens without copy-paste: `extends` inherits from a template,
  `[vars]` factors values out, and `checkout` shares one clone (and its build)
  between builders. Variants that only change the game's environment
  (`run_env`) do not rebuild.
- In the app: **Pull** (fast-forward), **Resync** (the repository exactly as
  the remote, `target/` untouched), **Fetch** (lists incoming commits),
  **Build** / **Rebuild**, **Play** (builds only if the commit or the recipe
  changed) and **Stop** (kills the whole process tree).
- Live logs, kept in `logs/<builder>/`. The log view strips terminal colors,
  hides timestamps and filters by level.
- A built-in editor validates builders and templates before saving; every
  replaced version goes to `.trash/`.
- A few builders ship with the launcher and are copied to your builders folder
  on first start. After that they are your files: edit or delete them, they
  are never overwritten or brought back.

**Build tools installed on demand**
- A builder declares what it needs: its `stack`, plus `requires`.
- Before a build the launcher checks **only those tools** and installs with
  `winget` **only the missing ones**. If winget itself is missing (a Windows 10
  that never signed in to the Store, a fresh profile) it first installs
  “App Installer” from Microsoft, without administrator rights.
- What is already installed is never touched. A Windows admin prompt only
  appears for tools that need it (Visual Studio Build Tools, .NET). PATH is
  reloaded after an install, no restart needed.

| The builder needs | Installed if missing |
|---|---|
| always | Git |
| `stack = "rust"` | Visual Studio Build Tools (C++, Windows) + Rust (rustup). The version pinned by the repository installs at the first `cargo`. |
| `stack = "python"` | Python 3.12 |
| `stack = "node"` | Node.js LTS |
| `stack = "dotnet"` | .NET SDK 8 |
| `stack = "cmake"` | Visual Studio Build Tools + CMake |
| `requires = ["uv", "ninja", "cmake", "npm", "cargo", "msvc", …]` | the matching tool |
| `requires = ["other-program"]` | checked on PATH, install it by hand |

Turn it off in Settings → “Install missing tools automatically”. The “Install
the tools” button on a builder stays available. CLI: `oedvi tools <builder>
[--install]`.

**Performance monitoring** — see [Performance](#performance).

**Languages** — English (default) and French, in Settings → Language.

**With a server**
- The server indexes the game folder (BLAKE3 per 32 MB chunk, cached, so a
  restart only re-hashes what changed), watches it and re-indexes by itself.
- The launcher downloads only **the chunks that differ**: resume after a
  cut, repair of a damaged file, partial update of a multi-GB archive. You
  choose what to keep locally, by group or by folder/file; `required` files
  cannot be unticked. **Verify all** re-reads every file and repairs what does
  not match.
- Launcher updates are published by the server and installed from the app
  (signature checked).

## Getting started

### Standalone

1. Install the launcher (installer from the
   [releases](../../releases)), or build it (below).
2. **Game** tab → choose the folder that holds your copy of the game.
3. **Builders** tab → pick a builder → **Play**. Missing tools are installed
   for you the first time.

Your own builders: **Editor** tab → *New*, or drop `.toml` files in the
`builders/` folder of the working folder (Settings → Folders → open).

### Connecting to a server

#### 1. Run the server

With Docker:

```bash
cd deploy
nano oedvi-server.toml                   # adapt the groups to your files
cat > .env <<EOF
GAME_DIR=/path/to/the/game
OEDVI_TOKEN=$(openssl rand -hex 24)
OEDVI_ADMIN_TOKEN=$(openssl rand -hex 24)
EOF
docker compose up -d --build
docker compose logs -f     # “manifest published” when indexing is done
```

Without Docker: `cargo build --release -p oedvi-server`, then
`oedvi-server -c oedvi-server.toml`.

Then:
- Copy the example builders:
  `docker compose cp ../examples/server/templates/. oedvi:/var/lib/oedvi/templates/`
  (same for `builders/`), or create them from the launcher's editor.
- **Network**: keep the server private. A VPN (Tailscale, WireGuard…) with the
  port bound to its address is the simplest. If you open a port on your router,
  restrict it to the IP addresses you trust: the server serves your files.
- Count 30 to 60 minutes for the first indexing of 250 GB on a hard disk. On
  an SSD set `hash_jobs = 2` to `4`.

In the server's config, set `public_url` if it is reached through a reverse
proxy or a name, so launcher updates get the right download address.

#### Server logs

`docker compose logs -f oedvi` shows:

- at start: paths, settings, groups (`*` = required), whether tokens are set,
  the launcher version served, the builders, and the broken ones with the
  reason;
- during a scan: what changed (new, modified, removed), a progress line every
  10 s (percentage, throughput, time left, current file), the time of each big
  file, then the manifest version (`content changed` = clients will update)
  and the size of each group;
- one line per request: `IP role METHOD /path → status size time`. The role is
  `read`, `admin` or `open` (no token configured). Refusals show as
  `WARN REFUSED … : no token | wrong token | read token used for a write`,
  handy to spot someone trying tokens;
- game file downloads, grouped every 15 s per IP (requests, volume, speed,
  last file), since a full sync makes thousands;
- builder edits (saved, refused with the reason, deleted) and each new IP.

The level is set with `RUST_LOG` (in `docker-compose.yml`):
`info,access=warn` keeps only access refusals and errors, `info,access=debug`
adds routine calls and every downloaded chunk. The IP shown is the connection's;
behind a reverse proxy it is the proxy's (`X-Forwarded-For` is shown, never
trusted). With Docker, the client IP is the bridge's (172.x) when the port is
bound to 127.0.0.1: publish the port on all interfaces to see real addresses.

#### 2. Connect the launcher

**Settings → Mode → Connected to a server**, enter the address and the token,
**Save and test**. The **Game** tab then shows the files to sync.

### Building the launcher

You need Rust (`rustup`), Node 22 and, on Windows, the Visual Studio Build
Tools (C++).

```powershell
cd launcher
npm ci
npm run tauri dev                                  # development
npm run tauri build -- --bundles nsis              # installer in target\release\bundle\nsis
```

## Releases and updates

Releases are built by GitHub Actions (see [docs/CI.md](docs/CI.md), including
how to use your own runner). To release:

1. Generate the signing key once: `npx tauri signer generate -w oedvi.key`.
   Keep the private key to yourself, never in git.
2. Add the repository secrets `TAURI_SIGNING_PRIVATE_KEY`,
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` and `OEDVI_UPDATER_PUBKEY` (the
   content of `oedvi.key.pub`).
3. Bump `version` in `Cargo.toml` (workspace),
   `launcher/src-tauri/tauri.conf.json` and `launcher/package.json`, then push
   a tag `vX.Y.Z`.

The workflow publishes a GitHub release with the signed installer, `latest.json`
(read by standalone launchers) and the Linux server and CLI binaries. Set the
`OEDVI_SERVER_URL` and `OEDVI_ADMIN_TOKEN` secrets to also upload the installer
to your server.

Without CI, a single script on a Windows PC with Node, Rust and the Build
Tools does everything (key, signed build, upload):

```powershell
cd launcher
.\release.ps1 -UpdateUrl https://github.com/<you>/<repo>/releases/latest/download/latest.json
.\release.ps1 -Server http://my-server:7878 -AdminToken $ADMIN
```

If your signing key has a password, set `$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
first. The installer and its `.sig` are in `target\release\bundle\nsis\`.

**HTTP without HTTPS**: the launcher accepts an `http://` update URL
(`dangerousInsecureTransportProtocol` in `tauri.conf.json`) because a server
is often run without TLS. The installer stays protected: the launcher refuses
any file whose signature does not match the public key compiled into it. What
plain HTTP does not protect is the (read) token, which travels in the clear:
keep the server behind an IP filter or a VPN, with a long token.

## Writing builders

Example: `examples/server/templates/victory.toml` and
`examples/server/builders/`.

```toml
# builders/amd-diag-warp.toml
extends = "amd-diag"                  # a template or another builder
name = "AMD · Diag WARP (no GPU)"
tags = ["victory", "amd", "diag"]

[run_env]                             # at launch only: no rebuild
VICTORY_D3D12_WARP = "1"
```

| Field | Purpose |
|---|---|
| `repo`, `ref` | git repository and branch, tag or commit |
| `stack` | `rust`, `python`, `node`, `dotnet`, `cmake`, `custom`: tools checked before the build |
| `checkout` | clone name; the same value means the same clone and build (same `repo` and `ref` required) |
| `build` | list of commands (`[["cargo", "build"], [...]]`) |
| `run` | the game command |
| `vars` | variables usable everywhere; they may reference each other |
| `env` / `build_env` / `run_env` | common / build / game environment |
| `workdir`, `run_workdir` | working folders |
| `requires` | extra tools to find on PATH |
| `fps_patterns`, `metrics`, `phases` | regexes that read numbers and phases in the game output (see *Performance*) |
| `tags`, `order`, `description` | display in the launcher |

Available placeholders:

- `{game_dir}`, `{repo_dir}`, `{work_dir}`, `{cache_dir}` (one per clone),
  `{data_dir}` (shared), `{root_dir}`, `{builder_id}`, `{exe_suffix}` (`.exe`
  on Windows), `{os}`;
- your `vars`;
- `"{x...}"`: one argument that spreads the variable into 0 to n arguments
  (handy for an optional `--features …`);
- `{{` and `}}` to write a brace.

On your PC, **Commands and variables** lets you add local environment
variables to a builder (e.g. `VICTORY_LOG=debug`) without editing the builder.

## Performance

While the game runs, the launcher measures two things every second:

- **What the game prints** in its output. Recognized out of the box: `fps`,
  `frame_ms` (`ms/frame`), `gpu` (`gpu 40% busy`), `cpu`, `ram`
  (`ram 11061 MB`) and `draws` (`9 draws/frame`), so a typical line is
  `30.0 fps, 33.3 ms/frame, 3 draws/frame, cpu 55% gpu 0% busy ram 5696 MB`.
- **What the PC does**: CPU, RAM and disk (read/write) of the game and its
  child processes, CPU and RAM of the PC, and on Windows the GPU and video
  memory the game uses (Windows performance counters through `typeperf`, any
  graphics card brand).

Live charts show all of it in the builder's detail, and **Open next to the
game** opens a small always-on-top window with the same charts.

When the game closes, the session is saved in `<root>/state/sessions.jsonl`
(and, in remote mode, sent to the server): average, min, 1 % and 99 %, the
curve of each measure, duration, the builder's commit, the PC name and its
**specs** (CPU, cores, RAM, system, graphics cards with driver version and
date). The **Stats** tab lets you open a session for all its charts (hover =
values, drag = zoom, wheel = zoom, double-click = see all) or tick up to 4
sessions to overlay them; with a server it also compares builders and commits.

To read other numbers in a builder's output (the first group of the regex is
the value; the same name replaces a built-in one):

```toml
fps_patterns = ["frames per second: (\\d+)"]
[metrics]
vram_game = "vram=(\\d+) MB"
```

**Chapters / markers.** A builder can describe the phases of the game with
regexes on its output (`[phases]`, name → regex; the last line that matches
starts the phase). During the game a field also lets you **mark a phase by
hand** (“Main menu”, “Press A”…). Phases show as colored bands on every
chart, with a per-chapter table in a session's detail, and with a server the
Stats tab compares builders **chapter by chapter**: what matters is the FPS in
game, not in the menu.

```toml
[phases]
"Menu" = 'file opened path=".{1,4}pack.{1,4}core_restofworld'
"Story" = 'file opened path=".{1,4}pack.{1,4}story000\\'
```

In remote mode the read token is enough to send your results, and
**Settings → Performance** has the PC name and an option to send nothing
(sessions then stay on your PC).

## CLI

`oedvi` does what the app does, without a window, with the same settings
(handy over SSH, to debug a builder, or on Linux):

```
oedvi config --mode standalone|remote --server http://host:7878 --token … --game-dir D:\game
oedvi builders                  oedvi git amd-diag pull|resync|fetch|status
oedvi tools amd-diag [--install]
oedvi build amd-diag [--force]  oedvi run amd-diag
oedvi edit builders amd-diag [--put f.toml|--delete]
# remote mode only:
oedvi status [--files]          oedvi sync [--verify]
oedvi select show|all|none|group videos off|path movies/extra on
oedvi server status|rescan
```

## Server API

Every route except `/api/health` needs `Authorization: Bearer <token>`. Writes
need the admin token when one is set.

| Route | |
|---|---|
| `GET /api/health` | version |
| `GET /api/status` | indexing state |
| `GET /api/manifest` | manifest (ETag) |
| `POST /api/rescan` | re-index |
| `GET /files/<path>` | game files (HTTP Range) |
| `GET /api/builders` | resolved builders + errors |
| `GET /api/ids/{builders\|templates}` | file names (broken builders included), for the editor |
| `GET\|PUT\|DELETE /api/{builders,templates}/<id>` | TOML files |
| `POST /api/{builders,templates}/<id>/check` | validate without saving |
| `GET /api/launcher/latest.json` | launcher update (Tauri format) |
| `PUT /api/launcher/files/<name>` | publish an installer |
| `GET /api/stats` | performance compared by builder, commit and chapter, and the latest sessions |
| `POST /api/stats` | send a session (the read token is accepted) |

## Development

```bash
cargo test --workspace                 # tests, no game needed
cargo clippy --workspace --all-targets
cd launcher && npm run check           # types and translations
```

On Linux the launcher needs `libwebkit2gtk-4.1-dev` (see the CI). Adding a
string to the UI: write it in English inside `t("…")` and add its French in
`launcher/src/lib/locales/fr.ts` (`npm run check` fails if one is missing).
Roadmap: [docs/ROADMAP.md](docs/ROADMAP.md).
