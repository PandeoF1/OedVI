<script lang="ts">
  import { app, isRemote, refreshBuilders } from "../lib/store.svelte";
  import { t } from "../lib/i18n.svelte";
  import Icon from "./Icon.svelte";
  import BuilderDetail from "./BuilderDetail.svelte";
  import RepoGroup from "./RepoGroup.svelte";

  let query = $state("");
  let tag = $state<string | null>(null);
  let refreshing = $state(false);

  let rows = $derived(app.builders?.builders ?? []);
  let tags = $derived([...new Set(rows.flatMap((r) => r.def.tags))].sort());
  let shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return rows.filter(
      (r) =>
        (!tag || r.def.tags.includes(tag)) &&
        (!q ||
          r.def.name.toLowerCase().includes(q) ||
          r.def.id.toLowerCase().includes(q) ||
          (r.def.description ?? "").toLowerCase().includes(q)),
    );
  });

  // Builders of the same clone, under their repository.
  let groups = $derived.by(() => {
    const m = new Map<string, { key: string; repo: string; rows: typeof shown }>();
    for (const r of shown) {
      const key = r.def.checkout ?? r.def.id;
      const g = m.get(key) ?? m.set(key, { key, repo: r.def.repo, rows: [] }).get(key)!;
      g.rows.push(r);
    }
    return [...m.values()];
  });

  // Folded by default, but the group of the selected builder, and every group
  // while searching, are open. A click overrides.
  let open = $state<Record<string, boolean>>({});
  const isOpen = (g: { key: string; rows: typeof shown }) =>
    open[g.key] ?? (!!query.trim() || g.rows.some((r) => r.def.id === app.selectedBuilder));

  $effect(() => {
    if (!app.selectedBuilder && rows.length) app.selectedBuilder = rows[0].def.id;
  });

  async function refresh() {
    refreshing = true;
    await refreshBuilders(true);
    refreshing = false;
  }

</script>

<div class="page">
  <div class="list-pane">
    <div class="list-head">
      <div class="row">
        <h1>{t("Builders")}</h1>
        <span class="badge">{rows.length}</span>
        <div class="spacer"></div>
        <button class="ghost sm" onclick={refresh} title={isRemote() ? t("Reload from the server") : t("Reload the builder files")}>
          <Icon name="refresh" size={14} class={refreshing ? "spin" : ""} />
        </button>
      </div>
      <div class="search">
        <Icon name="search" size={14} />
        <input placeholder={t("Search…")} bind:value={query} />
      </div>
      {#if tags.length}
        <div class="tags">
          <button class="chip" class:on={tag === null} onclick={() => (tag = null)}>{t("all")}</button>
          {#each tags as tg}
            <button class="chip" class:on={tag === tg} onclick={() => (tag = tag === tg ? null : tg)}>{tg}</button>
          {/each}
        </div>
      {/if}
      {#if app.builders?.offline}
        <div class="badge warn" title={app.builders.offline}>{t("offline, cached list")}</div>
      {/if}
    </div>

    <div class="items">
      {#each groups as g (g.key)}
        <RepoGroup
          repo={g.repo}
          rows={g.rows}
          expanded={isOpen(g)}
          ontoggle={() => (open[g.key] = !isOpen(g))}
          onselect={(id) => (app.selectedBuilder = id)}
        />
      {:else}
        <div class="empty small">
          {rows.length ? t("No result") : isRemote() ? t("No builder on the server. Create one in the editor.") : t("No builder yet. Create one in the editor.")}
        </div>
      {/each}

      {#if app.builders?.errors.length}
        <div class="errs">
          <div class="errs-head"><Icon name="alert" size={14} /> {t("Invalid builders")}</div>
          {#each app.builders.errors as e}
            <button class="err-item" onclick={() => (app.page = "editor")} title={t("Open the editor")}>
              <b>{e.id}</b>
              <span>{e.message}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </div>

  <div class="detail-pane">
    {#if app.selectedBuilder && rows.some((r) => r.def.id === app.selectedBuilder)}
      {#key app.selectedBuilder}
        <BuilderDetail id={app.selectedBuilder} />
      {/key}
    {:else}
      <div class="empty full">
        <Icon name="hammer" size={40} />
        <div>{t("Pick a builder")}</div>
      </div>
    {/if}
  </div>
</div>

<style>
  .page {
    display: grid;
    grid-template-columns: 330px 1fr;
    height: 100%;
  }
  .list-pane {
    border-right: 1px solid var(--line);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .list-head {
    padding: 22px 16px 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 7px;
    background: var(--bg-2);
    border: 1px solid var(--line-2);
    border-radius: 9px;
    padding: 0 10px;
    color: var(--dim);
  }
  .search input {
    border: none;
    background: transparent;
    box-shadow: none;
    padding: 7px 0;
    flex: 1;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }
  .chip {
    padding: 3px 9px;
    font-size: 12px;
    border-radius: 999px;
    color: var(--muted);
  }
  .chip.on {
    background: rgba(124, 92, 255, 0.18);
    border-color: var(--accent);
    color: var(--text);
  }
  .items {
    flex: 1;
    overflow: auto;
    padding: 0 10px 16px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .small {
    font-size: 11.5px;
  }
  .errs {
    margin-top: 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .errs-head {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--err);
    font-size: 12.5px;
    font-weight: 600;
    padding: 0 6px 4px;
  }
  .err-item {
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    text-align: left;
    white-space: normal;
    border-color: rgba(248, 113, 113, 0.25);
    background: rgba(248, 113, 113, 0.05);
    font-size: 12px;
  }
  .err-item span {
    color: #fca5a5;
  }
  .detail-pane {
    overflow: auto;
    min-width: 0;
  }
  .full {
    height: 100%;
  }
</style>
