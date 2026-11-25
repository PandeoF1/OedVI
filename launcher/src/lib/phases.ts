// Phases of the game (menu, intro, story…) as bands on charts and as stats.
import type { PerfSession } from "./api";

export interface Band {
  name: string;
  from: number;
  to: number;
}

const COLORS = ["#60a5fa", "#f472b6", "#fbbf24", "#34d399", "#a78bfa", "#fb923c", "#22d3ee", "#f87171"];

/** The same name always has the same color, in every chart. */
export function phaseColor(name: string): string {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return COLORS[h % COLORS.length];
}

export function bandsOfSession(s: PerfSession): Band[] {
  const p = s.phases ?? [];
  return p.map((ph, i) => ({ name: ph.name, from: ph.start, to: p[i + 1]?.start ?? s.duration_secs }));
}

/** Bands of a live run: one value per second, so an index is a second. */
export function bandsOfPoints(points: { phase?: string | null }[]): Band[] {
  const out: Band[] = [];
  points.forEach((p, i) => {
    if (!p.phase) return;
    const last = out[out.length - 1];
    if (last && last.name === p.phase && last.to >= i) last.to = i + 1;
    else out.push({ name: p.phase, from: i, to: i + 1 });
  });
  return out;
}

export interface PhaseNumbers {
  avg: number;
  min: number;
  max: number;
  p1: number;
}

export interface PhaseSummary extends Band {
  secs: number;
  metrics: Record<string, PhaseNumbers>;
}

/** Numbers of each metric inside each phase of a session. */
export function summarizePhases(s: PerfSession): PhaseSummary[] {
  const ms = s.metrics ?? {};
  return bandsOfSession(s).map((b) => {
    const metrics: Record<string, PhaseNumbers> = {};
    for (const [name, m] of Object.entries(ms)) {
      const n = m.series.length;
      if (n < 2) continue;
      const dt = s.duration_secs / (n - 1);
      const v = m.series.filter((_, i) => i * dt >= b.from && i * dt < b.to && Number.isFinite(_)).sort((a, c) => a - c);
      if (!v.length) continue;
      metrics[name] = {
        avg: v.reduce((a, c) => a + c, 0) / v.length,
        min: v[0],
        max: v[v.length - 1],
        p1: v[Math.round((v.length - 1) * 0.01)],
      };
    }
    return { ...b, secs: b.to - b.from, metrics };
  });
}
