#![allow(clippy::unwrap_used, clippy::expect_used)]
//! 到达编排集成测试：插卡之后到底会发生什么。
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/preset-autorun/spec.md`
//!
//! 纪律 T3：真实 tempdir 当「卡」，设备到达用构造的 `Volume`（不能真插拔卡，
//! 这是允许的两类替身之一）。

use std::path::{Path, PathBuf};

use steadcopy_core::i18n::Locale;
use steadcopy_core::config::model::{Config, DestinationConfig, Project};
use steadcopy_core::device::{BusType, DeviceKind, Volume, VolumeState};
use steadcopy_core::platform::{volume_io, VolumeIo};
use steadcopy_core::preset::{on_arrival, ArrivalOutcome, Preset, PresetMatch};
use time::macros::datetime;
use time::OffsetDateTime;

fn now() -> OffsetDateTime {
    datetime!(2026-08-10 09:00:00 UTC)
}

/// 把一个临时目录伪装成插上来的卷。
fn volume_at(root: &Path, label: &str) -> Volume {
    Volume {
        // 无盘符的卷：root_path() 会回落到 guid_path，正好让测试用真实目录
        guid_path: root.display().to_string(),
        drive_letter: None,
        label: label.into(),
        serial: Some(0xA1B2_C3D4),
        file_system: "exFAT".into(),
        total_bytes: 128 * 1024 * 1024 * 1024,
        free_bytes: 100 * 1024 * 1024 * 1024,
        bus_type: BusType::Usb,
        is_system: false,
        state: VolumeState::Online,
        fingerprints: vec!["影像设备卡".into()],
    }
}

fn card_with_media(root: &Path) {
    let d = root.join("DCIM").join("100MSDCF");
    std::fs::create_dir_all(&d).expect("建目录");
    std::fs::write(d.join("A001.MP4"), vec![b'v'; 50_000]).expect("写");
    std::fs::write(d.join("A001.XML"), b"<meta/>").expect("写");
}

struct Fixture {
    _dir: tempfile::TempDir,
    card: PathBuf,
    dest: PathBuf,
    config: Config,
    io: Box<dyn VolumeIo>,
}

/// 一套「已经配好了」的环境：一个项目、一个目的地、一条匹配摄影卡的预设。
fn configured() -> Fixture {
    let dir = tempfile::tempdir().expect("临时目录");
    let card = dir.path().join("card");
    let dest = dir.path().join("工作盘");
    card_with_media(&card);

    let mut config = Config::default();
    let mut p = Project::new("婚礼", now());
    p.destinations.push(DestinationConfig::new(&dest));
    let pid = p.id.clone();
    config.current_project = Some(pid.clone());
    config.projects.push(p);

    let mut preset = Preset::new("摄影卡进婚礼").matching(PresetMatch::Kind {
        device_kind: DeviceKind::Camera,
    });
    preset.project_id = Some(pid);
    config.presets.push(preset);

    Fixture {
        _dir: dir,
        card,
        dest,
        config,
        io: volume_io(),
    }
}

/// 把设备指认为某个类型（模拟用户在界面上点了一下）。
fn classify(f: &mut Fixture, vol: &Volume, kind: DeviceKind) {
    let id = vol.composite_id();
    let d = f.config.device_mut(&id).expect("设备应已被登记");
    d.kind = kind;
}

fn arrive(f: &mut Fixture, vol: &Volume, running: &[String]) -> ArrivalOutcome {
    on_arrival(&mut f.config, vol, running, f.io.as_ref(), now())
}

// spec: preset-autorun → 未分类设备永不自动开跑 → Scenario: 新卡先指认
#[test]
fn scenario_preset_autorun_new_card_needs_classification_first() {
    let mut f = configured();
    let vol = volume_at(&f.card, "A7M4");

    let out = arrive(&mut f, &vol, &[]);
    match &out {
        ArrivalOutcome::NeedsClassification { suggested_name, .. } => {
            assert_eq!(suggested_name, "A7M4");
        }
        other => panic!("新卡应停在指认步，实际 {other:?}"),
    }
    assert!(!f.dest.exists(), "指认之前 MUST 无任何写入");
    // 但它已经被登记进记忆库了，用户才有得可指认
    assert!(f.config.device(&vol.composite_id()).is_some());
    assert!(out.needs_attention());
}

// spec: → Scenario: 无人值守档同样不处理未分类设备
#[test]
fn scenario_preset_autorun_unattended_still_refuses_unclassified() {
    let mut f = configured();
    // 危险区全开
    f.config.settings.skip_confirmation = true;
    f.config.settings.auto_prefill = true;

    let vol = volume_at(&f.card, "陌生卡");
    let out = arrive(&mut f, &vol, &[]);
    assert!(
        matches!(out, ArrivalOutcome::NeedsClassification { .. }),
        "危险区也绕不过未分类这一关，实际 {out:?}"
    );
    assert!(!f.dest.exists(), "MUST 无任何写入");
}

// spec: → 档位模型 → Scenario: 确认档在点击前零写入
#[test]
fn scenario_preset_autorun_confirm_mode_writes_nothing_before_click() {
    let mut f = configured();
    let vol = volume_at(&f.card, "A7M4");
    arrive(&mut f, &vol, &[]); // 第一次：登记 + 请求指认
    classify(&mut f, &vol, DeviceKind::Camera);

    let out = arrive(&mut f, &vol, &[]);
    match &out {
        ArrivalOutcome::Planned {
            requires_confirmation,
            plan,
            preset_name,
            ..
        } => {
            assert!(*requires_confirmation, "确认档 MUST 要求点一次");
            assert_eq!(preset_name, "摄影卡进婚礼");
            assert_eq!(plan.files.len(), 2, "待拷两个文件");
            // 落地路径在确认前就能看见完整形态
            let landing = plan.destinations[0].landing_dir.display().to_string();
            assert!(landing.contains("婚礼") && landing.contains("2026-08-10"));
            assert_eq!(plan.destinations[0].sufficient(), Some(true));
        }
        other => panic!("已分类且匹配到预设应进入 Planned，实际 {other:?}"),
    }
    assert!(!f.dest.exists(), "点击之前 MUST 无任何写入");
}

// spec: → Scenario: 无人值守档直接开跑
#[test]
fn scenario_preset_autorun_unattended_needs_no_confirmation() {
    let mut f = configured();
    let vol = volume_at(&f.card, "A7M4");
    arrive(&mut f, &vol, &[]);
    classify(&mut f, &vol, DeviceKind::Camera);
    f.config.settings.skip_confirmation = true;

    match arrive(&mut f, &vol, &[]) {
        ArrivalOutcome::Planned {
            requires_confirmation,
            ..
        } => assert!(!requires_confirmation, "无人值守档不该再要确认"),
        other => panic!("实际 {other:?}"),
    }
}

// spec: → Scenario: 被忽略的设备不打扰
#[test]
fn scenario_preset_autorun_ignored_device_is_silent() {
    let mut f = configured();
    // 加一条指名道姓的预设，验证「忽略」能压过它
    let vol = volume_at(&f.card, "杂盘");
    arrive(&mut f, &vol, &[]);
    let id = vol.composite_id();
    f.config.presets.insert(
        0,
        Preset::new("就认这张").matching(PresetMatch::Device {
            device_id: id.clone(),
        }),
    );
    classify(&mut f, &vol, DeviceKind::Ignored);

    let out = arrive(&mut f, &vol, &[]);
    assert!(matches!(out, ArrivalOutcome::Ignored { .. }), "实际 {out:?}");
    assert!(!out.needs_attention(), "被忽略的设备不该打扰用户");
    assert!(!f.dest.exists());
    // 仍留在记忆库里，用户可取消忽略
    assert!(f.config.device(&id).is_some());
}

// spec: → 到达编排 → Scenario: 同一设备不重复建任务
#[test]
fn scenario_preset_autorun_no_duplicate_task_for_running_device() {
    let mut f = configured();
    let vol = volume_at(&f.card, "A7M4");
    arrive(&mut f, &vol, &[]);
    classify(&mut f, &vol, DeviceKind::Camera);

    let running = vec![vol.composite_id()];
    let out = arrive(&mut f, &vol, &running);
    assert!(
        matches!(out, ArrivalOutcome::AlreadyRunning { .. }),
        "实际 {out:?}"
    );
    assert!(!out.needs_attention());
}

// spec: → 预设匹配顺序 → Scenario: 无匹配时不臆造默认
#[test]
fn scenario_preset_autorun_no_preset_is_reported_not_defaulted() {
    let mut f = configured();
    let vol = volume_at(&f.card, "录音机卡");
    arrive(&mut f, &vol, &[]);
    // 指认成录音卡，但唯一的预设只认摄影卡
    classify(&mut f, &vol, DeviceKind::Recorder);

    let out = arrive(&mut f, &vol, &[]);
    match &out {
        ArrivalOutcome::NoPreset { device_name, .. } => {
            assert!(device_name.contains("录音机卡"));
        }
        other => panic!("无匹配应如实报告，实际 {other:?}"),
    }
    assert!(!f.dest.exists(), "无预设 MUST NOT 用默认项目静默开跑");
    assert!(out.summary(Locale::Zh).contains("预设"), "结论要能告诉用户怎么办");
}

#[test]
fn scenario_preset_autorun_no_project_is_reported() {
    let mut f = configured();
    f.config.projects.clear();
    f.config.current_project = None;
    f.config.presets[0].project_id = None; // 用当前项目——但一个都没有了

    let vol = volume_at(&f.card, "A7M4");
    arrive(&mut f, &vol, &[]);
    classify(&mut f, &vol, DeviceKind::Camera);

    let out = arrive(&mut f, &vol, &[]);
    assert!(matches!(out, ArrivalOutcome::NoProject { .. }), "实际 {out:?}");
    assert!(out.summary(Locale::Zh).contains("项目"));
}

#[test]
fn scenario_preset_autorun_empty_card_reports_no_source() {
    let dir = tempfile::tempdir().expect("临时目录");
    let empty = dir.path().join("空卡");
    std::fs::create_dir_all(&empty).expect("建目录");

    let mut f = configured();
    let vol = volume_at(&empty, "空卡");
    arrive(&mut f, &vol, &[]);
    classify(&mut f, &vol, DeviceKind::Camera);

    let out = arrive(&mut f, &vol, &[]);
    assert!(matches!(out, ArrivalOutcome::NoSource { .. }), "实际 {out:?}");
}

// spec: → Scenario: 编排结论可呈现
#[test]
fn scenario_preset_autorun_every_outcome_has_readable_summary() {
    let mut f = configured();
    let vol = volume_at(&f.card, "A7M4");

    // 未分类
    let a = arrive(&mut f, &vol, &[]);
    assert!(a.summary(Locale::Zh).contains("指认"), "{}", a.summary(Locale::Zh));

    // 已规划
    classify(&mut f, &vol, DeviceKind::Camera);
    let b = arrive(&mut f, &vol, &[]);
    assert!(b.summary(Locale::Zh).contains("婚礼") || b.summary(Locale::Zh).contains("确认"), "{}", b.summary(Locale::Zh));

    // 每种结论都得是中文人话，不能是空串或英文枚举名
    for s in [a.summary(Locale::Zh), b.summary(Locale::Zh)] {
        assert!(!s.is_empty());
        assert!(!s.is_ascii(), "结论应为中文：{s}");
    }
}

#[test]
fn scenario_preset_autorun_second_arrival_keeps_user_naming() {
    let mut f = configured();
    let vol = volume_at(&f.card, "A7M4");
    arrive(&mut f, &vol, &[]);
    let id = vol.composite_id();
    {
        let d = f.config.device_mut(&id).expect("设备");
        d.custom_name = "婚礼主卡".into();
        d.kind = DeviceKind::Camera;
    }

    // 再次到达：不该被重新登记、不该覆盖用户改的名字
    let out = arrive(&mut f, &vol, &[]);
    assert!(matches!(out, ArrivalOutcome::Planned { .. }));
    assert_eq!(f.config.devices.len(), 1);
    assert_eq!(f.config.device(&id).expect("设备").custom_name, "婚礼主卡");
    match out {
        ArrivalOutcome::Planned { device_name, .. } => assert_eq!(device_name, "婚礼主卡"),
        _ => unreachable!(),
    }
}

#[test]
fn scenario_preset_autorun_device_specific_preset_wins() {
    let mut f = configured();
    let vol = volume_at(&f.card, "A7M4");
    arrive(&mut f, &vol, &[]);
    classify(&mut f, &vol, DeviceKind::Camera);

    // 再建一个项目与一条指名道姓的预设
    let mut p2 = Project::new("专项", now());
    p2.destinations
        .push(DestinationConfig::new(f.dest.with_file_name("专项盘")));
    let pid2 = p2.id.clone();
    f.config.projects.push(p2);
    let mut narrow = Preset::new("这张卡走专项").matching(PresetMatch::Device {
        device_id: vol.composite_id(),
    });
    narrow.project_id = Some(pid2);
    f.config.presets.push(narrow);

    match arrive(&mut f, &vol, &[]) {
        ArrivalOutcome::Planned {
            preset_name, plan, ..
        } => {
            assert_eq!(preset_name, "这张卡走专项", "窄规则应压过类型规则");
            assert!(plan.destinations[0]
                .landing_dir
                .display()
                .to_string()
                .contains("专项"));
        }
        other => panic!("实际 {other:?}"),
    }
}

// spec: preset-autorun → 到达编排 → Scenario: 每个结论带出口
#[test]
fn scenario_preset_autorun_every_outcome_offers_a_next_step() {
    use steadcopy_core::preset::NextStep;

    // 「不能做」的结论里，只有两种允许没有出路：设备是用户自己标的忽略、
    // 以及该设备上已有任务在跑（等着就行）。其余每一个都必须给出路。
    let cases: Vec<(&str, ArrivalOutcome, bool)> = vec![
        (
            "需指认",
            ArrivalOutcome::NeedsClassification {
                device_id: "vol:1".into(),
                suggested_name: "扩展".into(),
            },
            true,
        ),
        ("已忽略", ArrivalOutcome::Ignored { device_id: "vol:1".into() }, false),
        (
            "已在跑",
            ArrivalOutcome::AlreadyRunning { device_id: "vol:1".into() },
            false,
        ),
        (
            "无预设",
            ArrivalOutcome::NoPreset {
                device_id: "vol:1".into(),
                device_name: "扩展".into(),
            },
            true,
        ),
        (
            "无项目",
            ArrivalOutcome::NoProject { preset_name: "摄影卡".into() },
            true,
        ),
        (
            "无素材",
            ArrivalOutcome::NoSource { device_name: "扩展".into() },
            true,
        ),
        (
            "无新素材",
            ArrivalOutcome::NoNewSource { device_name: "扩展".into() },
            true,
        ),
        (
            "空间不足",
            ArrivalOutcome::InsufficientSpace {
                device_name: "扩展".into(),
                landing_dir: r"D:\素材".into(),
                required_bytes: 100,
                available_bytes: Some(10),
            },
            true,
        ),
    ];

    for (name, outcome, wants_exit) in cases {
        let step = outcome.next_step();
        if wants_exit {
            assert_ne!(
                step,
                NextStep::Nothing,
                "「{name}」这个结论必须给一条出路——只告知不给下一步等于死路装修了一下"
            );
            assert!(!step.label(Locale::Zh).is_empty(), "「{name}」的出口得有文案");
        } else {
            assert_eq!(step, NextStep::Nothing, "「{name}」不该催用户做什么");
        }
        assert!(!outcome.summary(Locale::Zh).is_empty(), "「{name}」必须有一句人话结论");
    }
}
