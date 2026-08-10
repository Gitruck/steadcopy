// 与 core 门面通信的**唯一**出入口。
// 铁律：前端零业务逻辑——这里只发命令、订阅事件，不做任何计算。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Device = {
  id: string;
  name: string;
  root: string;
  file_system: string;
  bus: string;
  total_bytes: number;
  free_bytes: number;
  is_system: boolean;
  can_be_source: boolean;
  fingerprints: string[];
};

export type ScanView = {
  files: number;
  total_bytes: number;
  junk_excluded: number;
  fingerprints: string[];
  categories: [string, number, number][];
};

export type PlanDest = {
  landing_dir: string;
  required_bytes: number;
  available_bytes: number | null;
  sufficient: boolean | null;
};

export type PlanView = {
  to_copy: number;
  to_copy_bytes: number;
  skipped: number;
  no_source: boolean;
  no_new_source: boolean;
  destinations: PlanDest[];
  notices: string[];
};

export type TaskInput = {
  source: string;
  destinations: string[];
  project: string;
  device_name: string;
  template: string;
  verify: boolean;
  algorithm: "xxh64" | "md5";
};

export type RunView = {
  copied: number;
  skipped: number;
  failed: number;
  bytes_copied: number;
  cancelled: boolean;
  all_succeeded: boolean;
  manifests: string[];
  notices: string[];
  failures: { path: string; reason: string }[];
};

export type HistoryItem = {
  manifest_path: string;
  project: string;
  device: string;
  landing_dir: string;
  created_at: string;
  files: number;
  verified: number;
  total_bytes: number;
  algorithm: string;
};

export type Progress = {
  stage: string;
  percent: number;
  current: string | null;
  done: number;
  total: number;
};

export type AuditResult = {
  algorithm: string;
  intact: { relative_path: string; size: number }[];
  moved: { from: string; to: string; size: number }[];
  missing: { relative_path: string; size: number; expected_hash: string }[];
  added: { relative_path: string; size: number; hash: string }[];
  complete: boolean;
  unverified_at_copy: number;
};

export const api = {
  listDevices: () => invoke<Device[]>("list_devices"),
  scan: (source: string) => invoke<ScanView>("scan", { source }),
  plan: (input: TaskInput) => invoke<PlanView>("plan", { input }),
  startCopy: (input: TaskInput) => invoke<RunView>("start_copy", { input }),
  cancelCopy: () => invoke<void>("cancel_copy"),
  listHistory: (roots: string[]) => invoke<HistoryItem[]>("list_history", { roots }),
  reportHtml: (manifestPath: string) =>
    invoke<string>("report_html", { manifestPath }),
  runAudit: (manifestPath: string) =>
    invoke<AuditResult>("run_audit", { manifestPath }),
  appVersion: () => invoke<string>("app_version"),
};

export const events = {
  onStage: (cb: (p: Progress) => void) => listen<Progress>("task-stage", (e) => cb(e.payload)),
  onProgress: (cb: (p: Progress) => void) =>
    listen<Progress>("task-progress", (e) => cb(e.payload)),
  onFileFailed: (cb: (p: { path: string; reason: string }) => void) =>
    listen<{ path: string; reason: string }>("task-file-failed", (e) => cb(e.payload)),
  onNotice: (cb: (m: string) => void) => listen<string>("task-notice", (e) => cb(e.payload)),
};

export type { UnlistenFn };

/** 展示用格式化。纯呈现，不参与任何判定。 */
export function bytes(n: number): string {
  const u = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return i === 0 ? `${n} B` : `${v.toFixed(2)} ${u[i]}`;
}
