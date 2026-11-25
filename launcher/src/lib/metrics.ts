import { t } from "./i18n.svelte";
// What each measured number is called and drawn like.
export interface MetricMeta {
  label: string;
  unit: string;
  color: string;
  /** Where it comes from, for the legend. */
  source: "game" | "system";
}

export const META: Record<string, MetricMeta> = {
  fps: { label: "FPS", unit: "", color: "#6ee7b7", source: "game" },
  frame_ms: { label: "Frame time", unit: "ms", color: "#fbbf24", source: "game" },
  gpu: { label: "GPU (reported by the game)", unit: "%", color: "#f472b6", source: "game" },
  cpu: { label: "CPU (reported by the game)", unit: "%", color: "#60a5fa", source: "game" },
  ram: { label: "RAM (reported by the game)", unit: "MB", color: "#a78bfa", source: "game" },
  draws: { label: "Draws per frame", unit: "", color: "#34d399", source: "game" },
  proc_cpu: { label: "Game CPU", unit: "%", color: "#38bdf8", source: "system" },
  proc_ram: { label: "Game RAM", unit: "MB", color: "#c084fc", source: "system" },
  proc_gpu: { label: "Game GPU", unit: "%", color: "#fb7185", source: "system" },
  proc_vram: { label: "Game VRAM", unit: "MB", color: "#e879f9", source: "system" },
  disk_read: { label: "Disk: read", unit: "MB/s", color: "#fdba74", source: "system" },
  disk_write: { label: "Disk: write", unit: "MB/s", color: "#fb923c", source: "system" },
  sys_cpu: { label: "PC CPU", unit: "%", color: "#93c5fd", source: "system" },
  sys_ram: { label: "PC RAM", unit: "MB", color: "#d8b4fe", source: "system" },
};

export const ORDER = Object.keys(META);

const PALETTE = ["#f87171", "#facc15", "#4ade80", "#22d3ee", "#818cf8", "#f0abfc"];

export function meta(name: string): MetricMeta {
  if (META[name]) return { ...META[name], label: t(META[name].label) };
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return { label: name, unit: "", color: PALETTE[h % PALETTE.length], source: "game" };
}

/** Known metrics first, in a stable order, then the builder's own. */
export function sortNames(names: Iterable<string>): string[] {
  const rank = (n: string) => (ORDER.includes(n) ? ORDER.indexOf(n) : ORDER.length);
  return [...names].sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
}

export function fmtValue(v: number, unit = ""): string {
  if (!Number.isFinite(v)) return "–";
  const a = Math.abs(v);
  const s = a >= 1000 ? v.toFixed(0) : a >= 100 ? v.toFixed(1) : a >= 10 ? v.toFixed(1) : v.toFixed(2);
  return unit ? `${s} ${unit}` : s;
}

export function fmtTime(sec: number): string {
  const s = Math.max(0, Math.round(sec));
  const m = Math.floor(s / 60);
  return m >= 60
    ? `${Math.floor(m / 60)}:${String(m % 60).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`
    : `${m}:${String(s % 60).padStart(2, "0")}`;
}
