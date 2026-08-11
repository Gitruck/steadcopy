//! 预设匹配：设备 → 预设。**由窄到宽，第一条命中。**
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/preset-autorun/spec.md`
//! → Requirement: 预设匹配顺序

use crate::device::{DeviceKind, DeviceRecord};
use crate::preset::model::{Preset, PresetMatch};

/// 为一个设备选出应当使用的预设。
///
/// 扫描顺序：**指定设备 → 设备类型 → 任何已分类源设备**。
/// 同一档内有多条时按列表顺序取第一条（列表顺序即用户排定的优先级）。
///
/// 返回 `None` 表示**无预设**——调用方 MUST 如实告知用户，
/// **MUST NOT** 回落到某个默认项目静默开跑。
pub fn select_preset<'a>(presets: &'a [Preset], device: &DeviceRecord) -> Option<&'a Preset> {
    // 未分类与被忽略的设备不参与任何匹配。
    // 未分类的处理在到达编排里更早一步就停住了，这里是第二道防线。
    if !device.kind.is_classified() || device.kind == DeviceKind::Ignored {
        return None;
    }

    for level in 0..=2u8 {
        if let Some(p) = presets
            .iter()
            .filter(|p| p.enabled)
            .filter(|p| p.matcher.narrowness() == level)
            .find(|p| matches(&p.matcher, device))
        {
            return Some(p);
        }
    }
    None
}

fn matches(m: &PresetMatch, device: &DeviceRecord) -> bool {
    match m {
        PresetMatch::Device { device_id } => device_id == &device.id,
        PresetMatch::Kind { device_kind } => *device_kind == device.kind,
        PresetMatch::AnyClassifiedSource => {
            matches!(device.kind, DeviceKind::Camera | DeviceKind::Recorder)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn dev(id: &str, kind: DeviceKind) -> DeviceRecord {
        let mut d = DeviceRecord::new(id, "卡", 128, datetime!(2026-08-10 09:00:00 UTC));
        d.kind = kind;
        d
    }

    // spec: preset-autorun → 预设匹配顺序 → Scenario: 窄规则优先于宽规则
    #[test]
    fn scenario_preset_autorun_narrow_beats_broad() {
        let presets = vec![
            // 故意把宽的放在列表前面——顺序不该压过窄度
            Preset::new("全部摄影卡").matching(PresetMatch::Kind {
                device_kind: DeviceKind::Camera,
            }),
            Preset::new("任何源").matching(PresetMatch::AnyClassifiedSource),
            Preset::new("这张卡").matching(PresetMatch::Device {
                device_id: "vol:1".into(),
            }),
        ];
        let d = dev("vol:1", DeviceKind::Camera);
        assert_eq!(
            select_preset(&presets, &d).map(|p| p.name.as_str()),
            Some("这张卡")
        );
    }

    #[test]
    fn scenario_preset_autorun_kind_beats_any() {
        let presets = vec![
            Preset::new("任何源").matching(PresetMatch::AnyClassifiedSource),
            Preset::new("全部录音卡").matching(PresetMatch::Kind {
                device_kind: DeviceKind::Recorder,
            }),
        ];
        let d = dev("vol:1", DeviceKind::Recorder);
        assert_eq!(
            select_preset(&presets, &d).map(|p| p.name.as_str()),
            Some("全部录音卡")
        );
    }

    #[test]
    fn scenario_preset_autorun_same_level_takes_list_order() {
        let presets = vec![
            Preset::new("甲").matching(PresetMatch::Kind {
                device_kind: DeviceKind::Camera,
            }),
            Preset::new("乙").matching(PresetMatch::Kind {
                device_kind: DeviceKind::Camera,
            }),
        ];
        let d = dev("vol:1", DeviceKind::Camera);
        assert_eq!(
            select_preset(&presets, &d).map(|p| p.name.as_str()),
            Some("甲"),
            "同档内应按列表顺序取第一条"
        );
    }

    // spec: → Scenario: 未启用的预设不参与匹配
    #[test]
    fn scenario_preset_autorun_disabled_preset_ignored() {
        let mut presets = vec![
            Preset::new("这张卡").matching(PresetMatch::Device {
                device_id: "vol:1".into(),
            }),
            Preset::new("全部摄影卡").matching(PresetMatch::Kind {
                device_kind: DeviceKind::Camera,
            }),
        ];
        let d = dev("vol:1", DeviceKind::Camera);
        assert_eq!(
            select_preset(&presets, &d).map(|p| p.name.as_str()),
            Some("这张卡")
        );
        // 停用最窄的那条 → 回落到下一档，而不是无匹配
        presets[0].enabled = false;
        assert_eq!(
            select_preset(&presets, &d).map(|p| p.name.as_str()),
            Some("全部摄影卡")
        );
        // 全停用 → 无匹配
        presets[1].enabled = false;
        assert!(select_preset(&presets, &d).is_none());
    }

    // spec: → Scenario: 无匹配时不臆造默认
    #[test]
    fn scenario_preset_autorun_no_match_returns_none() {
        let presets = vec![Preset::new("只认录音卡").matching(PresetMatch::Kind {
            device_kind: DeviceKind::Recorder,
        })];
        let d = dev("vol:1", DeviceKind::Camera);
        assert!(
            select_preset(&presets, &d).is_none(),
            "无匹配 MUST 返回 None，MUST NOT 回落到不匹配的预设"
        );
        assert!(select_preset(&[], &d).is_none());
    }

    #[test]
    fn scenario_preset_autorun_unclassified_never_matches() {
        // 第二道防线：未分类设备即便有「任何源」预设也不匹配
        let presets = vec![Preset::new("任何源").matching(PresetMatch::AnyClassifiedSource)];
        let d = dev("vol:new", DeviceKind::Unclassified);
        assert!(select_preset(&presets, &d).is_none());
    }

    #[test]
    fn scenario_preset_autorun_ignored_never_matches() {
        let presets = vec![
            Preset::new("任何源").matching(PresetMatch::AnyClassifiedSource),
            Preset::new("这张卡").matching(PresetMatch::Device {
                device_id: "vol:1".into(),
            }),
        ];
        let d = dev("vol:1", DeviceKind::Ignored);
        assert!(
            select_preset(&presets, &d).is_none(),
            "被忽略的设备 MUST NOT 匹配任何预设，包括指名道姓的那条"
        );
    }

    #[test]
    fn scenario_preset_autorun_any_source_excludes_storage() {
        // 「任何已分类的源设备」指的是卡，不含素材盘——素材盘通常同时是目的地
        let presets = vec![Preset::new("任何源").matching(PresetMatch::AnyClassifiedSource)];
        assert!(select_preset(&presets, &dev("v", DeviceKind::Camera)).is_some());
        assert!(select_preset(&presets, &dev("v", DeviceKind::Recorder)).is_some());
        assert!(select_preset(&presets, &dev("v", DeviceKind::Storage)).is_none());
    }

    #[test]
    fn scenario_preset_autorun_kind_match_is_exact() {
        let presets = vec![Preset::new("素材盘").matching(PresetMatch::Kind {
            device_kind: DeviceKind::Storage,
        })];
        assert!(select_preset(&presets, &dev("v", DeviceKind::Storage)).is_some());
        assert!(select_preset(&presets, &dev("v", DeviceKind::Camera)).is_none());
    }
}
