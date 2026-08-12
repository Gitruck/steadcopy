//! 任务阶段模型。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/copy-engine/spec.md`
//! → Requirement: 任务阶段模型
//!
//! 阶段是**离散**的，百分比是阶段**内**的。两者 MUST NOT 混淆——
//! 「拷贝 78%、校验 21%」是两条独立进度，不是一条 99%。

use serde::{Deserialize, Serialize};

/// 任务阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStage {
    /// 扫描源、算出待拷集合与落地路径
    Planning,
    /// 空间与目的地可用性预检
    Prechecking,
    Copying,
    Verifying,
    /// 落清单、收尾
    Finishing,
    Finished,
}

impl TaskStage {
    /// 稳定的机读代码。**界面用它做判定**——拿本地化后的文案去比对，
    /// 换个语言判定就会静默失效，而且失效得毫无征兆。
    pub const fn code(self) -> &'static str {
        match self {
            TaskStage::Planning => "planning",
            TaskStage::Prechecking => "prechecking",
            TaskStage::Copying => "copying",
            TaskStage::Verifying => "verifying",
            TaskStage::Finishing => "finishing",
            TaskStage::Finished => "finished",
        }
    }

    /// 给人看的名字。只用于呈现，MUST NOT 参与任何判定。
    pub const fn label(self, lang: crate::i18n::Locale) -> &'static str {
        match self {
            TaskStage::Planning => lang.pick("规划", "Planning"),
            TaskStage::Prechecking => lang.pick("预检", "Pre-check"),
            TaskStage::Copying => lang.pick("拷贝", "Copying"),
            TaskStage::Verifying => lang.pick("校验", "Verifying"),
            TaskStage::Finishing => lang.pick("收尾", "Finishing"),
            TaskStage::Finished => lang.pick("完成", "Done"),
        }
    }

    /// 开启校验时的完整阶段序列。
    pub const WITH_VERIFY: [TaskStage; 6] = [
        TaskStage::Planning,
        TaskStage::Prechecking,
        TaskStage::Copying,
        TaskStage::Verifying,
        TaskStage::Finishing,
        TaskStage::Finished,
    ];

    /// 关闭校验时跳过 `Verifying`。
    pub const WITHOUT_VERIFY: [TaskStage; 5] = [
        TaskStage::Planning,
        TaskStage::Prechecking,
        TaskStage::Copying,
        TaskStage::Finishing,
        TaskStage::Finished,
    ];
}

/// 进度与阶段事件。
///
/// 事件的**限流由消费方负责**——引擎按真实进展发，界面层决定多久渲染一次。
/// 引擎内不限流是刻意的：把节流策略焊死在引擎里，CLI 与自动化就没法拿到全量事件了。
#[derive(Debug, Clone, PartialEq)]
pub enum StageEvent {
    Stage(TaskStage),
    /// 阶段内进度。`done` / `total` 单位由阶段决定（拷贝是字节，校验是文件数）。
    Progress {
        stage: TaskStage,
        done: u64,
        total: u64,
        current: Option<String>,
    },
    /// 单个文件失败——**立即**上报，不等任务结束
    FileFailed { relative_path: String, reason: String },
    /// 需要让用户知道的提示（如账本降级为全量）
    Notice(String),
}

/// 计算阶段内百分比。
///
/// `total` 为 0 时返回 100（空任务视为已完成），**绝不**产生除零或超出 0..=100 的值。
pub fn percent(done: u64, total: u64) -> f64 {
    if total == 0 {
        return 100.0;
    }
    ((done as f64 / total as f64) * 100.0).clamp(0.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: copy-engine → 任务阶段模型 → Scenario: 关闭校验跳过校验阶段
    #[test]
    fn scenario_copy_engine_stage_sequence_skips_verify_when_disabled() {
        assert!(TaskStage::WITH_VERIFY.contains(&TaskStage::Verifying));
        assert!(!TaskStage::WITHOUT_VERIFY.contains(&TaskStage::Verifying));
        // 两条序列都以 Planning 开头、Finished 结尾
        assert_eq!(TaskStage::WITH_VERIFY[0], TaskStage::Planning);
        assert_eq!(
            *TaskStage::WITH_VERIFY.last().expect("末阶段"),
            TaskStage::Finished
        );
        assert_eq!(TaskStage::WITHOUT_VERIFY[0], TaskStage::Planning);
        assert_eq!(
            *TaskStage::WITHOUT_VERIFY.last().expect("末阶段"),
            TaskStage::Finished
        );
    }

    #[test]
    fn scenario_copy_engine_stage_labels_are_localized() {
        use crate::i18n::{has_cjk, Locale};
        for s in TaskStage::WITH_VERIFY {
            assert!(!s.label(Locale::Zh).is_empty());
            assert!(!s.label(Locale::En).is_empty());
            assert!(has_cjk(s.label(Locale::Zh)), "中文名没翻：{}", s.code());
            assert!(!has_cjk(s.label(Locale::En)), "英文名混了中文：{}", s.code());

            // 代码是判定用的，必须是稳定的 ASCII 且不随语言变
            assert!(s.code().is_ascii(), "阶段代码必须是 ASCII：{}", s.code());
            assert_ne!(s.code(), s.label(Locale::Zh), "代码与文案必须是两样东西");
        }
    }

    // spec: → 进度上报与限流 → Scenario: 零字节文件与空任务
    #[test]
    fn scenario_copy_engine_percent_never_divides_by_zero() {
        assert_eq!(percent(0, 0), 100.0);
        assert_eq!(percent(0, 100), 0.0);
        assert_eq!(percent(50, 100), 50.0);
        assert_eq!(percent(100, 100), 100.0);
        // 越界输入被夹住，不产生 NaN 或 >100
        assert_eq!(percent(200, 100), 100.0);
        for (d, t) in [(0u64, 0u64), (1, 0), (u64::MAX, 1), (1, u64::MAX)] {
            let p = percent(d, t);
            assert!(p.is_finite(), "percent({d},{t}) 不应是 NaN/Inf");
            assert!((0.0..=100.0).contains(&p), "percent({d},{t}) = {p} 越界");
        }
    }
}
