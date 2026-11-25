<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  // A line chart you can explore: hover for the values, drag to zoom into a
  // range, wheel to zoom around the pointer, double-click to see everything.
  import { fmtTime, fmtValue } from "../lib/metrics";
  import { phaseColor, type Band } from "../lib/phases";

  export interface Line {
    label: string;
    color: string;
    values: number[];
    /** Seconds between two values. */
    dt: number;
  }

  let {
    lines,
    unit = "",
    height = 200,
    compact = false,
    window: windowSecs = 0,
    zeroBased = true,
    bands = [],
  }: {
    lines: Line[];
    unit?: string;
    height?: number;
    compact?: boolean;
    window?: number;
    zeroBased?: boolean;
    /** Phases of the game, drawn behind the lines. */
    bands?: Band[];
  } = $props();

  let width = $state(600);
  let box: HTMLDivElement | undefined = $state();
  // Zoomed range in seconds; null shows everything (or the live window).
  let zoom = $state<[number, number] | null>(null);
  let hover = $state<number | null>(null); // px
  let drag = $state<{ from: number; to: number } | null>(null);

  const pad = $derived(compact ? { l: 34, r: 6, t: 6, b: 16 } : { l: 48, r: 10, t: 10, b: 22 });
  const innerW = $derived(Math.max(10, width - pad.l - pad.r));
  const innerH = $derived(Math.max(10, height - pad.t - pad.b));

  const total = $derived(Math.max(0, ...lines.map((l) => Math.max(0, l.values.length - 1) * l.dt)));
  const range = $derived.by<[number, number]>(() => {
    if (zoom) return zoom;
    if (windowSecs > 0 && total > windowSecs) return [total - windowSecs, total];
    return [0, Math.max(total, 1)];
  });
  const span = $derived(Math.max(range[1] - range[0], 1e-6));

  const xOf = (t: number) => pad.l + ((t - range[0]) / span) * innerW;

  /** Values of a line inside the range, with their time. */
  function visible(l: Line): { t: number; v: number }[] {
    const i0 = Math.max(0, Math.floor(range[0] / l.dt) - 1);
    const i1 = Math.min(l.values.length - 1, Math.ceil(range[1] / l.dt) + 1);
    const out: { t: number; v: number }[] = [];
    for (let i = i0; i <= i1; i++) out.push({ t: i * l.dt, v: l.values[i] });
    return out;
  }

  const shown = $derived(lines.map((l) => ({ l, pts: visible(l) })));

  const bounds = $derived.by(() => {
    let lo = Infinity;
    let hi = -Infinity;
    for (const { pts } of shown)
      for (const p of pts) {
        if (!Number.isFinite(p.v)) continue;
        lo = Math.min(lo, p.v);
        hi = Math.max(hi, p.v);
      }
    if (!Number.isFinite(lo)) return { lo: 0, hi: 1 };
    if (zeroBased) lo = Math.min(0, lo);
    if (hi <= lo) hi = lo + 1;
    return { lo, hi: hi + (hi - lo) * 0.08 };
  });

  const yOf = (v: number) => pad.t + innerH - ((v - bounds.lo) / (bounds.hi - bounds.lo)) * innerH;

  function niceStep(rough: number): number {
    const p = 10 ** Math.floor(Math.log10(rough));
    const f = rough / p;
    return (f < 1.5 ? 1 : f < 3.5 ? 2 : f < 7.5 ? 5 : 10) * p;
  }
  const yStep = $derived(niceStep((bounds.hi - bounds.lo) / (compact ? 3 : 4)));
  const yTicks = $derived.by(() => {
    const out: number[] = [];
    for (let v = Math.ceil(bounds.lo / yStep) * yStep; v <= bounds.hi; v += yStep) out.push(v);
    return out;
  });
  const tickLabel = (v: number) => (yStep >= 1 ? v.toFixed(0) : v.toFixed(yStep >= 0.1 ? 1 : 2));
  const xTicks = $derived.by(() => {
    const step = niceStep(span / Math.max(2, Math.floor(innerW / (compact ? 90 : 110))));
    const out: number[] = [];
    for (let t = Math.ceil(range[0] / step) * step; t <= range[1]; t += step) out.push(t);
    return out;
  });

  function pathOf(pts: { t: number; v: number }[]): string {
    // At most about two points per pixel are drawn.
    const stride = Math.max(1, Math.floor(pts.length / (innerW * 2)));
    let d = "";
    let pen = false;
    for (let i = 0; i < pts.length; i += stride) {
      const p = pts[i];
      if (!Number.isFinite(p.v)) {
        pen = false;
        continue;
      }
      d += `${pen ? "L" : "M"}${xOf(p.t).toFixed(1)},${yOf(p.v).toFixed(1)}`;
      pen = true;
    }
    return d;
  }

  const paths = $derived(shown.map(({ l, pts }) => ({ l, d: pathOf(pts), pts })));

  // One line: its average over what is shown.
  const avg = $derived.by(() => {
    if (lines.length !== 1) return null;
    const v = shown[0].pts.filter((p) => p.t >= range[0] && p.t <= range[1] && Number.isFinite(p.v)).map((p) => p.v);
    return v.length ? v.reduce((a, b) => a + b, 0) / v.length : null;
  });

  const hoverT = $derived(hover === null ? null : range[0] + ((hover - pad.l) / innerW) * span);
  const hoverBand = $derived(hoverT === null ? null : (bands.find((b) => hoverT >= b.from && hoverT < b.to) ?? null));
  const hoverRows = $derived.by(() => {
    if (hoverT === null) return [];
    return lines.map((l) => {
      const i = Math.min(l.values.length - 1, Math.max(0, Math.round(hoverT / l.dt)));
      return { l, v: l.values[i], t: i * l.dt };
    });
  });

  const tFromPx = (px: number) => range[0] + ((Math.min(Math.max(px, pad.l), pad.l + innerW) - pad.l) / innerW) * span;
  const local = (e: PointerEvent | MouseEvent) => e.clientX - box!.getBoundingClientRect().left;

  function down(e: PointerEvent) {
    if (compact || e.button !== 0) return;
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    drag = { from: local(e), to: local(e) };
  }
  function move(e: PointerEvent) {
    const x = local(e);
    hover = x >= pad.l && x <= pad.l + innerW ? x : null;
    if (drag) drag.to = x;
  }
  function up() {
    if (drag && Math.abs(drag.to - drag.from) > 8) {
      const a = tFromPx(Math.min(drag.from, drag.to));
      const b = tFromPx(Math.max(drag.from, drag.to));
      zoom = [a, b];
    }
    drag = null;
  }

  function wheel(node: HTMLElement) {
    const on = (e: WheelEvent) => {
      if (compact) return;
      e.preventDefault();
      const x = Math.min(Math.max(local(e), pad.l), pad.l + innerW);
      const at = range[0] + ((x - pad.l) / innerW) * span;
      const k = e.deltaY < 0 ? 0.8 : 1.25;
      let a = at - (at - range[0]) * k;
      let b = at + (range[1] - at) * k;
      if (b - a >= total) return void (zoom = null);
      if (a < 0) [a, b] = [0, b - a];
      if (b > total) [a, b] = [a - (b - total), total];
      zoom = [Math.max(0, a), b];
    };
    node.addEventListener("wheel", on, { passive: false });
    return { destroy: () => node.removeEventListener("wheel", on) };
  }
</script>

<div class="chart" bind:this={box} bind:clientWidth={width} style:height="{height}px" use:wheel>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <svg
    {width}
    {height}
    onpointerdown={down}
    onpointermove={move}
    onpointerup={up}
    onpointerleave={() => (hover = null)}
    ondblclick={() => (zoom = null)}
    class:grab={!compact}
  >
    {#each yTicks as v}
      <line x1={pad.l} x2={pad.l + innerW} y1={yOf(v)} y2={yOf(v)} class="grid" />
      <text x={pad.l - 5} y={yOf(v) + 3.5} class="tick" text-anchor="end">{tickLabel(v)}</text>
    {/each}
    {#each xTicks as t}
      <text x={xOf(t)} y={height - 5} class="tick" text-anchor="middle">{fmtTime(t)}</text>
    {/each}
    {#each bands as b}
      {@const x0 = Math.max(pad.l, xOf(b.from))}
      {@const x1 = Math.min(pad.l + innerW, xOf(b.to))}
      {#if x1 > x0}
        <rect x={x0} y={pad.t} width={x1 - x0} height={innerH} fill={phaseColor(b.name)} opacity="0.09" />
        <line x1={x0} x2={x0} y1={pad.t} y2={pad.t + innerH} stroke={phaseColor(b.name)} opacity="0.5" stroke-dasharray="3 3" />
        {#if !compact && x1 - x0 > 46}
          <text x={x0 + 4} y={pad.t + 11} class="band" fill={phaseColor(b.name)}>{b.name.length > (x1 - x0) / 6.5 ? b.name.slice(0, Math.max(3, Math.floor((x1 - x0) / 6.5) - 1)) + "…" : b.name}</text>
        {/if}
      {/if}
    {/each}
    {#if avg !== null}
      <line x1={pad.l} x2={pad.l + innerW} y1={yOf(avg)} y2={yOf(avg)} class="avg" />
    {/if}
    {#each paths as { l, d }}
      {#if lines.length === 1 && d}
        <path d="{d}L{xOf(range[1]).toFixed(1)},{yOf(bounds.lo)}L{xOf(range[0]).toFixed(1)},{yOf(bounds.lo)}Z" fill={l.color} opacity="0.12" stroke="none" />
      {/if}
      <path {d} fill="none" stroke={l.color} stroke-width="1.6" stroke-linejoin="round" />
    {/each}
    {#if drag}
      <rect x={Math.min(drag.from, drag.to)} y={pad.t} width={Math.abs(drag.to - drag.from)} height={innerH} class="sel" />
    {/if}
    {#if hover !== null && !drag}
      <line x1={hover} x2={hover} y1={pad.t} y2={pad.t + innerH} class="cross" />
      {#each hoverRows as r}
        {#if Number.isFinite(r.v)}<circle cx={xOf(r.t)} cy={yOf(r.v)} r="3.2" fill={r.l.color} />{/if}
      {/each}
    {/if}
  </svg>

  {#if hover !== null && !drag && hoverRows.length}
    <div class="tip" style:left="{hover > width / 2 ? hover - 8 : hover + 8}px" style:transform={hover > width / 2 ? "translateX(-100%)" : "none"}>
      <div class="tt">
        {fmtTime(hoverT ?? 0)}{#if hoverBand} · <span style:color={phaseColor(hoverBand.name)}>{hoverBand.name}</span>{/if}
      </div>
      {#each hoverRows as r}
        <div class="tv"><i style:background={r.l.color}></i>{#if lines.length > 1}<span class="lab">{r.l.label}</span>{/if}<b>{fmtValue(r.v, unit)}</b></div>
      {/each}
    </div>
  {/if}

  {#if zoom && !compact}
    <button class="reset sm ghost" onclick={() => (zoom = null)}>{t("Show all")}</button>
  {/if}
  {#if avg !== null && !compact}
    <div class="avgtag">{t("avg")} {fmtValue(avg, unit)}</div>
  {/if}
</div>

<style>
  .chart {
    position: relative;
    width: 100%;
    min-width: 0;
    overflow: hidden;
    user-select: none;
  }
  svg {
    display: block;
    touch-action: none;
  }
  svg.grab {
    cursor: crosshair;
  }
  .grid {
    stroke: #1c2030;
    stroke-width: 1;
  }
  .tick {
    fill: #6b7387;
    font-size: 10px;
    font-family: var(--mono, ui-monospace, monospace);
  }
  .band {
    font-size: 10px;
    opacity: 0.9;
  }
  .avg {
    stroke: #8b93a7;
    stroke-width: 1;
    stroke-dasharray: 4 4;
    opacity: 0.6;
  }
  .cross {
    stroke: #8b93a7;
    stroke-width: 1;
    opacity: 0.6;
  }
  .sel {
    fill: var(--accent, #7c8cff);
    opacity: 0.18;
    stroke: var(--accent, #7c8cff);
  }
  .tip {
    position: absolute;
    top: 6px;
    pointer-events: none;
    background: #10131c;
    border: 1px solid var(--line, #262b3b);
    border-radius: 8px;
    padding: 6px 9px;
    font-size: 12px;
    box-shadow: 0 6px 20px #0008;
    white-space: nowrap;
    z-index: 2;
  }
  .tt {
    color: var(--muted, #8b93a7);
    margin-bottom: 3px;
  }
  .tv {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .tv i {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
  }
  .lab {
    color: var(--muted, #8b93a7);
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .reset {
    position: absolute;
    top: 4px;
    right: 8px;
  }
  .avgtag {
    position: absolute;
    right: 12px;
    bottom: 24px;
    font-size: 11px;
    color: var(--muted, #8b93a7);
    pointer-events: none;
  }
</style>
