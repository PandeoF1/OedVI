<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { api, type FileState, type FileStatus, type Status } from "../lib/api";
  import { bytes } from "../lib/format";
  import { attempt } from "../lib/toast.svelte";
  import Icon from "./Icon.svelte";

  let {
    files,
    disabled = false,
    onchange,
  }: { files: FileStatus[]; disabled?: boolean; onchange: (s: Status) => void } = $props();

  interface Node {
    name: string;
    path: string;
    children: Node[];
    file: FileStatus | null;
    size: number;
    selSize: number;
    count: number;
    selCount: number;
    lockedCount: number;
    issues: number;
    removals: number;
  }

  const ISSUES: FileState[] = ["missing", "partial", "outdated", "unverified"];

  function build(list: FileStatus[]): Node {
    const root: Node = mk("", "");
    const dirs = new Map<string, Node>([["", root]]);
    for (const f of list) {
      if (f.state === "extra") continue;
      const parts = f.path.split("/");
      let parent = root;
      let path = "";
      for (let i = 0; i < parts.length - 1; i++) {
        path = path ? `${path}/${parts[i]}` : parts[i];
        let d = dirs.get(path);
        if (!d) {
          d = mk(parts[i], path);
          dirs.set(path, d);
          parent.children.push(d);
        }
        parent = d;
      }
      const leaf = mk(parts[parts.length - 1], f.path);
      leaf.file = f;
      parent.children.push(leaf);
      // Totals up the chain.
      let p = "";
      const chain = [root];
      for (let i = 0; i < parts.length - 1; i++) {
        p = p ? `${p}/${parts[i]}` : parts[i];
        chain.push(dirs.get(p)!);
      }
      chain.push(leaf);
      for (const n of chain) {
        n.size += f.size;
        n.count++;
        if (f.selected) {
          n.selSize += f.size;
          n.selCount++;
        }
        if (f.required) n.lockedCount++;
        if (ISSUES.includes(f.state)) n.issues++;
        if (f.state === "to_remove") n.removals++;
      }
    }
    sort(root);
    return root;
  }

  function mk(name: string, path: string): Node {
    return { name, path, children: [], file: null, size: 0, selSize: 0, count: 0, selCount: 0, lockedCount: 0, issues: 0, removals: 0 };
  }

  function sort(n: Node) {
    n.children.sort((a, b) => (a.file ? 1 : 0) - (b.file ? 1 : 0) || a.name.localeCompare(b.name));
    n.children.forEach(sort);
  }

  let tree = $derived(build(files));
  let extras = $derived(files.filter((f) => f.state === "extra"));
  const expanded = new SvelteSet<string>();
  let query = $state("");
  let filter = $state<"all" | "issues" | "selected">("all");

  let flat = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q && filter === "all") return null;
    return files
      .filter((f) => f.state !== "extra")
      .filter((f) => !q || f.path.toLowerCase().includes(q))
      .filter((f) => filter !== "issues" || ISSUES.includes(f.state) || f.state === "to_remove")
      .filter((f) => filter !== "selected" || f.selected)
      .slice(0, 800);
  });

  async function toggle(path: string, on: boolean) {
    const st = await attempt(() => api.setSelection("path", path, on));
    if (st) onchange(st);
  }

  const labels: Record<FileState, [string, string]> = $derived({
    ok: [t("up to date"), "ok"],
    missing: [t("missing"), "warn"],
    partial: [t("partial"), "info"],
    outdated: [t("outdated"), "warn"],
    unverified: [t("to verify"), "info"],
    unselected: [t("excluded"), ""],
    to_remove: [t("will be deleted"), "err"],
    extra: [t("extra"), "err"],
  });
</script>

{#snippet stateBadge(f: FileStatus)}
  {@const [text, tone] = labels[f.state]}
  <span class="badge {tone}">{text}</span>
{/snippet}

{#snippet check(n: Node)}
  {@const allLocked = n.lockedCount === n.count}
  <input
    type="checkbox"
    checked={n.selCount === n.count}
    indeterminate={n.selCount > 0 && n.selCount < n.count}
    disabled={disabled || allLocked}
    title={allLocked ? t("Required files") : ""}
    onclick={(e) => e.stopPropagation()}
    onchange={(e) => toggle(n.path, e.currentTarget.checked)}
  />
{/snippet}

{#snippet node(n: Node, depth: number)}
  {#if n.file}
    <div class="line" class:off={!n.file.selected} style:padding-left="{12 + depth * 18}px">
      <span class="chev-space"></span>
      {@render check(n)}
      <Icon name="file" size={14} class="ico" />
      <span class="nm ellipsis" title={n.path}>{n.name}</span>
      {#if n.file.required}<span class="badge accent">{t("required")}</span>{/if}
      {@render stateBadge(n.file)}
      <span class="sz">{bytes(n.size)}</span>
    </div>
  {:else}
    <div
      class="line dir"
      class:off={n.selCount === 0}
      style:padding-left="{12 + depth * 18}px"
      role="button"
      tabindex="0"
      onclick={() => (expanded.has(n.path) ? expanded.delete(n.path) : expanded.add(n.path))}
      onkeydown={(e) => e.key === "Enter" && (expanded.has(n.path) ? expanded.delete(n.path) : expanded.add(n.path))}
    >
      <span class="chev" class:open={expanded.has(n.path)}><Icon name="chevron" size={14} /></span>
      {@render check(n)}
      <Icon name="folder" size={14} class="ico dir-ico" />
      <span class="nm ellipsis">{n.name}</span>
      <span class="dim cnt">{n.selCount}/{n.count}</span>
      {#if n.issues}<span class="badge warn">{t("{n} to download", { n: n.issues })}</span>{/if}
      {#if n.removals}<span class="badge err">{t("{n} to delete", { n: n.removals })}</span>{/if}
      <span class="sz">{bytes(n.selSize)}<span class="dim"> / {bytes(n.size)}</span></span>
    </div>
    {#if expanded.has(n.path)}
      {#each n.children as c (c.path)}
        {@render node(c, depth + 1)}
      {/each}
    {/if}
  {/if}
{/snippet}

<div class="head">
  <h3>{t("Files")}</h3>
  <div class="search">
    <Icon name="search" size={14} />
    <input placeholder={t("Search a file…")} bind:value={query} />
  </div>
  <div class="seg">
    <button class:on={filter === "all"} onclick={() => (filter = "all")}>{t("All")}</button>
    <button class:on={filter === "issues"} onclick={() => (filter = "issues")}>{t("To process")}</button>
    <button class:on={filter === "selected"} onclick={() => (filter = "selected")}>{t("Selected")}</button>
  </div>
</div>

<div class="list">
  {#if flat}
    {#each flat as f (f.path)}
      <div class="line" class:off={!f.selected}>
        <input
          type="checkbox"
          checked={f.selected}
          disabled={disabled || f.required}
          onchange={(e) => toggle(f.path, e.currentTarget.checked)}
        />
        <span class="nm ellipsis mono" title={f.path}>{f.path}</span>
        {@render stateBadge(f)}
        <span class="sz">{bytes(f.size)}</span>
      </div>
    {:else}
      <div class="empty">{t("No file")}</div>
    {/each}
    {#if flat.length === 800}<div class="dim more">{t("Only the first 800 results are shown.")}</div>{/if}
  {:else}
    {#each tree.children as c (c.path)}
      {@render node(c, 0)}
    {/each}
  {/if}

  {#if extras.length && !flat}
    <div class="extra-head">
      <Icon name="alert" size={14} /> {t("Local files the server does not publish ({n})", { n: extras.length })}
    </div>
    {#each extras as f (f.path)}
      <div class="line">
        <span class="nm ellipsis mono">{f.path}</span>
        {@render stateBadge(f)}
        <span class="sz">{bytes(f.size)}</span>
      </div>
    {/each}
  {/if}
</div>

<style>
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--line);
  }
  .search {
    flex: 1;
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
  .seg {
    display: flex;
    background: var(--bg-2);
    border: 1px solid var(--line-2);
    border-radius: 9px;
    padding: 2px;
  }
  .seg button {
    border: none;
    background: transparent;
    padding: 5px 10px;
    font-size: 12.5px;
    color: var(--muted);
    border-radius: 7px;
  }
  .seg button.on {
    background: var(--panel-2);
    color: var(--text);
  }
  .list {
    max-height: calc(100vh - 360px);
    min-height: 260px;
    overflow: auto;
    padding: 6px 0;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 14px;
    font-size: 13px;
    min-height: 32px;
  }
  .line:hover {
    background: rgba(255, 255, 255, 0.025);
  }
  .line.dir {
    cursor: pointer;
    user-select: none;
  }
  .line.off .nm {
    color: var(--dim);
  }
  .nm {
    flex: 1;
  }
  .dir .nm {
    font-weight: 550;
  }
  .chev,
  .chev-space {
    width: 14px;
    display: inline-flex;
    color: var(--dim);
    transition: transform 0.15s;
    flex: none;
  }
  .chev.open {
    transform: rotate(90deg);
  }
  :global(.ico) {
    color: var(--dim);
    flex: none;
  }
  :global(.dir-ico) {
    color: #a78bfa !important;
  }
  .cnt {
    font-size: 11.5px;
  }
  .sz {
    font-size: 12px;
    color: var(--muted);
    min-width: 120px;
    text-align: right;
    white-space: nowrap;
  }
  .more {
    padding: 10px 14px;
    font-size: 12px;
  }
  .extra-head {
    display: flex;
    gap: 7px;
    align-items: center;
    padding: 12px 14px 6px;
    color: var(--err);
    font-size: 12.5px;
    font-weight: 600;
    border-top: 1px solid var(--line);
    margin-top: 6px;
  }
</style>
