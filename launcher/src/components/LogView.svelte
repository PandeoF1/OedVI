<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import type { LogLine } from "../lib/api";
  import Icon from "./Icon.svelte";

  let { lines, onclear, height = 320 }: { lines: LogLine[]; onclear?: () => void; height?: number } = $props();

  let box: HTMLDivElement | undefined = $state();
  let follow = $state(true);
  // Lines in the DOM; the filter and "Copy" still cover every line.
  const RENDER = 500;
  let query = $state("");
  // Minimum level shown; lines without a level always are.
  let level = $state<"all" | "info" | "warn" | "error">("all");
  let stamps = $state(false);

  const RANK: Record<string, number> = { TRACE: 0, DEBUG: 1, INFO: 2, WARN: 3, ERROR: 4 };
  const MIN: Record<string, number> = { all: 0, info: 2, warn: 3, error: 4 };
  const LEVEL = /\b(TRACE|DEBUG|INFO|WARN|ERROR)\b/;
  const STAMP = /^\s*\d{4}-\d\d-\d\dT[\d:.]+Z?\s+/;

  function rank(text: string): number | null {
    const m = LEVEL.exec(text.slice(0, 60));
    return m ? RANK[m[1]] : null;
  }
  const show = (text: string) => (stamps ? text : text.replace(STAMP, ""));

  let shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const min = MIN[level];
    const src = lines.filter((l) => {
      if (q && !l.text.toLowerCase().includes(q)) return false;
      if (!min) return true;
      const r = rank(l.text);
      return r === null || r >= min;
    });
    return src.slice(-RENDER);
  });

  $effect(() => {
    shown.length;
    if (follow && box) queueMicrotask(() => box && (box.scrollTop = box.scrollHeight));
  });

  function onscroll() {
    if (!box) return;
    follow = box.scrollHeight - box.scrollTop - box.clientHeight < 30;
  }

  function tone(l: LogLine): string {
    if (l.stream === "info") return l.text.startsWith("✘") ? "err" : l.text.startsWith("✔") ? "ok" : "info";
    const r = rank(l.text);
    if (r === 4) return "err";
    if (r === 3) return "warn";
    if (r !== null && r <= 1) return "dimmed";
    if (/\b(error|erreur|panicked|fatal|failed)\b/i.test(l.text)) return "err";
    if (/\bwarn(ing)?\b/i.test(l.text)) return "warn";
    return l.stream === "stderr" ? "stderr" : "";
  }

  function copy() {
    navigator.clipboard.writeText(lines.map((l) => l.text).join("\n"));
  }
</script>

<div class="logs">
  <div class="bar">
    <Icon name="terminal" size={14} />
    <span class="title">{t("Log")}</span>
    <span class="dim">{t("{n} lines", { n: lines.length })}</span>
    <div class="spacer"></div>
    <select bind:value={level} title={t("Minimum level")}>
      <option value="all">{t("All levels")}</option>
      <option value="info">{t("Info and above")}</option>
      <option value="warn">{t("Warnings and above")}</option>
      <option value="error">{t("Errors")}</option>
    </select>
    <label class="chk" title={t("Show the timestamp of each line")}><input type="checkbox" bind:checked={stamps} /> {t("time")}</label>
    <input placeholder={t("Filter…")} bind:value={query} />
    <button class="ghost sm" onclick={copy} title={t("Copy")}>{t("Copy")}</button>
    {#if onclear}<button class="ghost sm" onclick={onclear} title={t("Clear")}><Icon name="eraser" size={13} /></button>{/if}
    {#if !follow}
      <button class="sm" onclick={() => { follow = true; box && (box.scrollTop = box.scrollHeight); }}>{t("↓ Follow")}</button>
    {/if}
  </div>
  <div class="body mono" bind:this={box} {onscroll} style:height="{height}px">
    {#each shown as l (l)}
      <div class="l {tone(l)}">{show(l.text) || " "}</div>
    {:else}
      <div class="dim pad">{t("Nothing yet. The output of git, the build and the game shows up here.")}</div>
    {/each}
  </div>
</div>

<style>
  .logs {
    border: 1px solid var(--line);
    border-radius: var(--radius);
    overflow: hidden;
    background: #07080c;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border-bottom: 1px solid var(--line);
    background: var(--panel);
    color: var(--muted);
    font-size: 12.5px;
  }
  .title {
    color: var(--text);
    font-weight: 600;
  }
  .bar input {
    padding: 4px 8px;
    font-size: 12px;
    width: 160px;
  }
  .body {
    overflow: auto;
    padding: 8px 12px;
    font-size: 12px;
    line-height: 1.55;
  }
  .l {
    white-space: pre-wrap;
    word-break: break-all;
    color: #c9cedb;
  }
  .l.stderr {
    color: #aab1c3;
  }
  .l.info {
    color: var(--accent-2);
  }
  .l.ok {
    color: var(--ok);
    font-weight: 600;
  }
  .l.err {
    color: var(--err);
  }
  .l.dimmed {
    color: #7d8498;
  }
  .bar select {
    padding: 3px 6px;
    font-size: 12px;
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 4px;
    cursor: pointer;
  }
  .l.warn {
    color: var(--warn);
  }
  .pad {
    padding: 8px 0;
    font-family: inherit;
  }
</style>
