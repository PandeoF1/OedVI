<script lang="ts">
  import { api } from "../lib/api";
  import { bytes, duration, pct, ago, plural } from "../lib/format";
  import { t } from "../lib/i18n.svelte";
  import { app, refreshGame, refreshStatus, refreshServer } from "../lib/store.svelte";
  import { attempt, toast } from "../lib/toast.svelte";
  import Icon from "./Icon.svelte";
  import Progress from "./Progress.svelte";
  import FileTree from "./FileTree.svelte";

  let s = $derived(app.status?.summary);
  let p = $derived(app.sync);
  let refreshing = $state(false);

  async function refresh() {
    refreshing = true;
    await Promise.all([refreshServer(), refreshGame()]);
    refreshing = false;
  }

  async function sync(verify: boolean) {
    app.lastReport = null;
    const ok = await attempt(() => api.startSync(verify));
    if (ok !== undefined) {
      app.syncing = true;
      toast(verify ? t("Full verification started") : t("Sync started"));
    }
  }

  // Speed over the last few seconds.
  let samples: { t: number; b: number }[] = [];
  let speed = $derived.by(() => {
    if (!p) return 0;
    const now = p.elapsed_secs;
    samples.push({ t: now, b: p.downloaded_bytes });
    samples = samples.filter((x) => now - x.t <= 5);
    const first = samples[0];
    return now > first.t ? (p.downloaded_bytes - first.b) / (now - first.t) : 0;
  });
  let remaining = $derived(p ? Math.max(0, p.download_bytes_needed - p.downloaded_bytes) : 0);

  // Group checkboxes: state computed from the files.
  let groupStates = $derived.by(() => {
    const out: Record<string, { sel: number; total: number; selBytes: number }> = {};
    for (const f of app.status?.files ?? []) {
      if (f.state === "extra") continue;
      const g = (out[f.group] ??= { sel: 0, total: 0, selBytes: 0 });
      g.total++;
      if (f.selected) {
        g.sel++;
        g.selBytes += f.size;
      }
    }
    return out;
  });

  async function toggleGroup(name: string, on: boolean) {
    const st = await attempt(() => api.setSelection("group", name, on));
    if (st) app.status = st;
  }

  async function selectAll(on: boolean) {
    const st = await attempt(() => api.setSelection("all", null, on));
    if (st) app.status = st;
  }

  const phases: Record<string, string> = $derived({
    scan: t("Scanning"),
    remove: t("Removing"),
    sync: t("Verifying and downloading"),
    done: t("Done"),
    cancelled: t("Interrupted"),
    failed: t("Finished with errors"),
  });
</script>

<div class="page">
  <header>
    <div>
      <h1>{t("Game files")}</h1>
      <div class="muted sub">
        {#if app.manifest}
          {t("Manifest v{v}", { v: app.manifest.version })} · {plural(app.manifest.file_count, "{n} file", "{n} files")} · {bytes(app.manifest.total_size)} · {t("published {when}", { when: ago(app.manifest.generated_at) })}
          {#if app.manifest.offline}<span class="badge warn">{t("offline, local copy")}</span>{/if}
        {:else}
          {t("No manifest loaded")}
        {/if}
      </div>
    </div>
    <div class="spacer"></div>
    <button onclick={refresh} disabled={refreshing || app.syncing}>
      <Icon name="refresh" class={refreshing ? "spin" : ""} /> {t("Refresh")}
    </button>
  </header>

  <div class="content">
    {#if app.manifestError && !app.manifest}
      <div class="card empty">
        <Icon name="server" size={36} />
        <h2>{t("Cannot reach the server")}</h2>
        <div class="mono err">{app.manifestError}</div>
        <div class="row">
          <button onclick={refresh}><Icon name="refresh" /> {t("Retry")}</button>
          <button class="primary" onclick={() => (app.page = "settings")}><Icon name="settings" /> {t("Settings")}</button>
        </div>
      </div>
    {:else if app.status && s}
      {#if app.server?.scan?.scanning}
        <div class="banner">
          <Icon name="loader" class="spin" />
          {t("The server is indexing its files ({pct} %). The manifest will be updated when it finishes; you can already sync the current one.", { pct: Math.round(pct(app.server.scan.bytes_hashed, app.server.scan.bytes_to_hash)) })}
        </div>
      {/if}
      {#if s.synced_version != null && s.synced_version < s.manifest_version && !app.syncing}
        <div class="banner update">
          <Icon name="zap" /> {t("Update available: v{from} → v{to}", { from: s.synced_version, to: s.manifest_version })}
        </div>
      {/if}

      <section class="hero card">
        <div class="hero-main">
          {#if app.syncing && p}
            <div class="state busy">
              <Icon name="loader" size={26} class="spin" />
              <div>
                <div class="big">{phases[p.phase] ?? p.phase}</div>
                <div class="muted">
                  {p.files_done}/{p.files_total} {t("files")} · {bytes(p.downloaded_bytes)} / {bytes(p.download_bytes_needed)}
                  · {bytes(speed)}/s
                  {#if speed > 0 && remaining > 0}· {t("{time} left", { time: duration(remaining / speed) })}{/if}
                </div>
              </div>
            </div>
          {:else if s.up_to_date}
            <div class="state ok">
              <Icon name="shield" size={28} />
              <div>
                <div class="big">{t("Everything is up to date")}</div>
                <div class="muted">{plural(s.ok_files, "{n} file", "{n} files")} · {t("{size} on disk", { size: bytes(s.ok_bytes) })}</div>
              </div>
            </div>
          {:else}
            <div class="state warn">
              <Icon name="download" size={28} />
              <div>
                <div class="big">{t("Sync needed")}</div>
                <div class="muted">
                  {[
                    s.missing && t("{n} missing", { n: s.missing }),
                    s.partial && t("{n} partial", { n: s.partial }),
                    s.outdated && t("{n} outdated", { n: s.outdated }),
                    s.unverified && t("{n} to verify", { n: s.unverified }),
                    s.to_remove && t("{n} to delete", { n: s.to_remove }),
                    s.extra && app.settings?.settings.delete_extra && t("{n} extra", { n: s.extra }),
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </div>
              </div>
            </div>
          {/if}
          <div class="spacer"></div>
          {#if app.syncing}
            <button class="danger big" onclick={() => attempt(() => api.cancelSync())}><Icon name="stop" /> {t("Cancel")}</button>
          {:else}
            <button onclick={() => sync(true)} title={t("Re-reads every file and repairs what does not match")}>
              <Icon name="shield" /> {t("Verify all")}
            </button>
            <button class="primary big" onclick={() => sync(false)} disabled={s.up_to_date}>
              <Icon name="download" /> {t("Sync")}
            </button>
          {/if}
        </div>

        {#if app.syncing && p}
          <div class="col bars">
            <div class="bar-label"><span>{t("Download")}</span><span class="muted">{Math.round(pct(p.downloaded_bytes, p.download_bytes_needed))} %</span></div>
            <Progress value={pct(p.downloaded_bytes, p.download_bytes_needed)} indeterminate={p.phase === "scan"} />
            {#if p.verify_bytes_total > 0}
              <div class="bar-label"><span>{t("Verifying existing files")}</span><span class="muted">{bytes(p.verified_bytes)} / {bytes(p.verify_bytes_total)}</span></div>
              <Progress value={pct(p.verified_bytes, p.verify_bytes_total)} tone="info" height={5} />
            {/if}
            {#if p.current.length}
              <div class="current mono">
                {#each p.current.slice(0, 6) as c}<div class="ellipsis">{c}</div>{/each}
              </div>
            {/if}
          </div>
        {/if}

        <div class="stats">
          <div><span class="lbl">{t("Selection")}</span><b>{bytes(s.selected_bytes)}</b><span class="dim">{plural(s.selected_files, "{n} file", "{n} files")}</span></div>
          <div><span class="lbl">{t("Up to date")}</span><b>{bytes(s.ok_bytes)}</b><span class="dim">{pct(s.ok_bytes, s.selected_bytes).toFixed(0)} %</span></div>
          <div><span class="lbl">{t("To download (max)")}</span><b>{bytes(s.download_max_bytes)}</b><span class="dim">{t("only the chunks that differ")}</span></div>
          <div>
            <span class="lbl">{t("Free space")}</span><b class:err={s.free_space != null && s.free_space < s.download_max_bytes}>{bytes(s.free_space)}</b>
            <button class="ghost sm" onclick={() => app.settings && api.openPath(app.settings.layout.game)}><Icon name="folder" size={13} /> {t("Open")}</button>
          </div>
        </div>

        {#if (p?.errors.length ?? 0) > 0 || (app.lastReport?.errors.length ?? 0) > 0}
          <div class="errors">
            {#each (app.syncing ? p?.errors : app.lastReport?.errors) ?? [] as e}
              <div><span class="mono">{e.path}</span> — {e.message}</div>
            {/each}
          </div>
        {/if}
      </section>

      <div class="grid">
        <section class="card groups">
          <div class="row">
            <h3>{t("Groups")}</h3>
            <div class="spacer"></div>
            <button class="ghost sm" disabled={app.syncing} onclick={() => selectAll(true)}>{t("All")}</button>
            <button class="ghost sm" disabled={app.syncing} onclick={() => selectAll(false)}>{t("Minimum")}</button>
          </div>
          {#each app.manifest?.groups ?? [] as g}
            {@const st = groupStates[g.name] ?? { sel: 0, total: 0, selBytes: 0 }}
            <label class="group" class:locked={g.required}>
              <input
                type="checkbox"
                checked={st.sel === st.total && st.total > 0}
                indeterminate={st.sel > 0 && st.sel < st.total}
                disabled={g.required || app.syncing}
                onchange={(e) => toggleGroup(g.name, e.currentTarget.checked)}
              />
              <div class="col g-text">
                <span class="g-name ellipsis">{g.label ?? g.name}</span>
                <span class="dim ellipsis">
                  {#if g.required}<span class="req">{t("required")}</span>&nbsp;·&nbsp;{/if}{g.name} · {plural(g.file_count, "{n} file", "{n} files")}
                </span>
              </div>
              <span class="g-size">{bytes(st.selBytes)}<span class="dim"> / {bytes(g.size)}</span></span>
            </label>
          {/each}
        </section>

        <section class="card tree-card">
          <FileTree files={app.status.files} disabled={app.syncing} onchange={(st) => (app.status = st)} />
        </section>
      </div>
    {:else}
      <div class="empty"><Icon name="loader" class="spin" size={28} /> {t("Loading…")}</div>
    {/if}
  </div>
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
  .sub {
    margin-top: 4px;
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .content {
    flex: 1;
    overflow: auto;
    padding: 4px 28px 28px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .banner {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 10px 14px;
    border-radius: 10px;
    border: 1px solid rgba(34, 211, 238, 0.35);
    background: rgba(34, 211, 238, 0.07);
    color: #a5f3fc;
  }
  .banner.update {
    border-color: rgba(124, 92, 255, 0.45);
    background: rgba(124, 92, 255, 0.1);
    color: #ddd6fe;
  }
  .hero {
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding: 20px;
    background:
      radial-gradient(600px 200px at 0% 0%, rgba(124, 92, 255, 0.14), transparent 70%),
      var(--panel);
  }
  .hero-main {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .state {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .state.ok :global(svg) {
    color: var(--ok);
  }
  .state.warn :global(svg) {
    color: var(--warn);
  }
  .state.busy :global(svg) {
    color: var(--accent-2);
  }
  .big {
    font-size: 19px;
    font-weight: 700;
    margin-bottom: 3px;
  }
  .bars {
    gap: 7px;
  }
  .bar-label {
    display: flex;
    justify-content: space-between;
    font-size: 12.5px;
  }
  .current {
    color: var(--muted);
    font-size: 11.5px;
    display: grid;
    gap: 2px;
    margin-top: 4px;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 10px;
  }
  .stats > div {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 11px 13px;
    border-radius: 10px;
    background: var(--bg-2);
    border: 1px solid var(--line);
    align-items: flex-start;
  }
  .lbl {
    color: var(--muted);
    font-size: 12.5px;
  }
  .stats b {
    font-size: 17px;
  }
  .stats .dim {
    font-size: 12px;
  }
  .err {
    color: var(--err);
  }
  .errors {
    max-height: 140px;
    overflow: auto;
    font-size: 12.5px;
    color: #fecaca;
    background: rgba(248, 113, 113, 0.07);
    border: 1px solid rgba(248, 113, 113, 0.3);
    border-radius: 9px;
    padding: 8px 11px;
    display: grid;
    gap: 3px;
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(280px, 360px) 1fr;
    gap: 14px;
    align-items: start;
    min-height: 0;
  }
  .groups {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 9px 10px;
    border-radius: 9px;
    color: var(--text);
    font-size: 13.5px;
    cursor: pointer;
    border: 1px solid transparent;
  }
  .group:hover {
    background: var(--panel-2);
    border-color: var(--line);
  }
  .group.locked {
    cursor: default;
  }
  .g-text {
    gap: 1px;
    min-width: 0;
    flex: 1;
  }
  .req {
    color: #c4b5fd;
  }
  .g-name {
    font-weight: 600;
  }
  .g-text .dim {
    font-size: 11.5px;
  }
  .g-size {
    font-size: 12.5px;
    white-space: nowrap;
    text-align: right;
  }
  .tree-card {
    padding: 0;
    overflow: hidden;
  }
</style>
