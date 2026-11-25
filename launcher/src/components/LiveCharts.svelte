<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  // Live charts of what the running game and the PC do, one per measure.
  import { fmtValue, meta, sortNames } from "../lib/metrics";
  import { loadLive, pointsOf } from "../lib/live.svelte";
  import { api } from "../lib/api";
  import { attempt } from "../lib/toast.svelte";
  import { bandsOfPoints, phaseColor } from "../lib/phases";
  import Chart, { type Line } from "./Chart.svelte";

  let { id, compact = false, running = true }: { id: string; compact?: boolean; running?: boolean } = $props();

  // Seconds shown: a sliding window, so the last minutes stay readable.
  let windowSecs = $state(120);
  const WINDOWS = [30, 60, 120, 300, 900];

  $effect(() => {
    loadLive(id);
  });

  const points = $derived(pointsOf(id));
  const bands = $derived(bandsOfPoints(points));
  const current = $derived(points[points.length - 1]?.phase ?? null);
  const used = $derived([...new Set(bands.map((b) => b.name))]);
  let markName = $state("");

  async function mark(name: string) {
    const n = name.trim();
    if (!n) return;
    await attempt(() => api.markPhase(id, n));
    markName = "";
  }
  const names = $derived(sortNames(new Set(points.flatMap((p) => Object.keys(p.values)))));
  const charts = $derived(
    names.map((name) => {
      const values = points.map((p) => p.values[name] ?? NaN);
      const m = meta(name);
      const ok = values.filter(Number.isFinite);
      const line: Line = { label: m.label, color: m.color, values, dt: 1 };
      return {
        name,
        m,
        line,
        now: values[values.length - 1],
        min: Math.min(...ok),
        max: Math.max(...ok),
        avg: ok.reduce((a, b) => a + b, 0) / Math.max(ok.length, 1),
      };
    }),
  );
</script>

<div class="live" class:compact>
  <div class="head">
    <span class="dot" class:on={running}></span>
    <span>{running ? t("Live") : t("Last session")}</span>
    <div class="spacer"></div>
    <label class="dim small">
      {t("Window")}
      <select bind:value={windowSecs}>
        {#each WINDOWS as w}<option value={w}>{w >= 60 ? `${w / 60} min` : `${w} s`}</option>{/each}
      </select>
    </label>
  </div>
  {#if current}
    <div class="phase" style:border-color={phaseColor(current)}>
      <span class="dim small">{t("Phase")}</span> <b style:color={phaseColor(current)}>{current}</b>
    </div>
  {/if}
  {#if running}
    <form class="mark" onsubmit={(e) => { e.preventDefault(); mark(markName); }}>
      <input bind:value={markName} placeholder={t("Mark a phase by hand (e.g. Main menu)")} list="phases-{id}" />
      <button class="sm" type="submit" disabled={!markName.trim()}>{t("Mark")}</button>
      <datalist id="phases-{id}">{#each used as u}<option value={u}></option>{/each}</datalist>
    </form>
  {/if}
  <div class="grid">
    {#each charts as c (c.name)}
      <section class="card">
        <div class="top">
          <span class="lab"><i style:background={c.m.color}></i>{c.m.label}</span>
          <span class="now" style:color={c.m.color}>{fmtValue(c.now, c.m.unit)}</span>
        </div>
        <Chart lines={[c.line]} unit={c.m.unit} height={compact ? 90 : 130} {compact} window={windowSecs} {bands} />
        <div class="sub dim">
          {t("min")} {fmtValue(c.min)} · {t("avg")} {fmtValue(c.avg)} · {t("max")} {fmtValue(c.max)}
        </div>
      </section>
    {:else}
      <div class="dim empty">
        {running
          ? t("Waiting for the first measures… (CPU, RAM and disk arrive after 1 s, GPU and FPS depend on what the game prints)")
          : t("No measure: start the game.")}
      </div>
    {/each}
  </div>
</div>

<style>
  .live {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    font-weight: 600;
  }
  .head select {
    padding: 2px 6px;
    font-size: 12px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #596075;
  }
  .dot.on {
    background: var(--ok, #4ade80);
    box-shadow: 0 0 8px var(--ok, #4ade80);
  }
  .phase {
    border: 1px solid;
    border-radius: var(--radius);
    padding: 5px 10px;
    font-size: 13px;
  }
  .mark {
    display: flex;
    gap: 6px;
  }
  .mark input {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    padding: 4px 8px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(300px, 100%), 1fr));
    gap: 10px;
  }
  .compact .grid {
    grid-template-columns: minmax(0, 1fr);
  }
  .card {
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--panel);
    padding: 8px 10px 6px;
    min-width: 0;
  }
  .top {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 2px;
  }
  .lab {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    color: var(--muted);
  }
  .lab i {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    display: inline-block;
  }
  .now {
    font-size: 19px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .compact .now {
    font-size: 16px;
  }
  .sub {
    font-size: 11px;
    margin-top: 2px;
  }
  .empty {
    padding: 14px 4px;
    font-size: 13px;
  }
</style>
