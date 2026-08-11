//! `steadcopy format`：格式化源卡。**默认拒绝执行。**
//!
//! 规范：`openspec/changes/add-steadcopy-format-card/specs/format-card/spec.md`
//! → Requirement: 命令行的额外防线
//!
//! 命令行会被脚本调用，脚本不会看确认对话框。所以危险参数做得又长又难打，
//! 是刻意的摩擦；而且 G1–G4 一个都不能绕。

use std::io::{IsTerminal, Write};

use steadcopy_core::config::{self, model::new_id};
use steadcopy_core::device::{
    check_safety, enumerate_volumes, formatter, validate_countdown, BackupEvidence, Volume,
};
use steadcopy_core::ledger::{HistoryQuery, Ledger};
use steadcopy_core::manifest::{load_manifests, Manifest};
use steadcopy_core::organize::{scan_source, ScanOptions};
use steadcopy_core::platform::{Clock, SystemClock};

use crate::output::{Emitter, ExitKind};

/// 危险确认参数的字面量。刻意冗长。
pub const DANGER_FLAG: &str = "--yes-i-know-this-erases-data";

pub fn run(
    out: &mut Emitter,
    target: &str,
    confirmed: bool,
    countdown: Option<u32>,
) -> Result<ExitKind, String> {
    if !confirmed {
        return Err(format!(
            "格式化会**永久抹掉**卡上的全部数据，默认不执行。\n\
             确实要格，请显式加上 {DANGER_FLAG}"
        ));
    }

    let cfg = config::load().map_err(|e| e.to_string())?;
    let secs = validate_countdown(countdown.unwrap_or(cfg.settings.countdown_secs))?;

    let vol = find_volume(target)?;
    let dest_roots: Vec<std::path::PathBuf> = cfg
        .projects
        .iter()
        .flat_map(|p| p.destinations.iter().map(|d| d.root.clone()))
        .collect();

    let device_id = vol.composite_id();
    let device_name = cfg
        .device(&device_id)
        .map(|d| d.display_name())
        .unwrap_or_else(|| vol.display_name());

    let ledger = Ledger::open_default().map_err(|e| e.to_string())?;
    let now0 = SystemClock.now();

    // **先跑便宜的 G1–G3。** 扫描整卷是昂贵操作（对着系统盘能跑到天荒地老），
    // 在确认目标合法之前绝不做——顺序错了不只是慢，是拿危险目标当正常目标对待。
    let cheap = check_safety(&vol, &dest_roots, false, None, &[]);
    if let Some(f) = cheap
        .checks
        .iter()
        .find(|c| !c.passed && c.id != "G4")
    {
        out.safety(&cheap, &device_name);
        let reason = format!("{}：{}", f.id, f.detail);
        let _ = ledger.record_format_attempt(
            &new_id("fmt"), now0, &device_id, &device_name, "cli",
            &cheap.compact(), None, "rejected", Some(&reason),
        );
        return Err(format!("格式化被拒绝——{reason}"));
    }

    // G1–G3 都过了，才值得花时间扫卡内容供 G4 判定
    let current: Vec<String> = scan_source(&vol.root_path(), &ScanOptions::mirror())
        .files
        .into_iter()
        .map(|f| f.relative_path)
        .collect();
    let evidence = find_backup_evidence(&ledger, &device_id);
    let report = check_safety(&vol, &dest_roots, false, evidence.as_ref(), &current);
    out.safety(&report, &device_name);

    let attempt_id = new_id("fmt");
    let now = SystemClock.now();

    if !report.passed() {
        let reason = report
            .first_failure()
            .map(|c| format!("{}：{}", c.id, c.detail))
            .unwrap_or_default();
        // 被拒的尝试同样留痕
        let _ = ledger.record_format_attempt(
            &attempt_id, now, &device_id, &device_name, "cli",
            &report.compact(), report.backup_task_id.as_deref(), "rejected", Some(&reason),
        );
        return Err(format!("格式化被拒绝——{reason}"));
    }

    // 倒计时 + 输卷标：三重确认里的后两重
    if !confirm_interactively(out, &vol, secs)? {
        let _ = ledger.record_format_attempt(
            &attempt_id, now, &device_id, &device_name, "cli",
            &report.compact(), report.backup_task_id.as_deref(), "cancelled", None,
        );
        out.note("已取消，未做任何改动");
        return Ok(ExitKind::Cancelled);
    }

    let f = formatter();
    let root = vol.root_path().display().to_string();
    let params = f.read_params(&root).map_err(|e| e.to_string())?;
    out.note(&format!(
        "正在格式化 {}（保留 {} 与卷标「{}」）…",
        root, params.file_system, params.label
    ));

    match f.quick_format(&params) {
        Ok(()) => {
            let _ = ledger.record_format_attempt(
                &attempt_id, SystemClock.now(), &device_id, &device_name, "cli",
                &report.compact(), report.backup_task_id.as_deref(), "ok", None,
            );
            out.note("格式化完成");
            Ok(ExitKind::Ok)
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = ledger.record_format_attempt(
                &attempt_id, SystemClock.now(), &device_id, &device_name, "cli",
                &report.compact(), report.backup_task_id.as_deref(), "failed", Some(&msg),
            );
            Err(format!("格式化失败：{msg}"))
        }
    }
}

fn find_volume(target: &str) -> Result<Volume, String> {
    let vols = enumerate_volumes().map_err(|e| e.to_string())?;
    vols.into_iter()
        .find(|v| {
            v.guid_path.eq_ignore_ascii_case(target)
                || v.drive_letter
                    .as_deref()
                    .map(str::to_ascii_uppercase)
                    .as_deref()
                    == Some(&target.to_ascii_uppercase())
        })
        .ok_or_else(|| format!("找不到这个卷：{target}"))
}

/// 找该设备最近一次「完成且全部校验通过」的备份作为 G4 依据。
fn find_backup_evidence(ledger: &Ledger, device_id: &str) -> Option<BackupEvidence> {
    let tasks = ledger
        .history(&HistoryQuery {
            source_id: Some(device_id.to_string()),
            limit: Some(20),
            ..Default::default()
        })
        .ok()?;

    for t in tasks {
        if t.status != steadcopy_core::ledger::TaskStatus::Ok || !t.verified {
            continue;
        }
        // 合并该次任务落下的全部清单
        let mut merged: Option<Manifest> = None;
        for mp in &t.manifests {
            let landing = std::path::Path::new(mp).parent().and_then(|p| p.parent());
            let Some(landing) = landing else { continue };
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

/// 倒计时 + 手输卷标。
fn confirm_interactively(out: &mut Emitter, vol: &Volume, secs: u32) -> Result<bool, String> {
    if !std::io::stdin().is_terminal() {
        // 非交互环境不能假装用户确认过
        return Err("当前不是交互终端，无法完成格式化确认。请在界面里操作".into());
    }

    out.warn(&format!(
        "即将格式化「{}」（{}，{}）。此操作**不可撤销**。",
        vol.display_name(),
        vol.file_system,
        vol.label
    ));
    eprint!("请输入这张卡的卷标以确认（当前卷标：{}）：", vol.label);
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if line.trim() != vol.label.trim() {
        out.note("卷标不匹配");
        return Ok(false);
    }

    // 冷静期：倒计时期间可以 Ctrl-C 退出
    for left in (1..=secs).rev() {
        eprint!("\r{left} 秒后开始格式化，现在按 Ctrl-C 还来得及…   ");
        let _ = std::io::stderr().flush();
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    eprintln!();
    Ok(true)
}
