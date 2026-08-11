//! 到达编排：设备插上之后，从「这是谁」一路走到「跑不跑」。
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/preset-autorun/spec.md`
//! → Requirement: 到达编排 / 未分类设备永不自动开跑 / 档位模型
//!
//! ```text
//! 1. 记忆库里有吗？  否 → 登记为未分类 + 请求指认   ← 停
//! 2. 类型是忽略吗？  是 → 静默结束                   ← 停
//! 3. 已有任务在跑？  是 → 不重复创建                 ← 停
//! 4. 匹配到预设吗？  否 → 提示用户去配               ← 停
//! 5. 规划（预演）    空间不足 → 报空间不足            ← 停
//! 6. 按档位：确认档弹卡片 / 无人值守档直接跑
//! ```
//!
//! 编排结果是**枚举**而不是 `Option<TaskSpec>`——每一种「没跑起来」的原因都要能被
//! 界面如实呈现。「插卡没反应」是这类工具最高频的困惑，把原因藏起来就是在制造它。

use time::OffsetDateTime;

use crate::config::model::{ArrivalMode, Config};
use crate::device::{DeviceKind, DeviceRecord, Volume};
use crate::manifest::model::SourceRef;
use crate::organize::ScanOptions;
use crate::platform::VolumeIo;
use crate::preset::matching::select_preset;
use crate::preset::model::Preset;
use crate::task::{plan_task, DestinationSpec, TaskPlan, TaskSpec};

/// 一次设备到达的编排结论。
#[derive(Debug)]
pub enum ArrivalOutcome {
    /// 从未见过：已登记为「未分类」，等用户指认类型。**此状态下绝不开跑。**
    NeedsClassification {
        device_id: String,
        suggested_name: String,
    },
    /// 已标记为忽略：静默结束，不打扰
    Ignored { device_id: String },
    /// 该设备上已有任务在跑，不重复创建
    AlreadyRunning { device_id: String },
    /// 没有任何预设匹配它
    NoPreset {
        device_id: String,
        device_name: String,
    },
    /// 预设指向的项目不存在，或根本还没建过项目
    NoProject { preset_name: String },
    /// 已规划好，但源上没素材
    NoSource { device_name: String },
    /// 已规划好，但没有新素材（此前已拷并校验通过）
    NoNewSource { device_name: String },
    /// 目的地空间不足
    InsufficientSpace {
        device_name: String,
        landing_dir: String,
        required_bytes: u64,
        available_bytes: Option<u64>,
    },
    /// 可以跑了。`requires_confirmation` 为真时**必须**等用户点一次
    Planned {
        device_name: String,
        preset_name: String,
        spec: Box<TaskSpec>,
        plan: Box<TaskPlan>,
        requires_confirmation: bool,
    },
}

impl ArrivalOutcome {
    /// 这次到达是否需要用户做点什么（界面据此决定要不要弹东西）。
    pub fn needs_attention(&self) -> bool {
        !matches!(
            self,
            ArrivalOutcome::Ignored { .. } | ArrivalOutcome::AlreadyRunning { .. }
        )
    }

    /// 给用户看的一句话结论。
    pub fn summary(&self) -> String {
        match self {
            ArrivalOutcome::NeedsClassification { suggested_name, .. } => {
                format!("发现新设备「{suggested_name}」，请先指认它是什么")
            }
            ArrivalOutcome::Ignored { .. } => "该设备已被忽略".into(),
            ArrivalOutcome::AlreadyRunning { .. } => "该设备上已有任务在进行".into(),
            ArrivalOutcome::NoPreset { device_name, .. } => {
                format!("「{device_name}」没有匹配的预设任务，去配一条或手动选参数")
            }
            ArrivalOutcome::NoProject { preset_name } => {
                format!("预设「{preset_name}」还没有可用的项目，请先建一个项目")
            }
            ArrivalOutcome::NoSource { device_name } => {
                format!("「{device_name}」上没有可拷贝的素材")
            }
            ArrivalOutcome::NoNewSource { device_name } => {
                format!("「{device_name}」没有新素材，此前已拷并校验通过")
            }
            ArrivalOutcome::InsufficientSpace {
                device_name,
                landing_dir,
                ..
            } => format!("拷贝「{device_name}」的目的地空间不足：{landing_dir}"),
            ArrivalOutcome::Planned {
                device_name,
                preset_name,
                requires_confirmation,
                ..
            } => {
                if *requires_confirmation {
                    format!("「{device_name}」已按预设「{preset_name}」准备好，确认后开始")
                } else {
                    format!("「{device_name}」已按预设「{preset_name}」自动开始")
                }
            }
        }
    }
}

/// 编排一次设备到达。
///
/// 会在 `config` 中登记首次见到的设备（这是「记住这张卡」的唯一入口）。
pub fn on_arrival(
    config: &mut Config,
    volume: &Volume,
    running_device_ids: &[String],
    io: &dyn VolumeIo,
    now: OffsetDateTime,
) -> ArrivalOutcome {
    let id = volume.composite_id();

    // ── 1. 记忆库里有吗 ────────────────────────────────────────────
    let known = config.device(&id).is_some();
    let label = if volume.label.trim().is_empty() {
        "未命名设备".to_string()
    } else {
        volume.label.trim().to_string()
    };
    let record = DeviceRecord::new(&id, &label, volume.total_bytes, now);
    config.remember_device(record);

    let device = match config.device(&id) {
        Some(d) => d.clone(),
        // 理论上不可达（刚刚登记过），但绝不 unwrap
        None => {
            return ArrivalOutcome::NeedsClassification {
                device_id: id,
                suggested_name: label,
            }
        }
    };

    if !known || !device.kind.is_classified() {
        // **铁律**：未分类设备永远停在这里。危险区的「跳过插卡确认」也绕不过去——
        // 拷贝是可恢复的，但往一个不知道是什么的设备上自动动手不是。
        return ArrivalOutcome::NeedsClassification {
            device_id: device.id.clone(),
            suggested_name: device.display_name(),
        };
    }

    // ── 2. 是否被忽略 ──────────────────────────────────────────────
    if device.kind == DeviceKind::Ignored {
        return ArrivalOutcome::Ignored {
            device_id: device.id,
        };
    }

    // ── 3. 是否已有任务在跑 ────────────────────────────────────────
    if running_device_ids.iter().any(|d| d == &device.id) {
        return ArrivalOutcome::AlreadyRunning {
            device_id: device.id,
        };
    }

    // ── 4. 匹配预设 ────────────────────────────────────────────────
    let Some(preset) = select_preset(&config.presets, &device).cloned() else {
        return ArrivalOutcome::NoPreset {
            device_id: device.id.clone(),
            device_name: device.display_name(),
        };
    };

    // ── 5. 规划（只读预演） ────────────────────────────────────────
    let Some(spec) = build_spec(config, &preset, &device, volume, now) else {
        return ArrivalOutcome::NoProject {
            preset_name: preset.name,
        };
    };

    let plan = match plan_task(&spec, io) {
        Ok(p) => p,
        Err(_) => {
            return ArrivalOutcome::NoSource {
                device_name: device.display_name(),
            }
        }
    };

    if plan.is_no_source() {
        return ArrivalOutcome::NoSource {
            device_name: device.display_name(),
        };
    }
    if plan.is_no_new_source() {
        return ArrivalOutcome::NoNewSource {
            device_name: device.display_name(),
        };
    }
    if let Some(d) = plan.insufficient().next() {
        return ArrivalOutcome::InsufficientSpace {
            device_name: device.display_name(),
            landing_dir: d.landing_dir.display().to_string(),
            required_bytes: d.required_bytes,
            available_bytes: d.available_bytes,
        };
    }

    // ── 6. 按档位决定 ──────────────────────────────────────────────
    let requires_confirmation = !matches!(config.settings.arrival_mode(), ArrivalMode::Unattended);

    ArrivalOutcome::Planned {
        device_name: device.display_name(),
        preset_name: preset.name.clone(),
        spec: Box::new(spec),
        plan: Box::new(plan),
        requires_confirmation,
    }
}

/// 由「预设 + 设备 + 卷」拼出任务规格。项目不可用时返回 `None`。
pub fn build_spec(
    config: &Config,
    preset: &Preset,
    device: &DeviceRecord,
    volume: &Volume,
    now: OffsetDateTime,
) -> Option<TaskSpec> {
    let project = match &preset.project_id {
        Some(id) => config.project(id)?,
        None => config.effective_project()?,
    };

    let mut destinations = Vec::new();
    for d in project.enabled_destinations() {
        // 模板在保存时已校验过；这里再失败只可能是配置被外部改坏，跳过该目的地
        let Ok(template) = d.parsed_template() else {
            continue;
        };
        destinations.push(DestinationSpec {
            root: d.root.clone(),
            template,
            enabled: true,
        });
    }
    if destinations.is_empty() {
        return None;
    }

    Some(TaskSpec {
        source_root: volume.root_path(),
        source: SourceRef {
            id: device.id.clone(),
            display_name: device.display_name(),
        },
        project: project.name.clone(),
        destinations,
        algorithm: preset.algorithm,
        verify: preset.verify,
        scan: ScanOptions::mirror(),
        retries: config.settings.retries,
        at: now,
    })
}
