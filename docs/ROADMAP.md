# OedVI roadmap

## Done

**Standalone and remote**
- [x] Standalone mode by default: your own game folder, builders as editable
      files (a few ship with the launcher), nothing downloaded.
- [x] Remote mode: connect to an OedVI server for game files, builders,
      launcher updates and the performance comparison.
- [x] English interface by default, French available (Settings → Language).

**Server**
- [x] BLAKE3 indexing by 32 MB chunks, multithreaded, with a persistent index
      (only modified files are re-hashed) and a save during the scan (resume
      after a cut).
- [x] Versioned manifest: the version only grows when the content changes. The
      last manifest is served during a scan.
- [x] Game folder watcher with debounce and an optional periodic rescan.
- [x] File groups (glob patterns, `required` files) and ignored files.
- [x] Files served with HTTP Range; read token and admin token.
- [x] Builders and templates in TOML: `extends`, `vars`, shared `checkout`,
      full validation and a copy kept in `.trash/`.
- [x] Launcher update publishing (Tauri format, upload from CI).
- [x] Performance reports from the launchers, compared by builder, commit and
      chapter.
- [x] Dockerfile and docker-compose.

**Client** (library shared by the app and the CLI)
- [x] Chunked sync: `.part` resume, in-place repair, parallel downloads,
      retries, disk space check, cancellation.
- [x] Rule-based selection (group, folder, file): new files follow their
      folder.
- [x] Deletion of unticked or server-removed files, and of empty folders.
- [x] Git through the `git` binary: clone, fetch, ff-only pull, resync
      (reset + clean without touching ignored files), incoming commits.
- [x] Builds: tool check, hashed recipe, rebuild only if the commit, the
      recipe or the output changed; live and on-disk logs; process tree kill.
- [x] Build tools installed on demand (winget): only the running builder's
      and only if missing (Git, Build Tools, Rust, Python, uv, Node, .NET,
      CMake, Ninja), PATH reloaded; winget itself installed if missing.
- [x] Live performance monitoring (game output + CPU/RAM/disk/GPU), PC specs,
      phases of the game (from the output or by hand), per-chapter stats.
- [x] `oedvi` CLI.

**Launcher** (Tauri 2 + Svelte 5)
- [x] Game page, Builders page, builder editor, Stats page with explorable
      charts and comparison, always-on-top live window, Settings.
- [x] Signed automatic updates.
- [x] CI/CD with configurable runners.

## Next: comfort

- [ ] Tray icon and Windows notification at the end of a sync or a build.
- [ ] Bandwidth limit, pause and resume of the sync.
- [ ] Favorite builder and a Play button on the Game page.
- [ ] Build history per builder (duration, commit, result) and links to old
      logs.
- [ ] Pick a commit or branch on the fly (manual bisect without touching the
      server).
- [ ] TOML highlighting and placeholder completion in the editor.
- [ ] Measure the frame rate of games that print no counter (PresentMon).

## Later

- [ ] Content-defined delta (variable-size chunking, casync style) for
      archives that shift instead of being modified in place.
- [ ] Builds on the server or in CI, with downloadable artifacts instead of
      compiling on the PC.
- [ ] Builders with parameters in the UI (pick `features` and `console`
      before launching).
- [ ] Several games per server.
- [ ] CLI and server updates from the releases.
