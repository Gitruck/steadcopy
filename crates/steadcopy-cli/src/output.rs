//! 人读（中文）与机读（`--json`）双输出。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/cli-driver/spec.md`
//! → Requirement: 人读与机读双输出 / 退出码契约
//!
//! 铁律：`--json` 时 **stdout 只出 JSON**，一切人读日志与进度走 stderr，
//! 否则自动化侧没法直接 `| jq`。

use serde::Serialize;
use std::sync::OnceLock;

use steadcopy_core::manifest::AuditReport;
use steadcopy_core::organize::ScanResult;
use steadcopy_core::task::{StageEvent, TaskPlan, TaskReport, TaskStage};

/// 退出码契约。数值即退出码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitKind {
    Ok = 0,
    /// 终态族：无素材 / 空间不足 / 配置非法，重跑一样
    Terminal = 1,
    /// 可重试族：拷贝失败 / 校验失败 / 设备移除
    Retryable = 2,
    Cancelled = 3,
    #[allow(dead_code)]
    Usage = 4,
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.2} {}", UNITS[i])
    }
}

pub struct Emitter {
    json: bool,
}

#[derive(Serialize)]
struct ScanJson {
    files: usize,
    total_bytes: u64,
    junk_excluded: usize,
    filtered_out: usize,
    fingerprints: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    list: Option<Vec<FileJson>>,
}

#[derive(Serialize)]
struct FileJson {
    path: String,
    size: u64,
}

#[derive(Serialize)]
struct PlanJson {
    scanned_files: usize,
    to_copy: usize,
    to_copy_bytes: u64,
    skipped: usize,
    destinations: Vec<DestJson>,
    no_source: bool,
    no_new_source: bool,
    notices: Vec<String>,
}

#[derive(Serialize)]
struct DestJson {
    landing_dir: String,
    required_bytes: u64,
    required_files: usize,
    available_bytes: Option<u64>,
    sufficient: Option<bool>,
}

#[derive(Serialize)]
struct ReportJson {
    copied: usize,
    skipped: usize,
    failed: usize,
    bytes_copied: u64,
    cancelled: bool,
    manifests: Vec<String>,
    notices: Vec<String>,
    failures: Vec<FailureJson>,
}

#[derive(Serialize)]
struct FailureJson {
    path: String,
    reason: String,
    retries: u32,
}

#[derive(Serialize)]
struct EmptyJson {
    result: &'static str,
    message: String,
}

#[derive(Serialize)]
struct ErrorJson {
    error: String,
}

impl Emitter {
    pub fn new(json: bool) -> Self {
        Self { json }
    }

    fn out(&self, value: &impl Serialize) {
        if let Ok(s) = serde_json::to_string(value) {
            println!("{s}");
        }
    }

    /// 人读信息一律走 stderr——`--json` 时 stdout 必须保持纯净。
    fn note_line(&self, msg: &str) {
        eprintln!("{msg}");
    }

    pub fn error(&self, msg: &str) {
        if self.json {
            self.out(&ErrorJson {
                error: msg.to_string(),
            });
        } else {
            eprintln!("✗ {msg}");
        }
    }

    pub fn finished_empty(&self, result: &'static str, message: &str) {
        if self.json {
            self.out(&EmptyJson {
                result,
                message: message.to_string(),
            });
        } else {
            self.note_line(&format!("· {message}"));
        }
    }

    pub fn warn(&self, msg: &str) {
        eprintln!("⚠ {msg}");
    }

    pub fn note(&self, msg: &str) {
        eprintln!("· {msg}");
    }

    pub fn watch_ready(&self, c: &steadcopy_core::config::Config, path: &str) {
        let enabled = c.presets.iter().filter(|p| p.enabled).count();
        eprintln!("稳拷正在守候插卡…（Ctrl-C 退出）");
        eprintln!("  项目 {} 个 · 启用的预设 {} 条 · 已记忆设备 {} 个",
            c.projects.len(), enabled, c.devices.len());
        eprintln!("  配置：{path}");
    }

    pub fn arrival(&self, o: &steadcopy_core::preset::ArrivalOutcome) {
        use steadcopy_core::preset::ArrivalOutcome as A;
        let mark = match o {
            A::Planned { .. } => "▸",
            A::NeedsClassification { .. } | A::NoPreset { .. } | A::NoProject { .. } => "?",
            A::InsufficientSpace { .. } => "✗",
            _ => "·",
        };
        eprintln!("
{mark} {}", o.summary(lang()));
        if let A::NeedsClassification { device_id, .. } = o {
            eprintln!("    指认：steadcopy device set-kind \"{device_id}\" camera");
        }
        if let A::Planned { plan, .. } = o {
            eprintln!("    待拷 {} 个 · {}", plan.files.len(), human_bytes(plan.total_bytes()));
            for d in &plan.destinations {
                eprintln!("    → {}", d.landing_dir.display());
            }
        }
    }

    pub fn projects(&self, c: &steadcopy_core::config::Config) {
        if self.json { self.out(&c.projects); return; }
        if c.projects.is_empty() { println!("还没有项目。用 `steadcopy project add <名称> -d <目的地>` 建一个"); return; }
        for p in &c.projects {
            let cur = if c.current_project.as_deref() == Some(p.id.as_str()) { " ← 当前" } else { "" };
            println!("{}  {}{}", p.id, p.name, cur);
            for d in &p.destinations {
                println!("    {} {}  模板 {}", if d.enabled { "☑" } else { "☐" }, d.root.display(), d.template);
            }
        }
    }

    pub fn presets(&self, c: &steadcopy_core::config::Config) {
        if self.json { self.out(&c.presets); return; }
        if c.presets.is_empty() { println!("还没有预设。用 `steadcopy preset add <名称> --matches kind:camera` 配一条"); return; }
        for p in &c.presets {
            let proj = p.project_id.as_ref()
                .and_then(|id| c.project(id))
                .map(|x| x.name.clone())
                .unwrap_or_else(|| "当前项目".into());
            println!("{} {}  匹配 {} → 项目「{}」 校验 {}",
                if p.enabled { "☑" } else { "☐" },
                p.name, p.matcher.describe(lang()), proj,
                if p.verify { "开" } else { "关" });
        }
    }

    pub fn device_records(&self, c: &steadcopy_core::config::Config) {
        if self.json { self.out(&c.devices); return; }
        if c.devices.is_empty() { println!("记忆库还是空的。插一张卡，稳拷会记住它"); return; }
        let (ignored, active): (Vec<_>, Vec<_>) = c.devices.iter()
            .partition(|d| d.kind == steadcopy_core::device::DeviceKind::Ignored);
        for d in &active {
            println!("{:<10} {:<20} {}", d.kind.label(lang()), d.display_name(), d.id);
        }
        if !ignored.is_empty() {
            println!("
已忽略（插上不会打扰，可用 device set-kind 取消）");
            for d in &ignored {
                println!("{:<10} {:<20} {}", d.kind.label(lang()), d.display_name(), d.id);
            }
        }
    }

    pub fn safety(&self, r: &steadcopy_core::device::SafetyReport, device: &str) {
        if self.json {
            self.out(r);
            return;
        }
        println!("格式化前置检查 · {device}");
        for c in &r.checks {
            println!("  {} {}  {}", if c.passed { "✓" } else { "✗" }, c.id, c.detail);
        }
    }

    pub fn devices(&self, vols: &[steadcopy_core::device::Volume]) {
        if self.json {
            self.out(&vols);
            return;
        }
        println!("本机卷");
        for v in vols {
            let src = if v.can_be_source(&[]) { "可作为源" } else { "—" };
            let sys = if v.is_system { " · 系统盘" } else { "" };
            println!(
                "  {:<28} {:>10} / {:<10} {:<12} {:<9} {}{}",
                v.display_name(),
                human_bytes(v.free_bytes),
                human_bytes(v.total_bytes),
                v.file_system,
                v.bus_type.label(),
                src,
                sys
            );
            if !v.fingerprints.is_empty() {
                println!("      设备推测：{}", v.fingerprints.join("、"));
            }
        }
    }

    pub fn scan(&self, r: &ScanResult, list: bool) {
        if self.json {
            self.out(&ScanJson {
                files: r.file_count(),
                total_bytes: r.total_bytes(),
                junk_excluded: r.junk_excluded,
                filtered_out: r.filtered_out,
                fingerprints: r.fingerprints.clone(),
                list: list.then(|| {
                    r.files
                        .iter()
                        .map(|f| FileJson {
                            path: f.relative_path.clone(),
                            size: f.size,
                        })
                        .collect()
                }),
            });
            return;
        }
        println!("扫描结果");
        if !r.fingerprints.is_empty() {
            println!("  设备推测：{}", r.fingerprints.join("、"));
        }
        println!(
            "  文件 {} 个 · {}",
            r.file_count(),
            human_bytes(r.total_bytes())
        );
        if r.junk_excluded > 0 {
            println!("  已排除系统垃圾 {} 个", r.junk_excluded);
        }
        if r.filtered_out > 0 {
            println!("  被类型过滤排除 {} 个", r.filtered_out);
        }
        if list {
            for f in &r.files {
                println!("  {}  {}", f.relative_path, human_bytes(f.size));
            }
        }
    }

    pub fn plan(&self, p: &TaskPlan) {
        let notices: Vec<String> = p
            .destinations
            .iter()
            .flat_map(|d| d.ledger_degraded.clone())
            .collect();

        if self.json {
            self.out(&PlanJson {
                scanned_files: p.scan.file_count(),
                to_copy: p.files.len(),
                to_copy_bytes: p.total_bytes(),
                skipped: p.skipped.len(),
                destinations: p
                    .destinations
                    .iter()
                    .map(|d| DestJson {
                        landing_dir: d.landing_dir.display().to_string(),
                        required_bytes: d.required_bytes,
                        required_files: d.required_files,
                        available_bytes: d.available_bytes,
                        sufficient: d.sufficient(),
                    })
                    .collect(),
                no_source: p.is_no_source(),
                no_new_source: p.is_no_new_source(),
                notices,
            });
            return;
        }

        println!("任务计划");
        println!(
            "  本次待拷 {} 个文件 · {}（已跳过 {} 个）",
            p.files.len(),
            human_bytes(p.total_bytes()),
            p.skipped.len()
        );
        for (i, d) in p.destinations.iter().enumerate() {
            println!("  目的地 {}：{}", i + 1, d.landing_dir.display());
            let avail = d
                .available_bytes
                .map(human_bytes)
                .unwrap_or_else(|| "未知".into());
            let verdict = match d.sufficient() {
                Some(true) => "空间充足",
                Some(false) => "空间不足",
                None => "空间无法确认",
            };
            println!(
                "    需要 {} · 可用 {} · {verdict}",
                human_bytes(d.required_bytes),
                avail
            );
        }
        for n in &notices {
            self.note_line(&format!("  ⚠ 历史清单不可读（{n}），本次执行全量拷贝"));
        }
    }

    /// 进度回调。全部走 stderr。
    ///
    /// **限流在消费方**：引擎按真实进展发全量事件（CLI 与自动化才拿得到完整流），
    /// 由这里决定多久渲染一次——距上次 <100ms 且进度变化 <0.5% 就不画。
    /// 否则大量小文件时终端会被刷爆。
    pub fn progress_sink(&self) -> impl FnMut(StageEvent) + '_ {
        let mut last_stage: Option<TaskStage> = None;
        let mut last_draw = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let mut last_pct = -1.0f64;
        let mut line_pending = false;
        // 管道里 \r 不产生视觉覆盖，此时改为不重画同一行，避免刷屏
        let tty = std::io::IsTerminal::is_terminal(&std::io::stderr());

        move |e: StageEvent| {
            // 任何非进度输出之前，先把未收尾的进度行收掉
            let end_line = |pending: &mut bool| {
                if *pending {
                    eprintln!();
                    *pending = false;
                }
            };

            match e {
                StageEvent::Stage(s) => {
                    if last_stage != Some(s) {
                        last_stage = Some(s);
                        end_line(&mut line_pending);
                        last_pct = -1.0;
                        eprintln!("[{}]", s.label(lang()));
                    }
                }
                StageEvent::Progress {
                    done,
                    total,
                    current,
                    ..
                } => {
                    let pct = steadcopy_core::task::stage::percent(done, total);
                    let elapsed_ok = last_draw.elapsed() >= std::time::Duration::from_millis(100);
                    let moved_enough = (pct - last_pct).abs() >= 0.5;
                    if !(elapsed_ok || moved_enough) {
                        return;
                    }
                    last_draw = std::time::Instant::now();
                    last_pct = pct;
                    let Some(c) = current else { return };
                    if tty {
                        eprint!("\r  {pct:5.1}%  {c:<60}");
                        line_pending = true;
                    } else {
                        // 非终端：每次单独一行，不用 \r
                        eprintln!("  {pct:5.1}%  {c}");
                    }
                }
                StageEvent::FileFailed {
                    relative_path,
                    reason,
                } => {
                    end_line(&mut line_pending);
                    eprintln!("  ✗ {relative_path}：{reason}");
                }
                StageEvent::Notice(msg) => {
                    end_line(&mut line_pending);
                    eprintln!("  ⚠ {msg}");
                }
            }
        }
    }

    pub fn report(&self, r: &TaskReport) {
        let failures: Vec<FailureJson> = r
            .failed_files()
            .map(|f| FailureJson {
                path: f.relative_path.clone(),
                reason: match &f.status {
                    steadcopy_core::task::FileStatus::Failed(m) => m.clone(),
                    _ => String::new(),
                },
                retries: f.retries,
            })
            .collect();

        if self.json {
            self.out(&ReportJson {
                copied: r.copied_count(),
                skipped: r.skipped_count(),
                failed: failures.len(),
                bytes_copied: r.bytes_copied,
                cancelled: r.cancelled,
                manifests: r.manifests.iter().map(|p| p.display().to_string()).collect(),
                notices: r.notices.clone(),
                failures,
            });
            return;
        }

        eprintln!();
        if r.cancelled {
            println!("任务已取消");
        } else if failures.is_empty() {
            println!(
                "拷贝完成：{} 个文件 · {} · 全部校验通过",
                r.copied_count(),
                human_bytes(r.bytes_copied)
            );
        } else {
            // 有失败时 MUST NOT 用「完成」作为主表述
            println!(
                "部分失败：成功 {} 个，失败 {} 个",
                r.copied_count(),
                failures.len()
            );
        }
        if r.skipped_count() > 0 {
            println!("  已跳过 {} 个（此前已拷并校验通过）", r.skipped_count());
        }
        for f in &failures {
            println!("  ✗ {}（重试 {} 次）：{}", f.path, f.retries, f.reason);
        }
        for m in &r.manifests {
            println!("  凭证：{}", m.display());
        }
    }

    pub fn report_written(&self, path: &std::path::Path) {
        if self.json {
            #[derive(Serialize)]
            struct R<'a> {
                report: &'a str,
            }
            let p = path.display().to_string();
            self.out(&R { report: &p });
        } else {
            println!("  报告：{}", path.display());
        }
    }

    pub fn audit(&self, r: &AuditReport) {
        if self.json {
            self.out(r);
            return;
        }
        let c = r.counts();
        println!("复验结果（算法 {}）", r.algorithm);
        println!("  一致 {}   已移动 {}   丢失 {}   新增 {}", c.intact, c.moved, c.missing, c.added);
        if !r.complete {
            println!("  ⚠ 结果不完整（复验被中断）");
        }
        if r.unverified_at_copy > 0 {
            println!(
                "  ⚠ 其中 {} 个条目在拷贝时未做校验，可信度较低",
                r.unverified_at_copy
            );
        }
        for m in &r.missing {
            println!("  ✗ 丢失：{}（期望 {}）", m.relative_path, m.expected_hash);
        }
        for m in &r.moved {
            println!("  → 已移动：{} → {}", m.from, m.to);
        }
        for a in &r.added {
            println!("  + 新增：{}", a.relative_path);
        }
        if r.is_data_intact() {
            println!("  数据完好——清单记录的内容全部找得到");
        }
    }
}

/// 本次运行的语言。
///
/// 命令行是短命进程，启动时解析一次就够——**只写一次、之后只读**，
/// 不是可变全局状态。用 `OnceLock` 而不是 `static mut` 是为了让这一点在类型上成立。
static LANG: OnceLock<steadcopy_core::i18n::Locale> = OnceLock::new();

/// 启动时定一次语言。`--lang` 优先于配置。
pub fn set_lang(explicit: Option<&str>) {
    let setting = explicit.map(str::to_string).unwrap_or_else(|| {
        steadcopy_core::config::load()
            .map(|c| c.settings.locale.clone())
            .unwrap_or_else(|_| steadcopy_core::i18n::LOCALE_AUTO.to_string())
    });
    let _ = LANG.set(steadcopy_core::i18n::Locale::resolve(&setting));
}

pub fn lang() -> steadcopy_core::i18n::Locale {
    *LANG.get_or_init(|| steadcopy_core::i18n::Locale::resolve(steadcopy_core::i18n::LOCALE_AUTO))
}
