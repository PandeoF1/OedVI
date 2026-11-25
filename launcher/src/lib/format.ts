import { t } from "./i18n.svelte";

export function bytes(n: number | null | undefined): string {
  if (n == null) return "—";
  const units = [t("B"), t("KB"), t("MB"), t("GB"), t("TB")];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : v >= 10 ? 1 : 2)} ${units[i]}`;
}

export function duration(secs: number): string {
  if (!isFinite(secs) || secs < 0) return "—";
  if (secs < 60) return `${Math.round(secs)} s`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m} min ${Math.round(secs % 60)} s`;
  const h = Math.floor(m / 60);
  return `${h} h ${m % 60} min`;
}

export function ago(unixSecs: number | null | undefined): string {
  if (!unixSecs) return t("never");
  const d = Date.now() / 1000 - unixSecs;
  if (d < 60) return t("just now");
  if (d < 3600) return t("{n} min ago", { n: Math.floor(d / 60) });
  if (d < 86400) return t("{n} h ago", { n: Math.floor(d / 3600) });
  return t("{n} d ago", { n: Math.floor(d / 86400) });
}

export function pct(part: number, total: number): number {
  return total > 0 ? Math.min(100, (part / total) * 100) : 0;
}

/** `plural(n, "{n} file", "{n} files")` */
export function plural(n: number, one: string, many: string): string {
  return t(n === 1 ? one : many, { n });
}
