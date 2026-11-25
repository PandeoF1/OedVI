// Types mirror the Rust structs serialized by the launcher's commands.
import { invoke } from "@tauri-apps/api/core";

export interface Settings {
  mode: "standalone" | "remote";
  server_url: string;
  token: string;
  root_dir: string;
  game_dir: string | null;
  download_jobs: number;
  delete_extra: boolean;
  auto_install_tools: boolean;
  builder_env: Record<string, Record<string, string>>;
  machine_name: string;
  share_stats: boolean;
}

export interface PerfSession {
  id: string;
  builder: string;
  commit: string;
  machine: string;
  os: string;
  launcher: string;
  started_at: number;
  duration_secs: number;
  samples: number;
  avg: number;
  min: number;
  max: number;
  low1: number;
  series: number[];
  metrics?: Record<string, Metric>;
  specs?: Specs | null;
  phases?: Phase[];
}

export interface Phase {
  name: string;
  start: number;
}

export interface PhaseRowDto {
  builder: string;
  commit: string;
  phase: string;
  sessions: number;
  machines: number;
  secs: number;
  fps: number;
  fps_low1: number;
  frame_ms: number;
  last_at: number;
}

export interface Metric {
  samples: number;
  avg: number;
  min: number;
  max: number;
  p1: number;
  p99: number;
  series: number[];
}

export interface Gpu {
  name: string;
  driver: string;
  driver_date: string;
}

export interface Specs {
  cpu: string;
  cores: number;
  threads: number;
  ram_mb: number;
  os: string;
  gpus: Gpu[];
}

export interface LivePointDto {
  t: number;
  values: Record<string, number>;
  phase?: string | null;
}

export interface PerfRow {
  builder: string;
  commit: string;
  sessions: number;
  machines: number;
  avg: number;
  best_avg: number;
  low1: number;
  last_at: number;
}

export interface StatsReport {
  rows: PerfRow[];
  phases?: PhaseRowDto[];
  recent: PerfSession[];
}

export interface Layout {
  root: string;
  game: string;
  repos: string;
  builders: string;
  templates: string;
  cache: string;
  data: string;
  state: string;
  logs: string;
}

export interface SettingsView {
  settings: Settings;
  layout: Layout;
  path: string;
}

export interface ScanStatus {
  scanning: boolean;
  files_total: number;
  files_done: number;
  bytes_total: number;
  bytes_to_hash: number;
  bytes_hashed: number;
  current: string | null;
  manifest_version: number | null;
  last_scan_at: number | null;
  last_scan_secs: number | null;
  last_error: string | null;
}

export interface ServerInfo {
  online: boolean;
  error: string | null;
  version: string | null;
  scan: ScanStatus | null;
}

export interface GroupInfo {
  name: string;
  label: string | null;
  required: boolean;
  file_count: number;
  size: number;
}

export interface ManifestInfo {
  version: number;
  generated_at: number;
  total_size: number;
  file_count: number;
  groups: GroupInfo[];
  offline: boolean;
  error: string | null;
}

export type FileState =
  | "ok"
  | "missing"
  | "partial"
  | "outdated"
  | "unverified"
  | "unselected"
  | "to_remove"
  | "extra";

export interface FileStatus {
  path: string;
  size: number;
  local_size: number | null;
  group: string;
  required: boolean;
  selected: boolean;
  state: FileState;
}

export interface Summary {
  manifest_version: number;
  synced_version: number | null;
  files_total: number;
  bytes_total: number;
  selected_files: number;
  selected_bytes: number;
  ok_files: number;
  ok_bytes: number;
  missing: number;
  partial: number;
  outdated: number;
  unverified: number;
  to_remove: number;
  extra: number;
  download_max_bytes: number;
  free_space: number | null;
  up_to_date: boolean;
}

export interface Status {
  summary: Summary;
  files: FileStatus[];
}

export interface SyncError {
  path: string;
  message: string;
}

export interface ProgressSnapshot {
  phase: "scan" | "remove" | "sync" | "done" | "cancelled" | "failed";
  files_total: number;
  files_done: number;
  verify_bytes_total: number;
  verified_bytes: number;
  download_bytes_needed: number;
  downloaded_bytes: number;
  removed: number;
  current: string[];
  errors: SyncError[];
  elapsed_secs: number;
}

export interface SyncReport {
  files_synced: number;
  bytes_downloaded: number;
  removed: number;
  errors: SyncError[];
  cancelled: boolean;
}

export interface BuilderDef {
  id: string;
  name: string;
  description: string | null;
  stack: string;
  tags: string[];
  order: number;
  repo: string;
  ref: string;
  checkout: string | null;
  workdir: string | null;
  vars: Record<string, string>;
  env: Record<string, string>;
  build_env: Record<string, string>;
  run_env: Record<string, string>;
  build: string[][];
  run: string[];
  run_workdir: string | null;
  requires: string[];
}

export interface BuilderLoadError {
  id: string;
  message: string;
}

export interface BuilderList {
  builders: BuilderDef[];
  errors: BuilderLoadError[];
}

export type TaskKind = "setup" | "git" | "build" | "run";

export interface TaskInfo {
  kind: TaskKind;
  action: string;
  started_at: number;
  pid: number | null;
}

export interface BuilderRow {
  def: BuilderDef;
  cloned: boolean;
  task: TaskInfo | null;
}

export interface BuilderListView {
  builders: BuilderRow[];
  errors: BuilderLoadError[];
  offline: string | null;
}

export interface Commit {
  hash: string;
  short: string;
  subject: string;
  author: string;
  time: number;
}

export interface GitStatus {
  cloned: boolean;
  head: Commit | null;
  branch: string | null;
  dirty: boolean;
  behind: number;
  ahead: number;
  incoming: Commit[];
  ref_is_branch: boolean;
  remote_url: string | null;
}

export interface CmdSpec {
  argv: string[];
  cwd: string;
  env: Record<string, string>;
}

export interface Prepared {
  def: BuilderDef;
  checkout_dir: string;
  work_dir: string;
  build_steps: CmdSpec[];
  run: CmdSpec;
  recipe: string;
}

export interface BuildRecord {
  commit: string;
  ok: boolean;
  finished_at: number;
  secs: number;
}

export interface BuildState {
  head: string | null;
  last: BuildRecord | null;
  up_to_date: boolean;
}

export interface ToolStatus {
  id: string;
  label: string;
  installed: boolean;
  found: string | null;
  installable: boolean;
  admin: boolean;
  size: string | null;
}

export interface BuilderDetail {
  def: BuilderDef;
  prepared: Prepared | null;
  prepare_error: string | null;
  git: GitStatus;
  build: BuildState | null;
  tools: ToolStatus[];
  task: TaskInfo | null;
  shared_with: string[];
}

export interface LogLine {
  stream: "stdout" | "stderr" | "info";
  text: string;
  t: number;
}

export interface UpdateInfo {
  current: string;
  available: string | null;
  notes: string | null;
  date: string | null;
  disabled: string | null;
  error: string | null;
}

export type BuilderAction = "tools" | "fetch" | "pull" | "resync" | "build" | "rebuild" | "play";
export type RawKind = "builders" | "templates";

export const api = {
  getSettings: () => invoke<SettingsView>("get_settings"),
  saveSettings: (settings: Settings) => invoke<SettingsView>("save_settings", { settings }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  serverInfo: () => invoke<ServerInfo>("server_info"),
  serverRescan: () => invoke<void>("server_rescan"),
  refreshManifest: () => invoke<ManifestInfo>("refresh_manifest"),
  localStatus: () => invoke<Status>("local_status"),
  setSelection: (kind: "all" | "group" | "path", target: string | null, selected: boolean) =>
    invoke<Status>("set_selection", { kind, target, selected }),
  startSync: (verify: boolean) => invoke<void>("start_sync", { verify }),
  cancelSync: () => invoke<void>("cancel_sync"),
  syncProgress: () => invoke<ProgressSnapshot | null>("sync_progress"),
  listBuilders: (refresh: boolean) => invoke<BuilderListView>("list_builders", { refresh }),
  gameFolderInfo: () => invoke<{ path: string; exists: boolean; entries: number }>("game_folder_info"),
  markPhase: (id: string, name: string) => invoke<void>("mark_phase", { id, name }),
  liveHistory: (id: string) => invoke<LivePointDto[]>("live_history", { id }),
  machineSpecs: () => invoke<Specs>("machine_specs"),
  openOverlay: (id: string) => invoke<void>("open_overlay", { id }),
  statsLocal: () => invoke<PerfSession[]>("stats_local"),
  statsRemote: () => invoke<StatsReport>("stats_remote"),
  builderDetail: (id: string) => invoke<BuilderDetail>("builder_detail", { id }),
  builderAction: (id: string, action: BuilderAction) => invoke<void>("builder_action", { id, action }),
  stopTask: (id: string) => invoke<void>("stop_task", { id }),
  taskLogs: (id: string) => invoke<LogLine[]>("task_logs", { id }),
  clearLogs: (id: string) => invoke<void>("clear_logs", { id }),
  setBuilderEnv: (id: string, env: Record<string, string>) => invoke<void>("set_builder_env", { id, env }),
  listIds: (kind: RawKind) => invoke<string[]>("list_ids", { kind }),
  readRaw: (kind: RawKind, id: string) => invoke<string | null>("read_raw", { kind, id }),
  checkRaw: (kind: RawKind, id: string, text: string) => invoke<BuilderList>("check_raw", { kind, id, text }),
  writeRaw: (kind: RawKind, id: string, text: string) => invoke<BuilderList>("write_raw", { kind, id, text }),
  deleteRaw: (kind: RawKind, id: string) => invoke<void>("delete_raw", { kind, id }),
  checkUpdate: () => invoke<UpdateInfo>("check_update"),
  installUpdate: () => invoke<void>("install_update"),
};
