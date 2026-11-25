<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, type Settings } from "../lib/api";
  import { ago, bytes, duration, pct } from "../lib/format";
  import { app, checkUpdate, connect, installUpdate, isRemote, refreshServer } from "../lib/store.svelte";
  import { LANGS, lang, setLang, t, type Lang } from "../lib/i18n.svelte";
  import { attempt, toast } from "../lib/toast.svelte";
  import Icon from "./Icon.svelte";
  import Progress from "./Progress.svelte";

  let form = $state<Settings | null>(null);
  let showToken = $state(false);
  let testing = $state(false);

  $effect(() => {
    if (app.settings && !form) form = structuredClone($state.snapshot(app.settings.settings));
  });

  let dirty = $derived(!!form && !!app.settings && JSON.stringify(form) !== JSON.stringify(app.settings.settings));

  async function pick(field: "root_dir" | "game_dir") {
    if (!form) return;
    const dir = await open({ directory: true, defaultPath: (field === "game_dir" ? form.game_dir : form.root_dir) ?? undefined });
    if (typeof dir === "string") form[field] = dir;
  }

  async function save(): Promise<boolean> {
    if (!form) return false;
    const r = await attempt(() => api.saveSettings($state.snapshot(form!) as Settings));
    if (!r) return false;
    app.settings = r;
    form = structuredClone(r.settings);
    return true;
  }

  async function saveAndTest() {
    testing = true;
    if (await save()) {
      await connect();
      if (!isRemote()) {
        toast(t("Settings saved"), "success");
      } else if (app.server?.online) {
        toast(t("Connected to the server ✔"), "success");
      } else {
        toast(t("Server unreachable: {error}", { error: app.server?.error ?? "" }), "error");
      }
    }
    testing = false;
  }

  let scan = $derived(app.server?.scan);
</script>

<div class="page">
  <header>
    <h1>{t("Settings")}</h1>
    <div class="spacer"></div>
    {#if dirty}<span class="badge warn">{t("not saved")}</span>{/if}
    <button class="primary" onclick={saveAndTest} disabled={testing || app.syncing}>
      <Icon name={testing ? "loader" : "save"} class={testing ? "spin" : ""} /> {form?.mode === "remote" ? t("Save and test") : t("Save")}
    </button>
  </header>

  {#if form}
    <div class="content">
      <section class="card">
        <h3>{t("Language")}</h3>
        <div class="row">
          <select value={lang()} onchange={(e) => setLang(e.currentTarget.value as Lang)}>
            {#each LANGS as l}<option value={l.id}>{l.label}</option>{/each}
          </select>
        </div>
      </section>

      <section class="card">
        <h3>{t("Mode")}</h3>
        <label class="toggle">
          <input type="radio" name="mode" value="standalone" bind:group={form.mode} />
          <div>
            <div class="t">{t("Standalone")}</div>
            <div class="hint">
              {t("Everything stays on this PC: you point the launcher to your copy of the game, the builders are files you edit here, and nothing is downloaded.")}
            </div>
          </div>
        </label>
        <label class="toggle">
          <input type="radio" name="mode" value="remote" bind:group={form.mode} />
          <div>
            <div class="t">{t("Connected to a server")}</div>
            <div class="hint">
              {t("An OedVI server publishes the game files (downloaded and verified here), the builders, launcher updates and the performance comparison.")}
            </div>
          </div>
        </label>
      </section>

      {#if form.mode === "remote"}
      <section class="card">
        <h3>{t("Server")}</h3>
        <div class="field">
          <label for="url">{t("Address")}</label>
          <input id="url" class="mono" bind:value={form.server_url} placeholder="http://my-server:7878" />
          <span class="hint">{t("The address of your OedVI server. Ideally through a VPN or with an IP allow-list rather than open to everyone.")}</span>
        </div>
        <div class="field">
          <label for="tok">Token</label>
          <div class="row">
            {#if showToken}
              <input id="tok" class="mono grow" bind:value={form.token} />
            {:else}
              <input id="tok" class="mono grow" type="password" bind:value={form.token} />
            {/if}
            <button onclick={() => (showToken = !showToken)}>{showToken ? t("Hide") : t("Show")}</button>
          </div>
        </div>
        {#if app.server}
          <div class="status">
            <span class="dot {app.server.online ? 'ok' : 'err'}"></span>
            {#if app.server.online}
              {t("Online")}{app.server.version ? ` · v${app.server.version}` : ""}
              {#if scan}
                · {t("manifest v{v}", { v: scan.manifest_version ?? "—" })} · {t("last indexing {when}", { when: ago(scan.last_scan_at) })}
                {#if scan.last_scan_secs != null}({duration(scan.last_scan_secs)}){/if}
              {/if}
            {:else}
              <span class="mono err">{app.server.error}</span>
            {/if}
            <div class="spacer"></div>
            {#if app.server.online}
              <button class="sm" onclick={() => attempt(async () => { await api.serverRescan(); await refreshServer(); }, t("Reindexing requested"))} disabled={scan?.scanning}>
                <Icon name="refresh" size={13} /> {t("Reindex")}
              </button>
            {/if}
          </div>
          {#if scan?.scanning}
            <div class="col">
              <div class="muted small">
                {t("Indexing")}: {scan.files_done}/{scan.files_total} {t("files")} · {bytes(scan.bytes_hashed)} / {bytes(scan.bytes_to_hash)} {t("to hash")}
                {#if scan.current}<span class="mono"> · {scan.current}</span>{/if}
              </div>
              <Progress value={pct(scan.bytes_hashed, scan.bytes_to_hash)} tone="info" height={6} />
            </div>
          {/if}
          {#if scan?.last_error}<div class="mono err small">{scan.last_error}</div>{/if}
        {/if}
      </section>
      {/if}

      <section class="card">
        <h3>{t("Folders")}</h3>
        <div class="field">
          <label for="game">{t("Game files")}</label>
          <div class="row">
            <input id="game" class="mono grow" bind:value={form.game_dir} placeholder={app.settings ? app.settings.layout.root + "/game" : ""} />
            <button onclick={() => pick("game_dir")}><Icon name="folder" /> {t("Choose")}</button>
            {#if form.game_dir}<button class="ghost" onclick={() => form && (form.game_dir = null)}>{t("Default")}</button>{/if}
          </div>
          <span class="hint">{form.mode === "remote" ? t("Pick a drive with enough space. Changing the folder does not move files already downloaded.") : t("The folder that holds your copy of the game: the builders find it as {game_dir}.")}</span>
        </div>
        <div class="field">
          <label for="root">{t("Working folder (clones, builds, cache, saves, logs, builders)")}</label>
          <div class="row">
            <input id="root" class="mono grow" bind:value={form.root_dir} />
            <button onclick={() => pick("root_dir")}><Icon name="folder" /> {t("Choose")}</button>
          </div>
        </div>
        {#if app.settings}
          <div class="paths">
            {#each Object.entries(app.settings.layout) as [k, v]}
              <button class="ghost path" onclick={() => api.openPath(v)} title={t("Open")}>
                <Icon name="folder" size={13} /><span class="k">{k}</span><span class="mono ellipsis">{v}</span>
              </button>
            {/each}
          </div>
        {/if}
      </section>

      {#if form.mode === "remote"}
      <section class="card">
        <h3>{t("Download")}</h3>
        <div class="field">
          <label for="jobs">{t("Parallel downloads (manifest chunks)")}</label>
          <div class="row">
            <input id="jobs" type="range" min="1" max="16" bind:value={form.download_jobs} class="grow" />
            <b class="num">{form.download_jobs}</b>
          </div>
          <span class="hint">{t("6 suits most connections. Raise it if the bandwidth is not saturated.")}</span>
        </div>
        <label class="toggle">
          <input type="checkbox" bind:checked={form.delete_extra} />
          <div>
            <div class="t">{t("Delete files the server no longer publishes")}</div>
            <div class="hint">{t("The game folder stays an exact mirror of the server (minus the files you unticked).")}</div>
          </div>
        </label>
      </section>
      {/if}

      <section class="card">
        <h3>{t("Launcher")}</h3>
        <div class="row wrap">
          <span>{t("Version")} {app.update?.current ?? "…"}</span>
          {#if app.update?.disabled}
            <span class="badge warn">{app.update.disabled}</span>
          {:else if app.update?.available}
            <span class="badge accent">{t("v{v} available", { v: app.update.available })}</span>
          {:else if app.update}
            <span class="badge ok">{t("up to date")}</span>
          {/if}
          <div class="spacer"></div>
          <button class="sm" onclick={checkUpdate}><Icon name="refresh" size={13} /> {t("Check")}</button>
          {#if app.update?.available}
            <button class="primary sm" onclick={installUpdate} disabled={!!app.updating || app.syncing}>{t("Install and restart")}</button>
          {/if}
        </div>
        {#if app.update?.notes}<div class="notes">{app.update.notes}</div>{/if}
        {#if app.updateError || app.update?.error}<div class="mono err small">{app.updateError ?? app.update?.error}</div>{/if}
        <span class="hint">{isRemote() ? t("New versions are published by the server (its launcher/ folder) and checked against the signature built into the launcher.") : t("New versions come from the GitHub releases and are checked against the signature built into the launcher.")}</span>
      </section>

      <section class="card">
        <h3>{t("Build tools")}</h3>
        <label class="toggle">
          <input type="checkbox" bind:checked={form.auto_install_tools} />
          <div>
            <div class="t">{t("Install missing tools automatically")}</div>
            <div class="hint">
              {t("Before a build, the launcher installs with winget only what the builder is missing (git, Rust, Visual Studio Build Tools for Rust/CMake, Python, Node, .NET, CMake…). What is already installed is never touched.")}
            </div>
          </div>
        </label>
      </section>

      <section class="card">
        <h3>{t("Performance")}</h3>
        {#if form.mode === "remote"}
        <label class="toggle">
          <input type="checkbox" bind:checked={form.share_stats} />
          <div>
            <div class="t">{t("Send my performance to the server")}</div>
            <div class="hint">
              {t("When the game closes, the averages, minimums and curves of what it measured (FPS, CPU, GPU, memory…) and this PC's specs are sent to compare builders and versions. Only those numbers and the name below leave. Results are always kept on this PC.")}
            </div>
          </div>
        </label>
        {/if}
        <div class="field">
          <label for="machine">{t("Name of this PC in the comparison")}</label>
          <input id="machine" bind:value={form.machine_name} placeholder={t("My PC (CPU / GPU)")} />
        </div>
      </section>

      <div class="dim small">{t("Settings file:")} <span class="mono">{app.settings?.path}</span> {t("(shared with the oedvi CLI)")}</div>
    </div>
  {/if}
</div>

<style>
  .page {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 22px 28px 14px;
  }
  .content {
    flex: 1;
    overflow: auto;
    padding: 4px 28px 28px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: 900px;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .grow {
    flex: 1;
  }
  .hint {
    font-size: 12px;
    color: var(--dim);
  }
  .status {
    display: flex;
    align-items: center;
    gap: 9px;
    font-size: 13px;
    padding: 10px 12px;
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: 10px;
  }
  .err {
    color: var(--err);
  }
  .small {
    font-size: 12px;
  }
  .paths {
    display: grid;
    gap: 2px;
  }
  .path {
    justify-content: flex-start;
    font-size: 12.5px;
    padding: 5px 8px;
  }
  .k {
    width: 52px;
    color: var(--muted);
    text-align: left;
  }
  .num {
    width: 28px;
    text-align: right;
  }
  input[type="range"] {
    accent-color: var(--accent);
    padding: 0;
    border: none;
    background: transparent;
    box-shadow: none;
  }
  .toggle {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    color: var(--text);
    font-size: 13.5px;
    cursor: pointer;
  }
  .toggle input {
    margin-top: 2px;
  }
  .t {
    margin-bottom: 2px;
  }
  .notes {
    white-space: pre-wrap;
    font-size: 13px;
    color: var(--muted);
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: 9px;
    padding: 9px 11px;
  }
</style>
