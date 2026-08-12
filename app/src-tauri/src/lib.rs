//! 稳拷桌面应用的 Tauri 后端。
//!
//! 规范：`openspec/changes/add-steadcopy-app/specs/app-shell/spec.md`
//!       `openspec/changes/add-steadcopy-preset-autorun/specs/preset-autorun/spec.md`
//!
//! 铁律：**前端零业务逻辑。** 本文件只做「门面命令 → core」的桥接与事件转发，
//! 路径渲染 / 增量判定 / 空间计算 / 哈希 / 安全检查一律在 core 里算。

// 纯护栏模块：不参与运行期，只在测试里钉住「两版安装包必须是同一个产品」。
// 它防的那个 bug（productName 分叉 ⇒ 装出第二份）没法靠人记住。
#[cfg(test)]
mod flavor_guard;
mod update_origin;
mod update_verify;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use steadcopy_core::config::{self, model::DestinationConfig, model::Project, Config};
use steadcopy_core::device::{
    can_eject, check_safety, confirmation_phrase, decide_auto_format, device_watcher, ejector,
    enumerate_volumes, formatter, label_matches, validate_countdown, AutoFormatDecision,
    BackupEvidence, DeviceEvent, DeviceKind, SafetyReport, Volume,
};
use steadcopy_core::engine::CancelToken;
use steadcopy_core::i18n::Locale;
use steadcopy_core::ledger::{
    record_run, FileRecord, HistoryQuery, Ledger, TaskRecord, TaskStatus,
};
use steadcopy_core::ledger::{render_report, ReportInput};
use steadcopy_core::manifest::{load_manifests, read_manifest, Manifest};
use steadcopy_core::organize::{scan_source, PathTemplate, RenderContext, ScanOptions};
use steadcopy_core::platform::{volume_io, Clock, SystemClock};
use steadcopy_core::preset::{
    derive_preset, needs_kind, on_arrival, select_preset, should_suggest, ArrivalOutcome, NextStep,
    Preset, SinkScope, SinkSuggestion,
};
use steadcopy_core::task::{
    adhoc_defaults, build_adhoc_spec, plan_task, run_task, AdhocRequest, ProjectChoice, StageEvent,
    TaskPlan, TaskSpec,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::UpdaterExt;

// ---------------------------------------------------------------- 状态

#[derive(Default)]
struct AppState {
    /// 每个在跑的任务一个控制令牌，键 = 设备身份。
    /// **不能只存一个**：两张卡先后到达时，后一个会把前一个的令牌顶掉，
    /// 「取消」就只作用在最后那个任务上，前一个变成停不下来的任务
    cancel: Mutex<HashMap<String, CancelToken>>,
    /// 正在跑任务的设备身份
    running: Mutex<Vec<String>>,
    /// 任务串行闸。界面只呈现一个任务，那就一次只跑一个——
    /// 并发跑两张卡而界面只显示一条进度，是在骗人
    run_lock: Mutex<()>,
    /// 已规划好、等用户确认的到达（key = 设备身份）
    pending: Mutex<HashMap<String, PendingArrival>>,
    /// 最近一次跑完的任务上下文，供「记住这个做法」用
    last_run: Mutex<Option<SinkContext>>,
    watching: Mutex<bool>,
}

struct PendingArrival {
    spec: TaskSpec,
    plan: TaskPlan,
    /// 本次匹配到的预设。临时拷贝时为 None——沉淀判定要用它
    matched: Option<Preset>,
    /// 本次实际用的项目 id
    project_id: Option<String>,
    /// 待建的项目（临时拷贝且用户选了「现建一个」时非空）。
    /// **只在用户按下开始时才落盘**——规划期零副作用对临时路径同样成立
    pending_project: Option<Project>,
}

/// 刚跑完的那次任务的上下文，供沉淀用。
///
/// 沉淀提示在任务结束后仍要能点（有人拷完就去拔卡了），所以上下文必须留着，
/// 不能随 pending 一起被消费掉。
#[derive(Clone)]
struct SinkContext {
    spec: TaskSpec,
    device_id: String,
    project_id: Option<String>,
}

fn load_cfg() -> Result<Config, String> {
    config::load().map_err(|e| e.to_string())
}

fn save_cfg(c: &Config) -> Result<(), String> {
    config::save(c).map_err(|e| e.to_string())
}

/// 本次请求用的语言。配置是唯一来源——界面与 core 用同一份设置。
fn lang_of(cfg: &Config) -> Locale {
    Locale::resolve(&cfg.settings.locale)
}

/// 读不到配置时的语言。跟系统，判不出来落中文。
fn lang() -> Locale {
    load_cfg()
        .map(|c| lang_of(&c))
        .unwrap_or_else(|_| Locale::resolve(steadcopy_core::i18n::LOCALE_AUTO))
}

fn ledger() -> Result<Ledger, String> {
    Ledger::open_default().map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- 视图模型

#[derive(Serialize)]
struct DeviceView {
    id: String,
    name: String,
    root: String,
    file_system: String,
    bus: String,
    total_bytes: u64,
    free_bytes: u64,
    is_system: bool,
    can_be_source: bool,
    fingerprints: Vec<String>,
    /// 记忆库里的类型（未记忆时为 null）
    kind: Option<String>,
    kind_label: Option<String>,
}

fn kind_str(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Unclassified => "unclassified",
        DeviceKind::Camera => "camera",
        DeviceKind::Recorder => "recorder",
        DeviceKind::Storage => "storage",
        DeviceKind::Ignored => "ignored",
    }
}

fn parse_kind(s: &str) -> DeviceKind {
    match s {
        "camera" => DeviceKind::Camera,
        "recorder" => DeviceKind::Recorder,
        "storage" => DeviceKind::Storage,
        "ignored" => DeviceKind::Ignored,
        _ => DeviceKind::Unclassified,
    }
}

fn device_view(v: &Volume, cfg: &Config) -> DeviceView {
    let lang = lang_of(cfg);
    let id = v.composite_id();
    let rec = cfg.device(&id);
    DeviceView {
        name: rec.map(|r| r.display_name()).unwrap_or_else(|| v.display_name()),
        id,
        root: v.root_path().display().to_string(),
        file_system: v.file_system.clone(),
        bus: v.bus_type.label(lang).to_string(),
        total_bytes: v.total_bytes,
        free_bytes: v.free_bytes,
        is_system: v.is_system,
        can_be_source: v.can_be_source(&[]),
        fingerprints: v.fingerprints.clone(),
        kind: rec.map(|r| kind_str(r.kind).to_string()),
        kind_label: rec.map(|r| r.kind.label(lang).to_string()),
    }
}

#[derive(Serialize, Clone)]
struct ArrivalView {
    device_id: String,
    device_name: String,
    outcome: String,
    summary: String,
    needs_attention: bool,
    /// 仅 Planned 时有值
    preset_name: Option<String>,
    requires_confirmation: bool,
    to_copy: usize,
    to_copy_bytes: u64,
    skipped: usize,
    destinations: Vec<PlanDestView>,
    categories: Vec<(String, usize, u64)>,
    /// 这个结论给用户留的出路。**每个「不能做」都要带一个「那就这样做」**
    next_step: String,
    next_step_label: String,
}

#[derive(Serialize, Clone)]
struct PlanDestView {
    landing_dir: String,
    required_bytes: u64,
    available_bytes: Option<u64>,
    sufficient: Option<bool>,
}

fn plan_dests(plan: &TaskPlan) -> Vec<PlanDestView> {
    plan.destinations
        .iter()
        .map(|d| PlanDestView {
            landing_dir: d.landing_dir.display().to_string(),
            required_bytes: d.required_bytes,
            available_bytes: d.available_bytes,
            sufficient: d.sufficient(),
        })
        .collect()
}

#[derive(Serialize, Clone)]
struct ProgressPayload {
    /// 稳定的机读代码。**界面用它做判定**——拿本地化文案比对，换语言就静默失效
    stage_code: String,
    /// 给人看的名字，只用于呈现
    stage: String,
    percent: f64,
    current: Option<String>,
    /// 瞬时速度（字节/秒）。算不出来时是 null，**不填 0 冒充**
    bytes_per_sec: Option<u64>,
    /// 预计剩余秒数。同上，算不出来就是 null
    eta_secs: Option<u64>,
}

#[derive(Serialize, Clone)]
struct FailurePayload {
    path: String,
    reason: String,
}

#[derive(Serialize)]
struct RunView {
    task_id: String,
    copied: usize,
    skipped: usize,
    failed: usize,
    bytes_copied: u64,
    cancelled: bool,
    all_succeeded: bool,
    manifests: Vec<String>,
    notices: Vec<String>,
    failures: Vec<FailurePayload>,
}

// ---------------------------------------------------------------- 配置类命令

#[tauri::command]
fn get_config() -> Result<Config, String> {
    load_cfg()
}

#[tauri::command]
fn save_settings(mut settings: steadcopy_core::config::Settings) -> Result<(), String> {
    // 倒计时下限在这里**硬拒**。界面上的 input 拦不住任何人，后端拦得住。
    // 语言取**将要保存的那份**设置，而不是当前生效的——用户可能正在同一次保存里改语言
    let lang = Locale::resolve(&settings.locale);
    settings.countdown_secs = validate_countdown(settings.countdown_secs, lang)?;
    let mut c = load_cfg()?;
    c.settings = settings;
    save_cfg(&c)
}

#[derive(Deserialize)]
struct ProjectInput {
    id: Option<String>,
    name: String,
    destinations: Vec<DestinationInput>,
}

#[derive(Deserialize)]
struct DestinationInput {
    id: Option<String>,
    root: String,
    template: String,
    enabled: bool,
}

#[tauri::command]
fn upsert_project(input: ProjectInput) -> Result<Config, String> {
    let mut c = load_cfg()?;
    let dests: Vec<DestinationConfig> = input
        .destinations
        .iter()
        .map(|d| DestinationConfig {
            id: d.id.clone().unwrap_or_else(|| config::new_id("dst")),
            root: PathBuf::from(&d.root),
            template: d.template.clone(),
            enabled: d.enabled,
        })
        .collect();

    match input.id.and_then(|id| {
        c.projects
            .iter()
            .position(|p| p.id == id)
            .map(|i| (i, id.clone()))
    }) {
        Some((i, _)) => {
            c.projects[i].name = input.name;
            c.projects[i].destinations = dests;
        }
        None => {
            let mut p = Project::new(input.name, SystemClock.now());
            p.destinations = dests;
            if c.current_project.is_none() {
                c.current_project = Some(p.id.clone());
            }
            c.projects.push(p);
        }
    }
    save_cfg(&c)?;
    Ok(c)
}

#[tauri::command]
fn delete_project(id: String) -> Result<Config, String> {
    let mut c = load_cfg()?;
    c.projects.retain(|p| p.id != id);
    c.presets.retain(|p| p.project_id.as_deref() != Some(id.as_str()));
    if c.current_project.as_deref() == Some(id.as_str()) {
        c.current_project = c.projects.first().map(|p| p.id.clone());
    }
    save_cfg(&c)?;
    Ok(c)
}

#[tauri::command]
fn set_current_project(id: String) -> Result<Config, String> {
    let mut c = load_cfg()?;
    c.current_project = Some(id);
    save_cfg(&c)?;
    Ok(c)
}

#[tauri::command]
fn upsert_preset(preset: Preset) -> Result<Config, String> {
    let mut c = load_cfg()?;
    match c.presets.iter().position(|p| p.id == preset.id) {
        Some(i) => c.presets[i] = preset,
        None => c.presets.push(preset),
    }
    save_cfg(&c)?;
    Ok(c)
}

#[tauri::command]
fn delete_preset(id: String) -> Result<Config, String> {
    let mut c = load_cfg()?;
    c.presets.retain(|p| p.id != id);
    save_cfg(&c)?;
    Ok(c)
}

#[tauri::command]
fn move_preset(id: String, up: bool) -> Result<Config, String> {
    let mut c = load_cfg()?;
    if let Some(i) = c.presets.iter().position(|p| p.id == id) {
        let j = if up { i.checked_sub(1) } else { Some(i + 1) };
        if let Some(j) = j.filter(|j| *j < c.presets.len()) {
            c.presets.swap(i, j);
        }
    }
    save_cfg(&c)?;
    Ok(c)
}

#[tauri::command]
fn set_device_kind(id: String, kind: String) -> Result<Config, String> {
    let mut c = load_cfg()?;
    let d = c
        .device_mut(&id)
        .ok_or_else(|| format!("记忆库里没有这个设备：{id}"))?;
    d.kind = parse_kind(&kind);
    save_cfg(&c)?;
    Ok(c)
}

#[tauri::command]
fn rename_device(id: String, name: String) -> Result<Config, String> {
    let mut c = load_cfg()?;
    let d = c
        .device_mut(&id)
        .ok_or_else(|| format!("记忆库里没有这个设备：{id}"))?;
    d.custom_name = name;
    save_cfg(&c)?;
    Ok(c)
}

#[tauri::command]
fn forget_device(id: String) -> Result<Config, String> {
    let mut c = load_cfg()?;
    c.devices.retain(|d| d.id != id);
    save_cfg(&c)?;
    Ok(c)
}

/// 路径模板预览。**前端 MUST 调它**，不许自己实现一份渲染。
#[tauri::command]
fn preview_path(root: String, template: String, project: String, device: String) -> Result<String, String> {
    let t = PathTemplate::parse(&template).map_err(|e| e.to_string())?;
    let ctx = RenderContext {
        project,
        device: device.clone(),
        card: device,
        at: SystemClock.now(),
    };
    let mut p = PathBuf::from(root);
    for seg in t.render_segments(&ctx) {
        p.push(seg);
    }
    Ok(p.display().to_string())
}

#[tauri::command]
fn config_path() -> String {
    config::config_path().display().to_string()
}

// ---------------------------------------------------------------- 设备与到达

#[tauri::command]
fn list_devices() -> Result<Vec<DeviceView>, String> {
    let cfg = load_cfg()?;
    let vols = enumerate_volumes().map_err(|e| e.to_string())?;
    Ok(vols.iter().map(|v| device_view(v, &cfg)).collect())
}

fn outcome_view(o: &ArrivalOutcome, plan_categories: Vec<(String, usize, u64)>) -> ArrivalView {
    let (name, kind) = match o {
        ArrivalOutcome::NeedsClassification {
            device_id,
            suggested_name,
        } => (suggested_name.clone(), ("needs_classification", device_id.clone())),
        ArrivalOutcome::Ignored { device_id } => ("".into(), ("ignored", device_id.clone())),
        ArrivalOutcome::AlreadyRunning { device_id } => {
            ("".into(), ("already_running", device_id.clone()))
        }
        ArrivalOutcome::NoPreset {
            device_id,
            device_name,
        } => (device_name.clone(), ("no_preset", device_id.clone())),
        ArrivalOutcome::NoProject { .. } => ("".into(), ("no_project", String::new())),
        ArrivalOutcome::NoSource { device_name } => {
            (device_name.clone(), ("no_source", String::new()))
        }
        ArrivalOutcome::NoNewSource { device_name } => {
            (device_name.clone(), ("no_new_source", String::new()))
        }
        ArrivalOutcome::InsufficientSpace { device_name, .. } => {
            (device_name.clone(), ("insufficient_space", String::new()))
        }
        // 身份必须是 spec 里的那个——pending 就是用它做键，
        // 前端拿着别的 id 回来确认会直接落空
        ArrivalOutcome::Planned {
            device_name, spec, ..
        } => (device_name.clone(), ("planned", spec.source.id.clone())),
    };

    let (preset_name, requires_confirmation, to_copy, to_copy_bytes, skipped, destinations) =
        match o {
            ArrivalOutcome::Planned {
                preset_name,
                plan,
                requires_confirmation,
                ..
            } => (
                Some(preset_name.clone()),
                *requires_confirmation,
                plan.files.len(),
                plan.total_bytes(),
                plan.skipped.len(),
                plan_dests(plan),
            ),
            _ => (None, false, 0, 0, 0, Vec::new()),
        };

    ArrivalView {
        device_id: kind.1,
        device_name: name,
        outcome: kind.0.to_string(),
        summary: o.summary(lang()),
        needs_attention: o.needs_attention(),
        preset_name,
        requires_confirmation,
        to_copy,
        to_copy_bytes,
        skipped,
        destinations,
        categories: plan_categories,
        next_step: next_step_str(o.next_step()).to_string(),
        next_step_label: o.next_step().label(lang()).to_string(),
    }
}

const fn next_step_str(s: NextStep) -> &'static str {
    match s {
        NextStep::ClassifyOrCopyOnce => "classify_or_copy_once",
        NextStep::CopyOnce => "copy_once",
        NextStep::ChooseAnotherDestination => "choose_another_destination",
        NextStep::ViewLastReport => "view_last_report",
        NextStep::ConfirmAndRun => "confirm_and_run",
        NextStep::Nothing => "nothing",
    }
}

/// 取设备记忆。记忆库里没有就造一个临时的未分类记录——
/// 只用于匹配判定，**不落盘**。
fn device_of(cfg: &Config, id: &str) -> steadcopy_core::device::DeviceRecord {
    cfg.device(id).cloned().unwrap_or_else(|| {
        steadcopy_core::device::DeviceRecord::new(id, "", 0, SystemClock.now())
    })
}

/// 处理一次设备到达，必要时把规划结果暂存起来等用户确认。
fn handle_arrival(app: &AppHandle, state: &AppState, vol: &Volume) -> Result<ArrivalView, String> {
    let mut cfg = load_cfg()?;
    let io = volume_io();
    // 锁中毒不能退化成「没有任务在跑」：那会给正在拷的设备再建一个任务
    let running = state
        .running
        .lock()
        .map(|r| r.clone())
        .map_err(|_| "任务状态锁异常，为安全起见本次不处理这次插卡")?;
    let outcome = on_arrival(&mut cfg, vol, &running, io.as_ref(), SystemClock.now());
    // 首次见到的设备已被登记，落盘，否则下次又是新设备
    save_cfg(&cfg)?;

    let categories = match &outcome {
        ArrivalOutcome::Planned { plan, .. } => plan
            .scan
            .by_category(&steadcopy_core::organize::FilterConfig::default())
            .into_iter()
            .map(|(k, (n, b))| (k.to_string(), n, b))
            .collect(),
        _ => Vec::new(),
    };
    let mut view = outcome_view(&outcome, categories);
    // 几种结论里 core 没带设备身份（它不需要），界面要用来做键，这里补上
    if view.device_id.is_empty() {
        view.device_id = vol.composite_id();
    }

    if let ArrivalOutcome::Planned { spec, plan, .. } = outcome {
        let id = spec.source.id.clone();
        // 匹配到的预设要留着——拷完判断「这次和记住的一样吗」要用
        let matched = select_preset(&cfg.presets, &device_of(&cfg, &id)).cloned();
        let project_id = cfg
            .projects
            .iter()
            .find(|p| p.name == spec.project)
            .map(|p| p.id.clone());
        if let Ok(mut p) = state.pending.lock() {
            p.insert(
                id,
                PendingArrival {
                    spec: *spec,
                    plan: *plan,
                    matched,
                    project_id,
                    pending_project: None,
                },
            );
        }
    }

    if view.needs_attention {
        let _ = app.emit("device-arrived", view.clone());
    }
    Ok(view)
}

/// 启动设备监听。幂等——重复调用不会起第二个监听。
#[tauri::command]
fn start_watching(app: AppHandle, state: State<'_, AppState>) -> Result<bool, String> {
    {
        let mut w = state.watching.lock().map_err(|_| "状态锁异常")?;
        if *w {
            return Ok(false);
        }
        *w = true;
    }

    let handle = app.clone();
    std::thread::spawn(move || {
        let mut watcher = device_watcher();
        let Ok(rx) = watcher.subscribe() else {
            let _ = handle.emit("watch-error", "设备监听启动失败".to_string());
            return;
        };
        // 启动时先把已经插着的设备过一遍——用户可能先插卡后开程序。
        // **这一遍只呈现、绝不自动开跑**：开程序不是插卡，
        // 无人值守档也不该在窗口刚亮起来的瞬间自己动手。
        if let Some(state) = handle.try_state::<AppState>() {
            let initial = match enumerate_volumes() {
                Ok(v) => v,
                Err(e) => {
                    // 静默当成「没有设备」会让监听器哑掉而没人知道
                    let _ = handle.emit("watch-error", format!("启动时枚举卷失败：{e}"));
                    Vec::new()
                }
            };
            for v in initial {
                if v.can_be_source(&[]) {
                    let _ = handle_arrival(&handle, &state, &v);
                }
            }
        }

        for event in rx {
            let DeviceEvent::Arrived { drive_letter } = event else {
                let _ = handle.emit("device-removed", ());
                continue;
            };
            // 卷到达时盘符可能尚未分配，退避重试
            let mut found: Option<Volume> = None;
            for delay in [0u64, 150, 400, 900] {
                if delay > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                }
                let vols = match enumerate_volumes() {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = handle.emit("watch-error", format!("枚举卷失败：{e}"));
                        Vec::new()
                    }
                };
                found = match &drive_letter {
                    Some(l) => vols.into_iter().find(|v| v.drive_letter.as_deref() == Some(l)),
                    None => vols
                        .into_iter()
                        .find(|v| v.drive_letter.is_none() && v.can_be_source(&[])),
                };
                if found.is_some() {
                    break;
                }
            }
            let Some(vol) = found else { continue };
            if !vol.can_be_source(&[]) {
                continue;
            }
            let Some(state) = handle.try_state::<AppState>() else {
                continue;
            };
            let Ok(view) = handle_arrival(&handle, &state, &vol) else {
                continue;
            };
            // 无人值守档：危险区里明确关掉了确认，才会走到这里。
            // requires_confirmation 由 core 判定，界面与此处都只是执行它的结论。
            if view.outcome == "planned" && !view.requires_confirmation {
                let id = view.device_id.clone();
                let h = handle.clone();
                std::thread::spawn(move || {
                    if let Err(e) = execute_run(&h, &id) {
                        let _ = h.emit("task-failed", e);
                    }
                });
            }
        }
    });
    Ok(true)
}

/// 手动对某个卷触发一次到达编排（「用它作为源」按钮）。
#[tauri::command]
fn arrive_now(app: AppHandle, state: State<'_, AppState>, device_root: String) -> Result<ArrivalView, String> {
    let vols = enumerate_volumes().map_err(|e| e.to_string())?;
    let vol = vols
        .into_iter()
        .find(|v| v.root_path().display().to_string() == device_root)
        .ok_or_else(|| format!("找不到这个卷：{device_root}"))?;
    handle_arrival(&app, &state, &vol)
}

// ---------------------------------------------------------------- 临时拷贝
//
// 规范：openspec/changes/add-steadcopy-copy-first-flow/specs/adhoc-copy/spec.md

#[derive(Serialize)]
struct AdhocDefaultsView {
    project_id: Option<String>,
    project_name: String,
    /// 项目字段旁要不要提示「会自动建这个项目」
    project_will_be_created: bool,
    destinations: Vec<String>,
    verify: bool,
    algorithm: String,
}

/// 临时拷贝面板的预填值。**每个字段都有能直接用的默认值**——
/// 「项目不强制」的意思是「有默认值可以一路回车过去」，不是「可以为空」。
#[tauri::command]
fn adhoc_prefill() -> Result<AdhocDefaultsView, String> {
    let cfg = load_cfg()?;
    let d = adhoc_defaults(&cfg);
    let (project_id, project_name) = match &d.project {
        ProjectChoice::Existing(id) => (
            Some(id.clone()),
            // 项目 id 来自 core 的预填，理论上必然存在；真取不到就退回默认名，
            // 而不是给个空串让界面显示一个没有名字的项目
            cfg.project(id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| steadcopy_core::task::DEFAULT_PROJECT_NAME.to_string()),
        ),
        ProjectChoice::Create { name, .. } => (None, name.clone()),
    };
    Ok(AdhocDefaultsView {
        project_id,
        project_name,
        project_will_be_created: d.project_will_be_created,
        destinations: d.destinations.iter().map(|p| p.display().to_string()).collect(),
        verify: d.verify,
        algorithm: match d.algorithm {
            steadcopy_core::engine::HashAlgorithm::Md5 => "md5".into(),
            _ => "xxh64".into(),
        },
    })
}

#[derive(Deserialize)]
struct AdhocInput {
    device_root: String,
    /// 非空表示沿用已有项目；空表示用 `project_name` 现建一个
    project_id: Option<String>,
    project_name: String,
    destinations: Vec<String>,
    verify: bool,
    algorithm: String,
    eject_after: bool,
}

/// 规划一次临时拷贝。**零副作用**：不建目录、不写文件、不落新项目。
#[tauri::command]
fn plan_adhoc(state: State<'_, AppState>, input: AdhocInput) -> Result<ArrivalView, String> {
    let cfg = load_cfg()?;
    let vols = enumerate_volumes().map_err(|e| e.to_string())?;
    let vol = vols
        .into_iter()
        .find(|v| v.root_path().display().to_string() == input.device_root)
        .ok_or_else(|| format!("找不到这个卷：{}", input.device_root))?;

    let device_id = vol.composite_id();
    let device_name = cfg
        .device(&device_id)
        .map(|d| d.display_name())
        .unwrap_or_else(|| vol.display_name());

    let project = match &input.project_id {
        Some(id) => ProjectChoice::Existing(id.clone()),
        None => ProjectChoice::Create {
            name: input.project_name.clone(),
            destinations: input.destinations.iter().map(PathBuf::from).collect(),
        },
    };

    let running = state
        .running
        .lock()
        .map(|r| r.clone())
        .map_err(|_| "任务状态锁异常")?;

    let req = AdhocRequest {
        source_root: vol.root_path(),
        device_id: device_id.clone(),
        device_name: device_name.clone(),
        project,
        destinations: input.destinations.iter().map(PathBuf::from).collect(),
        verify: Some(input.verify),
        algorithm: Some(match input.algorithm.as_str() {
            "md5" => steadcopy_core::engine::HashAlgorithm::Md5,
            _ => steadcopy_core::engine::HashAlgorithm::Xxh64,
        }),
        eject_after: input.eject_after,
    };

    let (spec, pending_project) =
        build_adhoc_spec(&cfg, &req, &running, SystemClock.now()).map_err(|e| e.to_string())?;

    let io = volume_io();
    let plan = plan_task(&spec, io.as_ref()).map_err(|e| e.to_string())?;

    // 复用到达编排的视图形状：界面只有一张确认卡片，两条路走同一个组件
    let categories: Vec<(String, usize, u64)> = plan
        .scan
        .by_category(&steadcopy_core::organize::FilterConfig::default())
        .into_iter()
        .map(|(k, (n, b))| (k.to_string(), n, b))
        .collect();

    let insufficient = plan.insufficient().next().is_some();
    let view = ArrivalView {
        device_id: device_id.clone(),
        device_name,
        outcome: if plan.is_no_source() {
            "no_source".into()
        } else if plan.is_no_new_source() {
            "no_new_source".into()
        } else if insufficient {
            "insufficient_space".into()
        } else {
            "planned".into()
        },
        summary: if plan.is_no_source() {
            "这张卡上没有可拷贝的素材".into()
        } else if plan.is_no_new_source() {
            "没有新素材，此前已拷并校验通过".into()
        } else if insufficient {
            "目的地空间不足".into()
        } else {
            format!("就拷这一次：{} 个文件", plan.files.len())
        },
        needs_attention: true,
        preset_name: None,
        // 临时拷贝**永远要点一次**——它整个就是「用户当场做决定」这件事，
        // 无人值守档在这里没有意义
        requires_confirmation: true,
        to_copy: plan.files.len(),
        to_copy_bytes: plan.total_bytes(),
        skipped: plan.skipped.len(),
        destinations: plan_dests(&plan),
        categories,
        next_step: if insufficient {
            "choose_another_destination".into()
        } else {
            "confirm_and_run".into()
        },
        next_step_label: if insufficient { "换个目的地" } else { "开始拷贝" }.to_string(),
    };

    let project_id = input.project_id.clone();
    if let Ok(mut p) = state.pending.lock() {
        p.insert(
            device_id,
            PendingArrival {
                spec,
                plan,
                matched: None,
                project_id,
                pending_project,
            },
        );
    }
    Ok(view)
}

// ---------------------------------------------------------------- 预设沉淀
//
// 规范：openspec/changes/add-steadcopy-copy-first-flow/specs/preset-sinking/spec.md

#[derive(Serialize, Clone)]
struct SinkView {
    kind: String,
    device_id: String,
    device_name: String,
    project_name: String,
    /// 与原预设的差异（`diverged` 时非空）
    changed: Vec<String>,
    preset_name: Option<String>,
    /// 该设备还没指认类型 —— 提示里要一并收，否则这条预设下次根本轮不到生效
    needs_kind: bool,
    /// 默认范围的人话描述
    default_scope_label: String,
}

fn sink_view(s: &SinkSuggestion, spec: &TaskSpec, device_id: &str) -> Result<SinkView, String> {
    let cfg = load_cfg()?;
    let device = device_of(&cfg, device_id);
    let name = if device.custom_name.is_empty() {
        spec.source.display_name.clone()
    } else {
        device.display_name()
    };
    let (kind, changed, preset_name) = match s {
        SinkSuggestion::NoPreset => ("no_preset", Vec::new(), None),
        SinkSuggestion::Diverged {
            preset_name,
            changed,
            ..
        } => (
            "diverged",
            changed.iter().map(|c| (*c).to_string()).collect(),
            Some(preset_name.clone()),
        ),
        SinkSuggestion::None => ("none", Vec::new(), None),
    };
    Ok(SinkView {
        kind: kind.to_string(),
        device_id: device_id.to_string(),
        default_scope_label: SinkScope::default().describe(lang(), &name),
        device_name: name,
        project_name: spec.project.clone(),
        changed,
        preset_name,
        needs_kind: needs_kind(&device),
    })
}

/// 把刚跑完那次的做法记成预设。
///
/// **一次点击完成**，不跳编辑器——用户刚刚才把参数说清楚，再让他填一遍
/// 等于把沉淀变成第二次配置。
#[tauri::command]
fn sink_preset(
    state: State<'_, AppState>,
    scope: String,
    kind: Option<String>,
    name: Option<String>,
) -> Result<Config, String> {
    let ctx = state
        .last_run
        .lock()
        .map_err(|_| "状态锁异常")?
        .clone()
        .ok_or("还没有可以记住的任务")?;

    let mut cfg = load_cfg()?;

    // 未指认的设备要顺带把类型收了。不收的话这条预设下次插卡时
    // 会卡在「未分类设备停在指认」那一步，等于白配
    if let Some(k) = kind.as_deref() {
        if let Some(d) = cfg.device_mut(&ctx.device_id) {
            d.kind = parse_kind(k);
        }
    }

    let device = device_of(&cfg, &ctx.device_id);
    let scope = match scope.as_str() {
        "kind" => SinkScope::ThisKind(device.kind),
        "any" => SinkScope::AnyClassified,
        // 认不出来一律退回最窄的。放宽必须是显式动作
        _ => SinkScope::ThisDevice,
    };

    let preset = derive_preset(&ctx.spec, &device, scope, ctx.project_id.as_deref(), name, lang_of(&cfg));
    // 同一台设备重复沉淀就更新那一条，不堆一串同名预设
    match cfg.presets.iter().position(|p| p.matcher == preset.matcher) {
        Some(i) => {
            let id = cfg.presets[i].id.clone();
            cfg.presets[i] = Preset { id, ..preset };
        }
        None => cfg.presets.insert(0, preset),
    }
    save_cfg(&cfg)?;
    Ok(cfg)
}

// ---------------------------------------------------------------- 执行

/// 真正跑一次任务。**阻塞**，可以从任何线程调用。
///
/// 确认路径（用户点了「开始拷贝」）与无人值守路径（危险区里关了确认）共用它，
/// 两条路径的行为因此不可能漂移。
fn execute_run(app: &AppHandle, device_id: &str) -> Result<RunView, String> {
    let state = app.state::<AppState>();
    let pending = {
        let mut p = state.pending.lock().map_err(|_| "状态锁异常")?;
        p.remove(device_id)
            .ok_or("这次到达的计划已失效，请重新插卡或手动发起")?
    };

    // 串行闸：前一个任务没跑完就在这里排队
    let queued = state.run_lock.try_lock().is_err();
    if queued {
        let _ = app.emit("task-notice", "已有任务在跑，这张卡排在后面".to_string());
    }
    let _guard = state.run_lock.lock().map_err(|_| "任务闸异常")?;

    let cancel = CancelToken::new();
    if let Ok(mut slot) = state.cancel.lock() {
        slot.insert(device_id.to_string(), cancel.clone());
    }
    if let Ok(mut r) = state.running.lock() {
        r.push(device_id.to_string());
    }
    let _ = app.emit("task-started", device_id.to_string());


    // 待建项目在**用户按下开始的这一刻**才落盘。规划期零副作用是刻意的：
    // 预演一下就在配置里多出个项目，是很难解释的副作用
    let mut project_id = pending.project_id.clone();
    if let Some(p) = pending.pending_project.clone() {
        if let Ok(mut cfg) = load_cfg() {
            project_id = Some(p.id.clone());
            if cfg.current_project.is_none() {
                cfg.current_project = Some(p.id.clone());
            }
            cfg.projects.push(p);
            if let Err(e) = save_cfg(&cfg) {
                let _ = app.emit("task-notice", format!("新建项目没能保存：{e}"));
            }
        }
    }

    let spec = pending.spec;
    let plan = pending.plan;

    // 沉淀提示：任务一开跑就挂上，**不是弹窗**。
    // 判定在 core——「这次的做法和已记住的不一样」才提示，一致就闭嘴。
    // 上下文单独留一份：任务结束后提示仍要能点（有人拷完就去拔卡了）
    let suggestion = should_suggest(&spec, pending.matched.as_ref(), project_id.as_deref());
    if let Ok(mut slot) = state.last_run.lock() {
        *slot = Some(SinkContext {
            spec: spec.clone(),
            device_id: device_id.to_string(),
            project_id: project_id.clone(),
        });
    }
    if suggestion.should_show() {
        let _ = app.emit("sink-suggested", sink_view(&suggestion, &spec, device_id)?);
    }
    // 拷完判定「能不能提议格式化」要用到的事实，在 spec/plan 被搬进线程之前取出来
    let verify = spec.verify;
    let dest_count = plan.destinations.len();
    let source_root = spec.source_root.clone();
    let eject_after = spec.eject_after;
    let device_id_owned = device_id.to_string();
    let handle = app.clone();

    // 阻塞式 IO 放独立线程，不占 Tauri 的异步运行时
    let result = std::thread::spawn(move || -> Result<RunView, String> {
        let io = volume_io();
        let clock = SystemClock;
        let started = clock.now();
        let t0 = std::time::Instant::now();
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let mut last_done: u64 = 0;
        let lang = lang();

        let report = run_task(&spec, &plan, io.as_ref(), &clock, &cancel, &mut |e| match e {
            StageEvent::Stage(s) => {
                let _ = handle.emit(
                    "task-stage",
                    ProgressPayload {
                        stage_code: s.code().to_string(),
                        stage: s.label(lang).to_string(),
                        percent: 0.0,
                        current: None,
                        bytes_per_sec: None,
                        eta_secs: None,
                    },
                );
            }
            StageEvent::Progress {
                stage,
                done,
                total,
                current,
            } => {
                // 事件限流在消费方：引擎发全量，界面按 100ms 收
                if last.elapsed() < std::time::Duration::from_millis(100) {
                    return;
                }
                let dt = last.elapsed().as_secs_f64();
                last = std::time::Instant::now();
                // 速度取「这一小段时间里推进了多少」，不是全程平均——
                // 全程平均在换文件、遇到慢盘时反应太迟钝，看着像卡住了
                let (bps, eta) = if dt > 0.0 && done >= last_done {
                    let bps = ((done - last_done) as f64 / dt) as u64;
                    let eta = (bps > 0).then(|| total.saturating_sub(done) / bps);
                    (Some(bps), eta)
                } else {
                    (None, None)
                };
                last_done = done;
                let _ = handle.emit(
                    "task-progress",
                    ProgressPayload {
                        stage_code: stage.code().to_string(),
                        stage: stage.label(lang).to_string(),
                        percent: steadcopy_core::task::stage::percent(done, total),
                        current,
                        bytes_per_sec: bps,
                        eta_secs: eta,
                    },
                );
            }
            StageEvent::FileFailed {
                relative_path,
                reason,
            } => {
                let _ = handle.emit(
                    "task-file-failed",
                    FailurePayload {
                        path: relative_path,
                        reason,
                    },
                );
            }
            StageEvent::Notice(msg) => {
                let _ = handle.emit("task-notice", msg);
            }
        })
        .map_err(|e| e.to_string())?;

        let finished = clock.now();
        let failures: Vec<(String, String, u32)> = report
            .failed_files()
            .map(|f| {
                let reason = match &f.status {
                    steadcopy_core::task::FileStatus::Failed(m) => m.clone(),
                    _ => String::new(),
                };
                (f.relative_path.clone(), reason, f.retries)
            })
            .collect();

        // 拷完自动出报告
        for mp in &report.manifests {
            if let Ok(m) = read_manifest(mp) {
                let input = ReportInput {
                    manifest: &m,
                    failures: &failures,
                    skipped: report.skipped_count(),
                    notices: &report.notices,
                    elapsed_secs: Some(t0.elapsed().as_secs()),
                    generated_at: clock.now(),
                    lang,
                    audit: None,
                };
                let _ = std::fs::write(mp.with_extension("html"), render_report(&input));
            }
        }

        // 记台账
        let task_id = match ledger()
            .and_then(|l| record_run(&l, &spec, &report, started, finished).map_err(|e| e.to_string()))
        {
            Ok(id) => id,
            Err(e) => {
                // 数据已经拷好了，台账没记上是另一件事——两件都要如实说
                let _ = handle.emit("task-notice", format!("这次任务没能写进台账：{e}"));
                String::new()
            }
        };

        Ok(RunView {
            task_id,
            copied: report.copied_count(),
            skipped: report.skipped_count(),
            failed: failures.len(),
            bytes_copied: report.bytes_copied,
            cancelled: report.cancelled,
            all_succeeded: report.all_succeeded(),
            manifests: report.manifests.iter().map(|p| p.display().to_string()).collect(),
            notices: report.notices.clone(),
            failures: failures
                .into_iter()
                .map(|(path, reason, _)| FailurePayload { path, reason })
                .collect(),
        })
    })
    .join()
    .map_err(|_| "拷贝线程异常终止".to_string())?;

    if let Ok(mut r) = state.running.lock() {
        r.retain(|d| d != device_id);
    }
    if let Ok(mut slot) = state.cancel.lock() {
        slot.remove(device_id);
    }
    match &result {
        Ok(v) => {
            let _ = app.emit("task-finished", v);
            notify_finished(app, v);
            let proposing = propose_format_if_green(app, v, verify, dest_count, &source_root);
            // 提议格式化时不自动弹卡——卡弹了就没法格了。
            // 两个开关都开着时，先格式化，弹卡交给用户
            if !proposing {
                auto_eject_if_green(app, v, eject_after, &device_id_owned, &source_root);
            }
        }
        Err(e) => {
            let _ = app.emit("task-failed", e.clone());
        }
    }
    result
}

/// 任务结束的系统通知。
///
/// **失败绝不以「完成」为主表述**——通知往往是无人值守时用户唯一会看到的东西，
/// 在这里把「部分失败」说成「完成」，等于让人以为拷好了。
fn notify_finished(app: &AppHandle, v: &RunView) {
    let (title, body) = if v.cancelled {
        ("拷贝已取消", format!("已完成 {} 个文件，可续传", v.copied))
    } else if v.failed > 0 {
        (
            "拷贝部分失败",
            format!("成功 {} 个，失败 {} 个——去台账看失败清单", v.copied, v.failed),
        )
    } else {
        (
            "拷贝完成",
            format!("{} 个文件全部校验通过，可以拔卡了", v.copied),
        )
    };
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}

/// 拷完之后，看要不要提议格式化源卡。
///
/// 判定在 core（`decide_auto_format`），这里只负责把结论变成一次事件。
/// **提议不是执行**：前端收到后仍要走完整的安全链 + 倒计时 + 手输卷标，
/// 且 `do_format` 会把安全链再跑一遍。
fn propose_format_if_green(
    app: &AppHandle,
    v: &RunView,
    verified: bool,
    dest_count: usize,
    source_root: &std::path::Path,
) -> bool {
    let Ok(cfg) = load_cfg() else { return false };
    let decision = decide_auto_format(
        cfg.settings.format_after_copy,
        verified,
        v.cancelled,
        v.failed,
        v.manifests.len(),
        dest_count,
    );

    if decision != AutoFormatDecision::Propose {
        // 开着开关却没提议时，把原因说出来——「为什么没弹」比「弹不弹」更需要答案
        if cfg.settings.format_after_copy {
            let _ = app.emit("task-notice", decision.reason(lang()).to_string());
        }
        return false;
    }

    match check_format(source_root.display().to_string()) {
        Ok(safety) => {
            let _ = app.emit("format-proposed", safety);
            true
        }
        Err(e) => {
            let _ = app.emit("task-notice", format!("拷完想提议格式化，但前置检查没跑通：{e}"));
            false
        }
    }
}

/// 全绿之后自动安全弹出源卡。
///
/// 只在**这次任务确实全部成功**时弹。弹不掉是常事（剪辑软件还开着素材），
/// 如实提示，不当成任务失败——数据已经拷好了，卡弹不掉不改变这个事实。
fn auto_eject_if_green(
    app: &AppHandle,
    v: &RunView,
    eject_after: bool,
    device_id: &str,
    source_root: &std::path::Path,
) {
    if !eject_after || !v.all_succeeded {
        return;
    }
    match eject_volume(app, device_id, source_root) {
        Ok(()) => {
            let _ = app.emit("task-notice", "源卡已安全弹出，可以拔了".to_string());
            let _ = app.emit("device-removed", ());
        }
        Err(e) => {
            let _ = app.emit("task-notice", format!("自动弹出没成功：{e}"));
        }
    }
}

/// 弹出一个卷。**任务进行中一律拒绝**，判定在 core。
fn eject_volume(app: &AppHandle, device_id: &str, root: &std::path::Path) -> Result<(), String> {
    // 锁中毒退化成空列表会直接允许「拷贝中弹卡」，这里必须硬失败
    let running = app
        .state::<AppState>()
        .running
        .lock()
        .map(|r| r.clone())
        .map_err(|_| "任务状态锁异常，为安全起见拒绝弹出")?;
    can_eject(device_id, &running).map_err(|e| e.to_string())?;
    ejector().eject(root).map_err(|e| e.to_string())
}

#[tauri::command]
fn eject_device(app: AppHandle, device_root: String) -> Result<(), String> {
    let vols = enumerate_volumes().map_err(|e| e.to_string())?;
    let vol = vols
        .into_iter()
        .find(|v| v.root_path().display().to_string() == device_root)
        .ok_or_else(|| format!("找不到这个卷：{device_root}"))?;
    eject_volume(&app, &vol.composite_id(), &vol.root_path())
}

/// 暂停 / 继续当前任务。
#[tauri::command]
fn set_paused(state: State<'_, AppState>, paused: bool) -> Result<(), String> {
    if let Ok(slot) = state.cancel.lock() {
        for c in slot.values() {
            if paused {
                c.pause();
            } else {
                c.resume();
            }
        }
    }
    Ok(())
}

#[tauri::command]
async fn confirm_and_run(app: AppHandle, device_id: String) -> Result<RunView, String> {
    let handle = app.clone();
    std::thread::spawn(move || execute_run(&handle, &device_id))
        .join()
        .map_err(|_| "拷贝线程异常终止".to_string())?
}

#[tauri::command]
fn dismiss_arrival(state: State<'_, AppState>, device_id: String) -> Result<(), String> {
    if let Ok(mut p) = state.pending.lock() {
        p.remove(&device_id);
    }
    Ok(())
}

#[tauri::command]
fn cancel_copy(state: State<'_, AppState>) -> Result<(), String> {
    // 界面只有一个取消按钮，对应「把在跑的都停掉」。
    // 排队中的任务同样要能停——它们的令牌在开跑那一刻才进表，
    // 所以真正的排队取消由串行闸之后的 is_cancelled 检查兜住
    if let Ok(slot) = state.cancel.lock() {
        for c in slot.values() {
            c.cancel();
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- 台账

#[tauri::command]
fn list_history(only_failed: bool, limit: Option<u32>) -> Result<Vec<TaskRecord>, String> {
    ledger()?
        .history(&HistoryQuery {
            only_failed,
            limit: limit.or(Some(300)),
            ..Default::default()
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn task_files(task_id: String, status: Option<String>) -> Result<Vec<FileRecord>, String> {
    ledger()?
        .task_files(&task_id, status.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_history() -> Result<(), String> {
    ledger()?.clear().map_err(|e| e.to_string())
}

#[tauri::command]
fn format_attempts() -> Result<Vec<steadcopy_core::ledger::FormatAttempt>, String> {
    ledger()?.format_attempts().map_err(|e| e.to_string())
}

/// **应用内直接展示报告**：返回报告 HTML 全文，前端塞进沙箱 iframe 渲染。
#[tauri::command]
fn report_html(manifest_path: String) -> Result<String, String> {
    let p = PathBuf::from(&manifest_path);
    if let Ok(s) = std::fs::read_to_string(p.with_extension("html")) {
        return Ok(s);
    }
    let m = read_manifest(&p).map_err(|e| e.to_string())?;
    Ok(render_report(&ReportInput {
        manifest: &m,
        failures: &[],
        skipped: 0,
        notices: &[],
        elapsed_secs: None,
        generated_at: SystemClock.now(),
        lang: lang(),
        audit: None,
    }))
}

#[tauri::command]
fn run_audit(manifest_path: String) -> Result<serde_json::Value, String> {
    let p = PathBuf::from(&manifest_path);
    let m = read_manifest(&p).map_err(|e| e.to_string())?;
    let target = p
        .parent()
        .and_then(|x| x.parent())
        .ok_or("无法定位被复验目录")?
        .to_path_buf();

    let io = volume_io();
    let mut observed = Vec::new();
    for f in scan_source(&target, &ScanOptions::mirror()).files {
        if steadcopy_core::manifest::is_manifest_path(&target, &f.absolute_path) {
            continue;
        }
        let hash =
            steadcopy_core::engine::hash_destination(io.as_ref(), &f.absolute_path, m.algorithm)
                .map_err(|e| format!("{} 读取失败：{e}", f.relative_path))?;
        observed.push(steadcopy_core::manifest::ObservedFile::new(
            &f.relative_path,
            f.size,
            hash,
        ));
    }
    let r = steadcopy_core::manifest::audit(&m, &observed, true);
    serde_json::to_value(&r).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- 格式化

#[derive(Serialize, Clone)]
struct FormatSafetyView {
    report: SafetyReport,
    passed: bool,
    /// 卷根路径。自动提议时前端没有别的途径知道对象是谁
    root: String,
    device_name: String,
    label: String,
    /// 要求用户手输的确认串。无卷标的卡不是空串，是固定词
    confirm_phrase: String,
    file_system: String,
    countdown_secs: u32,
}

#[tauri::command]
fn check_format(device_root: String) -> Result<FormatSafetyView, String> {
    let cfg = load_cfg()?;
    let vols = enumerate_volumes().map_err(|e| e.to_string())?;
    let vol = vols
        .into_iter()
        .find(|v| v.root_path().display().to_string() == device_root)
        .ok_or_else(|| format!("找不到这个卷：{device_root}"))?;

    let dest_roots: Vec<PathBuf> = cfg
        .projects
        .iter()
        .flat_map(|p| p.destinations.iter().map(|d| d.root.clone()))
        .collect();
    let device_id = vol.composite_id();
    let device_name = cfg
        .device(&device_id)
        .map(|d| d.display_name())
        .unwrap_or_else(|| vol.display_name());

    // 先跑便宜的 G1–G3；不过就别去扫整卷了
    let cheap = check_safety(&vol, &dest_roots, false, None, &[], lang());
    if cheap.checks.iter().any(|c| !c.passed && c.id != "G4") {
        return Ok(FormatSafetyView {
            passed: false,
            report: cheap,
            root: vol.root_path().display().to_string(),
            device_name,
            confirm_phrase: confirmation_phrase(&vol.label).to_string(),
            label: vol.label.clone(),
            file_system: vol.file_system.clone(),
            countdown_secs: cfg.settings.countdown_secs,
        });
    }

    let current: Vec<String> = scan_source(&vol.root_path(), &ScanOptions::mirror())
        .files
        .into_iter()
        .map(|f| f.relative_path)
        .collect();
    let l = ledger()?;
    let evidence = find_backup_evidence(&l, &device_id);
    let report = check_safety(&vol, &dest_roots, false, evidence.as_ref(), &current, lang());
    Ok(FormatSafetyView {
        passed: report.passed(),
        report,
        root: vol.root_path().display().to_string(),
        device_name,
        confirm_phrase: confirmation_phrase(&vol.label).to_string(),
        label: vol.label.clone(),
        file_system: vol.file_system.clone(),
        countdown_secs: cfg.settings.countdown_secs,
    })
}

fn find_backup_evidence(l: &Ledger, device_id: &str) -> Option<BackupEvidence> {
    let tasks = l
        .history(&HistoryQuery {
            source_id: Some(device_id.to_string()),
            limit: Some(20),
            ..Default::default()
        })
        .ok()?;
    for t in tasks {
        if t.status != TaskStatus::Ok || !t.verified {
            continue;
        }
        let mut merged: Option<Manifest> = None;
        for mp in &t.manifests {
            let Some(landing) = std::path::Path::new(mp).parent().and_then(|p| p.parent()) else {
                continue;
            };
            for (_, m) in load_manifests(landing).manifests {
                if m.source.id != device_id {
                    continue;
                }
                match &mut merged {
                    Some(acc) => acc.entries.extend(m.entries),
                    None => merged = Some(m),
                }
            }
        }
        if let Some(m) = merged {
            return Some(BackupEvidence {
                task_id: t.id,
                manifest: m,
            });
        }
    }
    None
}

/// 执行格式化。**调用前 MUST 已过 `check_format` 且用户完成三重确认。**
#[tauri::command]
fn do_format(device_root: String, typed_label: String) -> Result<(), String> {
    let cfg = load_cfg()?;
    let vols = enumerate_volumes().map_err(|e| e.to_string())?;
    let vol = vols
        .into_iter()
        .find(|v| v.root_path().display().to_string() == device_root)
        .ok_or_else(|| format!("找不到这个卷：{device_root}"))?;

    // 后端再校一次卷标——前端可以被绕过，这里不能。
    // 判据与命令行共用 core 的同一个函数，两边不可能各走各的规则。
    if !label_matches(&typed_label, &vol.label) {
        return Err("卷标不匹配，已中止".into());
    }

    let dest_roots: Vec<PathBuf> = cfg
        .projects
        .iter()
        .flat_map(|p| p.destinations.iter().map(|d| d.root.clone()))
        .collect();
    let device_id = vol.composite_id();
    let device_name = cfg
        .device(&device_id)
        .map(|d| d.display_name())
        .unwrap_or_else(|| vol.display_name());
    let current: Vec<String> = scan_source(&vol.root_path(), &ScanOptions::mirror())
        .files
        .into_iter()
        .map(|f| f.relative_path)
        .collect();

    let l = ledger()?;
    let evidence = find_backup_evidence(&l, &device_id);
    // **检查链在执行前必跑一遍**——前端点过什么不作数
    let report = check_safety(&vol, &dest_roots, false, evidence.as_ref(), &current, lang());
    let attempt = config::new_id("fmt");
    let now = SystemClock.now();

    if !report.passed() {
        let reason = report
            .first_failure()
            .map(|c| format!("{}：{}", c.id, c.detail))
            .unwrap_or_else(|| "安全检查未通过（未能定位到具体是哪一项）".to_string());
        let _ = l.record_format_attempt(
            &attempt, now, &device_id, &device_name, "gui",
            &report.compact(), None, "rejected", Some(&reason),
        );
        return Err(format!("格式化被拒绝——{reason}"));
    }

    let f = formatter();
    let params = f
        .read_params(&vol.root_path().display().to_string())
        .map_err(|e| e.to_string())?;
    match f.quick_format(&params) {
        Ok(()) => {
            let _ = l.record_format_attempt(
                &attempt, SystemClock.now(), &device_id, &device_name, "gui",
                &report.compact(), report.backup_task_id.as_deref(), "ok", None,
            );
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = l.record_format_attempt(
                &attempt, SystemClock.now(), &device_id, &device_name, "gui",
                &report.compact(), report.backup_task_id.as_deref(), "failed", Some(&msg),
            );
            Err(format!("格式化失败：{msg}"))
        }
    }
}

#[tauri::command]
fn validate_countdown_secs(secs: u32) -> Result<u32, String> {
    validate_countdown(secs, lang())
}

// ---------------------------------------------------------------- 交给系统打开
//
// 「用系统默认程序打开某个路径」是把任意路径交给 shell，能力不小。
// 所以不给前端开放 `opener:allow-open-path`——路径由后端自己算，
// 前端连传路径的机会都没有（配置文件），或者传了也要先过校验（报告）。

/// 上手教程的地址。**写死在后端**，前端传不进来别的 URL——
/// 「打开一个链接」这个能力一旦接受参数，就等于把外链决定权交给了前端。
pub const GUIDE_URL: &str = "https://hocassian.feishu.cn/docx/BAALdIhzvoKkPLxlr8icZ4Nwn6d";

/// 打开上手教程。这是一次**用户主动点击**的外链，不是后台请求——
/// 「零遥测」说的是程序不自己联网，不是禁止用户点链接。
#[tauri::command]
fn open_guide(app: AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(GUIDE_URL, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_config_file(app: AppHandle) -> Result<(), String> {
    let p = config::config_path();
    if !p.exists() {
        return Err("配置文件还没有生成".into());
    }
    app.opener()
        .open_path(p.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// 在资源管理器里定位落地目录。
///
/// 与 `open_report_file` 同样只接受**稳拷自己落的清单路径**——
/// 它的上两级就是落地目录，用不着前端传目录进来。
#[tauri::command]
fn reveal_landing_dir(app: AppHandle, manifest_path: String) -> Result<(), String> {
    let p = PathBuf::from(&manifest_path);
    let in_manifest_dir = p
        .parent()
        .and_then(|d| d.file_name())
        .is_some_and(|n| n.eq_ignore_ascii_case("steadcopy"));
    if !in_manifest_dir {
        return Err("这不是稳拷生成的清单".into());
    }
    let landing = p.parent().and_then(|d| d.parent()).ok_or("定位不到落地目录")?;
    if !landing.exists() {
        return Err("落地目录不存在，可能已被移动或删除".into());
    }
    app.opener()
        .reveal_item_in_dir(landing)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_report_file(app: AppHandle, manifest_path: String) -> Result<(), String> {
    let html = PathBuf::from(&manifest_path).with_extension("html");
    // 只放行**我们自己落的**报告：必须是 steadcopy 凭证目录里的既存 .html
    let in_manifest_dir = html
        .parent()
        .and_then(|d| d.file_name())
        .is_some_and(|n| n.eq_ignore_ascii_case("steadcopy"));
    if !in_manifest_dir || html.extension().and_then(|e| e.to_str()) != Some("html") {
        return Err("这不是稳拷生成的报告".into());
    }
    if !html.exists() {
        return Err("报告文件不存在，可能已被移动或删除".into());
    }
    app.opener()
        .open_path(html.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- 更新
//
// 规范：能力 `build-release` 的 spec → Requirement: 更新检查非强制
//
// **这里的每一条约束都是刻意的：**
//
// - **只在用户点了才查。** 没有后台轮询、没有启动时自动检查。
//   「零遥测」的含义因此仍然成立：程序自己从不联网，联网只发生在用户按下按钮之后。
// - **可以彻底关掉。** 关掉之后连按钮都没有，更谈不上请求。
// - **绝不自动安装。** 查到新版本只告诉你，装不装你说了算。
// - **更新地址写死在编译期。** 不从配置读——更新地址一旦可配，
//   谁能改配置文件谁就能让所有客户端静默安装任意程序。
// - **验签由官方 updater 用编译进程序的公钥做。** 私钥只在发布机与 CI secret 里。
// - **下载地址还要再过一遍主机白名单。** 见 `update_origin`：验签发生在下载之后，
//   而清单里的 url 来自网络。不先卡主机，一个被劫持的端点就能让程序去请求任意地址——
//   包是装不上（验签会拒），但「谁在什么时候查更新」已经泄给第三方了。

#[derive(Serialize)]
struct UpdateInfo {
    available: bool,
    current: String,
    /// 有新版本时才有值
    version: Option<String>,
    notes: Option<String>,
    date: Option<String>,
}

/// 查更新的超时。镜像挂在自建 NAS 的非标端口上，「连得上但不应答」是真实存在的
/// 故障态（负载高、被中间设备挂住、只丢包不 RST）。没有超时的话按钮会永久卡在
/// 「检查中…」，用户只能重启程序——而他并不知道是网络问题还是程序死了。
const CHECK_TIMEOUT_SECS: u64 = 15;

/// 下载安装包的超时。与查更新分开是因为 `RequestBuilder::timeout` 管的是**整个请求**，
/// 包括读完 body——用 15 秒会把慢网上的正常下载一起掐死。
/// 到这一步用户已经点过「安装」、知道在下东西了，等得起。
const DOWNLOAD_TIMEOUT_SECS: u64 = 600;

/// 建一个更新器。
///
/// 两件默认值必须改掉：
/// - **超时**：`UpdaterBuilder` 默认 `None`，reqwest 也默认不超时，等于没有上界。
/// - **重定向**：reqwest 默认跟随 10 跳且不看主机。只在第一跳查白名单是纸面防线，
///   见 `update_origin::redirect_policy`。
fn updater_for(app: &AppHandle, timeout: Duration) -> Result<tauri_plugin_updater::Updater, String> {
    app.updater_builder()
        .timeout(timeout)
        .configure_client(|b| b.redirect(update_origin::redirect_policy()))
        .build()
        .map_err(|e| e.to_string())
}

/// 把 updater 的错误翻成人话。
///
/// 直接 `e.to_string()` 透出去的后果：中文界面上甩一句
/// `Could not fetch a valid release JSON from the remote`——别处错误全走 `pick()`，
/// 唯独这里是英文原文。更糟的是连不上时 reqwest 会把**镜像的域名和端口**
/// 一起印进错误串给用户看，而那是内部基础设施。
fn update_error(e: &tauri_plugin_updater::Error, lang: Locale) -> String {
    use tauri_plugin_updater::Error;
    match e {
        // 两个端点都没给出可用的清单。最常见的成因就是网络不通或还没发过版
        Error::ReleaseNotFound => lang.pick(
            "取不到更新信息：更新源都没有应答，或者还没有发布过版本。过会儿再试",
            "Could not reach any update source. They may be unreachable, or nothing has been published yet. Try again later",
        ).to_string(),
        Error::Network(_) | Error::Reqwest(_) => lang.pick(
            "连不上更新源。检查一下网络，或者过会儿再试",
            "Could not connect to the update source. Check your network and try again",
        ).to_string(),
        Error::Io(_) => lang.pick(
            "写入更新包时出错：磁盘可能满了，或者文件被占用",
            "Failed to write the update package. The disk may be full, or the file is in use",
        ).to_string(),
        // 验签失败是最该说清楚的一类：它意味着拿到的字节不是发布密钥签的
        Error::Minisign(_) | Error::SignatureUtf8(_) | Error::Base64(_) => lang.pick(
            "更新包验签失败，已拒绝安装——包不是由发布密钥签名的，或者在传输中被改过",
            "Signature check failed; refused to install. The package was not signed by the release key, or was altered in transit",
        ).to_string(),
        // 剩下的（清单格式不对、平台缺条目等）没法给出更具体的指引，
        // 但仍然不把原文透出去——那是给日志看的，不是给用户看的
        _ => lang.pick(
            "更新失败。详情见日志",
            "The update failed. See the log for details",
        ).to_string(),
    }
}

/// 查一次有没有新版本。**只有用户按了按钮才会走到这里。**
#[tauri::command]
async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let cfg = load_cfg()?;
    let lang = Locale::resolve(&cfg.settings.locale);
    if !cfg.settings.update_check {
        // 关掉之后连请求都不该发出去，而不是「发了但不提示」
        return Err(lang
            .pick(
                "更新检查已在设置里关闭",
                "Update checking is turned off in settings",
            )
            .to_string());
    }
    if config::is_portable() {
        return Err(portable_no_update(lang));
    }

    let current = env!("CARGO_PKG_VERSION").to_string();

    // 上一次点安装的结果先核对。对不上就**停在这儿**——
    // 不停的话就是那个循环：更新源说 9.9.9，装上的是旧版，下次再查还是 9.9.9，
    // 于是反复重装，永远停在旧版，而用户只看到「更新好像没用」。
    if let Some(anomaly) = update_verify::take_anomaly(&current) {
        let mut c = load_cfg()?;
        c.settings.update_check = false;
        save_cfg(&c)?;
        return Err(anomaly.describe(lang));
    }

    let updater = updater_for(&app, Duration::from_secs(CHECK_TIMEOUT_SECS))?;
    match updater.check().await {
        Ok(Some(u)) => Ok(UpdateInfo {
            available: true,
            current,
            version: Some(u.version.clone()),
            notes: u.body.clone(),
            date: u.date.map(|d| d.to_string()),
        }),
        Ok(None) => Ok(UpdateInfo {
            available: false,
            current,
            version: None,
            notes: None,
            date: None,
        }),
        Err(e) => Err(update_error(&e, lang)),
    }
}

/// 便携版为什么不能走更新器。
///
/// 更新清单只指一个包，而那是 **NSIS 安装包**。便携版用户点下去的结果是：
/// 安装包装进 `%LOCALAPPDATA%\Programs\稳拷`、重启的是**新装的那份**，
/// 而便携目录原封不动还是旧版。更要命的是新装的那份旁边没有便携标记，
/// 配置目录从 `<便携目录>\data` 切到 `%APPDATA%`——项目、预设、设备记忆、
/// 任务台账在用户眼里**全部消失**。
///
/// 便携版的更新方式就是「下载新的 zip 覆盖」，这也符合它「解压即用、删文件夹即卸载」
/// 的性质：它本来就不该往系统里装东西。
fn portable_no_update(lang: Locale) -> String {
    lang.pick(
        "便携版不走自动更新——请直接下载新版压缩包覆盖当前文件夹，你的数据都在同目录的 data\\ 里，不会丢",
        "The portable build does not auto-update. Download the new zip and replace this folder; your data lives in data\\ next to the executable and is not affected",
    )
    .to_string()
}

/// 下载并安装。**必须由用户在看到版本号之后再点一次**——
/// 检查与安装是两个动作，中间隔着用户的一次决定。
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    let cfg = load_cfg()?;
    let lang = Locale::resolve(&cfg.settings.locale);
    if !cfg.settings.update_check {
        return Err(lang
            .pick(
                "更新检查已在设置里关闭",
                "Update checking is turned off in settings",
            )
            .to_string());
    }
    if config::is_portable() {
        return Err(portable_no_update(lang));
    }
    // 有任务在跑就不装——装更新会重启程序，正在拷的卡就断在半路
    let running = app
        .state::<AppState>()
        .running
        .lock()
        .map(|r| r.clone())
        .map_err(|_| "任务状态锁异常")?;
    if !running.is_empty() {
        return Err(lang
            .pick(
                "有任务正在进行，等它跑完再更新——装更新会重启程序",
                "A task is running. Wait for it to finish — installing an update restarts the app",
            )
            .to_string());
    }

    let updater = updater_for(&app, Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))?;
    let Some(update) = updater.check().await.map_err(|e| update_error(&e, lang))? else {
        return Err(lang
            .pick("已经是最新版本", "Already on the latest version")
            .to_string());
    };

    // 下载之前先卡主机。清单来自网络，`download_url` 是清单说了算的——
    // 端点被劫持时它可以是任意地址。验签虽然最终会拒掉坏包，但那是**请求发出之后**的事。
    let url = update.download_url.as_str();
    if !update_origin::is_allowed_update_url(url) {
        return Err(lang.pick(
            "更新包的下载地址不在允许的来源里，已拒绝——这通常意味着更新清单被篡改过",
            "The update download address is not an allowed origin; refused. This usually means the update manifest was tampered with",
        ).to_string());
    }

    // 记下「更新源答应的版本」与「现在的版本」，重启后核对。
    // 验签保证不了这件事：它签的是安装包的字节，清单里的 version 是明文、不受签名保护。
    // 见 update_verify 模块头。
    update_verify::record_promised(&update.version, env!("CARGO_PKG_VERSION"));

    // 验签由 updater 用编译进程序的公钥做；签名对不上这里就会失败
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| update_error(&e, lang))?;
    app.restart();
}

#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ---------------------------------------------------------------- 关于

#[derive(Serialize)]
struct BuildInfo {
    version: String,
    commit: String,
    build_time: String,
    rustc: String,
    tauri: String,
    portable: bool,
    data_dir: String,
    /// 一行可复制的定位串，用户报问题时贴这个就够
    signature: String,
}

/// 构建元信息。用户报问题时，「哪次编译」比「哪个版本号」定得住得多。
#[tauri::command]
fn build_info() -> BuildInfo {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let commit = env!("STEADCOPY_COMMIT").to_string();
    let build_time = env!("STEADCOPY_BUILD_TIME")
        .parse::<i64>()
        .ok()
        .and_then(|s| time::OffsetDateTime::from_unix_timestamp(s).ok())
        .map(steadcopy_core::manifest::format_time_human)
        .unwrap_or_else(|| "未知".into());
    let rustc = env!("STEADCOPY_RUSTC").to_string();
    let portable = config::is_portable();

    BuildInfo {
        signature: format!(
            "steadcopy {version} ({commit}) {}",
            if portable { "便携版" } else { "安装版" }
        ),
        version,
        commit,
        build_time,
        rustc,
        tauri: tauri::VERSION.to_string(),
        portable,
        data_dir: config::config_dir().display().to_string(),
    }
}

/// 第三方依赖许可清单。由 `scripts/gen-licenses.py` 生成后编进程序，
/// 与随包分发的 `THIRD-PARTY-LICENSES.md` 同源，不会两处对不上。
#[tauri::command]
fn third_party_licenses() -> Result<serde_json::Value, String> {
    serde_json::from_str(include_str!("../licenses.json")).map_err(|e| e.to_string())
}

#[tauri::command]
fn scan(source: String) -> Result<serde_json::Value, String> {
    let root = PathBuf::from(&source);
    if !root.exists() {
        return Err(format!("路径不存在：{source}"));
    }
    let r = scan_source(&root, &ScanOptions::mirror());
    let filter = steadcopy_core::organize::FilterConfig::default();
    let categories: Vec<(String, usize, u64)> = r
        .by_category(&filter)
        .into_iter()
        .map(|(k, (n, b))| (k.to_string(), n, b))
        .collect();
    Ok(serde_json::json!({
        "files": r.file_count(),
        "total_bytes": r.total_bytes(),
        "junk_excluded": r.junk_excluded,
        "fingerprints": r.fingerprints,
        "categories": categories,
    }))
}

// ---------------------------------------------------------------- 入口

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例必须第一个装。跑起第二个进程会有第二套设备监听，
        // 同一张卡被两个实例各拷一遍——这不是界面问题，是数据问题
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            app.manage(AppState::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_settings,
            upsert_project,
            delete_project,
            set_current_project,
            upsert_preset,
            delete_preset,
            move_preset,
            set_device_kind,
            rename_device,
            forget_device,
            preview_path,
            config_path,
            list_devices,
            start_watching,
            arrive_now,
            confirm_and_run,
            dismiss_arrival,
            cancel_copy,
            set_paused,
            eject_device,
            adhoc_prefill,
            plan_adhoc,
            sink_preset,
            list_history,
            task_files,
            clear_history,
            format_attempts,
            report_html,
            run_audit,
            check_format,
            do_format,
            validate_countdown_secs,
            open_config_file,
            open_guide,
            open_report_file,
            reveal_landing_dir,
            app_version,
            check_update,
            install_update,
            build_info,
            third_party_licenses,
            scan,
        ])
        .run(tauri::generate_context!())
        .expect("启动应用失败");
}
