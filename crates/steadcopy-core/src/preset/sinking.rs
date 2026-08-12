//! 预设沉淀：把「刚刚这次是怎么拷的」变成常驻规则。
//!
//! 规范：`openspec/changes/add-steadcopy-copy-first-flow/specs/preset-sinking/spec.md`
//!
//! # 为什么是这个时机
//!
//! 预设的价值只在**第二次**兑现，而人在第一次的时候根本想不到自己会有第二次。
//! 所以不能指望用户主动去预设页配——要在他刚刚亲手把意图表达完整的那一刻问他。
//!
//! 传输中是最好的窗口：意图刚说清楚、人正在等、注意力空闲。但**提示必须是行内的**，
//! 不能是弹窗——他可能正盯着进度，也可能拷完就去拔卡了。所以提示从任务开始挂到结果里。
//!
//! # 为什么默认最窄
//!
//! 沉淀是「把一次性的决定变成长期的默认」。放宽匹配范围的代价由**未来的每一次插卡**承担，
//! 而那时用户早忘了自己点过什么。所以默认「就这张卡」，放宽必须是显式动作。

use crate::device::{DeviceKind, DeviceRecord};
use crate::preset::model::{Preset, PresetMatch};
use crate::task::plan::TaskSpec;

/// 沉淀的匹配范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SinkScope {
    /// 就这张卡。**默认**——最窄的匹配误伤不了别的卡
    #[default]
    ThisDevice,
    /// 这一类设备
    ThisKind(DeviceKind),
    /// 任何已分类的源设备
    AnyClassified,
}

impl SinkScope {
    pub fn describe(self, lang: crate::i18n::Locale, device_name: &str) -> String {
        use crate::i18n::Locale;
        match (self, lang) {
            (SinkScope::ThisDevice, Locale::Zh) => format!("就「{device_name}」这张卡"),
            (SinkScope::ThisDevice, Locale::En) => format!("Just \"{device_name}\""),
            (SinkScope::ThisKind(k), Locale::Zh) => format!("所有{}", k.label(lang)),
            (SinkScope::ThisKind(k), Locale::En) => {
                format!("All {}s", k.label(lang).to_lowercase())
            }
            (SinkScope::AnyClassified, _) => lang
                .pick("任何已分类的源设备", "Any identified source device")
                .into(),
        }
    }
}

/// 要不要提示用户沉淀。
///
/// 判据只有一条：**这次的做法和已记住的不一样。** 一致就闭嘴——
/// 一个每次都问「要不要保存」的工具，第三次之后就没人看了。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkSuggestion {
    /// 这次是临时拷贝，没有任何预设记住过它
    NoPreset,
    /// 有预设，但这次的参数被改过。`changed` 列出改了哪些
    Diverged {
        preset_id: String,
        preset_name: String,
        changed: Vec<&'static str>,
    },
    /// 与已记住的一致，不打扰
    None,
}

impl SinkSuggestion {
    pub fn should_show(&self) -> bool {
        !matches!(self, SinkSuggestion::None)
    }
}

/// 判断这次执行要不要提示沉淀。
///
/// `matched` 是本次到达实际匹配到的预设（临时拷贝时为 `None`）。
/// `project_id` 是本次实际用的项目——用来判断预设指向的项目是否被当场换掉。
pub fn should_suggest(
    spec: &TaskSpec,
    matched: Option<&Preset>,
    project_id: Option<&str>,
) -> SinkSuggestion {
    let Some(p) = matched else {
        return SinkSuggestion::NoPreset;
    };

    let mut changed = Vec::new();
    if p.verify != spec.verify {
        changed.push("校验");
    }
    if p.algorithm != spec.algorithm {
        changed.push("算法");
    }
    if p.eject_after != spec.eject_after {
        changed.push("拷完弹出");
    }
    // 预设的 project_id 为 None 表示「用当前项目」，此时换了当前项目不算改预设
    if let (Some(want), Some(actual)) = (p.project_id.as_deref(), project_id) {
        if want != actual {
            changed.push("项目");
        }
    }

    if changed.is_empty() {
        SinkSuggestion::None
    } else {
        SinkSuggestion::Diverged {
            preset_id: p.id.clone(),
            preset_name: p.name.clone(),
            changed,
        }
    }
}

/// 目的地集合与预设指向的项目是否一致。
///
/// 单独一个函数是因为调用方手里的信息形状不同：确认卡片改了目的地时传改后的集合，
/// 临时拷贝根本没有「原集合」可比。判定放在这里，两边口径才不会漂。
pub fn destinations_changed(spec: &TaskSpec, project_destinations: &[std::path::PathBuf]) -> bool {
    let mut a: Vec<_> = spec.destinations.iter().map(|d| &d.root).collect();
    let mut b: Vec<_> = project_destinations.iter().collect();
    a.sort();
    b.sort();
    a != b
}

/// 由一次实际执行生成预设。
///
/// **不重新问任何已经指定过的参数**——用户刚刚才说完，再问一遍是把沉淀变成第二次配置。
pub fn derive_preset(
    spec: &TaskSpec,
    device: &DeviceRecord,
    scope: SinkScope,
    project_id: Option<&str>,
    name: Option<String>,
    lang: crate::i18n::Locale,
) -> Preset {
    let matcher = match scope {
        SinkScope::ThisDevice => PresetMatch::Device {
            device_id: device.id.clone(),
        },
        SinkScope::ThisKind(k) => PresetMatch::Kind { device_kind: k },
        SinkScope::AnyClassified => PresetMatch::AnyClassifiedSource,
    };

    let default_name = match scope {
        SinkScope::ThisDevice => format!("{} → {}", device.display_name(), spec.project),
        SinkScope::ThisKind(k) => format!("{} → {}", k.label(lang), spec.project),
        SinkScope::AnyClassified => {
            format!("{} → {}", lang.pick("任何源卡", "Any source card"), spec.project)
        }
    };

    let mut p = Preset::new(name.unwrap_or(default_name));
    p.matcher = matcher;
    p.project_id = project_id.map(str::to_string);
    p.verify = spec.verify;
    p.algorithm = spec.algorithm;
    p.eject_after = spec.eject_after;
    p
}

/// 沉淀时是否需要顺带收一个设备类型。
///
/// 仅按设备身份匹配的预设**不会**改变「未分类设备停在指认」这个编排结论——
/// 不收类型的话，这条预设下次插卡时根本轮不到它生效，等于白配。
///
/// 所以在这里一次交互解决两件事，而不是去放宽「未分类设备永不自动开跑」那条铁律。
pub fn needs_kind(device: &DeviceRecord) -> bool {
    !device.kind.is_classified()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::HashAlgorithm;
    use crate::manifest::model::SourceRef;
    use crate::organize::{PathTemplate, ScanOptions};
    use crate::task::plan::DestinationSpec;
    use std::path::PathBuf;
    use time::macros::datetime;

    fn at() -> time::OffsetDateTime {
        datetime!(2026-08-11 09:00:00 UTC)
    }

    fn spec_with(verify: bool, algorithm: HashAlgorithm, dests: &[&str]) -> TaskSpec {
        TaskSpec {
            source_root: PathBuf::from(r"E:\"),
            source: SourceRef {
                id: "vol:1".into(),
                display_name: "A7M4".into(),
            },
            project: "婚礼".into(),
            destinations: dests
                .iter()
                .map(|r| DestinationSpec {
                    root: PathBuf::from(r),
                    template: PathTemplate::parse("{项目}/{日期}").expect("模板"),
                    enabled: true,
                })
                .collect(),
            algorithm,
            verify,
            scan: ScanOptions::mirror(),
            retries: 1,
            eject_after: false,
            at: at(),
        }
    }

    fn device(kind: DeviceKind) -> DeviceRecord {
        let mut d = DeviceRecord::new("vol:1", "A7M4", 128, at());
        d.kind = kind;
        d
    }

    // spec: → Scenario: 临时拷贝后提示沉淀
    #[test]
    fn scenario_preset_sinking_suggests_after_adhoc() {
        let s = spec_with(true, HashAlgorithm::Xxh64, &[r"D:\素材"]);
        assert_eq!(should_suggest(&s, None, Some("pjt-1")), SinkSuggestion::NoPreset);
        assert!(SinkSuggestion::NoPreset.should_show());
    }

    // spec: → Scenario: 一致时不打扰
    #[test]
    fn scenario_preset_sinking_stays_quiet_when_unchanged() {
        let s = spec_with(true, HashAlgorithm::Xxh64, &[r"D:\素材"]);
        let mut p = Preset::new("摄影卡");
        p.verify = true;
        p.algorithm = HashAlgorithm::Xxh64;
        p.eject_after = false;
        p.project_id = Some("pjt-1".into());

        assert_eq!(
            should_suggest(&s, Some(&p), Some("pjt-1")),
            SinkSuggestion::None,
            "完全照预设跑就该闭嘴——反复提示保存会让提示本身失去意义"
        );
        assert!(!SinkSuggestion::None.should_show());
    }

    // spec: → Scenario: 参数被改过才提示
    #[test]
    fn scenario_preset_sinking_suggests_only_when_diverged() {
        let mut p = Preset::new("摄影卡");
        p.verify = true;
        p.algorithm = HashAlgorithm::Xxh64;
        p.project_id = Some("pjt-1".into());

        // 关了校验
        let s = spec_with(false, HashAlgorithm::Xxh64, &[r"D:\素材"]);
        match should_suggest(&s, Some(&p), Some("pjt-1")) {
            SinkSuggestion::Diverged { changed, .. } => assert_eq!(changed, vec!["校验"]),
            other => panic!("改了校验就该提示，实际是 {other:?}"),
        }

        // 换了算法
        let s = spec_with(true, HashAlgorithm::Md5, &[r"D:\素材"]);
        match should_suggest(&s, Some(&p), Some("pjt-1")) {
            SinkSuggestion::Diverged { changed, .. } => assert_eq!(changed, vec!["算法"]),
            other => panic!("改了算法就该提示，实际是 {other:?}"),
        }

        // 换了项目
        let s = spec_with(true, HashAlgorithm::Xxh64, &[r"D:\素材"]);
        match should_suggest(&s, Some(&p), Some("pjt-2")) {
            SinkSuggestion::Diverged { changed, .. } => assert_eq!(changed, vec!["项目"]),
            other => panic!("换了项目就该提示，实际是 {other:?}"),
        }

        // 预设指向「当前项目」（None）时，换当前项目不算改预设
        let mut floating = p.clone();
        floating.project_id = None;
        assert_eq!(
            should_suggest(&s, Some(&floating), Some("pjt-2")),
            SinkSuggestion::None,
            "预设本来就写着「用当前项目」，那换当前项目就是它预期的行为"
        );
    }

    // spec: → Scenario: 目的地改了也算不一致
    #[test]
    fn scenario_preset_sinking_destination_change_counts() {
        let s = spec_with(true, HashAlgorithm::Xxh64, &[r"D:\素材", r"F:\备份"]);
        let same = [PathBuf::from(r"F:\备份"), PathBuf::from(r"D:\素材")];
        assert!(!destinations_changed(&s, &same), "只是顺序不同不算改");

        let fewer = [PathBuf::from(r"D:\素材")];
        assert!(destinations_changed(&s, &fewer), "少一个目的地必须算改");

        let other = [PathBuf::from(r"D:\素材"), PathBuf::from(r"X:\临时")];
        assert!(destinations_changed(&s, &other));
    }

    // spec: → Scenario: 默认最窄范围
    #[test]
    fn scenario_preset_sinking_defaults_to_this_device() {
        assert_eq!(SinkScope::default(), SinkScope::ThisDevice);

        let s = spec_with(true, HashAlgorithm::Xxh64, &[r"D:\素材"]);
        let d = device(DeviceKind::Camera);
        let p = derive_preset(&s, &d, SinkScope::default(), Some("pjt-1"), None, crate::i18n::Locale::Zh);
        assert_eq!(
            p.matcher,
            PresetMatch::Device {
                device_id: "vol:1".into()
            },
            "默认必须是最窄的——放宽的代价由未来每一次插卡承担"
        );
    }

    // spec: → Scenario: 范围不自行放宽
    #[test]
    fn scenario_preset_sinking_never_widens_silently() {
        let s = spec_with(true, HashAlgorithm::Xxh64, &[r"D:\素材"]);
        let d = device(DeviceKind::Camera);

        for (scope, want) in [
            (
                SinkScope::ThisDevice,
                PresetMatch::Device {
                    device_id: "vol:1".into(),
                },
            ),
            (
                SinkScope::ThisKind(DeviceKind::Camera),
                PresetMatch::Kind {
                    device_kind: DeviceKind::Camera,
                },
            ),
            (SinkScope::AnyClassified, PresetMatch::AnyClassifiedSource),
        ] {
            let p = derive_preset(&s, &d, scope, Some("pjt-1"), None, crate::i18n::Locale::Zh);
            assert_eq!(p.matcher, want, "生成的范围必须恰好是用户选的那个");
        }
    }

    // spec: → Scenario: 不重新问已指定过的参数
    #[test]
    fn scenario_preset_sinking_carries_over_run_parameters() {
        let s = spec_with(false, HashAlgorithm::Md5, &[r"D:\素材"]);
        let d = device(DeviceKind::Recorder);
        let p = derive_preset(&s, &d, SinkScope::ThisDevice, Some("pjt-9"), None, crate::i18n::Locale::Zh);

        assert!(!p.verify, "这次关了校验，沉淀出来的预设也该是关的");
        assert_eq!(p.algorithm, HashAlgorithm::Md5);
        assert_eq!(p.project_id.as_deref(), Some("pjt-9"));
        assert!(p.enabled, "沉淀出来就该是启用的，否则等于白点");
        assert!(p.name.contains("A7M4"), "名字要能认出来：{}", p.name);
    }

    // spec: → Scenario: 未指认设备一并收类型
    #[test]
    fn scenario_preset_sinking_collects_kind_for_unclassified() {
        assert!(
            needs_kind(&device(DeviceKind::Unclassified)),
            "未指认的设备沉淀时必须顺带收类型，否则这条预设下次根本轮不到生效"
        );
        assert!(!needs_kind(&device(DeviceKind::Camera)));
        assert!(!needs_kind(&device(DeviceKind::Storage)));
    }

    // spec: → Scenario: 沉淀后下次自动匹配
    #[test]
    fn scenario_preset_sinking_result_matches_next_arrival() {
        use crate::preset::matching::select_preset;

        let s = spec_with(true, HashAlgorithm::Xxh64, &[r"D:\素材"]);
        let d = device(DeviceKind::Camera);

        // 沉淀出来的东西要真能被下一次到达匹配上，否则等于白点
        for scope in [
            SinkScope::ThisDevice,
            SinkScope::ThisKind(DeviceKind::Camera),
            SinkScope::AnyClassified,
        ] {
            let p = derive_preset(&s, &d, scope, Some("pjt-1"), None, crate::i18n::Locale::Zh);
            let presets = vec![p.clone()];
            assert_eq!(
                select_preset(&presets, &d).map(|x| &x.id),
                Some(&p.id),
                "{scope:?} 沉淀出来的预设必须能被同一台设备的下次到达匹配上"
            );
        }

        // 换一台别的摄影卡：只有「就这张卡」那条不该命中
        let mut other = DeviceRecord::new("vol:9", "GH6", 64, at());
        other.kind = DeviceKind::Camera;
        let narrow = vec![derive_preset(&s, &d, SinkScope::ThisDevice, Some("pjt-1"), None, crate::i18n::Locale::Zh)];
        assert!(
            select_preset(&narrow, &other).is_none(),
            "「就这张卡」MUST NOT 误伤别的卡——这正是它当默认的理由"
        );
    }

    // spec: → Scenario: 铁律不因沉淀而放宽
    #[test]
    fn scenario_preset_sinking_does_not_relax_classification_rule() {
        // 沉淀只产出 Preset，不碰设备类型也不碰编排。
        // 对一个未分类设备沉淀出的预设，下次到达仍会停在指认——
        // 这正是 needs_kind 存在的理由：靠一次交互补齐，而不是靠放宽铁律。
        let s = spec_with(true, HashAlgorithm::Xxh64, &[r"D:\素材"]);
        let d = device(DeviceKind::Unclassified);
        let p = derive_preset(&s, &d, SinkScope::ThisDevice, Some("pjt-1"), None, crate::i18n::Locale::Zh);

        assert_eq!(
            p.matcher,
            PresetMatch::Device {
                device_id: "vol:1".into()
            }
        );
        assert!(
            needs_kind(&d),
            "生成预设这件事本身不给设备贴类型，所以必须提示调用方去收"
        );
    }
}
