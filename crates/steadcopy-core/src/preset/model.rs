//! 预设任务：把「什么设备」与「怎么拷」绑在一起的规则。
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/preset-autorun/spec.md`
//! → Requirement: 预设任务模型
//!
//! 这是「插卡即跑」的规则本体。没有它，认出了设备也不知道该怎么拷。

use serde::{Deserialize, Serialize};

use crate::device::DeviceKind;
use crate::engine::HashAlgorithm;

/// 预设的匹配条件。三档**由窄到宽**，顺序即优先级。
///
/// 刻意不加正交的「优先级数字」字段——三档天然有序，再加一个就是冗余，
/// 而且会制造「窄规则优先级低于宽规则」这种没人想得明白的状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PresetMatch {
    /// 匹配某一张具体的已记忆设备
    Device { device_id: String },
    /// 匹配某一类设备（全部摄影卡 / 全部录音卡 / 全部素材盘）
    Kind { device_kind: DeviceKind },
    /// 匹配任何已分类的源设备
    AnyClassifiedSource,
}

impl PresetMatch {
    /// 匹配的「窄度」等级，越小越窄、越优先。
    pub const fn narrowness(&self) -> u8 {
        match self {
            PresetMatch::Device { .. } => 0,
            PresetMatch::Kind { .. } => 1,
            PresetMatch::AnyClassifiedSource => 2,
        }
    }

    pub fn describe(&self, lang: crate::i18n::Locale) -> String {
        use crate::i18n::Locale;
        match self {
            PresetMatch::Device { .. } => lang.pick("指定设备", "A specific device").into(),
            PresetMatch::Kind { device_kind } => match lang {
                Locale::Zh => format!("全部{}", device_kind.label(lang)),
                Locale::En => format!("All {}s", device_kind.label(lang).to_lowercase()),
            },
            PresetMatch::AnyClassifiedSource => lang
                .pick("任何已分类的源设备", "Any identified source device")
                .into(),
        }
    }
}

/// 一条预设任务。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(rename = "match")]
    pub matcher: PresetMatch,
    /// 用哪个项目。`None` 表示用当前项目
    pub project_id: Option<String>,
    pub verify: bool,
    pub algorithm: HashAlgorithm,
    /// 拷完并校验通过后自动安全弹出
    pub eject_after: bool,
}

impl Preset {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: crate::config::model::new_id("pst"),
            name: name.into(),
            enabled: true,
            matcher: PresetMatch::AnyClassifiedSource,
            project_id: None,
            verify: true,
            algorithm: HashAlgorithm::Xxh64,
            eject_after: false,
        }
    }

    pub fn matching(mut self, matcher: PresetMatch) -> Self {
        self.matcher = matcher;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: preset-autorun → 预设任务模型 → Scenario: 三档匹配条件
    #[test]
    fn scenario_preset_autorun_three_match_kinds_roundtrip() {
        let cases = [
            PresetMatch::Device {
                device_id: "vol:1".into(),
            },
            PresetMatch::Kind {
                device_kind: DeviceKind::Camera,
            },
            PresetMatch::AnyClassifiedSource,
        ];
        for m in cases {
            let p = Preset::new("测试").matching(m.clone());
            let json = serde_json::to_string(&p).expect("序列化");
            let back: Preset = serde_json::from_str(&json).expect("反序列化");
            assert_eq!(back.matcher, m, "往返后匹配条件应一致：{json}");
        }
    }

    #[test]
    fn scenario_preset_autorun_narrowness_ordering() {
        let dev = PresetMatch::Device {
            device_id: "x".into(),
        };
        let kind = PresetMatch::Kind {
            device_kind: DeviceKind::Camera,
        };
        let any = PresetMatch::AnyClassifiedSource;
        assert!(dev.narrowness() < kind.narrowness());
        assert!(kind.narrowness() < any.narrowness());
    }

    #[test]
    fn scenario_preset_autorun_defaults_are_safe() {
        let p = Preset::new("默认预设");
        assert!(p.enabled);
        assert!(p.verify, "默认必须开校验");
        assert!(!p.eject_after, "自动弹出默认关");
        assert_eq!(p.algorithm, HashAlgorithm::Xxh64);
    }

    #[test]
    fn scenario_preset_autorun_match_description_is_chinese() {
        assert_eq!(
            PresetMatch::Kind {
                device_kind: DeviceKind::Camera
            }
            .describe(crate::i18n::Locale::Zh),
            "全部摄影卡"
        );
        assert!(!PresetMatch::AnyClassifiedSource.describe(crate::i18n::Locale::Zh).is_ascii());
    }
}
