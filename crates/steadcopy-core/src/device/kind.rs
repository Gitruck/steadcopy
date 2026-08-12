//! 设备类型与记忆库记录。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/device-registry/spec.md`
//! → Requirement: 设备类型与记忆库
//!
//! 铁律：**首次见到的设备是「未分类」，不自动开始任何拷贝。**
//! 对未知设备自动写入的风险不可接受——哪怕用户开了无人值守。

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    /// 尚未指认——**不会**触发任何自动流程
    Unclassified,
    Camera,
    Recorder,
    /// 素材盘（移动硬盘 / SSD），可作源也常作目的地
    Storage,
    /// 黑名单：插上不再打扰，但仍在管理列表里可取消
    Ignored,
}

impl DeviceKind {
    pub const fn label(self, lang: crate::i18n::Locale) -> &'static str {
        match self {
            DeviceKind::Unclassified => lang.pick("未分类", "Unclassified"),
            DeviceKind::Camera => lang.pick("摄影卡", "Camera card"),
            DeviceKind::Recorder => lang.pick("录音卡", "Recorder card"),
            DeviceKind::Storage => lang.pick("素材盘", "Media drive"),
            DeviceKind::Ignored => lang.pick("忽略", "Ignored"),
        }
    }

    /// 该类型是否会触发插卡提示。
    pub const fn prompts_on_arrival(self) -> bool {
        matches!(self, DeviceKind::Camera | DeviceKind::Recorder)
    }

    /// 该类型是否已被用户指认过（未分类 = 没有）。
    pub const fn is_classified(self) -> bool {
        !matches!(self, DeviceKind::Unclassified)
    }
}

/// 记忆库里的一条设备记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRecord {
    /// 复合身份（见 `Volume::composite_id`）
    pub id: String,
    /// 用户自定义名。为空时用卷标
    pub custom_name: String,
    pub kind: DeviceKind,
    #[serde(with = "crate::serde_time")]
    pub last_seen: OffsetDateTime,
    /// 重名时的区分序号（1 表示无后缀）
    pub instance: u32,
    /// 最近一次见到时的卷标与容量，用于界面辨认
    pub last_label: String,
    pub total_bytes: u64,
}

impl DeviceRecord {
    pub fn new(id: impl Into<String>, label: impl Into<String>, total_bytes: u64, at: OffsetDateTime) -> Self {
        let label = label.into();
        Self {
            id: id.into(),
            custom_name: label.clone(),
            kind: DeviceKind::Unclassified,
            last_seen: at,
            instance: 1,
            last_label: label,
            total_bytes,
        }
    }

    /// 界面上显示的名字（重名时带序号）。
    pub fn display_name(&self) -> String {
        if self.instance <= 1 {
            self.custom_name.clone()
        } else {
            format!("{} ({})", self.custom_name, self.instance)
        }
    }
}

/// 在既有记录中为一个新名字分配区分序号。
pub fn next_instance(existing: &[DeviceRecord], name: &str) -> u32 {
    let max = existing
        .iter()
        .filter(|r| r.custom_name == name)
        .map(|r| r.instance)
        .max()
        .unwrap_or(0);
    max + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn at() -> OffsetDateTime {
        datetime!(2026-08-08 09:30:00 UTC)
    }

    // spec: device-registry → 设备类型与记忆库 → Scenario: 新设备待指认
    #[test]
    fn scenario_device_registry_new_device_is_unclassified() {
        let r = DeviceRecord::new("vol:1", "A7M4", 128, at());
        assert_eq!(r.kind, DeviceKind::Unclassified);
        assert!(!r.kind.is_classified());
        assert!(
            !r.kind.prompts_on_arrival(),
            "未分类设备 MUST NOT 触发自动拷贝流程"
        );
    }

    // spec: → Scenario: 忽略类型不触发任何流程
    #[test]
    fn scenario_device_registry_ignored_does_not_prompt_but_stays_listed() {
        assert!(!DeviceKind::Ignored.prompts_on_arrival());
        // 已分类：所以它出现在管理列表里、可被取消忽略
        assert!(DeviceKind::Ignored.is_classified());
    }

    #[test]
    fn scenario_device_registry_classified_source_kinds_prompt() {
        assert!(DeviceKind::Camera.prompts_on_arrival());
        assert!(DeviceKind::Recorder.prompts_on_arrival());
        // 素材盘常常同时是目的地，插上不该每次弹拷贝提示
        assert!(!DeviceKind::Storage.prompts_on_arrival());
    }

    // spec: → Scenario: 重名自动编号
    #[test]
    fn scenario_device_registry_duplicate_names_get_instance_numbers() {
        let mut a = DeviceRecord::new("vol:1", "A7M4主卡", 128, at());
        assert_eq!(a.display_name(), "A7M4主卡");

        let existing = vec![a.clone()];
        let mut b = DeviceRecord::new("vol:2", "A7M4主卡", 128, at());
        b.instance = next_instance(&existing, "A7M4主卡");
        assert_eq!(b.instance, 2);
        assert_eq!(b.display_name(), "A7M4主卡 (2)");
        assert_ne!(a.display_name(), b.display_name(), "重名必须可区分");

        // 第三个继续递增
        let existing = vec![a.clone(), b.clone()];
        let mut c = DeviceRecord::new("vol:3", "A7M4主卡", 128, at());
        c.instance = next_instance(&existing, "A7M4主卡");
        assert_eq!(c.instance, 3);

        // 改名后不再重名
        a.custom_name = "A7M4备卡".into();
        assert_eq!(a.display_name(), "A7M4备卡");
    }

    #[test]
    fn scenario_device_registry_labels_are_chinese() {
        for k in [
            DeviceKind::Unclassified,
            DeviceKind::Camera,
            DeviceKind::Recorder,
            DeviceKind::Storage,
            DeviceKind::Ignored,
        ] {
            assert!(!k.label(crate::i18n::Locale::Zh).is_ascii(), "类型名应为中文：{}", k.label(crate::i18n::Locale::Zh));
        }
    }

    #[test]
    fn scenario_device_registry_record_roundtrips() {
        let r = DeviceRecord::new("vol:1", "A7M4", 128, at());
        let json = serde_json::to_string(&r).expect("序列化");
        let back: DeviceRecord = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(back, r);
    }
}
