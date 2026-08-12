//! `steadcopy watch`：插卡即跑的前台守候。
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/preset-autorun/spec.md`
//!
//! 这是「插卡自动备份」这条产品主张在命令行上的完整形态：
//! 事件驱动等设备 → 认出是谁 → 匹配预设 → 规划 → 按档位确认或直接跑。

use std::io::{IsTerminal, Write};

use steadcopy_core::config::{self, model::ArrivalMode, Config};
use steadcopy_core::device::{device_watcher, enumerate_volumes, DeviceEvent, Volume};
use steadcopy_core::engine::CancelToken;
use steadcopy_core::platform::{volume_io, Clock, SystemClock};
use steadcopy_core::preset::{on_arrival, ArrivalOutcome};
use steadcopy_core::task::run_task;

use crate::output::{human_bytes, Emitter, ExitKind};

/// 把一个普通目录伪装成到达的源设备。
///
/// 规范：`specs/cli-driver/spec.md` → Requirement: 设备模拟入口（测试专用）
///
/// **只影响设备来源**：拷贝、校验、manifest、账本全部走真实逻辑，一个都不绕过。
/// 该入口不在 GUI 中暴露。
fn simulated_volume(path: &std::path::Path) -> Result<Volume, String> {
    if !path.is_dir() {
        return Err(format!("模拟设备目录不存在：{}", path.display()));
    }
    let label = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "模拟设备".into());
    // 复合身份要稳定——同一目录多次模拟必须被认成同一个设备
    let serial = label
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    Ok(Volume {
        guid_path: path.display().to_string(),
        drive_letter: None,
        label,
        serial: Some(serial),
        file_system: "SIMULATED".into(),
        total_bytes: 0,
        free_bytes: 0,
        bus_type: steadcopy_core::device::BusType::Usb,
        is_system: false,
        state: steadcopy_core::device::VolumeState::Online,
        fingerprints: steadcopy_core::organize::detect_fingerprints(path),
    })
}

/// 前台守候。返回时表示监听结束（Ctrl-C 或错误）。
pub fn run(
    out: &mut Emitter,
    once: bool,
    yes: bool,
    simulate: Option<&std::path::Path>,
) -> Result<ExitKind, String> {
    let mut cfg = config::load().map_err(|e| e.to_string())?;
    guard_config(&cfg, out)?;

    let io = volume_io();
    let clock = SystemClock;

    // 模拟入口：不起监听，直接把指定目录当作一次到达处理
    if let Some(dir) = simulate {
        let vol = simulated_volume(dir)?;
        handle(&mut cfg, &vol, io.as_ref(), &clock, out, yes)?;
        return Ok(ExitKind::Ok);
    }

    let mut watcher = device_watcher();
    let rx = watcher.subscribe().map_err(|e| e.to_string())?;

    out.watch_ready(&cfg, &config::config_path().display().to_string());

    // 启动时先把已经插着的设备过一遍——用户可能先插卡后开程序
    // 枚举失败不能静默变成「没有设备」——那会让 watch 看起来在跑其实什么也收不到
    let initial = enumerate_volumes().unwrap_or_else(|e| {
        out.warn(&format!("枚举卷失败：{e}"));
        Vec::new()
    });
    for vol in initial {
        if vol.can_be_source(&[]) {
            handle(&mut cfg, &vol, io.as_ref(), &clock, out, yes)?;
        }
    }

    for event in rx {
        let DeviceEvent::Arrived { drive_letter } = event else {
            continue; // 移除事件在这里不需要处理——正在跑的任务由引擎自己感知
        };
        // 到达时盘符可能尚未分配完毕，退避重试后再枚举
        let Some(vol) = resolve_volume(drive_letter.as_deref()) else {
            continue;
        };
        if !vol.can_be_source(&[]) {
            continue;
        }
        handle(&mut cfg, &vol, io.as_ref(), &clock, out, yes)?;
        if once {
            break;
        }
    }
    Ok(ExitKind::Ok)
}

/// 卷到达时盘符可能尚未分配，做有限次退避重试。
fn resolve_volume(letter: Option<&str>) -> Option<Volume> {
    for delay_ms in [0u64, 150, 400, 900] {
        if delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }
        // 这一轮枚举失败就换下一轮退避重试；四轮都失败会走到函数末尾的 None，
        // 调用方据此报「设备暂不可用」——不会静默当成「没有这个卷」
        let vols = match enumerate_volumes() {
            Ok(v) => v,
            Err(e) => {
                tracing::info!("枚举卷失败（{e}），退避后重试");
                continue;
            }
        };
        let found = match letter {
            Some(l) => vols
                .into_iter()
                .find(|v| v.drive_letter.as_deref() == Some(l)),
            // 无盘符的卷：取第一个可作为源且尚未见过的
            None => vols.into_iter().find(|v| v.drive_letter.is_none() && v.can_be_source(&[])),
        };
        if let Some(v) = found {
            return Some(v);
        }
    }
    None
}

fn handle(
    cfg: &mut Config,
    vol: &Volume,
    io: &dyn steadcopy_core::platform::VolumeIo,
    clock: &dyn Clock,
    out: &mut Emitter,
    yes: bool,
) -> Result<(), String> {
    let outcome = on_arrival(cfg, vol, &[], io, clock.now());

    // 首次见到的设备已被登记，配置要落盘，否则下次又是新设备
    if let Err(e) = config::save(cfg) {
        out.error(&format!("配置保存失败：{e}"));
    }

    if !outcome.needs_attention() {
        return Ok(());
    }
    out.arrival(&outcome);

    let ArrivalOutcome::Planned {
        spec,
        plan,
        requires_confirmation,
        ..
    } = outcome
    else {
        return Ok(());
    };

    // 档位：确认档要点一次；`--yes` 或无人值守档直接跑
    if requires_confirmation && !yes && !confirm(&plan)? {
        out.note("已跳过本次拷贝");
        return Ok(());
    }

    let cancel = CancelToken::new();
    let report = {
        let mut sink = out.progress_sink();
        run_task(&spec, &plan, io, clock, &cancel, &mut sink).map_err(|e| e.to_string())?
    };
    out.report(&report);
    Ok(())
}

fn confirm(plan: &steadcopy_core::task::TaskPlan) -> Result<bool, String> {
    if !std::io::stdin().is_terminal() {
        // 非交互环境（脚本/服务）不能假装用户点了确认
        return Err("当前不是交互终端，无法确认。用 --yes 显式授权，或在界面里操作".into());
    }
    eprint!(
        "\n即将拷贝 {} 个文件 · {}。开始吗？[y/N] ",
        plan.files.len(),
        human_bytes(plan.total_bytes())
    );
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

/// 守候前的配置自检——没配好就直说，别让用户插了卡才发现没反应。
fn guard_config(cfg: &Config, out: &mut Emitter) -> Result<(), String> {
    if cfg.projects.is_empty() {
        return Err("还没有任何项目。先用 `steadcopy project add` 建一个，再来守候".into());
    }
    if cfg.presets.iter().filter(|p| p.enabled).count() == 0 {
        return Err("还没有启用的预设任务。先用 `steadcopy preset add` 配一条".into());
    }
    if cfg.settings.arrival_mode() == ArrivalMode::Unattended {
        out.warn("危险区「跳过插卡确认」已开启：插卡将直接开始拷贝，不再询问");
    }
    Ok(())
}
