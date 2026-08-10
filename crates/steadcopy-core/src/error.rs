//! 错误双族模型。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/copy-engine/spec.md` → Requirement: 错误双族分类
//!
//! 铁律：任何可失败操作返回 `Result`，**禁止**「出错返回零值/空值/默认值」。
//! 前身项目的一号缺陷正是哈希函数异常时返回空串，导致源与目标双双失败时
//! `"" == ""` 判定校验通过且日志全绿——用户以为有备份，其实没有。

use std::path::PathBuf;

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
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let desc = match self {
            CoreError::Retryable(RetryableKind::CopyIo, _) => "拷贝过程中读写失败",
            CoreError::Retryable(RetryableKind::VerifyMismatch, _) => "校验不一致，重试后仍未通过",
            CoreError::Retryable(RetryableKind::DeviceRemoved, _) => "设备在任务进行中被移除",
            CoreError::Retryable(RetryableKind::DestinationUnwritable, _) => "目的地当前不可写入",
            CoreError::Terminal(TerminalKind::NoSource, _) => "源设备上没有符合条件的素材",
            CoreError::Terminal(TerminalKind::NoNewSource, _) => "没有新素材，本次无需拷贝",
            CoreError::Terminal(TerminalKind::InsufficientSpace, _) => "目的地可用空间不足",
            CoreError::Terminal(TerminalKind::SourceUnreadable, _) => "源设备无法读取",
            CoreError::Terminal(TerminalKind::InvalidConfig, _) => "配置不合法",
            CoreError::Terminal(TerminalKind::Unsupported, _) => "当前平台尚不支持该功能",
        };
        write!(f, "{desc}")?;
        let ctx = self.context();
        if let Some(p) = &ctx.path {
            write!(f, "（{}）", p.display())?;
        }
        Ok(())
    }
}

impl std::error::Error for CoreError {}

pub type Result<T> = std::result::Result<T, CoreError>;
