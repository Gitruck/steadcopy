//! 临时拷贝：不依赖预设的一次性任务。
//!
//! 规范：`openspec/changes/add-steadcopy-copy-first-flow/specs/adhoc-copy/spec.md`
//!
//! # 它存在的理由
//!
//! 预设是**加速器，不是通行证**。第一次用的人没有预设，别人给的一张卡不值得为它建预设，
//! 「这次想拷去另一块盘」也不该逼人改预设。所以要有一条路：用户当场把参数说清楚，跑一次，
//! 什么都不留下。
//!
//! # 它不是什么
//!
//! **不是简化版拷贝。** 产出的 `TaskSpec` 与预设路径产出的结构完全相同，下游无从分辨来源——
//! 这是刻意的：下游能分辨来源，迟早会有人为「临时」写一条捷径，而捷径总是从跳过校验开始。
//!
//! 「临时」只描述参数的来源与去留，不影响任何数据安全属性。

use std::path::PathBuf;

use time::OffsetDateTime;

use crate::config::model::{Config, DestinationConfig, Project};
use crate::engine::HashAlgorithm;
use crate::i18n::Locale;
use crate::manifest::model::SourceRef;
use crate::organize::{ScanOptions, TemplateError};
use crate::task::plan::{DestinationSpec, TaskSpec};

/// 一个项目都没有时预填的名字。用户可以直接过，也可以改。
pub const DEFAULT_PROJECT_NAME: &str = "我的素材";

/// 项目选择。
///
/// 是枚举而不是 `Option<String>`：**「不填」不是「没有项目」，是「现建一个」。**
/// 把这个区别做进类型里，就不可能出现「拷完了但不知道拷进了哪个项目」的状态——
/// 每一次拷贝都归属于某个项目，只是项目可能是刚刚为它建的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectChoice {
    /// 沿用已有项目
    Existing(String),
    /// 现建一个
    Create {
        name: String,
        destinations: Vec<PathBuf>,
    },
}

/// 用户当场指定的一次性拷贝请求。
///
/// 除了目的地，字段全部可以缺省——**目的地是唯一别人替不了的输入**，
/// 拷到哪儿只有用户知道。「零配置可完成一次拷贝」的准确含义是这个，
/// 不是「不需要任何输入」。
#[derive(Debug, Clone)]
pub struct AdhocRequest {
    pub source_root: PathBuf,
    pub device_id: String,
    pub device_name: String,
    pub project: ProjectChoice,
    /// 覆盖项目里的目的地。空表示「用项目里启用的全部目的地」
    pub destinations: Vec<PathBuf>,
    pub verify: Option<bool>,
    pub algorithm: Option<HashAlgorithm>,
    pub eject_after: bool,
    /// 目的地模板覆盖。`None` 沿用各目的地自己配的模板；
    /// `Some` 时**所有**启用目的地统一用这一串（按导图节点路径的宽松口径解析，
    /// 见 [`crate::organize::PathTemplate::parse_map_path`]）。
    ///
    /// 为导图派发而设：节点在树里的路径就是模板。放在这里而不是让导图自己拼
    /// `TaskSpec`，是因为「正在跑的设备拒绝」「校验不可跳过」这些不变量都长在
    /// 这条构造路上——导图想绕开这条路，就得先绕开这些不变量，所以不给第二条路。
    pub template_override: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdhocError {
    /// 该设备上已有任务在跑。与来源无关——并发保护不因「是手动点的」而放宽
    AlreadyRunning { device_name: String },
    /// 一个目的地都没有。这是唯一的硬性输入
    NoDestination,
    /// 指定的项目不存在（配置被外部改过）
    ProjectMissing { id: String },
    /// 目的地的路径模板不合法。
    ///
    /// 带的是 [`TemplateError`] 本体而不是它的中文串——把上游错误在这里先渲染成字，
    /// 等于把语言在这一层定死，下游再想换语言就只剩一份中文了。
    BadTemplate {
        root: PathBuf,
        reason: TemplateError,
    },
}

impl AdhocError {
    /// 给用户看的一句话，跟随语言。
    pub fn describe(&self, lang: Locale) -> String {
        match (self, lang) {
            (AdhocError::AlreadyRunning { device_name }, Locale::Zh) => {
                format!("「{device_name}」上已有任务在进行，等它跑完再来")
            }
            (AdhocError::AlreadyRunning { device_name }, Locale::En) => {
                format!("A task is already running on \"{device_name}\" — wait for it to finish")
            }
            (AdhocError::NoDestination, Locale::Zh) => "至少要选一个拷到哪儿的目的地".into(),
            (AdhocError::NoDestination, Locale::En) => {
                "Pick at least one destination to copy to".into()
            }
            (AdhocError::ProjectMissing { id }, Locale::Zh) => format!("找不到这个项目：{id}"),
            (AdhocError::ProjectMissing { id }, Locale::En) => format!("No such project: {id}"),
            (AdhocError::BadTemplate { root, reason }, Locale::Zh) => format!(
                "目的地 {} 的路径模板不合法：{}",
                root.display(),
                reason.describe(lang)
            ),
            (AdhocError::BadTemplate { root, reason }, Locale::En) => format!(
                "The path template for destination {} is not valid: {}",
                root.display(),
                reason.describe(lang)
            ),
        }
    }
}

impl std::fmt::Display for AdhocError {
    /// `Display` 恒为中文，理由同 [`crate::error::CoreError`]。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe(Locale::Zh))
    }
}

impl std::error::Error for AdhocError {}

/// 界面打开临时拷贝面板时的预填值。
///
/// **每个字段都有一个能直接用的默认值**——这是主理人拍板「项目出现但不强制」的落点：
/// 不强制 ≠ 可以为空，而是「有默认值可以一路回车过去」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdhocDefaults {
    pub project: ProjectChoice,
    /// 该项目里已启用的目的地。为空说明用户必须现选
    pub destinations: Vec<PathBuf>,
    pub verify: bool,
    pub algorithm: HashAlgorithm,
    /// 项目字段旁边要不要提示「会自动建这个项目」
    pub project_will_be_created: bool,
}

/// 算出临时拷贝面板的预填值。
pub fn adhoc_defaults(config: &Config) -> AdhocDefaults {
    match config.effective_project() {
        Some(p) => AdhocDefaults {
            project: ProjectChoice::Existing(p.id.clone()),
            destinations: p.enabled_destinations().map(|d| d.root.clone()).collect(),
            verify: config.settings.verify_default,
            algorithm: config.settings.algorithm,
            project_will_be_created: false,
        },
        // 一个项目都没有：预填一个名字并说明会自动建，而不是让用户先去建项目
        None => AdhocDefaults {
            project: ProjectChoice::Create {
                name: DEFAULT_PROJECT_NAME.to_string(),
                destinations: Vec::new(),
            },
            destinations: Vec::new(),
            verify: config.settings.verify_default,
            algorithm: config.settings.algorithm,
            project_will_be_created: true,
        },
    }
}

/// 由临时请求拼出任务规格。
///
/// 第二个返回值是**需要新建的项目**：`ProjectChoice::Create` 时非空。
/// 它由调用方在用户按下开始时才落盘——**规划阶段零副作用**这条对临时路径同样成立，
/// 预演一下就在配置里多出个项目，是很难解释的副作用。
pub fn build_adhoc_spec(
    config: &Config,
    req: &AdhocRequest,
    running_device_ids: &[String],
    now: OffsetDateTime,
) -> Result<(TaskSpec, Option<Project>), AdhocError> {
    if running_device_ids.iter().any(|d| d == &req.device_id) {
        return Err(AdhocError::AlreadyRunning {
            device_name: req.device_name.clone(),
        });
    }

    // 项目：沿用或现建。两条路产出同一种 Project，下游分不出来
    let (project_name, project_dests, pending) = match &req.project {
        ProjectChoice::Existing(id) => {
            let p = config
                .project(id)
                .ok_or_else(|| AdhocError::ProjectMissing { id: id.clone() })?;
            let dests: Vec<DestinationConfig> = p.enabled_destinations().cloned().collect();
            (p.name.clone(), dests, None)
        }
        ProjectChoice::Create { name, destinations } => {
            let mut p = Project::new(name.clone(), now);
            let roots = if destinations.is_empty() {
                &req.destinations
            } else {
                destinations
            };
            p.destinations = roots.iter().map(DestinationConfig::new).collect();
            let dests = p.destinations.clone();
            (p.name.clone(), dests, Some(p))
        }
    };

    // 请求里显式给了目的地就以它为准，否则用项目里的
    let chosen: Vec<DestinationConfig> = if req.destinations.is_empty() {
        project_dests
    } else {
        req.destinations
            .iter()
            .map(|root| {
                // 目的地已在项目里配过就沿用它的模板，否则用默认模板
                project_dests
                    .iter()
                    .find(|d| d.root == *root)
                    .cloned()
                    .unwrap_or_else(|| DestinationConfig::new(root))
            })
            .collect()
    };

    if chosen.is_empty() {
        return Err(AdhocError::NoDestination);
    }

    let mut destinations = Vec::with_capacity(chosen.len());
    for d in &chosen {
        // 模板不合法就明说是哪个目的地，别让用户对着一句「配置错误」猜
        let template = match &req.template_override {
            Some(raw) => crate::organize::PathTemplate::parse_map_path(raw),
            None => d.parsed_template(),
        }
        .map_err(|e| AdhocError::BadTemplate {
            root: d.root.clone(),
            reason: e,
        })?;
        destinations.push(DestinationSpec {
            root: d.root.clone(),
            template,
            enabled: true,
        });
    }

    let spec = TaskSpec {
        source_root: req.source_root.clone(),
        source: SourceRef {
            id: req.device_id.clone(),
            display_name: req.device_name.clone(),
        },
        project: project_name,
        destinations,
        algorithm: req.algorithm.unwrap_or(config.settings.algorithm),
        verify: req.verify.unwrap_or(config.settings.verify_default),
        scan: ScanOptions::mirror(),
        retries: config.settings.retries,
        eject_after: req.eject_after,
        at: now,
    };

    Ok((spec, pending))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::DestinationConfig;
    use crate::device::{DeviceKind, DeviceRecord};
    use time::macros::datetime;

    fn at() -> OffsetDateTime {
        datetime!(2026-08-11 09:00:00 UTC)
    }

    fn empty_config() -> Config {
        Config::default()
    }

    fn config_with_project() -> Config {
        let mut c = Config::default();
        let mut p = Project::new("婚礼", at());
        p.destinations.push(DestinationConfig::new(r"D:\素材"));
        p.destinations.push(DestinationConfig::new(r"F:\备份"));
        c.current_project = Some(p.id.clone());
        c.projects.push(p);
        c
    }

    fn req(project: ProjectChoice, dests: Vec<PathBuf>) -> AdhocRequest {
        AdhocRequest {
            source_root: PathBuf::from(r"E:\"),
            device_id: "vol:1".into(),
            device_name: "A7M4".into(),
            project,
            destinations: dests,
            verify: None,
            algorithm: None,
            eject_after: false,
            template_override: None,
        }
    }

    // spec: → Scenario: 无预设也能拷
    #[test]
    fn scenario_adhoc_copy_runs_without_any_preset() {
        let c = config_with_project();
        assert!(c.presets.is_empty(), "这个测试的前提就是没有预设");
        let pid = c.projects[0].id.clone();
        let (spec, pending) =
            build_adhoc_spec(&c, &req(ProjectChoice::Existing(pid), vec![]), &[], at())
                .expect("没有预设也该拷得成");
        assert_eq!(spec.destinations.len(), 2);
        assert_eq!(spec.project, "婚礼");
        assert!(pending.is_none(), "沿用已有项目就不该产出待建项目");
    }

    // spec: → Scenario: 目的地是唯一必填
    #[test]
    fn scenario_adhoc_copy_only_destination_is_required() {
        let c = empty_config();
        // 只给目的地，项目名用默认、校验算法都不给
        let r = req(
            ProjectChoice::Create {
                name: DEFAULT_PROJECT_NAME.into(),
                destinations: vec![],
            },
            vec![PathBuf::from(r"D:\素材")],
        );
        let (spec, pending) = build_adhoc_spec(&c, &r, &[], at()).expect("只给目的地就该能跑");
        assert_eq!(spec.destinations.len(), 1);
        assert_eq!(spec.verify, c.settings.verify_default);
        assert_eq!(spec.algorithm, c.settings.algorithm);
        assert!(pending.is_some(), "没有项目时应产出一个待建项目");

        // 一个目的地都没有才是真的走不下去
        let r = req(
            ProjectChoice::Create {
                name: DEFAULT_PROJECT_NAME.into(),
                destinations: vec![],
            },
            vec![],
        );
        assert_eq!(
            build_adhoc_spec(&c, &r, &[], at()).expect_err("没有目的地必须被拒"),
            AdhocError::NoDestination
        );
    }

    // spec: → Scenario: 项目字段有默认值
    #[test]
    fn scenario_adhoc_copy_project_field_is_prefilled() {
        let c = config_with_project();
        let d = adhoc_defaults(&c);
        assert_eq!(
            d.project,
            ProjectChoice::Existing(c.projects[0].id.clone()),
            "有当前项目时应预填它"
        );
        assert_eq!(d.destinations.len(), 2, "目的地也该预填好");
        assert!(!d.project_will_be_created);
    }

    // spec: → Scenario: 不填项目则现建一个
    #[test]
    fn scenario_adhoc_copy_creates_project_when_absent() {
        let c = empty_config();
        let d = adhoc_defaults(&c);
        assert!(d.project_will_be_created, "一个项目都没有时要说明会自动建");
        match &d.project {
            ProjectChoice::Create { name, .. } => assert_eq!(name, DEFAULT_PROJECT_NAME),
            other => panic!("应该预填一个待建项目，实际是 {other:?}"),
        }

        // 用户不改直接开始：任务归属于新建的项目
        let r = req(d.project.clone(), vec![PathBuf::from(r"D:\素材")]);
        let (spec, pending) = build_adhoc_spec(&c, &r, &[], at()).expect("规格");
        assert_eq!(spec.project, DEFAULT_PROJECT_NAME);
        let p = pending.expect("待建项目");
        assert_eq!(p.name, DEFAULT_PROJECT_NAME);
        assert_eq!(p.destinations.len(), 1);
    }

    // spec: → Scenario: 规划阶段不创建项目
    #[test]
    fn scenario_adhoc_copy_plan_has_no_side_effects() {
        let mut c = empty_config();
        let before = c.clone();
        let r = req(
            ProjectChoice::Create {
                name: "临时".into(),
                destinations: vec![],
            },
            vec![PathBuf::from(r"D:\素材")],
        );
        let (_spec, pending) = build_adhoc_spec(&c, &r, &[], at()).expect("规格");
        assert_eq!(c, before, "规划期 MUST NOT 动配置");
        assert!(pending.is_some(), "待建项目只是返回值，不是副作用");

        // 只有调用方显式落盘才生效
        c.projects.push(pending.expect("待建项目"));
        assert_ne!(c, before);
    }

    // spec: → Scenario: 已有任务在跑时拒绝
    #[test]
    fn scenario_adhoc_copy_rejected_while_task_running() {
        let c = config_with_project();
        let pid = c.projects[0].id.clone();
        let running = vec!["vol:1".to_string()];
        assert_eq!(
            build_adhoc_spec(&c, &req(ProjectChoice::Existing(pid.clone()), vec![]), &running, at())
                .expect_err("同一设备上已有任务必须拒绝"),
            AdhocError::AlreadyRunning {
                device_name: "A7M4".into()
            }
        );
        // 别的设备不受影响
        let mut other = req(ProjectChoice::Existing(pid), vec![]);
        other.device_id = "vol:2".into();
        assert!(build_adhoc_spec(&c, &other, &running, at()).is_ok());
    }

    // spec: → Scenario: 未分类设备可手动拷贝
    #[test]
    fn scenario_adhoc_copy_unclassified_device_can_be_copied_manually() {
        let mut c = config_with_project();
        let pid = c.projects[0].id.clone();
        // 记忆库里有它，但从没指认过类型
        c.remember_device(DeviceRecord::new("vol:1", "扩展", 128, at()));
        assert_eq!(
            c.device("vol:1").map(|d| d.kind),
            Some(DeviceKind::Unclassified)
        );

        // 手动路径不看类型——用户已经当场把参数说清楚了
        let r = req(ProjectChoice::Existing(pid), vec![]);
        assert!(
            build_adhoc_spec(&c, &r, &[], at()).is_ok(),
            "未分类设备必须能被手动拷贝"
        );
        assert_eq!(
            c.device("vol:1").map(|d| d.kind),
            Some(DeviceKind::Unclassified),
            "拷一次不该顺手给它贴类型标签"
        );
    }

    // spec: → Scenario: 项目不存在时明确报错
    #[test]
    fn scenario_adhoc_copy_missing_project_is_an_error() {
        let c = config_with_project();
        let e = build_adhoc_spec(
            &c,
            &req(ProjectChoice::Existing("pjt-不存在".into()), vec![]),
            &[],
            at(),
        )
        .expect_err("指向不存在的项目必须报错");
        assert!(matches!(e, AdhocError::ProjectMissing { .. }));
        // 不能悄悄退回默认项目——那会让素材落到用户没预期的地方
        assert!(e.to_string().contains("pjt-不存在"), "{e}");
    }

    // spec: → Scenario: 与预设路径产出等价（结构层面）
    #[test]
    fn scenario_adhoc_copy_spec_is_indistinguishable_from_preset_spec() {
        use crate::preset::{build_spec, Preset, PresetMatch};

        let mut c = config_with_project();
        let pid = c.projects[0].id.clone();
        c.remember_device(DeviceRecord::new("vol:1", "A7M4", 128, at()));
        if let Some(d) = c.device_mut("vol:1") {
            d.kind = DeviceKind::Camera;
        }
        let device = c.device("vol:1").cloned().expect("设备");

        let mut preset = Preset::new("摄影卡").matching(PresetMatch::Kind {
            device_kind: DeviceKind::Camera,
        });
        preset.project_id = Some(pid.clone());

        let volume = crate::device::Volume {
            guid_path: r"\\?\Volume{1}\".into(),
            drive_letter: Some("E:".into()),
            label: "A7M4".into(),
            serial: Some(1),
            file_system: "exFAT".into(),
            total_bytes: 128,
            free_bytes: 100,
            bus_type: crate::device::BusType::Usb,
            state: crate::device::VolumeState::Online,
            is_system: false,
            fingerprints: vec![],
        };

        let from_preset = build_spec(&c, &preset, &device, &volume, at()).expect("预设路径");
        let (from_adhoc, _) = build_adhoc_spec(
            &c,
            &AdhocRequest {
                source_root: volume.root_path(),
                device_id: device.id.clone(),
                device_name: device.display_name(),
                project: ProjectChoice::Existing(pid),
                destinations: vec![],
                verify: Some(from_preset.verify),
                algorithm: Some(from_preset.algorithm),
                eject_after: from_preset.eject_after,
                template_override: None,
            },
            &[],
            at(),
        )
        .expect("临时路径");

        // 逐字段比对：下游能分辨来源，迟早有人为「临时」写捷径
        assert_eq!(from_adhoc.source_root, from_preset.source_root);
        assert_eq!(from_adhoc.source.id, from_preset.source.id);
        assert_eq!(from_adhoc.project, from_preset.project);
        assert_eq!(from_adhoc.algorithm, from_preset.algorithm);
        assert_eq!(from_adhoc.verify, from_preset.verify);
        assert_eq!(from_adhoc.retries, from_preset.retries);
        assert_eq!(from_adhoc.at, from_preset.at);
        assert_eq!(
            from_adhoc.destinations.len(),
            from_preset.destinations.len()
        );
        for (a, b) in from_adhoc
            .destinations
            .iter()
            .zip(from_preset.destinations.iter())
        {
            assert_eq!(a.root, b.root);
            assert_eq!(a.enabled, b.enabled);
        }
    }

    // spec: → Scenario: 安全标准不打折
    #[test]
    fn scenario_adhoc_copy_cannot_skip_verification() {
        let c = config_with_project();
        let pid = c.projects[0].id.clone();

        // 请求里唯一能影响校验的就是 verify，而它只有 true/false/沿用默认三种取值——
        // 不存在「跳过校验但清单仍标记为已校验」的字段组合
        for v in [None, Some(true), Some(false)] {
            let mut r = req(ProjectChoice::Existing(pid.clone()), vec![]);
            r.verify = v;
            let (spec, _) = build_adhoc_spec(&c, &r, &[], at()).expect("规格");
            assert_eq!(spec.verify, v.unwrap_or(c.settings.verify_default));
        }

        // 扫描口径同样是整卡镜像，不给「临时就少拷点」的余地
        let r = req(ProjectChoice::Existing(pid), vec![]);
        let (spec, _) = build_adhoc_spec(&c, &r, &[], at()).expect("规格");
        assert!(
            spec.scan.filter.is_none(),
            "临时拷贝同样是整卡镜像，MUST NOT 偷偷过滤"
        );
    }

    // spec: → Scenario: 未在项目里配过的目的地也能用
    #[test]
    fn scenario_adhoc_copy_ad_hoc_destination_gets_default_template() {
        let c = config_with_project();
        let pid = c.projects[0].id.clone();
        let r = req(
            ProjectChoice::Existing(pid),
            vec![PathBuf::from(r"X:\临时盘")],
        );
        let (spec, _) = build_adhoc_spec(&c, &r, &[], at()).expect("规格");
        assert_eq!(spec.destinations.len(), 1);
        assert_eq!(spec.destinations[0].root, PathBuf::from(r"X:\临时盘"));
    }
}
