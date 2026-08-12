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
    check_safety, confirmation_phrase, enumerate_volumes, formatter, label_matches,
    validate_countdown, BackupEvidence, Volume,
};
use steadcopy_core::ledger::{HistoryQuery, Ledger};
use steadcopy_core::manifest::{load_manifests, Manifest};
use steadcopy_core::organize::{scan_source, ScanOptions};
use steadcopy_core::platform::{Clock, SystemClock};

use crate::output::{lang, w, Emitter, ExitKind};

/// 危险确认参数的字面量。刻意冗长。
pub const DANGER_FLAG: &str = "--yes-i-know-this-erases-data";

pub fn run(
    out: &mut Emitter,
    target: &str,
    confirmed: bool,
    countdown: Option<u32>,
) -> Result<ExitKind, String> {
    if !confirmed {
        return Err(wf!(
            "格式化会**永久抹掉**卡上的全部数据，默认不执行。\n\
             确实要格，请显式加上 {}",
            "Formatting **permanently erases** everything on the card, so it does not run by \
             default.\n\
             If you really mean it, pass {} explicitly",
            DANGER_FLAG
        ));
    }

    let lang = lang();
    let cfg = config::load().map_err(|e| e.describe(lang))?;
    let secs = validate_countdown(countdown.unwrap_or(cfg.settings.countdown_secs), lang)?;

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

    let ledger = Ledger::open_default().map_err(|e| e.describe(lang))?;
    let now0 = SystemClock.now();

    // **先跑便宜的 G1–G3。** 扫描整卷是昂贵操作（对着系统盘能跑到天荒地老），
    // 在确认目标合法之前绝不做——顺序错了不只是慢，是拿危险目标当正常目标对待。
    let cheap = check_safety(&vol, &dest_roots, false, None, &[], lang);
    if let Some(f) = cheap
        .checks
        .iter()
        .find(|c| !c.passed && c.id != "G4")
    {
        out.safety(&cheap, &device_name);
        let reason = format!("{} {}", f.id, f.detail);
        let _ = ledger.record_format_attempt(
            &new_id("fmt"), now0, &device_id, &device_name, "cli",
            &cheap.compact(), None, "rejected", Some(&reason),
        );
        return Err(rejected(&reason));
    }

    // G1–G3 都过了，才值得花时间扫卡内容供 G4 判定
    let current: Vec<String> = scan_source(&vol.root_path(), &ScanOptions::mirror())
        .files
        .into_iter()
        .map(|f| f.relative_path)
        .collect();
    let evidence = find_backup_evidence(&ledger, &device_id);
    let report = check_safety(&vol, &dest_roots, false, evidence.as_ref(), &current, lang);
    out.safety(&report, &device_name);

    let attempt_id = new_id("fmt");
    let now = SystemClock.now();

    if !report.passed() {
        let reason = report
            .first_failure()
            .map(|c| format!("{} {}", c.id, c.detail))
            .unwrap_or_else(|| {
                w(
                    "安全检查未通过（未能定位到具体是哪一项）",
                    "A safety check did not pass (could not tell which one)",
                )
                .to_string()
            });
        // 被拒的尝试同样留痕
        let _ = ledger.record_format_attempt(
            &attempt_id, now, &device_id, &device_name, "cli",
            &report.compact(), report.backup_task_id.as_deref(), "rejected", Some(&reason),
        );
        return Err(rejected(&reason));
    }

    // 倒计时 + 输卷标：三重确认里的后两重
    if !confirm_interactively(out, &vol, secs)? {
        let _ = ledger.record_format_attempt(
            &attempt_id, now, &device_id, &device_name, "cli",
            &report.compact(), report.backup_task_id.as_deref(), "cancelled", None,
        );
        out.note(w("已取消，未做任何改动", "Cancelled — nothing was changed"));
        return Ok(ExitKind::Cancelled);
    }

    let f = formatter();
    let root = vol.root_path().display().to_string();
    let params = f.read_params(&root).map_err(|e| e.describe(lang))?;
    out.note(&wf!(
        "正在格式化 {}（保留 {} 与卷标「{}」）…",
        "Formatting {} (keeping {} and the label \"{}\")...",
        root,
        params.file_system,
        params.label
    ));

    match f.quick_format(&params) {
        Ok(()) => {
            let _ = ledger.record_format_attempt(
                &attempt_id, SystemClock.now(), &device_id, &device_name, "cli",
                &report.compact(), report.backup_task_id.as_deref(), "ok", None,
            );
            out.note(w("格式化完成", "Format complete"));
            Ok(ExitKind::Ok)
        }
        Err(e) => {
            let msg = e.describe(lang);
            let _ = ledger.record_format_attempt(
                &attempt_id, SystemClock.now(), &device_id, &device_name, "cli",
                &report.compact(), report.backup_task_id.as_deref(), "failed", Some(&msg),
            );
            Err(wf!("格式化失败：{}", "Format failed: {}", msg))
        }
    }
}

/// 被拒时的统一表述。**理由一定要指出是哪一道闸门**——
/// 只说「被拒绝」，用户下一步无从下手。
fn rejected(reason: &str) -> String {
    wf!("格式化被拒绝——{}", "Format refused — {}", reason)
}

fn find_volume(target: &str) -> Result<Volume, String> {
    let vols = enumerate_volumes().map_err(|e| e.describe(lang()))?;
    vols.into_iter()
        .find(|v| {
            v.guid_path.eq_ignore_ascii_case(target)
                || v.drive_letter
                    .as_deref()
                    .map(str::to_ascii_uppercase)
                    .as_deref()
                    == Some(&target.to_ascii_uppercase())
        })
        .ok_or_else(|| wf!("找不到这个卷：{}", "No such volume: {}", target))
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
        return Err(w(
            "当前不是交互终端，无法完成格式化确认。请在界面里操作",
            "This is not an interactive terminal, so the format cannot be confirmed. \
             Do it in the app",
        )
        .into());
    }

    out.warn(&wf!(
        "即将格式化「{}」（{}，{}）。此操作**不可撤销**。",
        "About to format \"{}\" ({}, {}). This **cannot be undone**.",
        vol.display_name(),
        vol.file_system,
        vol.label
    ));
    // 无卷标的卡用固定词，否则「输入卷标」会退化成直接回车。
    // 这个词**不随语言变**（它是 label_matches 的判据），英文提示里也原样摆出来给人照抄
    let phrase = confirmation_phrase(&vol.label);
    eprint!(
        "{}",
        wf!("请输入「{}」以确认：", "Type \"{}\" to confirm: ", phrase)
    );
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if !label_matches(&line, &vol.label) {
        out.note(w("卷标不匹配", "The label does not match"));
        return Ok(false);
    }

    // 冷静期：倒计时期间可以 Ctrl-C 退出
    for left in (1..=secs).rev() {
        eprint!(
            "\r{}   ",
            wf!(
                "{} 秒后开始格式化，现在按 Ctrl-C 还来得及…",
                "Formatting starts in {}s — Ctrl-C still works",
                left
            )
        );
        let _ = std::io::stderr().flush();
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    eprintln!();
    Ok(true)
}
