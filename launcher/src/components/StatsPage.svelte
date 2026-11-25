<script lang="ts">
  import { api, type Metric, type PerfRow, type PerfSession, type Specs, type StatsReport } from "../lib/api";
  import { plural } from "../lib/format";
  import { fmtValue, meta, sortNames } from "../lib/metrics";
  import { bandsOfSession, phaseColor, summarizePhases } from "../lib/phases";
  import { app, isRemote } from "../lib/store.svelte";
  import { t } from "../lib/i18n.svelte";
  import Chart, { type Line } from "./Chart.svelte";
  import Icon from "./Icon.svelte";

  let remote = $state<StatsReport | null>(null);
  let local = $state<PerfSession[]>([]);
  let mine = $state<Specs | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let tab = $state<"compare" | "mine">(isRemote() ? "compare" : "mine");
  let selected = $state<PerfSession | null>(null);
  let picked = $state<string[]>([]);
  let cmpMetric = $state("fps");
  let chapter = $state("");

  async function load() {
    loading = true;
    local = await api.statsLocal().catch(() => []);
    api.machineSpecs().then((s) => (mine = s)).catch(() => {});
    if (!isRemote()) {
      remote = null;
      error = null;
      loading = false;
      return;
    }
    try {
      remote = await api.statsRemote();
      error = null;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    app.statsEpoch;
    load();
  });

  const name = (id: string) => app.builders?.builders.find((b) => b.def.id === id)?.def.name ?? id;
  const fmt = (n: number) => (n >= 100 ? n.toFixed(0) : n.toFixed(1));
  const date = (s: number) => new Date(s * 1000).toLocaleString("fr-FR", { dateStyle: "short", timeStyle: "short" });
  const dur = (s: number) => (s >= 60 ? `${Math.floor(s / 60)} min ${Math.round(s % 60)} s` : `${Math.round(s)} s`);
  const gb = (mb: number) => `${(mb / 1024).toFixed(mb >= 10240 ? 0 : 1)} Go`;
  const gpuLine = (s: PerfSession) => s.specs?.gpus.map((g) => g.name).join(" + ") ?? "";

  // Chapters (phases) known by the server, to compare builders on the same one.
  const chapters = $derived([...new Set((remote?.phases ?? []).map((r) => r.phase))].sort());
  // The last chapter by default: it is the one that is played.
  $effect(() => {
    if (!chapters.includes(chapter)) chapter = chapters[chapters.length - 1] ?? "";
  });
  const chapterNow = $derived(chapter);
  const chapterRows = $derived((remote?.phases ?? []).filter((r) => r.phase === chapterNow).sort((a, b) => b.fps - a.fps));
  const chapterTop = $derived(Math.max(1, ...chapterRows.map((r) => r.fps)));

  let top = $derived(Math.max(1, ...(remote?.rows ?? []).map((r) => r.avg)));
  let byBuilder = $derived.by(() => {
    const m = new Map<string, PerfRow[]>();
    for (const r of remote?.rows ?? []) (m.get(r.builder) ?? m.set(r.builder, []).get(r.builder)!).push(r);
    return [...m.entries()].sort((a, b) => Math.max(...b[1].map((r) => r.avg)) - Math.max(...a[1].map((r) => r.avg)));
  });

  /** Metrics of a session; older ones only have the FPS curve. */
  function metricsOf(s: PerfSession): Record<string, Metric> {
    if (s.metrics && Object.keys(s.metrics).length) return s.metrics;
    if (!s.series.length) return {};
    return { fps: { samples: s.samples, avg: s.avg, min: s.min, max: s.max, p1: s.low1, p99: s.max, series: s.series } };
  }
  const dtOf = (s: PerfSession, m: Metric) => (m.series.length > 1 ? s.duration_secs / (m.series.length - 1) : 1);

  const all = $derived([...(remote?.recent ?? []), ...local]);
  const byId = $derived(new Map(all.map((s) => [s.id, s])));
  const compared = $derived(picked.map((id) => byId.get(id)).filter((s): s is PerfSession => !!s));
  const cmpNames = $derived(sortNames(new Set(compared.flatMap((s) => Object.keys(metricsOf(s))))));
  const cmpLines = $derived.by<Line[]>(() => {
    const colors = ["#6ee7b7", "#f472b6", "#60a5fa", "#fbbf24"];
    return compared.flatMap((s, i) => {
      const m = metricsOf(s)[cmpMetric];
      if (!m) return [];
      return [{ label: `${name(s.builder)} @${s.commit.slice(0, 7)} · ${s.machine}`, color: colors[i % colors.length], values: m.series, dt: dtOf(s, m) }];
    });
  });

  function toggle(id: string) {
    picked = picked.includes(id) ? picked.filter((p) => p !== id) : [...picked, id].slice(-4);
  }

  function spark(series: number[], w = 140, h = 32): string {
    if (series.length < 2) return "";
    const max = Math.max(...series, 1);
    return series.map((v, i) => `${i ? "L" : "M"}${((i / (series.length - 1)) * w).toFixed(1)},${(h - (v / max) * h).toFixed(1)}`).join("");
  }
</script>

{#snippet specsBox(sp: Specs | null | undefined)}
  {#if sp}
    <div class="specs">
      <div><span class="dim">CPU</span>{sp.cpu || "?"}{#if sp.cores} <span class="dim">· {t("{cores} cores / {threads} threads", { cores: sp.cores, threads: sp.threads })}</span>{/if}</div>
      {#each sp.gpus as g}
        <div><span class="dim">GPU</span>{g.name} <span class="dim">· {t("driver")} {g.driver || "?"}{g.driver_date ? ` (${g.driver_date})` : ""}</span></div>
      {/each}
      <div><span class="dim">RAM</span>{sp.ram_mb ? gb(sp.ram_mb) : "?"}</div>
      <div><span class="dim">{t("System")}</span>{sp.os || "?"}</div>
    </div>
  {:else}
    <div class="dim small">{t("PC specs unknown (session recorded before version 0.4).")}</div>
  {/if}
{/snippet}

{#snippet runs(list: PerfSession[], who: boolean)}
  <table>
    <thead>
      <tr>
        <th></th><th>{t("When")}</th><th>{t("Builder")}</th><th>{t("Version")}</th>{#if who}<th>{t("PC")}</th>{/if}<th>GPU</th>
        <th class="n">{t("Avg FPS")}</th><th class="n">{t("1 % low")}</th><th>{t("Duration")}</th><th>{t("FPS curve")}</th>
      </tr>
    </thead>
    <tbody>
      {#each list as s (s.id)}
        <tr class:sel={selected?.id === s.id} onclick={() => (selected = s)}>
          <td onclick={(e) => e.stopPropagation()}>
            <input type="checkbox" checked={picked.includes(s.id)} onchange={() => toggle(s.id)} title={t("Compare (4 max)")} />
          </td>
          <td class="dim">{date(s.started_at)}</td>
          <td>{name(s.builder)}</td>
          <td class="mono">{s.commit.slice(0, 7) || "?"}</td>
          {#if who}<td>{s.machine}</td>{/if}
          <td class="dim gpu" title={gpuLine(s)}>{gpuLine(s) || "–"}</td>
          <td class="n strong">{s.samples ? fmt(s.avg) : "–"}</td>
          <td class="n">{s.samples ? fmt(s.low1) : "–"}</td>
          <td class="dim">{dur(s.duration_secs)}</td>
          <td><svg width="140" height="32" aria-hidden="true"><path d={spark(s.series)} fill="none" stroke="var(--accent-2)" stroke-width="1.5" /></svg></td>
        </tr>
      {:else}
        <tr><td colspan="10" class="dim pad">{t("No session.")}</td></tr>
      {/each}
    </tbody>
  </table>
{/snippet}

<div class="page">
  <header>
    <h1>Stats</h1>
    <div class="tabs">
      {#if isRemote()}<button class:on={tab === "compare"} onclick={() => (tab = "compare")}>{t("Comparison")}</button>{/if}
      <button class:on={tab === "mine"} onclick={() => (tab = "mine")}>{t("My sessions")} ({local.length})</button>
    </div>
    <div class="spacer"></div>
    <button class="ghost sm" onclick={load} disabled={loading}><Icon name="refresh" size={13} /> {t("Refresh")}</button>
  </header>

  <div class="body">
    {#if selected}
      {@const s = selected}
      {@const ms = metricsOf(s)}
      <section class="card detail">
        <div class="row">
          <h3>{name(s.builder)} <span class="mono dim">@{s.commit.slice(0, 7) || "?"}</span></h3>
          <span class="dim small">{date(s.started_at)} · {dur(s.duration_secs)} · {s.machine || "?"} · launcher {s.launcher}</span>
          <div class="spacer"></div>
          <button class="ghost sm" onclick={() => (selected = null)}>{t("Close")}</button>
        </div>
        {@render specsBox(s.specs)}
        {#if s.phases?.length}
          {@const ph = summarizePhases(s)}
          <div class="tablebox">
            <table>
              <thead>
                <tr><th>{t("Chapter / marker")}</th><th>{t("From")}</th><th>{t("Duration")}</th><th class="n">{t("Avg FPS")}</th><th class="n">{t("1 % low")}</th><th class="n">{t("Min FPS")}</th><th class="n">ms/frame</th><th class="n">{t("Game GPU")}</th><th class="n">VRAM</th><th class="n">{t("Game CPU")}</th></tr>
              </thead>
              <tbody>
                {#each ph as p}
                  <tr>
                    <td><i class="dot" style:background={phaseColor(p.name)}></i>{p.name}</td>
                    <td class="dim">{dur(p.from)}</td>
                    <td class="dim">{dur(p.secs)}</td>
                    <td class="n strong">{p.metrics.fps ? fmt(p.metrics.fps.avg) : "–"}</td>
                    <td class="n">{p.metrics.fps ? fmt(p.metrics.fps.p1) : "–"}</td>
                    <td class="n">{p.metrics.fps ? fmt(p.metrics.fps.min) : "–"}</td>
                    <td class="n">{p.metrics.frame_ms ? fmt(p.metrics.frame_ms.avg) : "–"}</td>
                    <td class="n">{p.metrics.proc_gpu ? fmt(p.metrics.proc_gpu.avg) + " %" : "–"}</td>
                    <td class="n">{p.metrics.proc_vram ? fmt(p.metrics.proc_vram.avg) + " MB" : "–"}</td>
                    <td class="n">{p.metrics.proc_cpu ? fmt(p.metrics.proc_cpu.avg) + " %" : "–"}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {:else}
          <p class="dim small hint">{t("No chapters for this session (the builder has no [phases], or it was recorded before 0.5).")}</p>
        {/if}
        <p class="dim small hint">{t("Hover for the values, drag to zoom into an area, wheel to zoom, double-click to see everything.")}</p>
        <div class="charts">
          {#each sortNames(Object.keys(ms)) as k (k)}
            {@const m = ms[k]}
            {@const mt = meta(k)}
            <div class="cbox">
              <div class="chead">
                <span><i style:background={mt.color}></i>{mt.label}</span>
                <span class="dim small">
                  {t("avg")} <b>{fmtValue(m.avg, mt.unit)}</b> · {t("min")} {fmtValue(m.min)} · 1 % {fmtValue(m.p1)} · 99 % {fmtValue(m.p99)} · {t("max")} {fmtValue(m.max)}
                </span>
              </div>
              <Chart lines={[{ label: mt.label, color: mt.color, values: m.series, dt: dtOf(s, m) }]} unit={mt.unit} height={210} bands={bandsOfSession(s)} />
            </div>
          {:else}
            <div class="dim">{t("No detailed measure for this session.")}</div>
          {/each}
        </div>
      </section>
    {/if}

    {#if compared.length > 1}
      <section class="card detail">
        <div class="row">
          <h3>{t("Comparing {n} sessions", { n: compared.length })}</h3>
          <select bind:value={cmpMetric}>
            {#each cmpNames as n}<option value={n}>{meta(n).label}</option>{/each}
          </select>
          <div class="spacer"></div>
          <button class="ghost sm" onclick={() => (picked = [])}>{t("Clear")}</button>
        </div>
        <div class="legend">
          {#each cmpLines as l}<span><i style:background={l.color}></i>{l.label}</span>{/each}
        </div>
        <Chart lines={cmpLines} unit={meta(cmpMetric).unit} height={260} />
        <div class="cmp">
          {#each compared as s}
            <div class="cmpcard">
              <b>{name(s.builder)} @{s.commit.slice(0, 7)}</b>
              <span class="dim small">{s.machine} · {date(s.started_at)}</span>
              {@render specsBox(s.specs)}
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {#if tab === "compare"}
      {#if error}<div class="note err">{t("Server")}: {error}</div>{/if}
      <p class="dim small">
        {t("Average FPS per builder and version (commit), all machines together. Tick sessions to compare them, click a row to see all its charts.")}
      </p>
      {#if chapters.length}
        <section class="card">
          <div class="row">
            <h3>{t("Per chapter / marker")}</h3>
            <select bind:value={chapter}>
              {#each chapters as c}<option value={c}>{c}</option>{/each}
            </select>
          </div>
          <p class="dim small hint">
            {t("Average FPS during this chapter only, per builder and version: compare what matters (in game) without the menus.")}
          </p>
          {#each chapterRows as r (r.builder + r.commit)}
            <div class="bar-row">
              <span class="mono ver" title={r.commit}>{r.commit.slice(0, 7) || "?"}</span>
              <div class="track"><div class="fill" style:width="{(r.fps / chapterTop) * 100}%" style:background={phaseColor(r.phase)}></div></div>
              <span class="val strong">{fmt(r.fps)} <span class="dim">FPS</span></span>
              <span class="dim meta">
                <b class="who">{name(r.builder)}</b> · {t("1 % low")} {fmt(r.fps_low1)} · {fmt(r.frame_ms)} ms/frame · {t("{time} in the chapter", { time: dur(r.secs) })} · {plural(r.sessions, "{n} run", "{n} runs")} · {plural(r.machines, "{n} PC", "{n} PCs")}
              </span>
            </div>
          {/each}
        </section>
      {/if}
      {#each byBuilder as [builder, rows] (builder)}
        <section class="card">
          <h3>{name(builder)}</h3>
          {#each rows as r (r.commit)}
            <div class="bar-row">
              <span class="mono ver">{r.commit.slice(0, 7) || "?"}</span>
              <div class="track"><div class="fill" style:width="{(r.avg / top) * 100}%"></div></div>
              <span class="val strong">{fmt(r.avg)} <span class="dim">FPS</span></span>
              <span class="dim meta">{t("1 % low")} {fmt(r.low1)} · {t("best")} {fmt(r.best_avg)} · {plural(r.sessions, "{n} run", "{n} runs")} · {plural(r.machines, "{n} PC", "{n} PCs")}</span>
            </div>
          {/each}
        </section>
      {:else}
        {#if !error}<div class="dim pad">{t("No result on the server yet. Run a builder: the measures are sent when the game closes.")}</div>{/if}
      {/each}
      {#if remote?.recent.length}
        <h3 class="sub">{t("Latest sessions from everyone")}</h3>
        <div class="tablebox">{@render runs(remote.recent, true)}</div>
      {/if}
    {:else}
      <section class="card">
        <h3>{t("This PC")}</h3>
        {@render specsBox(mine)}
      </section>
      <div class="tablebox">{@render runs(local, false)}</div>
    {/if}
  </div>
</div>

<style>
  .page { height: 100%; display: flex; flex-direction: column; }
  header { display: flex; align-items: center; gap: 14px; padding: 22px 28px 14px; }
  h1 { margin: 0; font-size: 20px; }
  .tabs { display: flex; gap: 4px; }
  .tabs button { background: transparent; border: 1px solid var(--line); color: var(--muted); padding: 4px 12px; border-radius: 8px; }
  .tabs button.on { background: var(--panel); color: var(--text); }
  .body { padding: 0 28px 28px; overflow: auto; flex: 1; display: flex; flex-direction: column; gap: 14px; }
  /* Blocks keep their height and the page scrolls, instead of squashing the tables. */
  .body > :global(*) { flex-shrink: 0; }
  .card { border: 1px solid var(--line); border-radius: var(--radius); padding: 14px 16px; background: var(--panel); }
  .card h3 { margin: 0 0 10px; font-size: 14px; }
  .detail { display: flex; flex-direction: column; gap: 10px; }
  .detail .row { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .detail h3 { margin: 0; }
  .sub { margin: 10px 0 0; font-size: 14px; }
  .hint { margin: 0; }
  .specs { display: grid; gap: 3px; font-size: 12.5px; }
  .specs .dim { display: inline-block; min-width: 62px; margin-right: 6px; }
  .charts { display: grid; grid-template-columns: repeat(auto-fit, minmax(420px, 1fr)); gap: 12px; }
  .cbox { border: 1px solid var(--line); border-radius: var(--radius); padding: 8px 10px; background: #0d1018; }
  .chead { display: flex; flex-direction: column; gap: 2px; font-size: 13px; margin-bottom: 4px; min-height: 38px; }
  .chead i, .legend i { width: 9px; height: 9px; border-radius: 2px; display: inline-block; margin-right: 6px; }
  .legend { display: flex; gap: 16px; flex-wrap: wrap; font-size: 12.5px; }
  .cmp { display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 10px; }
  .cmpcard { border: 1px solid var(--line); border-radius: var(--radius); padding: 8px 10px; display: flex; flex-direction: column; gap: 4px; }
  .dot { width: 9px; height: 9px; border-radius: 50%; display: inline-block; margin-right: 7px; }
  .who { color: var(--text); font-weight: 600; }
  .bar-row { display: grid; grid-template-columns: 70px minmax(120px, 1fr) 90px auto; gap: 12px; align-items: center; padding: 4px 0; }
  .track { height: 8px; background: #0b0d12; border-radius: 4px; overflow: hidden; }
  .fill { height: 100%; background: linear-gradient(90deg, var(--accent), var(--accent-2)); border-radius: 4px; }
  .val { text-align: right; }
  .meta { font-size: 12px; }
  .strong { font-weight: 600; }
  .tablebox { border: 1px solid var(--line); border-radius: var(--radius); overflow: auto; background: var(--panel); }
  table { width: 100%; border-collapse: collapse; font-size: 12.5px; }
  th, td { padding: 6px 10px; text-align: left; border-bottom: 1px solid var(--line); white-space: nowrap; }
  th { color: var(--muted); font-weight: 500; }
  tbody tr { cursor: pointer; }
  tbody tr:hover, tr.sel { background: #151925; }
  .gpu { max-width: 190px; overflow: hidden; text-overflow: ellipsis; }
  .n { text-align: right; }
  .pad { padding: 14px; }
  .note.err { color: var(--err); font-size: 13px; }
</style>
