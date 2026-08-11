//! 校验清单：格式、落盘、续传账本、复验四态、MHL v1 兼容输出。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/verify-manifest/spec.md`

pub mod audit;
pub mod ledger_account;
pub mod mhl;
pub mod model;
pub mod store;

pub use mhl::{render_mhl, write_mhl};
pub use audit::{audit, AuditCounts, AuditReport, ObservedFile};
pub use ledger_account::{DoneEntry, ResumeLedger};
pub use store::{
    format_time_human,
    is_manifest_path, load_manifests, manifest_dir, read_manifest, write_manifest, LoadedManifests,
    ManifestReadIssue, MANIFEST_DIR,
};
pub use model::{
    normalize_relative, relative_of, Generator, Manifest, ManifestEntry, SourceRef, VerifyState,
    MANIFEST_FORMAT_VERSION,
};
