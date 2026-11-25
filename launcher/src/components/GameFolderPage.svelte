<script lang="ts">
  // Standalone mode: the game is a folder the user already has. Nothing is
  // downloaded; the builders find it as {game_dir}.
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, type Settings } from "../lib/api";
  import { app } from "../lib/store.svelte";
  import { t } from "../lib/i18n.svelte";
  import { attempt } from "../lib/toast.svelte";
  import Icon from "./Icon.svelte";

  let info = $state<{ path: string; exists: boolean; entries: number } | null>(null);

  async function load() {
    info = await api.gameFolderInfo().catch(() => null);
  }
  $effect(() => {
    app.settings;
    load();
  });

  async function choose() {
    if (!app.settings) return;
    const dir = await open({ directory: true, defaultPath: info?.path });
    if (typeof dir !== "string") return;
    const next: Settings = { ...$state.snapshot(app.settings.settings), game_dir: dir };
    const r = await attempt(() => api.saveSettings(next));
    if (r) app.settings = r;
  }
</script>

<div class="page">
  <header>
    <h1>{t("Game")}</h1>
  </header>
  <div class="content">
    <section class="card">
      <h3>{t("Game folder")}</h3>
      <p class="muted">
        {t("Point the launcher to your copy of the game. Builders find it as {game_dir}; nothing is downloaded or changed in this folder.")}
      </p>
      <div class="row path">
        <Icon name="folder" size={16} />
        <span class="mono grow ellipsis">{info?.path ?? "…"}</span>
        {#if info}
          {#if !info.exists}
            <span class="badge err">{t("folder not found")}</span>
          {:else if info.entries === 0}
            <span class="badge warn">{t("folder is empty")}</span>
          {:else}
            <span class="badge ok">{t("{n} entries", { n: info.entries })}</span>
          {/if}
        {/if}
      </div>
      <div class="row">
        <button class="primary" onclick={choose}><Icon name="folder" /> {t("Choose the game folder")}</button>
        {#if info?.exists}
          <button onclick={() => info && api.openPath(info.path)}>{t("Open")}</button>
        {/if}
      </div>
    </section>

    <section class="card">
      <h3>{t("Standalone mode")}</h3>
      <p class="muted">
        {t("Builders are files on this PC (Editor tab); the ones shipped with the launcher were copied there on first start. To download the game and the builders from an OedVI server, connect to one in Settings.")}
      </p>
      <button onclick={() => (app.page = "settings")}><Icon name="settings" /> {t("Settings")}</button>
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
</style>
