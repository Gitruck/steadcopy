#![allow(clippy::unwrap_used, clippy::expect_used)]
//! 语言护栏：英文输出里不许混进中文，中文输出里不许留占位符。
//!
//! 规范：`openspec/changes/add-steadcopy-i18n/specs/i18n/spec.md`
//!
//! 穷尽 `match` 保证「不会漏分支」，这组测试保证「分支里填的确实是译文」——
//! 前者管结构，后者管内容，缺一不可。

use steadcopy_core::device::DeviceKind;
use steadcopy_core::i18n::{has_cjk, Locale, PLACEHOLDERS};
use steadcopy_core::preset::{ArrivalOutcome, PresetMatch, SinkScope};

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

// spec: i18n → Scenario: 英文输出无 CJK
#[test]
fn scenario_i18n_english_output_has_no_cjk() {
    let mut checked = 0;

    for o in all_outcomes() {
        let s = o.summary(Locale::En);
        assert!(!has_cjk(&s), "英文结论里混进了中文：{s}");
        let l = o.next_step().label(Locale::En);
        assert!(!has_cjk(l), "英文出口文案里混进了中文：{l}");
        checked += 2;
    }

    for k in KINDS {
        let l = k.label(Locale::En);
        assert!(!has_cjk(l), "英文设备类型里混进了中文：{l}");
        checked += 1;
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
        let s = m.describe(Locale::En);
        assert!(!has_cjk(&s), "英文匹配描述里混进了中文：{s}");
        checked += 1;
    }

    for sc in [
        SinkScope::ThisDevice,
        SinkScope::ThisKind(DeviceKind::Camera),
        SinkScope::AnyClassified,
    ] {
        let s = sc.describe(Locale::En, "A7M4");
        assert!(!has_cjk(&s), "英文沉淀范围里混进了中文：{s}");
        checked += 1;
    }

    assert!(checked >= 27, "护栏只查了 {checked} 条，覆盖面不够");
}

// spec: i18n → Scenario: 中文输出无占位符
#[test]
fn scenario_i18n_chinese_output_has_no_placeholder() {
    let mut texts: Vec<String> = all_outcomes()
        .iter()
        .map(|o| o.summary(Locale::Zh))
        .collect();
    texts.extend(KINDS.iter().map(|k| k.label(Locale::Zh).to_string()));

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
