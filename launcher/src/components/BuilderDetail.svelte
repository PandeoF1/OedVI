<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api, type BuilderAction, type BuilderDetail } from "../lib/api";
  import { ago, duration } from "../lib/format";
  import { t } from "../lib/i18n.svelte";
  import { app, clearLogs, loadLogs, logsOf } from "../lib/store.svelte";
  import { attempt } from "../lib/toast.svelte";
  import Icon from "./Icon.svelte";
  import LiveCharts from "./LiveCharts.svelte";
  import LogView from "./LogView.svelte";
  import { pointsOf } from "../lib/live.svelte";

  let { id }: { id: string } = $props();

  let d = $state<BuilderDetail | null>(null);
  let error = $state<string | null>(null);
  let showCmds = $state(false);
  let envText = $state("");

  async function load() {
    try {
      d = await api.builderDetail(id);
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    app.taskEpoch; // reload when a task ends
    load();
  });

  $effect(() => {
    loadLogs(id);
    const local = app.settings?.settings.builder_env[id] ?? {};
    envText = Object.entries(local)
      .map(([k, v]) => `${k}=${v}`)
      .join("\n");
  });

  let task = $derived(app.tasks[id]);
  let busy = $derived(!!task);
  let livePoints = $derived(pointsOf(id));
  let logs = $derived(logsOf(id));
  let missingTools = $derived(d?.tools.filter((x) => !x.installed) ?? []);

  async function run(action: BuilderAction) {
    if (action === "resync") {
      const ok = await ask(
        t("Local changes to files tracked by git will be lost and untracked files deleted (target/ and other ignored files are kept)."),
        { title: t("Resync the repository?"), kind: "warning", okLabel: "Resync", cancelLabel: t("Cancel") },
      );
      if (!ok) return;
    }
    await attempt(() => api.builderAction(id, action));
  }

  async function saveEnv() {
    const env: Record<string, string> = {};
    for (const line of envText.split("\n")) {
      const l = line.trim();
      if (!l || l.startsWith("#")) continue;
      const i = l.indexOf("=");
      if (i <= 0) continue;
      env[l.slice(0, i).trim()] = l.slice(i + 1).trim();
    }
    const ok = await attempt(() => api.setBuilderEnv(id, env), t("Local variables saved"));
    if (ok !== undefined && app.settings) {
      app.settings.settings.builder_env[id] = env;
      load();
    }
  }

  function argv(a: string[]): string {
    return a.map((x) => (/\s/.test(x) || x === "" ? `"${x}"` : x)).join(" ");
  }

  const actionLabel: Record<string, string> = $derived({
    tools: t("installing tools"),
    fetch: "fetch",
    pull: "pull",
    resync: "resync",
    build: "build",
    rebuild: "rebuild",
    play: t("playing"),
  });
</script>

{#if error}
  <div class="empty"><Icon name="alert" size={30} /><div class="mono">{error}</div><button onclick={load}>{t("Retry")}</button></div>
{:else if d}
  {@const def = d.def}
  <div class="detail">
    <header>
      <div class="col title">
        <div class="row wrap">
          <h1>{def.name}</h1>
          <span class="badge">{def.stack}</span>
          {#each def.tags as tg}<span class="badge accent">{tg}</span>{/each}
        </div>
        {#if def.description}<div class="muted">{def.description}</div>{/if}
        <div class="row dim mono small wrap">
          <Icon name="branch" size={13} />
          <span class="ellipsis">{def.repo}</span>
          <span>@ {def.ref}</span>
          {#if d.shared_with.length}
            <span class="badge" title={t("These builders use the same clone")}>{t("clone shared with {names}", { names: d.shared_with.join(", ") })}</span>
          {/if}
        </div>
      </div>
      <div class="spacer"></div>
      {#if d.prepared}
        <button class="ghost" onclick={() => api.openPath(d!.prepared!.checkout_dir)} title={t("Open the clone")}><Icon name="folder" /></button>
      {/if}
      {#if app.settings}
        <button class="ghost" onclick={() => app.settings && api.openPath(`${app.settings.layout.logs}/${id}`)} title={t("Logs")}><Icon name="file" /></button>
      {/if}
    </header>

    <div class="actions">
      {#if task?.kind === "run"}
        <button class="danger big" onclick={() => attempt(() => api.stopTask(id))}><Icon name="stop" /> {t("Stop the game")}</button>
      {:else if busy}
        <button class="danger big" onclick={() => attempt(() => api.stopTask(id))}>
          <Icon name="loader" class="spin" /> {actionLabel[task.action] ?? task.action}… {t("Cancel")}
        </button>
      {:else}
        <button class="primary big play" onclick={() => run("play")} disabled={app.syncing} title={app.syncing ? t("Sync in progress") : ""}>
          <Icon name="play" /> {t("Play")}
        </button>
      {/if}
      <div class="sep"></div>
      <button onclick={() => run("pull")} disabled={busy} title="git pull (fast-forward)"><Icon name="pull" /> Pull</button>
      <button onclick={() => run("resync")} disabled={busy} title={t("Puts the repository exactly as on the git server")}><Icon name="sync" /> Resync</button>
      <button onclick={() => run("fetch")} disabled={busy} title={t("Looks for new commits")}><Icon name="refresh" /> Fetch</button>
      <div class="sep"></div>
      <button onclick={() => run("build")} disabled={busy}><Icon name="hammer" /> Build</button>
      <button onclick={() => run("rebuild")} disabled={busy} title={t("Rebuilds even if nothing changed")}><Icon name="zap" /> Rebuild</button>
    </div>

    <div class="cards">
      <section class="card">
        <h3>{t("Repository")}</h3>
        {#if !d.git.cloned}
          <div class="muted">{t("Not cloned yet: Play, Build or Pull will clone it.")}</div>
        {:else}
          {#if d.git.head}
            <div class="commit">
              <span class="mono hash">{d.git.head.short}</span>
              <span class="ellipsis">{d.git.head.subject}</span>
            </div>
            <div class="dim small">{d.git.head.author} · {ago(d.git.head.time)}</div>
          {/if}
          <div class="row wrap">
            <span class="badge">{d.git.branch ?? t("detached")}</span>
            {#if d.git.behind}<span class="badge warn">{t("{n} commit(s) to fetch", { n: d.git.behind })}</span>{:else}<span class="badge ok">{t("up to date")}</span>{/if}
            {#if d.git.ahead}<span class="badge info">{t("{n} local commit(s)", { n: d.git.ahead })}</span>{/if}
            {#if d.git.dirty}<span class="badge err">{t("modified locally")}</span>{/if}
          </div>
          {#if d.git.incoming.length}
            <div class="incoming">
              {#each d.git.incoming as c}
                <div class="commit small"><span class="mono hash">{c.short}</span><span class="ellipsis">{c.subject}</span><span class="dim">{ago(c.time)}</span></div>
              {/each}
            </div>
          {/if}
          <div class="dim small">{t("Fetch to see new commits, Pull to get them.")}</div>
        {/if}
      </section>

      <section class="card">
        <h3>Build</h3>
        {#if d.build}
          <div class="row wrap">
            {#if d.build.up_to_date}
              <span class="badge ok"><Icon name="check" size={12} /> {t("built for this commit")}</span>
            {:else if d.build.last && !d.build.last.ok}
              <span class="badge err">{t("last build failed")}</span>
            {:else}
              <span class="badge warn">{t("needs a build")}</span>
            {/if}
          </div>
          {#if d.build.last}
            <div class="kv">
              <span>{t("Last build")}</span><span>{ago(d.build.last.finished_at)} · {duration(d.build.last.secs)}</span>
              <span>{t("Commit")}</span><span class="mono">{d.build.last.commit.slice(0, 10)}</span>
            </div>
          {/if}
        {:else if d.prepare_error}
          <div class="mono err small">{d.prepare_error}</div>
        {/if}
        <div class="tools">
          {#each d.tools as tool}
            <span
              class="badge {tool.installed ? 'ok' : 'warn'}"
              title={tool.found ?? (tool.installable ? t("missing, can be installed (~{size})", { size: tool.size ?? "" }) : t("missing, install it by hand"))}
            >
              {tool.installed ? "✔" : "✘"} {tool.label}{#if !tool.installed && tool.admin}&nbsp;· admin{/if}
            </span>
          {/each}
        </div>
        {#if missingTools.length}
          <div class="row wrap">
            <span class="dim small">
              {t("{n} tool(s) missing for this builder", { n: missingTools.length })}
              {#if missingTools.some((x) => !x.installable)}
                — {t("install by hand: {names}", { names: missingTools.filter((x) => !x.installable).map((x) => x.label).join(", ") })}
              {:else if app.settings?.settings.auto_install_tools}
                — {t("installed automatically at the first build")}
              {/if}
            </span>
            <div class="spacer"></div>
            {#if missingTools.some((x) => x.installable)}
              <button class="sm" disabled={busy} onclick={() => run("tools")}><Icon name="download" size={13} /> {t("Install the tools")}</button>
            {/if}
          </div>
        {/if}
      </section>
    </div>

    <section class="card">
      <button class="ghost toggle" onclick={() => (showCmds = !showCmds)}>
        <span class="chev" class:open={showCmds}><Icon name="chevron" size={14} /></span>
        <h3>{t("Commands and variables")}</h3>
      </button>
      {#if showCmds && d.prepared}
        <div class="cmds">
          {#each d.prepared.build_steps as s, i}
            <div><span class="dim">build {i + 1}</span><code>{argv(s.argv)}</code></div>
          {/each}
          <div><span class="dim">run</span><code>{argv(d.prepared.run.argv)}</code></div>
          <div><span class="dim">{t("folder")}</span><code>{d.prepared.run.cwd}</code></div>
          {#each Object.entries(d.prepared.run.env) as [k, v]}
            <div><span class="dim">env</span><code>{k}={v}</code></div>
          {/each}
        </div>
        <div class="col env">
          <label for="env-{id}">{t("Local variables (when the game runs only, on this PC) — one per line,")} <code>KEY=value</code></label>
          <textarea id="env-{id}" class="mono" rows="4" bind:value={envText} placeholder="VICTORY_LOG=debug"></textarea>
          <div class="row"><div class="spacer"></div><button class="sm" onclick={saveEnv}><Icon name="save" size={13} /> {t("Save")}</button></div>
        </div>
      {/if}
    </section>

    {#if task?.kind === "run" || livePoints.length}
      <section class="livebox">
        <div class="row">
          <h3>{t("Performance")}</h3>
          <div class="spacer"></div>
          <button class="sm" onclick={() => attempt(() => api.openOverlay(id))} title={t("Small window always on top, next to the game")}>
            <Icon name="chart" size={13} /> {t("Open next to the game")}
          </button>
        </div>
        <LiveCharts {id} running={task?.kind === "run"} />
      </section>
    {/if}

    <LogView lines={logs} height={360} onclear={() => clearLogs(id)} />
  </div>
{:else}
  <div class="empty"><Icon name="loader" class="spin" size={26} /></div>
{/if}

<style>
  .livebox {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .livebox h3 {
    margin: 0;
    font-size: 14px;
  }
  .detail {
    padding: 22px 26px 28px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  header {
    display: flex;
    gap: 12px;
    align-items: flex-start;
  }
  .title {
    gap: 6px;
    min-width: 0;
  }
  .small {
    font-size: 12px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding: 12px;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background:
      radial-gradient(500px 140px at 0% 0%, rgba(124, 92, 255, 0.16), transparent 70%),
      var(--panel);
  }
  .play {
    padding-left: 28px;
    padding-right: 32px;
    font-size: 16px;
  }
  .sep {
    width: 1px;
    height: 26px;
    background: var(--line-2);
    margin: 0 2px;
  }
  .cards {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 14px;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .commit {
    display: flex;
    gap: 9px;
    align-items: baseline;
    min-width: 0;
  }
  .hash {
    color: var(--accent-2);
    flex: none;
  }
  .incoming {
    border-left: 2px solid var(--warn);
    padding-left: 10px;
    display: grid;
    gap: 3px;
    max-height: 150px;
    overflow: auto;
  }
  .tools {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    margin-top: 4px;
  }
  .err {
    color: var(--err);
  }
  .toggle {
    padding: 0;
    gap: 8px;
    align-self: flex-start;
  }
  .toggle:hover:not(:disabled) {
    background: transparent;
  }
  .chev {
    display: inline-flex;
    transition: transform 0.15s;
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .cmds {
    display: grid;
    gap: 5px;
  }
  .cmds div {
    display: grid;
    grid-template-columns: 70px 1fr;
    gap: 10px;
    font-size: 12.5px;
  }
  .cmds code {
    word-break: break-all;
    color: #d4d8e4;
  }
  .env {
    margin-top: 8px;
    gap: 6px;
  }
  textarea {
    resize: vertical;
  }
</style>
