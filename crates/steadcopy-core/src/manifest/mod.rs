//! 校验清单：格式、落盘、续传账本、复验四态、MHL v1 兼容输出。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/verify-manifest/spec.md`

pub mod audit;
pub mod model;

pub use audit::{audit, AuditCounts, AuditReport, ObservedFile};
pub use model::{
    normalize_relative, relative_of, Generator, Manifest, ManifestEntry, SourceRef, VerifyState,
    MANIFEST_FORMAT_VERSION,
};
