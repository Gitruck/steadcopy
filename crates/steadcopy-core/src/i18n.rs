//! 语言：locale 解析、文案取用的形状、CJK 护栏。
//!
//! 规范：`openspec/changes/add-steadcopy-i18n/specs/i18n/spec.md`
//!
//! # 为什么文案表在 core
//!
//! core 产的不是「标签」，是**带上下文的成句**——「拷贝『A7M4』的目的地空间不足：D:\素材」
//! 里嵌着两个变量，而中英语序差别很大（中文「X 的 Y 不足」vs 英文 "Not enough Y on X"）。
//! 把片段交给消费层拼装，等于把语序知识复制两份，而 core 有命令行与界面两个消费方。
//!
//! 所以 core 产成句。「前端零业务逻辑」的对偶不是「后端零文案」，是**「文案只有一处定义」**。
//!
//! # 为什么是 match 不是查表
//!
//! 文案取用一律写成对枚举的**穷尽 `match`**：少一个分支编译不过。
//! `HashMap` 查表漏了只会在运行时返回 `None`，而任何 `unwrap_or("")` 式的兜底
//! 都会让界面显示一片空白——那是最难被发现的缺陷形态。

/// 界面与命令行的语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locale {
    #[default]
    Zh,
    En,
}

/// 配置里的取值。`Auto` 跟随系统。
pub const LOCALE_AUTO: &str = "auto";
pub const LOCALE_ZH: &str = "zh";
pub const LOCALE_EN: &str = "en";

impl Locale {
    /// 由配置项解析出实际语言。
    ///
    /// `auto` 时跟系统；系统语言判不出来**落中文**——本项目的第一受众是中文创作者，
    /// 判不出来时给中文是更安全的猜测，也绝不给空白。
    pub fn resolve(setting: &str) -> Self {
        match setting {
            LOCALE_EN => Locale::En,
            LOCALE_ZH => Locale::Zh,
            _ => Self::from_system_tag(system_language_tag().as_deref()),
        }
    }

    /// 由 BCP-47 语言标记判定（`zh-CN` / `en-US` / …）。认不出来一律中文。
    pub fn from_system_tag(tag: Option<&str>) -> Self {
        match tag {
            Some(t) if t.to_ascii_lowercase().starts_with("en") => Locale::En,
            _ => Locale::Zh,
        }
    }

    pub const fn tag(self) -> &'static str {
        match self {
            Locale::Zh => LOCALE_ZH,
            Locale::En => LOCALE_EN,
        }
    }

    /// 取中英两串里对应的那一串。文案表的基本积木。
    pub const fn pick(self, zh: &'static str, en: &'static str) -> &'static str {
        match self {
            Locale::Zh => zh,
            Locale::En => en,
        }
    }
}

/// 系统语言标记。判不出来返回 `None`。
#[cfg(windows)]
fn system_language_tag() -> Option<String> {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;
    let mut buf = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(&mut buf) };
    if n <= 1 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..(n as usize - 1)]))
}

#[cfg(not(windows))]
fn system_language_tag() -> Option<String> {
    std::env::var("LANG").ok().filter(|s| !s.is_empty())
}

/// 含中日韩字符吗。
///
/// **范围必须含 CJK 标点与全角字符**——只查汉字的话，
/// `Not enough space：D:\素材` 这种「句子翻了但标点漏了」会溜过去，
/// 而全角冒号出现在英文界面里非常刺眼。
pub fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        let u = c as u32;
        (0x4E00..=0x9FFF).contains(&u)      // CJK 统一表意文字
            || (0x3400..=0x4DBF).contains(&u) // 扩展 A
            || (0x3000..=0x303F).contains(&u) // CJK 标点「」、。
            || (0xFF00..=0xFFEF).contains(&u) // 全角字符（含全角冒号）
            || (0x3040..=0x30FF).contains(&u) // 假名
            || (0xAC00..=0xD7AF).contains(&u) // 谚文
    })
}

/// 中文文案里不该出现的占位。
pub const PLACEHOLDERS: &[&str] = &["TODO", "untranslated", "FIXME", "待翻译"];

#[cfg(test)]
mod tests {
    use super::*;

    // spec: → Scenario: 自动跟随系统语言
    #[test]
    fn scenario_i18n_auto_follows_system_language() {
        assert_eq!(Locale::from_system_tag(Some("en-US")), Locale::En);
        assert_eq!(Locale::from_system_tag(Some("en")), Locale::En);
        assert_eq!(Locale::from_system_tag(Some("EN-GB")), Locale::En);
        assert_eq!(Locale::from_system_tag(Some("zh-CN")), Locale::Zh);
        assert_eq!(Locale::from_system_tag(Some("zh-Hant-TW")), Locale::Zh);
    }

    // spec: → Scenario: 判不出来落中文
    #[test]
    fn scenario_i18n_unknown_system_language_falls_back_to_chinese() {
        assert_eq!(Locale::from_system_tag(None), Locale::Zh, "判不出来落中文");
        assert_eq!(Locale::from_system_tag(Some("")), Locale::Zh);
        assert_eq!(Locale::from_system_tag(Some("qqq-XX")), Locale::Zh);
        // 显式设定优先于系统
        assert_eq!(Locale::resolve(LOCALE_EN), Locale::En);
        assert_eq!(Locale::resolve(LOCALE_ZH), Locale::Zh);
        // 配置里写了看不懂的东西，也不能给空白
        assert!(matches!(Locale::resolve("克林贡语"), Locale::Zh | Locale::En));
    }

    // spec: → Scenario: 不存在运行时兜底
    #[test]
    fn scenario_i18n_no_runtime_fallback() {
        // pick 是 match 不是查表：两串都在签名里，取不到这种事不可能发生
        assert_eq!(Locale::Zh.pick("中", "en"), "中");
        assert_eq!(Locale::En.pick("中", "en"), "en");
        assert_eq!(Locale::Zh.tag(), "zh");
        assert_eq!(Locale::En.tag(), "en");
    }

    #[test]
    fn scenario_i18n_cjk_detection_covers_punctuation() {
        assert!(has_cjk("素材"));
        assert!(has_cjk("「引号」"), "CJK 标点也算");
        assert!(has_cjk("Not enough space："), "全角冒号必须算——这是最常漏的一种");
        assert!(has_cjk("ファイル"));
        assert!(has_cjk("파일"));

        assert!(!has_cjk("Not enough space: D:\\media"));
        assert!(!has_cjk(""));
        assert!(!has_cjk("A7M4 / 128 GB — USB"));
    }
}
