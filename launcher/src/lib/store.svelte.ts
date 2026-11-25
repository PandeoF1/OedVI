// Shared app state, fed by commands and backend events.
import { listen } from "@tauri-apps/api/event";
import {
  api,
  type BuilderListView,
  type LogLine,
  type ManifestInfo,
  type ProgressSnapshot,
  type ServerInfo,
  type SettingsView,
  type Status,
  type SyncReport,
  type TaskKind,
  type UpdateInfo,
} from "./api";
import { initLive, resetLive } from "./live.svelte";
import { t } from "./i18n.svelte";
import { toast } from "./toast.svelte";

export type Page = "game" | "builders" | "stats" | "editor" | "settings";

export const app = $state({
  page: "game" as Page,
  settings: null as SettingsView | null,
  server: null as ServerInfo | null,
  manifest: null as ManifestInfo | null,
  manifestError: null as string | null,
  status: null as Status | null,
  statusLoading: false,
  sync: null as ProgressSnapshot | null,
  syncing: false,
  lastReport: null as SyncReport | null,
  builders: null as BuilderListView | null,
  selectedBuilder: null as string | null,
  /** Builder id → running task. */
  tasks: {} as Record<string, { kind: TaskKind; action: string }>,
  /** Bumped when log lines arrive (the lines themselves are in `logBuf`). */
  logVersion: 0,
  /** Bumped when a run is recorded, so the stats reload. */
  statsEpoch: 0,
  /** Bumped when a task ends, so detail panels reload. */
  taskEpoch: 0,
  update: null as UpdateInfo | null,
  updateError: null as string | null,
  /** Bytes downloaded / total while an update installs. */
  updating: null as { downloaded: number; total: number | null } | null,
});

export async function checkUpdate() {
  try {
    app.update = await api.checkUpdate();
    app.updateError = null;
  } catch (e) {
    app.updateError = String(e);
  }
}

export async function installUpdate() {
  app.updating = { downloaded: 0, total: null };
  try {
    await api.installUpdate();
  } catch (e) {
    app.updating = null;
    toast(t("Update failed: {e}", { e: String(e) }), "error");
  }
}

const MAX_LOG = 5000;

export async function loadSettings() {
  app.settings = await api.getSettings();
}

export async function refreshServer() {
  try {
    app.server = await api.serverInfo();
  } catch (e) {
    app.server = { online: false, error: String(e), version: null, scan: null };
  }
}

export async function refreshGame() {
  try {
    app.manifest = await api.refreshManifest();
    app.manifestError = null;
  } catch (e) {
    app.manifestError = String(e);
    app.manifest = null;
    app.status = null;
    return;
  }
  await refreshStatus();
}

export async function refreshStatus() {
  app.statusLoading = true;
  try {
    app.status = await api.localStatus();
  } catch (e) {
    toast(String(e), "error");
  } finally {
    app.statusLoading = false;
  }
}

export async function refreshBuilders(refresh = true) {
  try {
    app.builders = await api.listBuilders(refresh);
    for (const row of app.builders.builders) {
      if (row.task) app.tasks[row.def.id] = { kind: row.task.kind, action: row.task.action };
    }
  } catch (e) {
    toast(String(e), "error");
  }
}

// Plain arrays: thousands of lines must not become reactive proxies.
const logBuf = new Map<string, LogLine[]>();

/** The lines of a builder; reading it subscribes to new ones. */
export function logsOf(id: string): LogLine[] {
  app.logVersion;
  return [...(logBuf.get(id) ?? [])];
}

export function clearLogs(id: string) {
  logBuf.set(id, []);
  app.logVersion++;
  api.clearLogs(id);
}

export async function loadLogs(id: string) {
  if (!logBuf.has(id)) {
    logBuf.set(id, await api.taskLogs(id));
    app.logVersion++;
  }
}

let started = false;

export async function init() {
  if (started) return;
  started = true;

  await listen<ProgressSnapshot>("sync-progress", (e) => {
    app.sync = e.payload;
    app.syncing = !["done", "cancelled", "failed"].includes(e.payload.phase);
  });
  await listen<SyncReport | { Err: string } | string>("sync-done", (e) => {
    app.syncing = false;
    const p = e.payload as any;
    if (p && typeof p === "object" && "Ok" in p) {
      const r = p.Ok as SyncReport;
      app.lastReport = r;
      if (r.cancelled) toast(t("Sync interrupted"), "info");
      else if (r.errors.length) toast(t("{n} file(s) failed", { n: r.errors.length }), "error");
      else toast(t("Game files up to date ✔"), "success");
    } else if (p && typeof p === "object" && "Err" in p) {
      toast(p.Err, "error");
    }
    refreshStatus();
  });
  await listen<{ id: string; lines: LogLine[] }>("task-logs", (e) => {
    const { id, lines } = e.payload;
    let buf = logBuf.get(id);
    if (!buf) logBuf.set(id, (buf = []));
    buf.push(...lines);
    if (buf.length > MAX_LOG) buf.splice(0, buf.length - MAX_LOG);
    app.logVersion++;
  });
  await initLive();
  await listen("stats-changed", () => {
    app.statsEpoch++;
  });
  await listen<{ id: string; kind: TaskKind; action: string }>("task-start", (e) => {
    app.tasks[e.payload.id] = { kind: e.payload.kind, action: e.payload.action };
    if (e.payload.kind === "run") resetLive(e.payload.id);
  });
  await listen<{ id: string; kind: TaskKind; action: string; ok: boolean; message: string }>("task-end", (e) => {
    const { id, ok, message } = e.payload;
    delete app.tasks[id];
    app.taskEpoch++;
    refreshBuilders(false);
    const name = app.builders?.builders.find((b) => b.def.id === id)?.def.name ?? id;
    toast(`${name}: ${message}`, ok ? "success" : "error");
  });

  await listen<{ downloaded: number; total: number | null }>("update-progress", (e) => {
    app.updating = e.payload;
  });

  await loadSettings();
  // A sync may still run if the window was reloaded.
  const p = await api.syncProgress();
  if (p) {
    app.sync = p;
    app.syncing = true;
  }
  await connect();
  setInterval(() => isRemote() && refreshServer(), 15000);
  setInterval(() => (!isRemote() || app.server?.online) && checkUpdate(), 30 * 60 * 1000);
}

/** Connected to a server (remote mode with an address). */
export function isRemote(): boolean {
  const s = app.settings?.settings;
  return s?.mode === "remote" && !!s.server_url.trim();
}

/** Loads what depends on the mode: the server and the game files only exist
 *  in remote mode, the builders always. Called at start and after the
 *  settings change. */
export async function connect() {
  if (isRemote()) {
    await Promise.all([refreshServer(), refreshGame(), refreshBuilders(true)]);
  } else {
    app.server = null;
    app.manifest = null;
    app.status = null;
    app.manifestError = null;
    await refreshBuilders(true);
  }
  if (!isRemote() || app.server?.online) checkUpdate();
}
