#![allow(clippy::unwrap_used, clippy::expect_used)]
//! 语言护栏：英文输出里不许混进中文，中文输出里不许留占位符。
//!
//! 规范：`openspec/changes/add-steadcopy-i18n/specs/i18n/spec.md`
//!
//! 穷尽 `match` 保证「不会漏分支」，这组测试保证「分支里填的确实是译文」——
//! 前者管结构，后者管内容，缺一不可。
//!
//! # 怎么保证护栏自己不漏
//!
//! 每一族文案都配一个 `all_*()` 样本表和一个 `witness_*()` **穷尽性见证**：
//! 见证是对该枚举的穷尽 `match`，新增一个变体，**这个文件先编译不过**——
//! 于是加变体的人必然会看到这里，不可能悄悄漏掉。
//! 只抽查几个变体的护栏，等于给「新加的那条没翻」发通行证。

use steadcopy_core::config::store::SaveError;
use steadcopy_core::config::{ConfigError, ConfigLoadError};
use steadcopy_core::device::{
    AutoFormatDecision, BusType, DeviceKind, EjectError, RemovabilityError,
};
use steadcopy_core::error::{CoreError, ErrorContext, RetryableKind, TerminalKind};
use steadcopy_core::i18n::{has_cjk, Locale, PLACEHOLDERS};
use steadcopy_core::ledger::LedgerError;
use steadcopy_core::manifest::ManifestReadIssue;
use steadcopy_core::map::MapError;
use steadcopy_core::organize::TemplateError;
use steadcopy_core::preset::{ArrivalOutcome, PresetMatch, SinkScope};
use steadcopy_core::task::AdhocError;

/// 全部会产出用户可读文本的编排结论。
///
/// 新增变体时这里要跟着加——漏了护栏就盖不到它。
/// （`summary` 那边的穷尽 `match` 会先让编译失败，这里是第二层。）
fn all_outcomes() -> Vec<ArrivalOutcome> {
    vec![
        ArrivalOutcome::NeedsClassification {
            device_id: "vol:1".into(),
            suggested_name: "A7M4".into(),
        },
        ArrivalOutcome::Ignored {
            device_id: "vol:1".into(),
        },
        ArrivalOutcome::AlreadyRunning {
            device_id: "vol:1".into(),
        },
        ArrivalOutcome::NoPreset {
            device_id: "vol:1".into(),
            device_name: "A7M4".into(),
        },
        ArrivalOutcome::NoProject {
            preset_name: "Camera".into(),
        },
        ArrivalOutcome::NoSource {
            device_name: "A7M4".into(),
        },
        ArrivalOutcome::NoNewSource {
            device_name: "A7M4".into(),
        },
        ArrivalOutcome::InsufficientSpace {
            device_name: "A7M4".into(),
            landing_dir: "D:\\media".into(),
            required_bytes: 100,
            available_bytes: Some(10),
        },
    ]
}

const KINDS: [DeviceKind; 5] = [
    DeviceKind::Unclassified,
    DeviceKind::Camera,
    DeviceKind::Recorder,
    DeviceKind::Storage,
    DeviceKind::Ignored,
];

const BUSES: [BusType; 10] = [
    BusType::Usb,
    BusType::Thunderbolt,
    BusType::Sd,
    BusType::Mmc,
    BusType::Nvme,
    BusType::Sata,
    BusType::Scsi,
    BusType::Network,
    BusType::Other,
    BusType::Unknown,
];

const AUTO_FORMAT: [AutoFormatDecision; 6] = [
    AutoFormatDecision::Disabled,
    AutoFormatDecision::Cancelled,
    AutoFormatDecision::HasFailures,
    AutoFormatDecision::NotVerified,
    AutoFormatDecision::DestinationIncomplete,
    AutoFormatDecision::Propose,
];

const REMOVABILITY: [RemovabilityError; 2] =
    [RemovabilityError::QueryFailed, RemovabilityError::Indeterminate];

/// 样本里的路径、项目名、素材名一律用 ASCII。
///
/// 它们是**数据**：中文项目名出现在英文句子里是对的。护栏只盯文案，
/// 所以样本刻意不带中文，免得把数据误判成漏译（报告那条护栏同理）。
fn all_core_errors() -> Vec<CoreError> {
    let ctx = || ErrorContext::new().path("D:\\media\\A001.MP4");
    let mut out = Vec::new();
    for k in [
        RetryableKind::CopyIo,
        RetryableKind::VerifyMismatch,
        RetryableKind::DeviceRemoved,
        RetryableKind::DestinationUnwritable,
    ] {
        witness_retryable(&k);
        out.push(CoreError::retryable(k.clone()));
        // 带上下文那一路会拼括号，中英括号不同，两条都要盖
        out.push(CoreError::retryable(k).with_context(ctx()));
    }
    for k in [
        TerminalKind::NoSource,
        TerminalKind::NoNewSource,
        TerminalKind::InsufficientSpace,
        TerminalKind::SourceUnreadable,
        TerminalKind::InvalidConfig,
        TerminalKind::Unsupported,
    ] {
        witness_terminal(&k);
        out.push(CoreError::terminal(k.clone()));
        out.push(CoreError::terminal(k).with_context(ctx()));
    }
    out
}

fn all_adhoc_errors() -> Vec<AdhocError> {
    vec![
        AdhocError::AlreadyRunning {
            device_name: "A7M4".into(),
        },
        AdhocError::NoDestination,
        AdhocError::ProjectMissing { id: "prj-404".into() },
        AdhocError::BadTemplate {
            root: "D:\\media".into(),
            reason: TemplateError::UnbalancedBrace,
        },
    ]
}

fn all_eject_errors() -> Vec<EjectError> {
    vec![
        EjectError::TaskRunning,
        EjectError::Busy("FSCTL_LOCK_VOLUME: access denied".into()),
        EjectError::Unsupported,
        EjectError::Failed("IOCTL_STORAGE_EJECT_MEDIA: invalid handle".into()),
    ]
}

fn all_config_errors() -> Vec<ConfigError> {
    vec![
        ConfigError::DestinationCount {
            project: "Wedding".into(),
            count: 7,
        },
        ConfigError::NoEnabledDestination {
            project: "Wedding".into(),
        },
        ConfigError::BadTemplate {
            project: "Wedding".into(),
            destination: "D:\\media".into(),
            reason: TemplateError::MissingRequiredPlaceholder,
        },
        ConfigError::PresetProjectMissing {
            preset: "Camera".into(),
            project_id: "prj-404".into(),
        },
        ConfigError::CountdownTooShort { secs: 5, min: 10 },
        ConfigError::BadMap {
            project: "Wedding".into(),
            reason: MapError::EmptyName,
        },
        ConfigError::BadMapTemplate {
            template: "Wedding tree".into(),
            reason: MapError::TooDeep { max: 12 },
        },
    ]
}

fn all_map_errors() -> Vec<MapError> {
    vec![
        MapError::EmptyName,
        MapError::NameTooLong {
            name: "A".repeat(101),
            max: 100,
            actual: 101,
        },
        MapError::IllegalCharacter { name: "a:b".into() },
        MapError::ReservedName { name: "CON".into() },
        MapError::PaddedName { name: "media ".into() },
        MapError::BadPlaceholder {
            name: "{x}".into(),
            reason: TemplateError::UnknownPlaceholder("x".into()),
        },
        MapError::DuplicateSibling { name: "DCIM".into() },
        MapError::TooDeep { max: 12 },
        MapError::NodeMissing { id: "map-404".into() },
        MapError::WouldCycle { name: "Video".into() },
        MapError::AssignmentMissing { id: "lnk-404".into() },
        MapError::DuplicateAssignment {
            device_name: "A7M4".into(),
            node_name: "Video".into(),
        },
        MapError::SourceOffline {
            device_name: "A7M4".into(),
        },
        // 分叉在中途与分叉在顶层是两条不同的句子，都要盖
        MapError::NotAChain {
            at: Some("Video".into()),
            branches: 2,
        },
        MapError::NotAChain {
            at: None,
            branches: 3,
        },
        MapError::EmptyMap,
        MapError::BadTemplateString {
            template: "media".into(),
            reason: TemplateError::MissingRequiredPlaceholder,
        },
        MapError::Dispatch {
            reason: AdhocError::AlreadyRunning {
                device_name: "A7M4".into(),
            },
        },
        MapError::Inconsistent {
            detail: "node map-1 refers to a missing parent".into(),
        },
        MapError::Unreadable {
            path: "D:\\media".into(),
            reason: "access denied".into(),
        },
    ]
}

fn all_config_load_errors() -> Vec<ConfigLoadError> {
    vec![
        ConfigLoadError::Unreadable("access denied".into()),
        // 损坏那条有「留没留下备份」两个分支，都要盖
        ConfigLoadError::Corrupt {
            reason: "unexpected end of input".into(),
            backup: None,
        },
        ConfigLoadError::Corrupt {
            reason: "unexpected end of input".into(),
            backup: Some("D:\\cfg\\config.corrupt-1.json".into()),
        },
        ConfigLoadError::FutureVersion {
            found: 9,
            supported: 1,
        },
    ]
}

fn all_save_errors() -> Vec<SaveError> {
    vec![
        SaveError::Io("disk full".into()),
        SaveError::Invalid(ConfigError::NoEnabledDestination {
            project: "Wedding".into(),
        }),
    ]
}

fn all_ledger_errors() -> Vec<LedgerError> {
    vec![
        LedgerError::Open {
            path: "D:\\cfg\\ledger.db".into(),
            reason: "database is locked".into(),
        },
        LedgerError::Query("no such table".into()),
        LedgerError::FutureSchema {
            found: 9,
            supported: 1,
        },
    ]
}

fn all_manifest_issues() -> Vec<ManifestReadIssue> {
    vec![
        ManifestReadIssue::Unreadable("access denied".into()),
        ManifestReadIssue::Malformed("unexpected end of input".into()),
        ManifestReadIssue::FutureVersion {
            found: 9,
            supported: 1,
        },
    ]
}

fn all_template_errors() -> Vec<TemplateError> {
    vec![
        TemplateError::MissingRequiredPlaceholder,
        TemplateError::UnknownPlaceholder("machine".into()),
        TemplateError::UnbalancedBrace,
        TemplateError::EmptyTemplate,
    ]
}

// ── 穷尽性见证 ────────────────────────────────────────────────────
// 只做一件事：新增变体时让这个文件编译不过，逼着人来补上面的样本表。

fn witness_retryable(k: &RetryableKind) {
    match k {
        RetryableKind::CopyIo
        | RetryableKind::VerifyMismatch
        | RetryableKind::DeviceRemoved
        | RetryableKind::DestinationUnwritable => {}
    }
}

fn witness_terminal(k: &TerminalKind) {
    match k {
        TerminalKind::NoSource
        | TerminalKind::NoNewSource
        | TerminalKind::InsufficientSpace
        | TerminalKind::SourceUnreadable
        | TerminalKind::InvalidConfig
        | TerminalKind::Unsupported => {}
    }
}

fn witness_all_enums() {
    for e in all_adhoc_errors() {
        match e {
            AdhocError::AlreadyRunning { .. }
            | AdhocError::NoDestination
            | AdhocError::ProjectMissing { .. }
            | AdhocError::BadTemplate { .. } => {}
        }
    }
    for e in all_eject_errors() {
        match e {
            EjectError::TaskRunning
            | EjectError::Busy(_)
            | EjectError::Unsupported
            | EjectError::Failed(_) => {}
        }
    }
    for e in all_config_errors() {
        match e {
            ConfigError::DestinationCount { .. }
            | ConfigError::NoEnabledDestination { .. }
            | ConfigError::BadTemplate { .. }
            | ConfigError::PresetProjectMissing { .. }
            | ConfigError::CountdownTooShort { .. }
            | ConfigError::BadMap { .. }
            | ConfigError::BadMapTemplate { .. } => {}
        }
    }
    for e in all_map_errors() {
        match e {
            MapError::EmptyName
            | MapError::NameTooLong { .. }
            | MapError::IllegalCharacter { .. }
            | MapError::ReservedName { .. }
            | MapError::PaddedName { .. }
            | MapError::BadPlaceholder { .. }
            | MapError::DuplicateSibling { .. }
            | MapError::TooDeep { .. }
            | MapError::NodeMissing { .. }
            | MapError::WouldCycle { .. }
            | MapError::AssignmentMissing { .. }
            | MapError::DuplicateAssignment { .. }
            | MapError::SourceOffline { .. }
            | MapError::NotAChain { .. }
            | MapError::EmptyMap
            | MapError::BadTemplateString { .. }
            | MapError::Dispatch { .. }
            | MapError::Inconsistent { .. }
            | MapError::Unreadable { .. } => {}
        }
    }
    for e in all_config_load_errors() {
        match e {
            ConfigLoadError::Unreadable(_)
            | ConfigLoadError::Corrupt { .. }
            | ConfigLoadError::FutureVersion { .. } => {}
        }
    }
    for e in all_save_errors() {
        match e {
            SaveError::Invalid(_) | SaveError::Io(_) => {}
        }
    }
    for e in all_ledger_errors() {
        match e {
            LedgerError::Open { .. } | LedgerError::Query(_) | LedgerError::FutureSchema { .. } => {}
        }
    }
    for e in all_manifest_issues() {
        match e {
            ManifestReadIssue::Unreadable(_)
            | ManifestReadIssue::Malformed(_)
            | ManifestReadIssue::FutureVersion { .. } => {}
        }
    }
    for e in all_template_errors() {
        match e {
            TemplateError::MissingRequiredPlaceholder
            | TemplateError::UnknownPlaceholder(_)
            | TemplateError::UnbalancedBrace
            | TemplateError::EmptyTemplate => {}
        }
    }
    for e in REMOVABILITY {
        match e {
            RemovabilityError::QueryFailed | RemovabilityError::Indeterminate => {}
        }
    }
    for d in AUTO_FORMAT {
        match d {
            AutoFormatDecision::Disabled
            | AutoFormatDecision::Cancelled
            | AutoFormatDecision::HasFailures
            | AutoFormatDecision::NotVerified
            | AutoFormatDecision::DestinationIncomplete
            | AutoFormatDecision::Propose => {}
        }
    }
    for b in BUSES {
        match b {
            BusType::Usb
            | BusType::Thunderbolt
            | BusType::Sd
            | BusType::Mmc
            | BusType::Nvme
            | BusType::Sata
            | BusType::Scsi
            | BusType::Network
            | BusType::Other
            | BusType::Unknown => {}
        }
    }
}

/// 去掉 `{…}` 花括号里的模板占位符再看。
///
/// `{项目}` `{日期}` 是用户要**照抄进模板**的字面量，和盘符、素材名一样属于数据。
/// 把它翻成 `{project}`，同一份配置在两种语言下就会落到不同目录——那是判定随语言变，
/// 铁律不允许。所以这几条英文文案里留着中文是**对的**，护栏只盯占位符之外的散文。
fn strip_placeholders(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for c in s.chars() {
        match c {
            '{' => inside = true,
            '}' => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

// spec: i18n → Scenario: 英文输出无 CJK
#[test]
fn scenario_i18n_english_output_has_no_cjk() {
    witness_all_enums();
    let mut checked = 0;
    let mut check = |what: &str, s: &str| {
        assert!(!has_cjk(s), "英文{what}里混进了中文：{s}");
        checked += 1;
    };

    for o in all_outcomes() {
        check("结论", &o.summary(Locale::En));
        check("出口文案", o.next_step().label(Locale::En));
    }

    for k in KINDS {
        check("设备类型", k.label(Locale::En));
    }

    for b in BUSES {
        check("总线名", b.label(Locale::En));
    }

    for m in [
        PresetMatch::Device {
            device_id: "vol:1".into(),
        },
        PresetMatch::Kind {
            device_kind: DeviceKind::Camera,
        },
        PresetMatch::AnyClassifiedSource,
    ] {
        check("匹配描述", &m.describe(Locale::En));
    }

    for sc in [
        SinkScope::ThisDevice,
        SinkScope::ThisKind(DeviceKind::Camera),
        SinkScope::AnyClassified,
    ] {
        check("沉淀范围", &sc.describe(Locale::En, "A7M4"));
    }

    // ── 错误族 ────────────────────────────────────────────────
    for e in all_core_errors() {
        check("核心错误", &e.describe(Locale::En));
    }
    for e in all_eject_errors() {
        check("弹出错误", &e.describe(Locale::En));
    }
    for e in all_config_load_errors() {
        check("配置读取错误", &e.describe(Locale::En));
    }
    for e in all_save_errors() {
        check("配置保存错误", &e.describe(Locale::En));
    }
    for e in all_ledger_errors() {
        check("台账错误", &e.describe(Locale::En));
    }
    for e in all_manifest_issues() {
        check("清单读取异常", &e.describe(Locale::En));
    }
    for e in REMOVABILITY {
        check("可移除性拒绝理由", e.describe(Locale::En));
    }
    for d in AUTO_FORMAT {
        check("自动格式化结论", d.reason(Locale::En));
    }
    check(
        "倒计时校验错误",
        &steadcopy_core::device::validate_countdown(5, Locale::En)
            .expect_err("低于下限应被拒"),
    );

    // 这三族的句子里嵌着模板占位符，占位符不翻是刻意的（见 strip_placeholders）
    for e in all_template_errors() {
        check("模板错误", &strip_placeholders(&e.describe(Locale::En)));
    }
    for e in all_adhoc_errors() {
        check("临时拷贝错误", &strip_placeholders(&e.describe(Locale::En)));
    }
    for e in all_config_errors() {
        check("配置校验错误", &strip_placeholders(&e.describe(Locale::En)));
    }
    for e in all_map_errors() {
        check("导图错误", &strip_placeholders(&e.describe(Locale::En)));
    }

    // ── 安全检查 detail ───────────────────────────────────────
    for r in all_safety_reports(Locale::En) {
        for c in &r.checks {
            check("安全检查说明", &c.detail);
        }
    }

    assert!(checked >= 100, "护栏只查了 {checked} 条，覆盖面不够");
}

// spec: i18n → Scenario: 中文输出无占位符
#[test]
fn scenario_i18n_chinese_output_has_no_placeholder() {
    let mut texts: Vec<String> = all_outcomes()
        .iter()
        .map(|o| o.summary(Locale::Zh))
        .collect();
    texts.extend(KINDS.iter().map(|k| k.label(Locale::Zh).to_string()));
    texts.extend(BUSES.iter().map(|b| b.label(Locale::Zh).to_string()));
    texts.extend(all_core_errors().iter().map(|e| e.describe(Locale::Zh)));
    texts.extend(all_adhoc_errors().iter().map(|e| e.describe(Locale::Zh)));
    texts.extend(all_eject_errors().iter().map(|e| e.describe(Locale::Zh)));
    texts.extend(all_config_errors().iter().map(|e| e.describe(Locale::Zh)));
    texts.extend(
        all_config_load_errors()
            .iter()
            .map(|e| e.describe(Locale::Zh)),
    );
    texts.extend(all_save_errors().iter().map(|e| e.describe(Locale::Zh)));
    texts.extend(all_ledger_errors().iter().map(|e| e.describe(Locale::Zh)));
    texts.extend(all_manifest_issues().iter().map(|e| e.describe(Locale::Zh)));
    texts.extend(all_template_errors().iter().map(|e| e.describe(Locale::Zh)));
    texts.extend(all_map_errors().iter().map(|e| e.describe(Locale::Zh)));
    texts.extend(REMOVABILITY.iter().map(|e| e.describe(Locale::Zh).to_string()));
    texts.extend(AUTO_FORMAT.iter().map(|d| d.reason(Locale::Zh).to_string()));
    for r in all_safety_reports(Locale::Zh) {
        texts.extend(r.checks.iter().map(|c| c.detail.clone()));
    }

    for s in texts {
        assert!(
            !s.trim().is_empty(),
            "中文文案不能是空的——空白在界面上像「这里本来就没东西」"
        );
        for p in PLACEHOLDERS {
            assert!(!s.contains(p), "中文文案里留了占位符 {p}：{s}");
        }
    }
}

// spec: i18n → Scenario: core 的输出随 locale 变化
#[test]
fn scenario_i18n_core_text_follows_locale() {
    for o in all_outcomes() {
        let zh = o.summary(Locale::Zh);
        let en = o.summary(Locale::En);
        // 两串相同说明有一边根本没翻——比漏分支更难发现，因为它编译得过
        assert_ne!(zh, en, "这条结论两种语言给了同一串文本：{zh}");
    }
    for k in KINDS {
        assert_ne!(k.label(Locale::Zh), k.label(Locale::En), "{k:?} 没翻");
    }
    for e in all_core_errors() {
        assert_ne!(e.describe(Locale::Zh), e.describe(Locale::En), "{e:?} 没翻");
    }
    for e in all_adhoc_errors() {
        assert_ne!(e.describe(Locale::Zh), e.describe(Locale::En), "{e:?} 没翻");
    }
    for e in all_eject_errors() {
        assert_ne!(e.describe(Locale::Zh), e.describe(Locale::En), "{e:?} 没翻");
    }
    for e in all_config_errors() {
        assert_ne!(e.describe(Locale::Zh), e.describe(Locale::En), "{e:?} 没翻");
    }
    for e in all_config_load_errors() {
        assert_ne!(e.describe(Locale::Zh), e.describe(Locale::En), "{e:?} 没翻");
    }
    for e in all_save_errors() {
        assert_ne!(e.describe(Locale::Zh), e.describe(Locale::En), "{e:?} 没翻");
    }
    for e in all_ledger_errors() {
        assert_ne!(e.describe(Locale::Zh), e.describe(Locale::En), "{e:?} 没翻");
    }
    for e in all_manifest_issues() {
        assert_ne!(e.describe(Locale::Zh), e.describe(Locale::En), "{e:?} 没翻");
    }
    for e in all_template_errors() {
        assert_ne!(e.describe(Locale::Zh), e.describe(Locale::En), "{e:?} 没翻");
    }
    for e in all_map_errors() {
        assert_ne!(e.describe(Locale::Zh), e.describe(Locale::En), "{e:?} 没翻");
    }
    for e in REMOVABILITY {
        assert_ne!(e.describe(Locale::Zh), e.describe(Locale::En), "{e:?} 没翻");
    }
    for d in AUTO_FORMAT {
        assert_ne!(d.reason(Locale::Zh), d.reason(Locale::En), "{d:?} 没翻");
    }

    // Display 恒为中文：它落在日志与命令行兜底里，那里没有 locale 可读
    for e in all_core_errors() {
        assert_eq!(e.to_string(), e.describe(Locale::Zh), "Display 应恒为中文");
    }
    for e in all_eject_errors() {
        assert_eq!(e.to_string(), e.describe(Locale::Zh), "Display 应恒为中文");
    }
    for e in all_map_errors() {
        assert_eq!(e.to_string(), e.describe(Locale::Zh), "Display 应恒为中文");
    }
}

/// 把安全检查链的每一条分支各跑一遍，返回全部报告。
///
/// 覆盖 G1 的五种结论（可移除 / 系统盘 / 固定盘 / 查不到 / 说不清）、
/// G2 的两种、G3 的两种、G4 的三种。
fn all_safety_reports(lang: Locale) -> Vec<steadcopy_core::device::SafetyReport> {
    use steadcopy_core::device::{check_safety, BackupEvidence, Volume, VolumeState};
    use steadcopy_core::engine::{hash_bytes, HashAlgorithm};
    use steadcopy_core::manifest::model::{ManifestEntry, SourceRef, VerifyState};
    use steadcopy_core::manifest::Manifest;
    use time::macros::datetime;

    let at = datetime!(2026-08-11 09:00:00 UTC);
    let vol = |bus: BusType, system: bool| Volume {
        guid_path: r"\\?\Volume{1}\".into(),
        drive_letter: Some("E:".into()),
        label: "A7M4".into(),
        serial: Some(1),
        file_system: "exFAT".into(),
        total_bytes: 128,
        free_bytes: 30,
        bus_type: bus,
        is_system: system,
        state: VolumeState::Online,
        fingerprints: vec![],
    };

    let mut m = Manifest::new(
        SourceRef {
            id: "vol:1".into(),
            display_name: "A7M4".into(),
        },
        "Wedding",
        r"D:\media",
        HashAlgorithm::Xxh64,
        at,
    );
    let h = hash_bytes(HashAlgorithm::Xxh64, b"x");
    m.entries.push(ManifestEntry {
        relative_path: "A001.MP4".into(),
        size: 1,
        source_hash: h,
        verify: VerifyState::Verified { destination_hash: h },
        source_modified_at: None,
        completed_at: at,
        retries: 0,
    });
    let ev = BackupEvidence {
        task_id: "task-1".into(),
        manifest: m,
    };

    let covered: Vec<String> = vec!["A001.MP4".into()];
    let extra: Vec<String> = vec!["A001.MP4".into(), "A002.MP4".into()];
    let dest = vec![std::path::PathBuf::from(r"E:\media")];

    vec![
        // G1 的五种结论
        check_safety(&vol(BusType::Usb, false), &[], false, Some(&ev), &covered, lang),
        check_safety(&vol(BusType::Usb, true), &[], false, Some(&ev), &covered, lang),
        check_safety(&vol(BusType::Nvme, false), &[], false, Some(&ev), &covered, lang),
        check_safety(&vol(BusType::Unknown, false), &[], false, Some(&ev), &covered, lang),
        check_safety(&vol(BusType::Other, false), &[], false, Some(&ev), &covered, lang),
        // G2 命中、G3 命中
        check_safety(&vol(BusType::Usb, false), &dest, false, Some(&ev), &covered, lang),
        check_safety(&vol(BusType::Usb, false), &[], true, Some(&ev), &covered, lang),
        // G4 的三种：没备份 / 备份齐 / 备份不齐
        check_safety(&vol(BusType::Usb, false), &[], false, None, &covered, lang),
        check_safety(&vol(BusType::Usb, false), &[], false, Some(&ev), &extra, lang),
    ]
}

// spec: i18n → Scenario: 两种语言下判定结果一致（安全检查）
#[test]
fn scenario_i18n_safety_check_verdicts_are_locale_independent() {
    let zh = all_safety_reports(Locale::Zh);
    let en = all_safety_reports(Locale::En);
    assert_eq!(zh.len(), en.len());

    for (a, b) in zh.iter().zip(en.iter()) {
        // 判定：通过与否、卡在哪一条、留痕串、G4 认下来的那次备份
        assert_eq!(a.passed(), b.passed(), "安全结论 MUST NOT 随语言变");
        assert_eq!(a.compact(), b.compact(), "留痕串 MUST NOT 随语言变");
        assert_eq!(
            a.first_failure().map(|c| c.id.as_str()),
            b.first_failure().map(|c| c.id.as_str()),
            "卡在哪一条 MUST NOT 随语言变"
        );
        assert_eq!(a.backup_task_id, b.backup_task_id);

        // 只有说明文本不同
        for (ca, cb) in a.checks.iter().zip(b.checks.iter()) {
            assert_eq!(ca.id, cb.id);
            assert_eq!(ca.passed, cb.passed);
            assert_ne!(ca.detail, cb.detail, "{} 这条说明没翻", ca.id);
        }
    }

    // 覆盖面自检：九个样本里通过与不通过都要有，否则这个测试可能只在证明「全都不通过」
    assert!(zh.iter().any(|r| r.passed()), "样本里要有能通过的");
    assert!(zh.iter().any(|r| !r.passed()), "样本里要有不通过的");
}

// spec: i18n → Scenario: 两种语言下判定结果一致
#[test]
fn scenario_i18n_locale_does_not_affect_decisions() {
    use steadcopy_core::config::model::{Config, DestinationConfig, Project};
    use steadcopy_core::device::{BusType, DeviceRecord, Volume, VolumeState};
    use steadcopy_core::platform::volume_io;
    use steadcopy_core::preset::{on_arrival, Preset, PresetMatch};
    use time::macros::datetime;

    let at = datetime!(2026-08-11 09:00:00 UTC);
    let dir = tempfile::tempdir().expect("临时目录");
    let src = dir.path().join("card");
    std::fs::create_dir_all(src.join("DCIM")).expect("建目录");
    std::fs::write(src.join("DCIM/a.mp4"), vec![b'x'; 1024]).expect("写文件");

    let vol = Volume {
        // 用真实源目录当卷根，这样规划能真的扫到文件
        guid_path: src.display().to_string(),
        drive_letter: None,
        label: "A7M4".into(),
        serial: Some(1),
        file_system: "exFAT".into(),
        total_bytes: 1 << 30,
        free_bytes: 1 << 29,
        bus_type: BusType::Usb,
        state: VolumeState::Online,
        is_system: false,
        fingerprints: vec![],
    };
    // 设备身份必须用真实的复合 id，否则记忆库对不上、编排会停在指认那一步，
    // 这个测试就退化成「两次都停在同一步」，什么也证明不了
    let device_id = vol.composite_id();

    let make_cfg = || {
        let mut c = Config::default();
        let mut p = Project::new("婚礼", at);
        p.destinations
            .push(DestinationConfig::new(dir.path().join("dst")));
        c.current_project = Some(p.id.clone());
        c.projects.push(p);
        let mut pr = Preset::new("摄影卡").matching(PresetMatch::Kind {
            device_kind: DeviceKind::Camera,
        });
        pr.project_id = c.current_project.clone();
        c.presets.push(pr);
        let mut d = DeviceRecord::new(&device_id, "A7M4", 1 << 30, at);
        d.kind = DeviceKind::Camera;
        c.devices.push(d);
        c
    };

    let io = volume_io();
    let mut c1 = make_cfg();
    let mut c2 = make_cfg();
    let a = on_arrival(&mut c1, &vol, &[], io.as_ref(), at);
    let b = on_arrival(&mut c2, &vol, &[], io.as_ref(), at);

    // 判定必须一致：文件集合、落地路径、空间结论、要不要确认
    match (&a, &b) {
        (
            ArrivalOutcome::Planned { plan: pa, requires_confirmation: ra, .. },
            ArrivalOutcome::Planned { plan: pb, requires_confirmation: rb, .. },
        ) => {
            assert_eq!(pa.files.len(), pb.files.len());
            assert_eq!(pa.total_bytes(), pb.total_bytes());
            assert_eq!(ra, rb);
            let da: Vec<_> = pa.destinations.iter().map(|d| &d.landing_dir).collect();
            let db: Vec<_> = pb.destinations.iter().map(|d| &d.landing_dir).collect();
            assert_eq!(da, db, "落地路径 MUST NOT 随语言变化——那意味着素材会落到不同地方");
        }
        _ => panic!("两次编排给了不同的结论种类：{a:?} / {b:?}"),
    }

    // 只有描述文本不同
    assert_ne!(a.summary(Locale::Zh), a.summary(Locale::En));
    assert_eq!(a.next_step(), b.next_step(), "出口判定也 MUST NOT 随语言变化");
}

// spec: i18n → Scenario: 英文输出无 CJK（报告 HTML）
#[test]
fn scenario_i18n_english_report_has_no_cjk() {
    use steadcopy_core::engine::{hash_bytes, HashAlgorithm};
    use steadcopy_core::ledger::{render_report, ReportInput};
    use steadcopy_core::manifest::model::{ManifestEntry, SourceRef, VerifyState};
    use steadcopy_core::manifest::Manifest;
    use time::macros::datetime;

    let at = datetime!(2026-08-11 09:00:00 UTC);
    let mut m = Manifest::new(
        SourceRef {
            id: "vol-1".into(),
            // 素材名本来就可能是中文，报告里出现它是**正确的**——
            // 这里刻意全用英文，好让护栏只盯模板文案
            display_name: "A7M4".into(),
        },
        "Wedding",
        r"D:\media",
        HashAlgorithm::Xxh64,
        at,
    );
    m.entries.push(ManifestEntry {
        relative_path: "DCIM/a.mp4".into(),
        size: 1024,
        source_hash: hash_bytes(HashAlgorithm::Xxh64, b"x"),
        verify: VerifyState::NotVerified,
        source_modified_at: None,
        completed_at: at,
        retries: 0,
    });

    let failures = [("DCIM/b.mp4".to_string(), "io error".to_string(), 2u32)];
    let html = render_report(&ReportInput {
        manifest: &m,
        failures: &failures,
        skipped: 3,
        notices: &[],
        elapsed_secs: Some(125),
        generated_at: at,
        audit: None,
        lang: Locale::En,
    });

    // 报告是要拿给客户看的。英文报告里冒出一句中文，比界面上更尴尬
    for line in html.lines() {
        assert!(!has_cjk(line), "英文报告里混进了中文：{line}");
    }
    assert!(html.contains("lang=\"en\""), "html 的 lang 属性也要跟着变");
    assert!(html.contains("Copy report"));

    // 中文那版照旧
    let zh = render_report(&ReportInput {
        manifest: &m,
        failures: &failures,
        skipped: 3,
        notices: &[],
        elapsed_secs: Some(125),
        generated_at: at,
        audit: None,
        lang: Locale::Zh,
    });
    assert!(has_cjk(&zh), "中文报告不该变成英文的");
    assert!(zh.contains("lang=\"zh-CN\""));
}
