//! 稳拷 steadcopy 的业务核。
//!
//! 平台无关。所有平台相关能力经 `platform` 的 trait 隔离，Windows 实现在
//! `#[cfg(windows)]` 分支，其余平台返回 `TerminalKind::Unsupported`（架构留门，见 P5 决策）。
//!
//! 工作制度见 `openspec/README.md`：SDD（OpenSpec）+ TDD（规格锚定的 Detroit 式）+ 双轨约束。

pub mod engine;
pub mod error;
pub mod manifest;
pub mod platform;
pub mod task;
pub mod organize;

pub use engine::{HashAlgorithm, HashValue};
pub use error::{CoreError, ErrorContext, Result, RetryableKind, TerminalKind};
