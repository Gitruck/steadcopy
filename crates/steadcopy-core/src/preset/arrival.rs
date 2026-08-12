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
use crate::i18n::Locale;
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

/// 一个未进入执行的结论该给用户什么出路。
///
/// **每一个「不能做」的结论都要带一个「那就这样做」。** 用户看到「没有匹配的预设」时
/// 需要的不是解释，是出路——把原因说清楚却不给下一步，等于把死路装修了一下。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextStep {
    /// 指认类型，或者「就拷这一次」
    ClassifyOrCopyOnce,
    /// 直接开一次临时拷贝
    CopyOnce,
    /// 换个目的地，或者「就拷这一次」拷去别处
    ChooseAnotherDestination,
    /// 去看上一次的报告（没新素材时，用户想确认的是「上次拷好了吗」）
    ViewLastReport,
    /// 确认后开跑
    ConfirmAndRun,
    /// 什么都不用做
    Nothing,
}

impl NextStep {
    /// 给按钮用的文案。措辞是动作，不是功能名。
    ///
    /// 穷尽 `match`：新增一个变体或一种语言，编译器会把这里点出来。
    pub const fn label(&self, lang: Locale) -> &'static str {
        match self {
            NextStep::ClassifyOrCopyOnce => lang.pick("指认类型，或就拷这一次", "Identify it, or copy just once"),
            NextStep::CopyOnce => lang.pick("就拷这一次", "Copy just once"),
            NextStep::ChooseAnotherDestination => lang.pick("换个目的地", "Choose another destination"),
            NextStep::ViewLastReport => lang.pick("看上次的报告", "View the last report"),
            NextStep::ConfirmAndRun => lang.pick("开始拷贝", "Start copying"),
            NextStep::Nothing => "",
        }
    }
}

impl ArrivalOutcome {
    /// 这个结论给用户留的出路。
    pub const fn next_step(&self) -> NextStep {
        match self {
            ArrivalOutcome::NeedsClassification { .. } => NextStep::ClassifyOrCopyOnce,
            // 已忽略是用户自己设的，不该反过来催他；已有任务在跑等着就行
            ArrivalOutcome::Ignored { .. } | ArrivalOutcome::AlreadyRunning { .. } => {
                NextStep::Nothing
            }
            // 这三种都是「配置没到位」，而配置不该是拷贝的前置条件
            ArrivalOutcome::NoPreset { .. }
            | ArrivalOutcome::NoProject { .. }
            | ArrivalOutcome::NoSource { .. } => NextStep::CopyOnce,
            ArrivalOutcome::NoNewSource { .. } => NextStep::ViewLastReport,
            ArrivalOutcome::InsufficientSpace { .. } => NextStep::ChooseAnotherDestination,
            ArrivalOutcome::Planned { .. } => NextStep::ConfirmAndRun,
        }
    }

    /// 这次到达是否需要用户做点什么（界面据此决定要不要弹东西）。
    pub fn needs_attention(&self) -> bool {
        !matches!(
            self,
            ArrivalOutcome::Ignored { .. } | ArrivalOutcome::AlreadyRunning { .. }
        )
    }

    /// 给用户看的一句话结论。
    ///
    /// 产的是**成句**而不是片段——中英语序差别很大（「X 的 Y 不足」vs "Not enough Y on X"），
    /// 交给消费层拼装等于把语序知识复制两份。
    pub fn summary(&self, lang: Locale) -> String {
        match self {
            ArrivalOutcome::NeedsClassification { suggested_name, .. } => match lang {
                Locale::Zh => format!("发现新设备「{suggested_name}」，请先指认它是什么"),
                Locale::En => format!("New device \"{suggested_name}\" — tell me what it is first"),
            },
            ArrivalOutcome::Ignored { .. } => lang
                .pick("该设备已被忽略", "This device is on the ignore list")
                .into(),
            ArrivalOutcome::AlreadyRunning { .. } => lang
                .pick("该设备上已有任务在进行", "A task is already running on this device")
                .into(),
            ArrivalOutcome::NoPreset { device_name, .. } => match lang {
                Locale::Zh => format!("「{device_name}」没有匹配的预设任务，去配一条或手动选参数"),
                Locale::En => {
                    format!("No preset matches \"{device_name}\" — set one up, or copy just once")
                }
            },
            ArrivalOutcome::NoProject { preset_name } => match lang {
                Locale::Zh => format!("预设「{preset_name}」还没有可用的项目，请先建一个项目"),
                Locale::En => format!("Preset \"{preset_name}\" has no usable project yet"),
            },
            ArrivalOutcome::NoSource { device_name } => match lang {
                Locale::Zh => format!("「{device_name}」上没有可拷贝的素材"),
                Locale::En => format!("Nothing to copy on \"{device_name}\""),
            },
            ArrivalOutcome::NoNewSource { device_name } => match lang {
                Locale::Zh => format!("「{device_name}」没有新素材，此前已拷并校验通过"),
                Locale::En => {
                    format!("Nothing new on \"{device_name}\" — already copied and verified")
                }
            },
            ArrivalOutcome::InsufficientSpace {
                device_name,
                landing_dir,
                ..
            } => match lang {
                Locale::Zh => format!("拷贝「{device_name}」的目的地空间不足：{landing_dir}"),
                Locale::En => {
                    format!("Not enough space to copy \"{device_name}\" to {landing_dir}")
                }
            },
            ArrivalOutcome::Planned {
                device_name,
                preset_name,
                requires_confirmation,
                ..
            } => match (lang, *requires_confirmation) {
                (Locale::Zh, true) => {
                    format!("「{device_name}」已按预设「{preset_name}」准备好，确认后开始")
                }
                (Locale::Zh, false) => format!("「{device_name}」已按预设「{preset_name}」自动开始"),
                (Locale::En, true) => {
                    format!("\"{device_name}\" is ready via preset \"{preset_name}\" — confirm to start")
                }
                (Locale::En, false) => {
                    format!("\"{device_name}\" started automatically via preset \"{preset_name}\"")
                }
            },
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
        eject_after: preset.eject_after || config.settings.eject_after,
        at: now,
    })
}
