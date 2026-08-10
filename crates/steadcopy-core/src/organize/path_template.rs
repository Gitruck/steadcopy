//! 路径模板：占位符解析、校验、渲染、净化。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/organize-rules/spec.md`
//! → Requirement: 路径模板与占位符 / 路径模板实时预览
//!
//! 铁律：渲染与预览走**同一个函数**。前端 MUST NOT 自己实现一份，否则预览与实际必然漂移。

use std::fmt;

use time::OffsetDateTime;

/// 模板中可用的占位符。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placeholder {
    Project,
    Date,
    Device,
    Card,
    HalfDay,
    Year,
    Month,
    Day,
}

impl Placeholder {
    /// 模板里书写的字面量（不含花括号）。
    pub const fn token(self) -> &'static str {
        match self {
            Placeholder::Project => "项目",
            Placeholder::Date => "日期",
            Placeholder::Device => "设备",
            Placeholder::Card => "卡",
            Placeholder::HalfDay => "时段",
            Placeholder::Year => "年",
            Placeholder::Month => "月",
            Placeholder::Day => "日",
        }
    }

    pub fn from_token(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|p| p.token() == s)
    }

    pub const ALL: [Placeholder; 8] = [
        Placeholder::Project,
        Placeholder::Date,
        Placeholder::Device,
        Placeholder::Card,
        Placeholder::HalfDay,
        Placeholder::Year,
        Placeholder::Month,
        Placeholder::Day,
    ];

    /// 至少要出现其中之一，否则不同来源的素材会混进同一目录。
    pub const REQUIRED_ANY: [Placeholder; 3] =
        [Placeholder::Project, Placeholder::Date, Placeholder::Device];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    /// 一个必需占位符都没有
    MissingRequiredPlaceholder,
    /// 出现了不认识的占位符
    UnknownPlaceholder(String),
    /// 花括号没有配对
    UnbalancedBrace,
    /// 模板渲染后不含任何有效路径段
    EmptyTemplate,
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemplateError::MissingRequiredPlaceholder => write!(
                f,
                "模板至少要包含 {{项目}}、{{日期}}、{{设备}} 三者之一，否则不同来源的素材会混进同一个目录"
            ),
            TemplateError::UnknownPlaceholder(t) => {
                let all: Vec<String> = Placeholder::ALL
                    .iter()
                    .map(|p| format!("{{{}}}", p.token()))
                    .collect();
                write!(f, "不认识的占位符 {{{t}}}，可用的有：{}", all.join(" "))
            }
            TemplateError::UnbalancedBrace => write!(f, "花括号没有配对"),
            TemplateError::EmptyTemplate => write!(f, "模板渲染后是空的，至少要有一层目录"),
        }
    }
}

impl std::error::Error for TemplateError {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Literal(String),
    Ph(Placeholder),
}

/// 已解析并校验通过的路径模板。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathTemplate {
    pieces: Vec<Piece>,
    raw: String,
}

/// 渲染上下文。时间统一由调用方传入，便于测试与保证同一任务内取值一致。
#[derive(Debug, Clone)]
pub struct RenderContext {
    pub project: String,
    pub device: String,
    pub card: String,
    pub at: OffsetDateTime,
}

impl PathTemplate {
    /// 解析并校验模板。非法模板在此处被拒，**不允许**拖到拷贝时才失败。
    pub fn parse(raw: &str) -> Result<Self, TemplateError> {
        let mut pieces = Vec::new();
        let mut literal = String::new();
        let mut chars = raw.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '{' => {
                    if !literal.is_empty() {
                        pieces.push(Piece::Literal(std::mem::take(&mut literal)));
                    }
                    let mut token = String::new();
                    let mut closed = false;
                    for c in chars.by_ref() {
                        if c == '}' {
                            closed = true;
                            break;
                        }
                        if c == '{' {
                            return Err(TemplateError::UnbalancedBrace);
                        }
                        token.push(c);
                    }
                    if !closed {
                        return Err(TemplateError::UnbalancedBrace);
                    }
                    match Placeholder::from_token(&token) {
                        Some(p) => pieces.push(Piece::Ph(p)),
                        None => return Err(TemplateError::UnknownPlaceholder(token)),
                    }
                }
                '}' => return Err(TemplateError::UnbalancedBrace),
                _ => literal.push(c),
            }
        }
        if !literal.is_empty() {
            pieces.push(Piece::Literal(literal));
        }

        let has_required = pieces.iter().any(|p| match p {
            Piece::Ph(ph) => Placeholder::REQUIRED_ANY.contains(ph),
            Piece::Literal(_) => false,
        });
        if !has_required {
            return Err(TemplateError::MissingRequiredPlaceholder);
        }

        let tpl = Self {
            pieces,
            raw: raw.to_string(),
        };

        // 用一个探针上下文渲染一次，确保不会产出空路径。
        if tpl.render_inner(&probe_context()).is_empty() {
            return Err(TemplateError::EmptyTemplate);
        }

        Ok(tpl)
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// 渲染为相对路径的各段。段内已完成非法字符净化与保留名改写。
    pub fn render_segments(&self, ctx: &RenderContext) -> Vec<String> {
        self.render_inner(ctx)
    }

    /// 渲染为以 `/` 分隔的相对路径（用于预览与展示）。
    pub fn render(&self, ctx: &RenderContext) -> String {
        self.render_inner(ctx).join("/")
    }

    fn render_inner(&self, ctx: &RenderContext) -> Vec<String> {
        let mut flat = String::new();
        for piece in &self.pieces {
            match piece {
                // 只有模板作者写下的分隔符才算分隔符
                Piece::Literal(s) => flat.push_str(s),
                // 占位符的**值**先做值级净化：值里的分隔符不得凭空多出一层目录，
                // 也不得成为 `..` 这类路径穿越的入口
                Piece::Ph(p) => flat.push_str(&sanitize_value(&substitute(*p, ctx))),
            }
        }
        // 反斜杠与正斜杠一律视为分隔符，避免用户写 Windows 风格模板时产生单段怪路径。
        flat.split(['/', '\\'])
            .map(sanitize_segment)
            .filter(|s| !s.is_empty())
            .collect()
    }
}

fn probe_context() -> RenderContext {
    RenderContext {
        project: "项目".into(),
        device: "设备".into(),
        card: "卡".into(),
        at: OffsetDateTime::UNIX_EPOCH,
    }
}

fn substitute(p: Placeholder, ctx: &RenderContext) -> String {
    match p {
        Placeholder::Project => ctx.project.clone(),
        Placeholder::Device => ctx.device.clone(),
        Placeholder::Card => ctx.card.clone(),
        Placeholder::Date => format!(
            "{:04}-{:02}-{:02}",
            ctx.at.year(),
            ctx.at.month() as u8,
            ctx.at.day()
        ),
        Placeholder::Year => format!("{:04}", ctx.at.year()),
        Placeholder::Month => format!("{:02}", ctx.at.month() as u8),
        Placeholder::Day => format!("{:02}", ctx.at.day()),
        Placeholder::HalfDay => if ctx.at.hour() < 12 { "上午" } else { "下午" }.to_string(),
    }
}

/// Windows 保留设备名（不区分大小写，且带扩展名时同样保留）。
const RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 非法字符的替换字符。用固定字符而非丢弃——净化 MUST 是可预期的。
const REPLACEMENT: char = '_';

fn is_illegal_char(c: char) -> bool {
    matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '/' | '\\') || (c as u32) < 0x20
}

/// 把一个**占位符的值**净化为可安全嵌入路径段的字符串。
///
/// 与 [`sanitize_segment`] 的区别：这里只做字符级替换，**不**做保留名改写与结尾点空格处理
/// ——那些是段级语义。例如项目名叫 `CON`、模板是 `{项目}-{设备}` 时，
/// 最终段 `CON-A7M4主卡` 并不是保留名，不该被改写。
///
/// 关键作用：值里的 `/` `\` 被替换掉，因此**值不可能凭空多出一层目录**，
/// 也就堵死了 `..` 之类的路径穿越入口（`..` 会在段级被改写）。
pub fn sanitize_value(raw: &str) -> String {
    raw.chars()
        .map(|c| if is_illegal_char(c) { REPLACEMENT } else { c })
        .collect()
}

/// 把一个路径段净化为 Windows 上合法且可创建的名字。
///
/// 处理：非法字符替换、控制字符替换、结尾的点与空格、保留设备名改写。
///
/// 空输入返回空串（用于折叠连续分隔符）；非空输入若净化后无内容，
/// 返回单个替换字符——**保留这一层目录**，避免层级被静默吞掉。
pub fn sanitize_segment(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }

    let mut out: String = raw
        .chars()
        .map(|c| if is_illegal_char(c) { REPLACEMENT } else { c })
        .collect();

    // Windows 不允许段以点或空格结尾（资源管理器会静默截断，导致路径对不上）。
    // 这一步同时处理了 `.` 与 `..`：它们会被 trim 成空，随后落到下面的兜底分支。
    let trimmed = out.trim_end_matches([' ', '.']);
    out = if trimmed.len() == out.len() {
        out
    } else if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}{REPLACEMENT}")
    };

    // 前导空格同样去掉（不影响可辨识度，但避免难以察觉的重名）。
    out = out.trim_start().to_string();

    // 非空输入不允许被净化成空——那会静默吞掉一层目录。
    if out.is_empty() {
        return REPLACEMENT.to_string();
    }

    // 保留设备名：整段等于保留名、或以「保留名.」开头，均需改写。
    let stem_upper = out.split('.').next().unwrap_or_default().to_ascii_uppercase();
    if RESERVED_NAMES.contains(&stem_upper.as_str()) {
        out.push(REPLACEMENT);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn ctx() -> RenderContext {
        RenderContext {
            project: "婚礼".into(),
            device: "A7M4主卡".into(),
            card: "SD-01".into(),
            at: datetime!(2026-08-08 09:30:00 UTC),
        }
    }

    // spec: organize-rules → 路径模板与占位符 → Scenario: 占位符正确渲染
    #[test]
    fn scenario_organize_rules_placeholders_render() {
        let t = PathTemplate::parse("/{项目}/{日期}/{设备}/").expect("模板应合法");
        assert_eq!(t.render(&ctx()), "婚礼/2026-08-08/A7M4主卡");
    }

    #[test]
    fn scenario_organize_rules_all_placeholders_render() {
        let t = PathTemplate::parse("{项目}/{年}/{月}/{日}/{设备}/{卡}/{时段}").expect("合法");
        assert_eq!(
            t.render(&ctx()),
            "婚礼/2026/08/08/A7M4主卡/SD-01/上午"
        );
    }

    #[test]
    fn scenario_organize_rules_halfday_afternoon() {
        let t = PathTemplate::parse("{项目}/{时段}").expect("合法");
        let mut c = ctx();
        c.at = datetime!(2026-08-08 13:00:00 UTC);
        assert_eq!(t.render(&c), "婚礼/下午");
    }

    // spec: organize-rules → 路径模板与占位符 → Scenario: 非法字符净化
    #[test]
    fn scenario_organize_rules_illegal_chars_sanitized() {
        let t = PathTemplate::parse("{项目}/{设备}").expect("合法");
        let mut c = ctx();
        c.project = "婚礼:2026*A?B".into();
        c.device = "卡<1>|2\"3".into();
        let rendered = t.render(&c);
        assert_eq!(rendered, "婚礼_2026_A_B/卡_1__2_3");
        // 净化是替换而非丢弃：字符数不减
        assert!(!rendered.contains(':'));
        assert!(!rendered.contains('*'));
        assert!(!rendered.contains('?'));
    }

    #[test]
    fn scenario_organize_rules_slash_in_value_does_not_escape() {
        // 用户把带斜杠的名字填进项目名，不应凭空多出一层目录
        let t = PathTemplate::parse("{项目}/{设备}").expect("合法");
        let mut c = ctx();
        c.project = "婚礼/隐藏层".into();
        assert_eq!(t.render(&c), "婚礼_隐藏层/A7M4主卡");
    }

    // spec: organize-rules → 路径模板与占位符 → Scenario: Windows 保留设备名
    #[test]
    fn scenario_organize_rules_reserved_names_rewritten() {
        for name in ["CON", "con", "PRN", "NUL", "COM1", "lpt9"] {
            let out = sanitize_segment(name);
            assert_ne!(
                out.to_ascii_uppercase(),
                name.to_ascii_uppercase(),
                "保留名 {name} 必须被改写"
            );
            assert!(out.starts_with(name), "改写应保留可辨识度：{name} -> {out}");
        }
        // 带扩展名的保留名同样受限
        assert_eq!(sanitize_segment("CON.txt"), "CON.txt_");
        // 不是保留名的相似串不应被改写
        assert_eq!(sanitize_segment("CONSOLE"), "CONSOLE");
        assert_eq!(sanitize_segment("COM10"), "COM10");
    }

    #[test]
    fn scenario_organize_rules_trailing_dot_and_space_rewritten() {
        assert_eq!(sanitize_segment("素材."), "素材_");
        assert_eq!(sanitize_segment("素材 "), "素材_");
        assert_eq!(sanitize_segment("  素材"), "素材");
        // 非空输入 MUST NOT 被净化成空——那会静默吞掉一层目录
        assert_eq!(sanitize_segment("..."), "_");
        assert_eq!(sanitize_segment(".."), "_");
        assert_eq!(sanitize_segment("   "), "_");
        // 空输入才返回空（用于折叠连续分隔符）
        assert_eq!(sanitize_segment(""), "");
    }

    // 路径穿越：占位符的值不得逃出它所在的那一层目录
    #[test]
    fn scenario_organize_rules_value_cannot_traverse_path() {
        let t = PathTemplate::parse("素材/{项目}/{设备}").expect("合法");
        let mut c = ctx();
        c.project = "../../Windows/System32".into();
        c.device = "..".into();
        let segments = t.render_segments(&c);
        assert!(
            segments.iter().all(|s| s != ".." && s != "."),
            "渲染结果不得含有穿越段：{segments:?}"
        );
        assert_eq!(segments.len(), 3, "层级数必须与模板一致：{segments:?}");
        assert_eq!(segments[0], "素材");
        assert_eq!(segments[1], ".._.._Windows_System32");
        assert_eq!(segments[2], "_");
    }

    #[test]
    fn scenario_organize_rules_value_level_keeps_segment_semantics() {
        // 值级净化不做保留名改写：整段不是保留名就不该被动
        let t = PathTemplate::parse("{项目}-{设备}").expect("合法");
        let mut c = ctx();
        c.project = "CON".into();
        assert_eq!(t.render(&c), "CON-A7M4主卡");
        // 但整段恰好是保留名时，段级改写照常生效
        let t2 = PathTemplate::parse("{项目}").expect("合法");
        assert_eq!(t2.render(&c), "CON_");
    }

    // spec: organize-rules → 路径模板与占位符 → Scenario: 缺少必需占位符被拒
    #[test]
    fn scenario_organize_rules_missing_required_rejected() {
        let err = PathTemplate::parse("/素材/{年}/{月}/").expect_err("缺必需占位符应被拒");
        assert_eq!(err, TemplateError::MissingRequiredPlaceholder);
        // 错误信息要说清楚缺什么
        let msg = err.to_string();
        assert!(msg.contains("项目") && msg.contains("日期") && msg.contains("设备"));
    }

    #[test]
    fn scenario_organize_rules_any_one_required_is_enough() {
        for tpl in ["{项目}", "{日期}", "{设备}"] {
            assert!(PathTemplate::parse(tpl).is_ok(), "{tpl} 应合法");
        }
    }

    // spec: organize-rules → 路径模板与占位符 → Scenario: 未知占位符被拒
    #[test]
    fn scenario_organize_rules_unknown_placeholder_rejected() {
        let err = PathTemplate::parse("{项目}/{不存在的占位符}").expect_err("未知占位符应被拒");
        assert_eq!(
            err,
            TemplateError::UnknownPlaceholder("不存在的占位符".into())
        );
        // 错误信息 MUST 列出全部可用占位符
        let msg = err.to_string();
        for p in Placeholder::ALL {
            assert!(msg.contains(p.token()), "错误信息应列出 {{{}}}", p.token());
        }
    }

    #[test]
    fn scenario_organize_rules_unbalanced_brace_rejected() {
        assert_eq!(
            PathTemplate::parse("{项目}/{日期").unwrap_err(),
            TemplateError::UnbalancedBrace
        );
        assert_eq!(
            PathTemplate::parse("{项目}/日期}").unwrap_err(),
            TemplateError::UnbalancedBrace
        );
        assert_eq!(
            PathTemplate::parse("{项目/{日期}").unwrap_err(),
            TemplateError::UnbalancedBrace
        );
    }

    // spec: organize-rules → 路径模板实时预览 → Scenario: 预览与实际落地一致
    #[test]
    fn scenario_organize_rules_preview_matches_landing() {
        // 预览与落地共用 render_segments，这里断言 render() 只是它的 join，
        // 从而保证不存在第二套近似实现。
        let t = PathTemplate::parse("/{项目}/{日期}/{设备}/").expect("合法");
        let c = ctx();
        assert_eq!(t.render(&c), t.render_segments(&c).join("/"));
    }

    #[test]
    fn scenario_organize_rules_redundant_separators_collapse() {
        let t = PathTemplate::parse("//{项目}///{设备}//").expect("合法");
        assert_eq!(t.render(&ctx()), "婚礼/A7M4主卡");
    }

    #[test]
    fn scenario_organize_rules_backslash_treated_as_separator() {
        let t = PathTemplate::parse(r"{项目}\{设备}").expect("合法");
        assert_eq!(t.render(&ctx()), "婚礼/A7M4主卡");
    }

    #[test]
    fn scenario_organize_rules_empty_after_sanitize_is_rejected() {
        // 全部由非法字符构成的模板渲染后为空，应在 parse 阶段就被拒
        assert_eq!(
            PathTemplate::parse("///").unwrap_err(),
            TemplateError::MissingRequiredPlaceholder
        );
    }
}
