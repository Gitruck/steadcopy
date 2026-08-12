//! 错误双族模型。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/copy-engine/spec.md` → Requirement: 错误双族分类
//!
//! 铁律：任何可失败操作返回 `Result`，**禁止**「出错返回零值/空值/默认值」。
//! 前身项目的一号缺陷正是哈希函数异常时返回空串，导致源与目标双双失败时
//! `"" == ""` 判定校验通过且日志全绿——用户以为有备份，其实没有。

use std::path::PathBuf;

use crate::i18n::Locale;

/// 可重试族：重新插卡重跑有可能成功。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryableKind {
    /// 拷贝过程中的 IO 失败
    CopyIo,
    /// 校验最终不一致（重试耗尽）
    VerifyMismatch,
    /// 设备在任务进行中被移除
    DeviceRemoved,
    /// 目的地暂时不可写
    DestinationUnwritable,
}

/// 终态族：重跑结果相同，只需告知。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalKind {
    /// 源上没有匹配的素材
    NoSource,
    /// 有素材但全部已完成（账本已覆盖）
    NoNewSource,
    /// 空间不足
    InsufficientSpace,
    /// 源不可读
    SourceUnreadable,
    /// 配置非法（模板、过滤规则等）
    InvalidConfig,
    /// 本平台不支持该能力（非 Windows 侧的空壳实现）
    Unsupported,
}

/// 错误上下文：足以定位问题的信息。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ErrorContext {
    pub path: Option<PathBuf>,
    pub destination: Option<PathBuf>,
    /// 底层原因的可读描述（**不是**给用户看的主表述，是诊断信息）
    pub cause: Option<String>,
}

impl ErrorContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn path(mut self, p: impl Into<PathBuf>) -> Self {
        self.path = Some(p.into());
        self
    }

    pub fn destination(mut self, p: impl Into<PathBuf>) -> Self {
        self.destination = Some(p.into());
        self
    }

    pub fn cause(mut self, c: impl Into<String>) -> Self {
        self.cause = Some(c.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// 可重试族——UI 应提示「重新插卡后可继续」
    Retryable(RetryableKind, ErrorContext),
    /// 终态族——UI **MUST NOT** 提示重新插卡
    Terminal(TerminalKind, ErrorContext),
}

impl CoreError {
    pub fn retryable(kind: RetryableKind) -> Self {
        CoreError::Retryable(kind, ErrorContext::new())
    }

    pub fn terminal(kind: TerminalKind) -> Self {
        CoreError::Terminal(kind, ErrorContext::new())
    }

    pub fn with_context(self, ctx: ErrorContext) -> Self {
        match self {
            CoreError::Retryable(k, _) => CoreError::Retryable(k, ctx),
            CoreError::Terminal(k, _) => CoreError::Terminal(k, ctx),
        }
    }

    /// 是否属于「建议用户重新插卡重跑」的一族。
    pub fn is_retryable(&self) -> bool {
        matches!(self, CoreError::Retryable(..))
    }

    pub fn context(&self) -> &ErrorContext {
        match self {
            CoreError::Retryable(_, c) | CoreError::Terminal(_, c) => c,
        }
    }

    /// 给用户看的一句话，跟随语言。
    ///
    /// 文案本体**只在这一处**——`Display` 转调 `describe(Locale::Zh)`，
    /// 不存在第二份中文可以漂移。
    pub fn describe(&self, lang: Locale) -> String {
        let desc = match self {
            CoreError::Retryable(RetryableKind::CopyIo, _) => {
                lang.pick("拷贝过程中读写失败", "A read or write failed while copying")
            }
            CoreError::Retryable(RetryableKind::VerifyMismatch, _) => lang.pick(
                "校验不一致，重试后仍未通过",
                "Verification still did not match after retrying",
            ),
            CoreError::Retryable(RetryableKind::DeviceRemoved, _) => lang.pick(
                "设备在任务进行中被移除",
                "The device was removed while the task was running",
            ),
            CoreError::Retryable(RetryableKind::DestinationUnwritable, _) => {
                lang.pick("目的地当前不可写入", "The destination is not writable right now")
            }
            CoreError::Terminal(TerminalKind::NoSource, _) => lang.pick(
                "源设备上没有符合条件的素材",
                "No matching media on the source device",
            ),
            CoreError::Terminal(TerminalKind::NoNewSource, _) => lang.pick(
                "没有新素材，本次无需拷贝",
                "Nothing new to copy this time",
            ),
            CoreError::Terminal(TerminalKind::InsufficientSpace, _) => {
                lang.pick("目的地可用空间不足", "Not enough free space at the destination")
            }
            CoreError::Terminal(TerminalKind::SourceUnreadable, _) => {
                lang.pick("源设备无法读取", "The source device cannot be read")
            }
            CoreError::Terminal(TerminalKind::InvalidConfig, _) => {
                lang.pick("配置不合法", "The configuration is not valid")
            }
            CoreError::Terminal(TerminalKind::Unsupported, _) => lang.pick(
                "当前平台尚不支持该功能",
                "This platform does not support that yet",
            ),
        };
        // 路径是**数据**不是文案：中文目录名出现在英文句子里是对的，不该被护栏当成漏译。
        // 变的只有括号——中文用全角，英文用半角加空格。
        match (&self.context().path, lang) {
            (Some(p), Locale::Zh) => format!("{desc}（{}）", p.display()),
            (Some(p), Locale::En) => format!("{desc} ({})", p.display()),
            (None, _) => desc.to_string(),
        }
    }
}

impl std::fmt::Display for CoreError {
    /// `Display` 恒为中文：它落在日志与命令行兜底里，那些地方拿不到 locale。
    /// 要跟随语言的调用方走 [`CoreError::describe`]。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe(Locale::Zh))
    }
}

impl std::error::Error for CoreError {}

pub type Result<T> = std::result::Result<T, CoreError>;
