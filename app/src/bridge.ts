// 与 core 门面通信的**唯一**出入口。
// 铁律：前端零业务逻辑——这里只发命令、订阅事件，不做任何计算。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type DeviceKind = "unclassified" | "camera" | "recorder" | "storage" | "ignored";

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
  kind: DeviceKind | null;
  kind_label: string | null;
};

export type DeviceRecord = {
  id: string;
  custom_name: string;
  kind: DeviceKind;
  last_seen: string;
  instance: number;
  last_label: string;
  total_bytes: number;
};

export type DestinationConfig = {
  id: string;
  root: string;
  template: string;
  enabled: boolean;
};

export type Project = {
  id: string;
  name: string;
  created_at: string;
  destinations: DestinationConfig[];
};

export type PresetMatch =
  | { kind: "device"; device_id: string }
  | { kind: "kind"; device_kind: DeviceKind }
  | { kind: "any_classified_source" };

export type Preset = {
  id: string;
  name: string;
  enabled: boolean;
  match: PresetMatch;
  project_id: string | null;
  verify: boolean;
  algorithm: "xxh64" | "md5";
  eject_after: boolean;
};

export type Settings = {
  auto_prefill: boolean;
  skip_confirmation: boolean;
  verify_default: boolean;
  algorithm: "xxh64" | "md5";
  retries: number;
  notify_on_finish: boolean;
  eject_after: boolean;
  format_after_copy: boolean;
  countdown_secs: number;
  /** `auto` / `zh` / `en`。默认跟随系统，判不出来落中文 */
  locale: string;
  /** 是否允许检查更新。默认关——不联网是默认承诺，联网得你主动开 */
  update_check: boolean;
};

export type Config = {
  version: number;
  projects: Project[];
  current_project: string | null;
  presets: Preset[];
  devices: DeviceRecord[];
  settings: Settings;
};

export type PlanDest = {
  landing_dir: string;
  required_bytes: number;
  available_bytes: number | null;
  sufficient: boolean | null;
};

export type ArrivalOutcomeKind =
  | "needs_classification"
  | "ignored"
  | "already_running"
  | "no_preset"
  | "no_project"
  | "no_source"
  | "no_new_source"
  | "insufficient_space"
  | "planned";

/** 每个「不能做」的结论都带一个「那就这样做」。 */
export type NextStep =
  | "classify_or_copy_once"
  | "copy_once"
  | "choose_another_destination"
  | "view_last_report"
  | "confirm_and_run"
  | "nothing";

export type Arrival = {
  next_step: NextStep;
  next_step_label: string;
  device_id: string;
  device_name: string;
  outcome: ArrivalOutcomeKind;
  summary: string;
  needs_attention: boolean;
  preset_name: string | null;
  requires_confirmation: boolean;
  to_copy: number;
  to_copy_bytes: number;
  skipped: number;
  destinations: PlanDest[];
  categories: [string, number, number][];
};

export type RunView = {
  task_id: string;
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

export type TaskRecord = {
  id: string;
  started_at: string;
  finished_at: string;
  source_id: string;
  source_name: string;
  project: string;
  algorithm: string;
  verified: boolean;
  total_files: number;
  total_bytes: number;
  copied: number;
  skipped: number;
  failed: number;
  status: "ok" | "partial" | "cancelled" | "failed";
  elapsed_secs: number;
  manifests: string[];
};

export type FileRecord = {
  relative_path: string;
  size: number;
  hash: string;
  status: string;
  reason: string | null;
  retries: number;
};

export type FormatAttempt = {
  id: string;
  at: string;
  device_id: string;
  device_name: string;
  trigger: string;
  checks: string;
  backup_task_id: string | null;
  result: string;
  reason: string | null;
};

export type CheckResult = { id: string; passed: boolean; detail: string };

export type FormatSafety = {
  report: { checks: CheckResult[]; backup_task_id: string | null };
  passed: boolean;
  root: string;
  device_name: string;
  label: string;
  /** 要求用户手输的确认串。无卷标的卡是固定词，不是空串 */
  confirm_phrase: string;
  file_system: string;
  countdown_secs: number;
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

export type AdhocDefaults = {
  project_id: string | null;
  project_name: string;
  /** 为 true 时项目字段旁要说明「会自动建这个项目」 */
  project_will_be_created: boolean;
  destinations: string[];
  verify: boolean;
  algorithm: "xxh64" | "md5";
};

export type AdhocInput = {
  device_root: string;
  /** 非空 = 沿用已有项目；空 = 用 project_name 现建一个 */
  project_id: string | null;
  project_name: string;
  destinations: string[];
  verify: boolean;
  algorithm: "xxh64" | "md5";
  eject_after: boolean;
};

/** 沉淀建议。`kind === "none"` 时后端根本不会发。 */
export type SinkSuggestion = {
  kind: "no_preset" | "diverged" | "none";
  device_id: string;
  device_name: string;
  project_name: string;
  changed: string[];
  preset_name: string | null;
  /** 该设备还没指认类型，沉淀时要一并收 */
  needs_kind: boolean;
  default_scope_label: string;
};

export type SinkScope = "device" | "kind" | "any";

export type Progress = {
  /** 稳定的机读代码。**判定用它**，不要拿 `stage` 比对——那是本地化文案 */
  stage_code: string;
  stage: string;
  percent: number;
  current: string | null;
  /** 算不出来时是 null——不拿 0 冒充 */
  bytes_per_sec: number | null;
  eta_secs: number | null;
  /** 导图派发才有（与 MapNode.path 同一口径）；其余入口为 null，按设备匹配即可 */
  node_path: string | null;
};

export type UpdateInfo = {
  available: boolean;
  current: string;
  version: string | null;
  notes: string | null;
  date: string | null;
};

export type BuildInfo = {
  version: string;
  commit: string;
  build_time: string;
  rustc: string;
  tauri: string;
  portable: boolean;
  data_dir: string;
  signature: string;
};

export type LicenseList = {
  self: { name: string; license: string };
  count: number;
  warning?: string;
  packages: { name: string; version: string; license: string; ecosystem: string }[];
};

export type ScanView = {
  files: number;
  total_bytes: number;
  junk_excluded: number;
  fingerprints: string[];
  categories: [string, number, number][];
};

/** 导图上的一条「设备 → 节点」落位（画布上的一根连线）。 */
export type MapAssignment = {
  id: string;
  device_id: string;
  /** 连线上挂的名字——颜色 MUST NOT 是唯一信息载体 */
  device_name: string;
};

export type MapNode = {
  id: string;
  name: string;
  parent: string | null;
  /** 子节点顺序即画布顺序，稳定，由 core 定 */
  children: string[];
  /**
   * 节点在树里的路径（各段以 `/` 相连），core 算好下发。
   * 与进度事件里的 `node_path` 同一口径——画布拿两串比对就能锚定
   * 同一张卡的哪根连线在跑，不在前端爬树拼路径
   */
  path: string;
  assignments: MapAssignment[];
};

export type MapView = {
  /** 导图长在哪个项目上。null = 还没有项目，界面显示空态引导 */
  project_id: string | null;
  project_name: string | null;
  nodes: MapNode[];
  templates: { id: string; name: string }[];
};

export type MapDispatchResult = {
  started: number;
  /** 没派出去的逐条带原因——不做 all-or-nothing */
  rejected: { device_name: string; reason: string }[];
};

/** 刷新预览：可并入的候选 + 名字进不了树、只呈现不合并的目录（原因是 core 成句） */
export type MapRefreshPreview = {
  additions: string[];
  skipped: { path: string; reason: string }[];
};

/** `task-started` 的载荷。node_path 只有导图派发才有，其余入口为 null */
export type TaskStarted = {
  device_id: string;
  node_path: string | null;
};

/** 正在跑任务的进度快照，面板重挂载时垫底用；排队中（尚未开跑）的不在里面 */
export type RunningTask = {
  device_id: string;
  percent: number;
  stage_code: string;
  node_path: string | null;
};

export const api = {
  // 配置
  getConfig: () => invoke<Config>("get_config"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  configPath: () => invoke<string>("config_path"),
  upsertProject: (input: {
    id?: string | null;
    name: string;
    destinations: { id?: string | null; root: string; template: string; enabled: boolean }[];
  }) => invoke<Config>("upsert_project", { input }),
  deleteProject: (id: string) => invoke<Config>("delete_project", { id }),
  setCurrentProject: (id: string) => invoke<Config>("set_current_project", { id }),
  upsertPreset: (preset: Preset) => invoke<Config>("upsert_preset", { preset }),
  deletePreset: (id: string) => invoke<Config>("delete_preset", { id }),
  movePreset: (id: string, up: boolean) => invoke<Config>("move_preset", { id, up }),
  setDeviceKind: (id: string, kind: DeviceKind) => invoke<Config>("set_device_kind", { id, kind }),
  renameDevice: (id: string, name: string) => invoke<Config>("rename_device", { id, name }),
  forgetDevice: (id: string) => invoke<Config>("forget_device", { id }),
  previewPath: (root: string, template: string, project: string, device: string) =>
    invoke<string>("preview_path", { root, template, project, device }),
  validateCountdown: (secs: number) => invoke<number>("validate_countdown_secs", { secs }),

  // 设备与到达
  listDevices: () => invoke<Device[]>("list_devices"),
  startWatching: () => invoke<boolean>("start_watching"),
  arriveNow: (deviceRoot: string) => invoke<Arrival>("arrive_now", { deviceRoot }),
  confirmAndRun: (deviceId: string) => invoke<RunView>("confirm_and_run", { deviceId }),
  dismissArrival: (deviceId: string) => invoke<void>("dismiss_arrival", { deviceId }),
  cancelCopy: () => invoke<void>("cancel_copy"),
  setPaused: (paused: boolean) => invoke<void>("set_paused", { paused }),
  ejectDevice: (deviceRoot: string) => invoke<void>("eject_device", { deviceRoot }),
  scan: (source: string) => invoke<ScanView>("scan", { source }),

  // 临时拷贝：不依赖预设的一次性任务
  adhocPrefill: () => invoke<AdhocDefaults>("adhoc_prefill"),
  planAdhoc: (input: AdhocInput) => invoke<Arrival>("plan_adhoc", { input }),
  /** 把刚跑完那次的做法记成预设。一次点击完成，不跳编辑器 */
  sinkPreset: (scope: SinkScope, kind?: DeviceKind, name?: string) =>
    invoke<Config>("sink_preset", { scope, kind, name }),

  // 导图：树与落位全在 core，这里只发命令。
  // 派发复用临时拷贝的执行路径——下游分不出任务来自导图（设计 D2）
  mapGet: () => invoke<MapView>("map_get"),
  mapAddNode: (parentId: string | null, name: string) =>
    invoke<MapView>("map_add_node", { parentId, name }),
  mapRenameNode: (nodeId: string, name: string) =>
    invoke<MapView>("map_rename_node", { nodeId, name }),
  mapDeleteNode: (nodeId: string) => invoke<MapView>("map_delete_node", { nodeId }),
  mapMoveNode: (nodeId: string, newParentId: string | null) =>
    invoke<MapView>("map_move_node", { nodeId, newParentId }),
  mapAssign: (deviceId: string, nodeId: string) =>
    invoke<MapView>("map_assign", { deviceId, nodeId }),
  mapUnassign: (assignmentId: string) => invoke<MapView>("map_unassign", { assignmentId }),
  mapDispatch: () => invoke<MapDispatchResult>("map_dispatch"),
  /** 刷新预览：候选 + 无法并入清单。只读，不动磁盘 */
  mapRefreshPreview: () => invoke<MapRefreshPreview>("map_refresh_preview"),
  /**
   * 确认后并入。`confirmed` 就是预览返回、用户点头的那份 additions **原样传回**——
   * 落地只并「重算 diff ∩ 确认集」，预览之后磁盘上新冒出来的目录不会被顺手收编
   */
  mapRefreshApply: (confirmed: string[]) =>
    invoke<MapView>("map_refresh_apply", { confirmed }),
  /** 进行中任务的进度快照。只读；面板挂载时先垫底，再接事件流 */
  runningSnapshot: () => invoke<RunningTask[]>("running_snapshot"),
  mapTemplateSave: (name: string) => invoke<MapView>("map_template_save", { name }),
  mapTemplateApply: (templateId: string) =>
    invoke<MapView>("map_template_apply", { templateId }),
  mapTemplateDelete: (templateId: string) =>
    invoke<MapView>("map_template_delete", { templateId }),

  // 台账
  listHistory: (onlyFailed = false, limit?: number) =>
    invoke<TaskRecord[]>("list_history", { onlyFailed, limit }),
  taskFiles: (taskId: string, status?: string) =>
    invoke<FileRecord[]>("task_files", { taskId, status }),
  clearHistory: () => invoke<void>("clear_history"),
  formatAttempts: () => invoke<FormatAttempt[]>("format_attempts"),
  reportHtml: (manifestPath: string) => invoke<string>("report_html", { manifestPath }),
  runAudit: (manifestPath: string) => invoke<AuditResult>("run_audit", { manifestPath }),

  // 格式化
  checkFormat: (deviceRoot: string) => invoke<FormatSafety>("check_format", { deviceRoot }),
  doFormat: (deviceRoot: string, typedLabel: string) =>
    invoke<void>("do_format", { deviceRoot, typedLabel }),

  // 交给系统默认程序打开。路径由后端自己算或先校验——
  // 前端拿不到「让 shell 打开任意路径」这个能力。
  openConfigFile: () => invoke<void>("open_config_file"),
  /** 打开上手教程。地址写死在后端，前端传不进别的 URL */
  openGuide: () => invoke<void>("open_guide"),
  openReportFile: (manifestPath: string) => invoke<void>("open_report_file", { manifestPath }),
  revealLandingDir: (manifestPath: string) =>
    invoke<void>("reveal_landing_dir", { manifestPath }),

  appVersion: () => invoke<string>("app_version"),
  /** 查一次有没有新版本。**只在用户点了按钮时调**，没有后台轮询 */
  checkUpdate: () => invoke<UpdateInfo>("check_update"),
  /** 下载并安装。检查与安装是两个动作，中间隔着用户的一次决定 */
  installUpdate: () => invoke<void>("install_update"),
  buildInfo: () => invoke<BuildInfo>("build_info"),
  thirdPartyLicenses: () => invoke<LicenseList>("third_party_licenses"),
};

export const events = {
  onArrival: (cb: (a: Arrival) => void) => listen<Arrival>("device-arrived", (e) => cb(e.payload)),
  onRemoved: (cb: () => void) => listen("device-removed", () => cb()),
  onStage: (cb: (p: Progress) => void) => listen<Progress>("task-stage", (e) => cb(e.payload)),
  onProgress: (cb: (p: Progress) => void) => listen<Progress>("task-progress", (e) => cb(e.payload)),
  onFileFailed: (cb: (p: { path: string; reason: string }) => void) =>
    listen<{ path: string; reason: string }>("task-file-failed", (e) => cb(e.payload)),
  onNotice: (cb: (m: string) => void) => listen<string>("task-notice", (e) => cb(e.payload)),
  onWatchError: (cb: (m: string) => void) => listen<string>("watch-error", (e) => cb(e.payload)),
  onTaskStarted: (cb: (p: TaskStarted) => void) =>
    listen<TaskStarted>("task-started", (e) => cb(e.payload)),
  onTaskFinished: (cb: (r: RunView) => void) => listen<RunView>("task-finished", (e) => cb(e.payload)),
  onTaskFailed: (cb: (m: string) => void) => listen<string>("task-failed", (e) => cb(e.payload)),
  /** 拷完全绿且危险区开了「拷完自动格式化」时，后端提议格式化源卡。提议 ≠ 执行。 */
  onFormatProposed: (cb: (s: FormatSafety) => void) =>
    listen<FormatSafety>("format-proposed", (e) => cb(e.payload)),
  /** 「这次的做法和记住的不一样」时后端发来的沉淀建议 */
  onSinkSuggested: (cb: (s: SinkSuggestion) => void) =>
    listen<SinkSuggestion>("sink-suggested", (e) => cb(e.payload)),
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

export function duration(secs: number): string {
  if (secs < 1) return "不到 1 秒";
  if (secs < 60) return `${secs} 秒`;
  if (secs < 3600) return `${Math.floor(secs / 60)} 分 ${secs % 60} 秒`;
  return `${Math.floor(secs / 3600)} 小时 ${Math.floor((secs % 3600) / 60)} 分`;
}

export const KIND_LABEL: Record<DeviceKind, string> = {
  unclassified: "未分类",
  camera: "摄影卡",
  recorder: "录音卡",
  storage: "素材盘",
  ignored: "忽略",
};

export const STATUS_LABEL: Record<TaskRecord["status"], string> = {
  ok: "全部通过",
  partial: "部分失败",
  cancelled: "已取消",
  failed: "失败",
};
