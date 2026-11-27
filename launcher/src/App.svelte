<script lang="ts">
  import { onMount } from "svelte";
  import { app, init, installUpdate, isRemote, type Page } from "./lib/store.svelte";
  import { t } from "./lib/i18n.svelte";
  import { bytes, pct } from "./lib/format";
  import Icon from "./components/Icon.svelte";
  import Toasts from "./components/Toasts.svelte";
  import GamePage from "./components/GamePage.svelte";
  import GameFolderPage from "./components/GameFolderPage.svelte";
  import BuildersPage from "./components/BuildersPage.svelte";
  import EditorPage from "./components/EditorPage.svelte";
  import SettingsPage from "./components/SettingsPage.svelte";
  import StatsPage from "./components/StatsPage.svelte";
  import BackupsPage from "./components/BackupsPage.svelte";

  onMount(() => {
    init();
  });

  const nav: { id: Page; label: string; icon: string }[] = $derived([
    { id: "game", label: t("Game"), icon: "gamepad" },
    { id: "builders", label: t("Builders"), icon: "hammer" },
    { id: "stats", label: t("Stats"), icon: "chart" },
    { id: "backups", label: t("Backups"), icon: "save" },
    { id: "editor", label: t("Editor"), icon: "code" },
    { id: "settings", label: t("Settings"), icon: "settings" },
  ]);

  let running = $derived(Object.keys(app.tasks).length);
  let gameBadge = $derived.by(() => {
    if (app.syncing) return "busy";
    const s = app.status?.summary;
    if (!s) return null;
    return s.up_to_date ? "ok" : "warn";
  });
  let scan = $derived(app.server?.scan);
</script>

<div class="shell">
  <aside>
    <div class="logo">
      <img class="mark" src="/logo.svg" alt="" />
      <div>
        <div class="name">OedVI</div>
        <div class="tag">launcher</div>
      </div>
    </div>

    <nav>
      {#each nav as item}
        <button class="nav" class:active={app.page === item.id} onclick={() => (app.page = item.id)}>
          <Icon name={item.icon} size={18} />
          <span>{item.label}</span>
          <span class="spacer"></span>
          {#if item.id === "game" && gameBadge}
            <span class="dot {gameBadge}"></span>
          {/if}
          {#if item.id === "builders" && running > 0}
            <span class="count">{running}</span>
          {/if}
        </button>
      {/each}
    </nav>

    <div class="spacer"></div>

    {#if app.update?.available}
      <div class="update">
        <div class="row"><Icon name="zap" size={15} /><b>Launcher v{app.update.available}</b></div>
        {#if app.updating}
          <div class="dim small">{t("Downloading")} {bytes(app.updating.downloaded)}{app.updating.total ? ` / ${bytes(app.updating.total)}` : ""}</div>
          <div class="mini"><div style:width="{app.updating.total ? pct(app.updating.downloaded, app.updating.total) : 30}%"></div></div>
        {:else}
          <div class="dim small">{t("You have v{v}", { v: app.update.current })}</div>
          <button class="primary sm" onclick={installUpdate} disabled={app.syncing}>{t("Install and restart")}</button>
        {/if}
      </div>
    {/if}

    {#if isRemote()}
    <div class="server">
      <div class="row">
        <span class="dot {app.server == null ? '' : app.server.online ? (scan?.scanning ? 'busy' : 'ok') : 'err'}"></span>
        <span class="ellipsis">
          {#if app.server == null}
            {t("Connecting…")}
          {:else if app.server.online}
            {t("Server online")}
          {:else}
            {t("Offline")}
          {/if}
        </span>
      </div>
      {#if scan?.scanning}
        <div class="scan">
          {t("Indexing")} {Math.round(pct(scan.bytes_hashed, scan.bytes_to_hash))} %
          <div class="mini"><div style:width="{pct(scan.bytes_hashed, scan.bytes_to_hash)}%"></div></div>
        </div>
      {:else if app.server?.online && scan?.manifest_version}
        <div class="dim small">manifest v{scan.manifest_version}{app.server.version ? ` · ${t("server")} ${app.server.version}` : ""}</div>
      {:else if app.server && !app.server.online}
        <div class="dim small ellipsis" title={app.server.error ?? ""}>{app.server.error}</div>
      {/if}
    </div>
    {/if}
  </aside>

  <main>
    {#if app.page === "game"}
      {#if isRemote()}<GamePage />{:else}<GameFolderPage />{/if}
    {:else if app.page === "builders"}
      <BuildersPage />
    {:else if app.page === "stats"}
      <StatsPage />
    {:else if app.page === "backups"}
      <BackupsPage />
    {:else if app.page === "editor"}
      <EditorPage />
    {:else}
      <SettingsPage />
    {/if}
  </main>
</div>

<Toasts />

<style>
  .shell {
    display: grid;
    grid-template-columns: 224px 1fr;
    height: 100vh;
  }
  aside {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 18px 12px;
    border-right: 1px solid var(--line);
    background: rgba(11, 13, 18, 0.6);
    backdrop-filter: blur(8px);
  }
  .logo {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 4px 8px 18px;
  }
  .mark {
    width: 40px;
    height: 40px;
    filter: drop-shadow(0 6px 18px rgba(255, 46, 136, 0.45));
  }
  .name {
    font-weight: 750;
    font-size: 17px;
    letter-spacing: -0.02em;
  }
  .tag {
    font-size: 11px;
    color: var(--dim);
    text-transform: uppercase;
    letter-spacing: 0.12em;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .nav {
    background: transparent;
    border: 1px solid transparent;
    color: var(--muted);
    padding: 9px 11px;
    border-radius: 10px;
    width: 100%;
  }
  .nav:hover:not(:disabled) {
    background: var(--panel);
    border-color: transparent;
    color: var(--text);
  }
  .nav.active {
    background: var(--panel-2);
    border-color: var(--line-2);
    color: var(--text);
  }
  .nav.active :global(svg) {
    color: var(--accent);
  }
  .count {
    background: var(--accent);
    color: white;
    font-size: 11px;
    font-weight: 700;
    border-radius: 999px;
    padding: 1px 7px;
  }
  .server {
    border: 1px solid var(--line);
    background: var(--panel);
    border-radius: 11px;
    padding: 10px 11px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 13px;
  }
  .small {
    font-size: 11.5px;
  }
  .update {
    border: 1px solid rgba(255, 79, 154, 0.45);
    background: linear-gradient(135deg, rgba(255, 79, 154, 0.14), rgba(255, 159, 67, 0.08));
    border-radius: 11px;
    padding: 10px 11px;
    display: flex;
    flex-direction: column;
    gap: 7px;
    font-size: 13px;
  }
  .update :global(svg) {
    color: #ff9f43;
  }
  .scan {
    font-size: 11.5px;
    color: var(--accent-2);
  }
  .mini {
    margin-top: 4px;
    height: 4px;
    border-radius: 4px;
    background: var(--bg-2);
    overflow: hidden;
  }
  .mini div {
    height: 100%;
    background: var(--accent-2);
    transition: width 0.3s;
  }
  main {
    overflow: hidden;
    min-width: 0;
  }
</style>
