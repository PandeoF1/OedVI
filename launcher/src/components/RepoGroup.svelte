<script lang="ts">
  // The builders of one repository: a header that folds the list, and the
  // choice of the branch, tag or commit the repository is on.
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api, type BuilderRow, type RepoRefs } from "../lib/api";
  import { t } from "../lib/i18n.svelte";
  import { app, refreshBuilders } from "../lib/store.svelte";
  import { attempt } from "../lib/toast.svelte";
  import Icon from "./Icon.svelte";

  let {
    repo,
    rows,
    expanded,
    ontoggle,
    onselect,
  }: { repo: string; rows: BuilderRow[]; expanded: boolean; ontoggle: () => void; onselect: (id: string) => void } = $props();

  const stackColors: Record<string, string> = {
    rust: "#f97316",
    python: "#3b82f6",
    node: "#22c55e",
    dotnet: "#a855f7",
    cmake: "#eab308",
  };

  const first = $derived(rows[0]);
  const current = $derived(first.def.ref);
  const overridden = $derived(current !== first.default_ref);
  const running = $derived(rows.filter((r) => app.tasks[r.def.id]).length);
  const name = $derived(repo.replace(/\.git$/, "").split("/").slice(-2).join("/"));

  let refs = $state<RepoRefs | null>(null);
  let loading = $state(false);
  let custom = $state(false);
  let customText = $state("");

  async function load() {
    loading = true;
    refs = await attempt(() => api.repoRefs(first.def.id)).then((r) => r ?? null);
    loading = false;
  }

  // The lists are fetched when the group opens, not for every repository at start.
  $effect(() => {
    if (expanded && !refs && !loading) load();
  });

  const known = $derived(
    refs ? new Set([refs.default_ref, ...refs.branches, ...refs.tags, ...refs.commits.map((c) => c.hash)]) : new Set<string>(),
  );

  async function choose(value: string) {
    if (value === "__custom__") {
      custom = true;
      customText = overridden ? current : "";
      return;
    }
    custom = false;
    await apply(value === "" ? null : value);
  }

  async function apply(gitRef: string | null) {
    const target = gitRef ?? first.default_ref;
    if (target === current) return;
    if ((await attempt(() => api.setRef(first.def.id, gitRef))) === undefined) return;
    await refreshBuilders(false);
    refs = null;
    // The clone follows with a resync (a checkout that drops local changes).
    if (first.cloned) {
      const go = await ask(
        t("Move the repository to {ref}? Local changes to tracked files will be lost (build output is kept).", { ref: target }),
        { title: t("Switch version?"), kind: "warning", okLabel: t("Switch"), cancelLabel: t("Later") },
      );
      if (go) await attempt(() => api.builderAction(first.def.id, "resync"));
    }
  }
</script>

<div class="group" class:open={expanded}>
  <button class="head" onclick={ontoggle} aria-expanded={expanded}>
    <span class="chev" class:open={expanded}><Icon name="chevron" size={14} /></span>
    <span class="repo ellipsis" title={repo}>{name}</span>
    <span class="ref mono ellipsis" class:custom={overridden} title={overridden ? t("chosen on this PC") : ""}>{current.slice(0, 12)}</span>
    {#if running}<span class="dot busy"></span>{/if}
    <span class="badge">{rows.length}</span>
  </button>

  {#if expanded}
    <div class="ref-row">
      <label for="ref-{first.def.id}">{t("Version")}</label>
      {#if loading && !refs}
        <Icon name="loader" class="spin" size={14} />
      {:else}
        <select
          id="ref-{first.def.id}"
          value={custom ? "__custom__" : overridden ? current : ""}
          onchange={(e) => choose(e.currentTarget.value)}
        >
          <option value="">{t("Default")} ({first.default_ref})</option>
          {#if overridden && refs && !known.has(current)}<option value={current}>{current}</option>{/if}
          {#if refs?.branches.length}
            <optgroup label={t("Branches")}>
              {#each refs.branches as b}<option value={b}>{b}</option>{/each}
            </optgroup>
          {/if}
          {#if refs?.tags.length}
            <optgroup label={t("Tags")}>
              {#each refs.tags as tg}<option value={tg}>{tg}</option>{/each}
            </optgroup>
          {/if}
          {#if refs?.commits.length}
            <optgroup label={t("Recent commits")}>
              {#each refs.commits as c}<option value={c.hash}>{c.short} {c.subject.slice(0, 48)}</option>{/each}
            </optgroup>
          {/if}
          <option value="__custom__">{t("Other (commit hash…)")}</option>
        </select>
        <button class="ghost sm" onclick={load} title={t("Reload the list")} disabled={loading}>
          <Icon name="refresh" size={13} class={loading ? "spin" : ""} />
        </button>
      {/if}
    </div>
    {#if custom}
      <form class="ref-row" onsubmit={(e) => { e.preventDefault(); apply(customText.trim() || null); custom = false; }}>
        <input class="mono" bind:value={customText} placeholder={t("branch, tag or commit")} />
        <button class="sm" type="submit">{t("Apply")}</button>
      </form>
    {/if}
    {#if refs?.error}<div class="dim small err-line" title={refs.error}>{t("Could not list the remote versions")}</div>{/if}
    <div class="rows">
      {#each rows as r (r.def.id)}
        {@const task = app.tasks[r.def.id]}
        <button class="item" class:active={app.selectedBuilder === r.def.id} onclick={() => onselect(r.def.id)}>
          <span class="stack" style:background={stackColors[r.def.stack] ?? "var(--dim)"}></span>
          <div class="col it-text">
            <span class="it-name ellipsis">{r.def.name}</span>
            <span class="it-sub ellipsis">{r.def.id}</span>
          </div>
          {#if task}
            <span class="badge {task.kind === 'run' ? 'ok' : 'info'}">
              <span class="dot busy"></span>{task.kind === "run" ? t("playing") : task.action}
            </span>
          {:else if !r.cloned}
            <span class="dim small">{t("not cloned")}</span>
          {/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .group {
    border: 1px solid var(--line);
    border-radius: 10px;
    background: var(--panel);
    overflow: hidden;
  }
  .head {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: none;
    background: transparent;
    text-align: left;
  }
  .head:hover {
    background: rgba(255, 255, 255, 0.03);
  }
  .chev {
    display: inline-flex;
    transition: transform 0.12s;
    color: var(--dim);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .repo {
    flex: 1;
    min-width: 0;
    font-weight: 600;
    font-size: 13px;
  }
  .ref {
    max-width: 96px;
    font-size: 11px;
    color: var(--muted);
    background: var(--bg-2);
    border-radius: 6px;
    padding: 1px 6px;
  }
  .ref.custom {
    color: var(--accent-2);
    border: 1px solid var(--accent);
  }
  .ref-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px 8px;
    font-size: 12px;
    color: var(--muted);
  }
  .ref-row select,
  .ref-row input {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    padding: 4px 6px;
  }
  .err-line {
    padding: 0 10px 6px;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 6px 6px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 7px 8px;
    border: 1px solid transparent;
    border-radius: 8px;
    background: transparent;
    text-align: left;
  }
  .item:hover {
    background: rgba(255, 255, 255, 0.04);
  }
  .item.active {
    background: rgba(124, 92, 255, 0.14);
    border-color: var(--accent);
  }
  .stack {
    width: 3px;
    align-self: stretch;
    border-radius: 2px;
    background: var(--dim);
  }
  .it-text {
    flex: 1;
    min-width: 0;
  }
  .it-name {
    font-size: 13px;
  }
  .it-sub {
    font-size: 11px;
    color: var(--dim);
  }
</style>
