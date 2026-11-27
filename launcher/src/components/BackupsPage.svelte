<script lang="ts">
  // Backups of the game's saves (the {data_dir} folder).
  import { api, type Backup, type BackupsView, type Settings } from "../lib/api";
  import { app } from "../lib/store.svelte";
  import { t } from "../lib/i18n.svelte";
  import { attempt, toast } from "../lib/toast.svelte";
  import { ago, bytes } from "../lib/format";
  import Icon from "./Icon.svelte";

  let view = $state<BackupsView | null>(null);
  let label = $state("");
  let busy = $state<string | null>(null);
  let confirming = $state<string | null>(null);

  async function load() {
    view = await api.backupsList().catch(() => null);
  }
  $effect(() => {
    app.settings;
    load();
  });

  async function create() {
    busy = "create";
    const r = await attempt(() => api.backupCreate(label), t("Backup created"));
    busy = null;
    if (r) label = "";
    load();
  }
  async function verify(b: Backup) {
    busy = b.id;
    const problems = await attempt(() => api.backupVerify(b.id));
    busy = null;
    if (!problems) return;
    if (problems.length === 0) toast(t("Backup verified: all {n} files are intact", { n: b.files.length }), "success");
    else toast(t("Damaged backup: {p}", { p: problems.join("; ") }), "error");
  }
  async function restore(b: Backup) {
    confirming = null;
    busy = b.id;
    await attempt(() => api.backupRestore(b.id), t("Backup restored (the previous saves were backed up first)"));
    busy = null;
    load();
  }
  async function remove(b: Backup) {
    busy = b.id;
    await attempt(() => api.backupDelete(b.id));
    busy = null;
    load();
  }
  async function option(patch: Partial<Settings>) {
    if (!app.settings) return;
    const next: Settings = { ...$state.snapshot(app.settings.settings), ...patch };
    const r = await attempt(() => api.saveSettings(next));
    if (r) app.settings = r;
  }
  function when(secs: number) {
    return new Date(secs * 1000).toLocaleString();
  }
</script>

<div class="page">
  <header>
    <h1>{t("Backups")}</h1>
  </header>
  <div class="content">
    <section class="card">
      <h3>{t("Saves")}</h3>
      <p class="muted">
        {t("A backup is a verified copy of the folder the games keep their saves in. Restoring one first backs up the current saves, so nothing is lost.")}
      </p>
      <div class="row path">
        <Icon name="folder" size={16} />
        <span class="mono grow ellipsis">{view?.data ?? "…"}</span>
        {#if view}<button onclick={() => api.openPath(view!.data)}>{t("Open")}</button>{/if}
      </div>
      <div class="row">
        <input class="grow" placeholder={t("Name (optional)")} maxlength="80" bind:value={label} />
        <button class="primary" onclick={create} disabled={busy !== null}>
          <Icon name="save" /> {t("Back up now")}
        </button>
      </div>
      {#if app.settings}
        <label class="toggle">
          <input
            type="checkbox"
            checked={app.settings.settings.backup_before_play}
            onchange={(e) => option({ backup_before_play: e.currentTarget.checked })}
          />
          <div>
            <div class="t">{t("Back up before each game start")}</div>
            <div class="hint">
              {t("Automatic backups are pruned: only the newest ones are kept. Backups you make by hand are never removed.")}
            </div>
          </div>
        </label>
        <div class="row">
          <span class="muted">{t("Automatic backups kept")}</span>
          <input
            type="number"
            min="1"
            max="100"
            class="num"
            value={app.settings.settings.backup_keep}
            onchange={(e) => option({ backup_keep: Math.max(1, Math.min(100, +e.currentTarget.value || 10)) })}
          />
        </div>
      {/if}
    </section>

    <section class="card">
      <div class="row">
        <h3 class="grow">{t("Backups")} {view ? `(${view.backups.length})` : ""}</h3>
        {#if view}<button onclick={() => api.openPath(view!.dir)}>{t("Open")}</button>{/if}
      </div>
      {#if view && view.backups.length === 0}
        <p class="muted">{t("No backup yet.")}</p>
      {/if}
      {#each view?.backups ?? [] as b (b.id)}
        <div class="item">
          <div class="grow">
            <div class="title">
              {b.label || when(b.created_at)}
              {#if b.auto}<span class="badge info">{t("automatic")}</span>{/if}
            </div>
            <div class="muted small">
              {when(b.created_at)} · {ago(b.created_at)} · {t("{n} files", { n: b.files.length })} · {bytes(b.bytes)}
            </div>
          </div>
          {#if confirming === b.id}
            <span class="muted small">{t("Replace the current saves?")}</span>
            <button class="danger" onclick={() => restore(b)} disabled={busy !== null}>{t("Yes, restore")}</button>
            <button onclick={() => (confirming = null)}>{t("Cancel")}</button>
          {:else}
            <button onclick={() => verify(b)} disabled={busy !== null}><Icon name="shield" /> {t("Verify")}</button>
            <button onclick={() => (confirming = b.id)} disabled={busy !== null}><Icon name="refresh" /> {t("Restore")}</button>
            <button class="danger" onclick={() => remove(b)} disabled={busy !== null} title={t("Delete")}><Icon name="trash" /></button>
          {/if}
        </div>
      {/each}
    </section>
  </div>
</div>

<style>
  .page {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  header {
    padding: 22px 28px 10px;
  }
  h1 {
    margin: 0;
    font-size: 20px;
  }
  .content {
    padding: 8px 28px 28px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    overflow: auto;
  }
  .card {
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 16px 18px;
    background: var(--panel);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .card h3 {
    margin: 0;
    font-size: 14px;
  }
  .card p {
    margin: 0;
    font-size: 13px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .path {
    padding: 8px 10px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: #0d1018;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .num {
    width: 80px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border: 1px solid var(--line);
    border-radius: 8px;
  }
  .title {
    font-weight: 600;
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .small {
    font-size: 12px;
  }
</style>
