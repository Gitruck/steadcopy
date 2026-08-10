//! 稳拷桌面应用的 Tauri 后端。
//!
//! 规范：`openspec/changes/add-steadcopy-app/specs/app-shell/spec.md`
//!
//! 铁律：**前端零业务逻辑。** 本文件只做「门面命令 → core」的桥接与事件转发，
//! 路径渲染 / 增量判定 / 空间计算 / 哈希一律在 core 里算，前端只发命令、订阅事件、渲染状态。

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use steadcopy_core::device::{enumerate_volumes, Volume};
use steadcopy_core::engine::{CancelToken, HashAlgorithm};
use steadcopy_core::ledger::{render_report, ReportInput};
use steadcopy_core::manifest::model::SourceRef;
use steadcopy_core::manifest::{load_manifests, read_manifest, Manifest};
use steadcopy_core::organize::{scan_source, PathTemplate, ScanOptions};
use steadcopy_core::platform::{volume_io, Clock, SystemClock};
use steadcopy_core::task::{
    plan_task, run_task, DestinationSpec, FileStatus, StageEvent, TaskSpec,
};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Default)]
struct AppState {
    cancel: Mutex<Option<CancelToken>>,
}

// ---------- 前端可见的数据形状 ----------

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
}

impl From<&Volume> for DeviceView {
    fn from(v: &Volume) -> Self {
        Self {
            id: v.composite_id(),
            name: v.display_name(),
            root: v.root_path().display().to_string(),
            file_system: v.file_system.clone(),
            bus: v.bus_type.label().to_string(),
            total_bytes: v.total_bytes,
            free_bytes: v.free_bytes,
            is_system: v.is_system,
            can_be_source: v.can_be_source(&[]),
            fingerprints: v.fingerprints.clone(),
        }
    }
}

#[derive(Deserialize, Clone)]
struct TaskInput {
    source: String,
    destinations: Vec<String>,
    project: String,
    device_name: String,
    template: String,
    verify: bool,
    algorithm: String,
}

impl TaskInput {
    fn to_spec(&self) -> Result<TaskSpec, String> {
        if self.destinations.is_empty() || self.destinations.len() > 4 {
            return Err("目的地数量应为 1..4 个".into());
        }
        let template = PathTemplate::parse(&self.template).map_err(|e| e.to_string())?;
        Ok(TaskSpec {
            source_root: PathBuf::from(&self.source),
            source: SourceRef {
                id: format!("path:{}", self.source),
                display_name: self.device_name.clone(),
            },
            project: self.project.clone(),
            destinations: self
                .destinations
                .iter()
                .map(|d| DestinationSpec {
                    root: PathBuf::from(d),
                    template: template.clone(),
                    enabled: true,
                })
                .collect(),
            algorithm: if self.algorithm == "md5" {
                HashAlgorithm::Md5
            } else {
                HashAlgorithm::Xxh64
            },
            verify: self.verify,
            scan: ScanOptions::mirror(),
            retries: 2,
            at: SystemClock.now(),
        })
    }
}

#[derive(Serialize)]
struct ScanView {
    files: usize,
    total_bytes: u64,
    junk_excluded: usize,
    fingerprints: Vec<String>,
    categories: Vec<(String, usize, u64)>,
}

#[derive(Serialize)]
struct PlanView {
    to_copy: usize,
    to_copy_bytes: u64,
    skipped: usize,
    no_source: bool,
    no_new_source: bool,
    destinations: Vec<PlanDestView>,
    notices: Vec<String>,
}

#[derive(Serialize)]
struct PlanDestView {
    landing_dir: String,
    required_bytes: u64,
    available_bytes: Option<u64>,
    sufficient: Option<bool>,
}

#[derive(Serialize, Clone)]
struct ProgressPayload {
    stage: String,
    percent: f64,
    current: Option<String>,
    done: u64,
    total: u64,
}

#[derive(Serialize, Clone)]
struct FailurePayload {
    path: String,
    reason: String,
}

#[derive(Serialize)]
struct RunView {
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

#[derive(Serialize)]
struct HistoryItem {
    manifest_path: String,
    project: String,
    device: String,
    landing_dir: String,
    created_at: String,
    files: usize,
    verified: usize,
    total_bytes: u64,
    algorithm: String,
}

// ---------- 命令 ----------

#[tauri::command]
fn list_devices() -> Result<Vec<DeviceView>, String> {
    let vols = enumerate_volumes().map_err(|e| e.to_string())?;
    Ok(vols.iter().map(DeviceView::from).collect())
}

#[tauri::command]
fn scan(source: String) -> Result<ScanView, String> {
    let root = PathBuf::from(&source);
    if !root.exists() {
        return Err(format!("路径不存在：{source}"));
    }
    let r = scan_source(&root, &ScanOptions::mirror());
    let filter = steadcopy_core::organize::FilterConfig::default();
    let categories = r
        .by_category(&filter)
        .into_iter()
        .map(|(k, (n, b))| (k.to_string(), n, b))
        .collect();
    Ok(ScanView {
        files: r.file_count(),
        total_bytes: r.total_bytes(),
        junk_excluded: r.junk_excluded,
        fingerprints: r.fingerprints.clone(),
        categories,
    })
}

#[tauri::command]
fn plan(input: TaskInput) -> Result<PlanView, String> {
    let spec = input.to_spec()?;
    let io = volume_io();
    let p = plan_task(&spec, io.as_ref()).map_err(|e| e.to_string())?;
    Ok(PlanView {
        to_copy: p.files.len(),
        to_copy_bytes: p.total_bytes(),
        skipped: p.skipped.len(),
        no_source: p.is_no_source(),
        no_new_source: p.is_no_new_source(),
        notices: p
            .destinations
            .iter()
            .flat_map(|d| d.ledger_degraded.clone())
            .collect(),
        destinations: p
            .destinations
            .iter()
            .map(|d| PlanDestView {
                landing_dir: d.landing_dir.display().to_string(),
                required_bytes: d.required_bytes,
                available_bytes: d.available_bytes,
                sufficient: d.sufficient(),
            })
            .collect(),
    })
}

#[tauri::command]
async fn start_copy(
    app: AppHandle,
    state: State<'_, AppState>,
    input: TaskInput,
) -> Result<RunView, String> {
    let spec = input.to_spec()?;
    let cancel = CancelToken::new();
    if let Ok(mut slot) = state.cancel.lock() {
        *slot = Some(cancel.clone());
    }

    // 阻塞式 IO 放到独立线程，不占住 Tauri 的异步运行时
    let handle = std::thread::spawn(move || -> Result<RunView, String> {
        let io = volume_io();
        let clock = SystemClock;
        let plan = plan_task(&spec, io.as_ref()).map_err(|e| e.to_string())?;

        if plan.is_no_source() {
            return Err("源上没有可拷贝的素材".into());
        }
        if let Some(d) = plan.insufficient().next() {
            return Err(format!(
                "目的地空间不足：{} 还差 {} 字节",
                d.landing_dir.display(),
                d.shortfall().unwrap_or(0)
            ));
        }

        let started = std::time::Instant::now();
        // 事件限流在此处（消费方）：引擎发全量，界面按 100ms 收
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let report = run_task(&spec, &plan, io.as_ref(), &clock, &cancel, &mut |e| match e {
            StageEvent::Stage(s) => {
                let _ = app.emit(
                    "task-stage",
                    ProgressPayload {
                        stage: s.label().to_string(),
                        percent: 0.0,
                        current: None,
                        done: 0,
                        total: 0,
                    },
                );
            }
            StageEvent::Progress {
                stage,
                done,
                total,
                current,
            } => {
                if last.elapsed() < std::time::Duration::from_millis(100) {
                    return;
                }
                last = std::time::Instant::now();
                let _ = app.emit(
                    "task-progress",
                    ProgressPayload {
                        stage: stage.label().to_string(),
                        percent: steadcopy_core::task::stage::percent(done, total),
                        current,
                        done,
                        total,
                    },
                );
            }
            StageEvent::FileFailed {
                relative_path,
                reason,
            } => {
                let _ = app.emit(
                    "task-file-failed",
                    FailurePayload {
                        path: relative_path,
                        reason,
                    },
                );
            }
            StageEvent::Notice(msg) => {
                let _ = app.emit("task-notice", msg);
            }
        })
        .map_err(|e| e.to_string())?;

        // 拷完自动出报告——「证」是这个产品的一半价值
        let failures: Vec<(String, String, u32)> = report
            .failed_files()
            .map(|f| {
                let reason = match &f.status {
                    FileStatus::Failed(m) => m.clone(),
                    _ => String::new(),
                };
                (f.relative_path.clone(), reason, f.retries)
            })
            .collect();
        for mp in &report.manifests {
            if let Ok(m) = read_manifest(mp) {
                let input = ReportInput {
                    manifest: &m,
                    failures: &failures,
                    skipped: report.skipped_count(),
                    notices: &report.notices,
                    elapsed_secs: Some(started.elapsed().as_secs()),
                    generated_at: SystemClock.now(),
                    audit: None,
                };
                let _ = std::fs::write(mp.with_extension("html"), render_report(&input));
            }
        }

        Ok(RunView {
            copied: report.copied_count(),
            skipped: report.skipped_count(),
            failed: failures.len(),
            bytes_copied: report.bytes_copied,
            cancelled: report.cancelled,
            all_succeeded: report.all_succeeded(),
            manifests: report
                .manifests
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            notices: report.notices.clone(),
            failures: failures
                .into_iter()
                .map(|(path, reason, _)| FailurePayload { path, reason })
                .collect(),
        })
    });

    handle.join().map_err(|_| "拷贝线程异常终止".to_string())?
}

#[tauri::command]
fn cancel_copy(state: State<'_, AppState>) -> Result<(), String> {
    if let Ok(slot) = state.cancel.lock() {
        if let Some(c) = slot.as_ref() {
            c.cancel();
        }
    }
    Ok(())
}

/// 列出某些目的地根目录下的历史任务（由 manifest 还原）。
#[tauri::command]
fn list_history(roots: Vec<String>) -> Result<Vec<HistoryItem>, String> {
    let mut out = Vec::new();
    for root in roots {
        collect_history(&PathBuf::from(root), &mut out, 0);
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

fn collect_history(dir: &PathBuf, out: &mut Vec<HistoryItem>, depth: usize) {
    if depth > 6 || !dir.is_dir() {
        return;
    }
    let loaded = load_manifests(dir);
    for (p, m) in loaded.manifests {
        out.push(history_item(&p, &m));
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir()
            && p.file_name()
                .is_some_and(|n| n != steadcopy_core::manifest::MANIFEST_DIR)
        {
            collect_history(&p, out, depth + 1);
        }
    }
}

fn history_item(path: &std::path::Path, m: &Manifest) -> HistoryItem {
    HistoryItem {
        manifest_path: path.display().to_string(),
        project: m.project.clone(),
        device: m.source.display_name.clone(),
        landing_dir: m.destination_root.display().to_string(),
        created_at: steadcopy_core::manifest::format_time_human(m.created_at),
        files: m.entries.len(),
        verified: m.verified_count(),
        total_bytes: m.total_bytes(),
        algorithm: m.algorithm.id().to_string(),
    }
}

/// **在界面内直接展示报告**：返回报告 HTML 全文，前端塞进沙箱 iframe 渲染。
///
/// 已有 .html 就读它（保留当时的失败清单与耗时），否则按清单现场生成。
#[tauri::command]
fn report_html(manifest_path: String) -> Result<String, String> {
    let p = PathBuf::from(&manifest_path);
    let html_path = p.with_extension("html");
    if let Ok(s) = std::fs::read_to_string(&html_path) {
        return Ok(s);
    }
    let m = read_manifest(&p).map_err(|e| e.to_string())?;
    let input = ReportInput {
        manifest: &m,
        failures: &[],
        skipped: 0,
        notices: &[],
        elapsed_secs: None,
        generated_at: SystemClock.now(),
        audit: None,
    };
    Ok(render_report(&input))
}

/// 复验：读清单、无缓冲读回目录、产四态结果。
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

#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            app.manage(AppState::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_devices,
            scan,
            plan,
            start_copy,
            cancel_copy,
            list_history,
            report_html,
            run_audit,
            app_version,
        ])
        .run(tauri::generate_context!())
        .expect("启动应用失败");
}
