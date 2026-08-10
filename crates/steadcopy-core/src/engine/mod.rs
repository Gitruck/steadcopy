//! 拷贝引擎：单遍读源多目的地并行写、边读边算哈希、无缓冲读回校验、重试、取消、进度。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/copy-engine/spec.md`

pub mod hasher;
pub mod pipeline;
pub mod verify;

pub use pipeline::{copy_file_to_many, CancelToken, CopyResult, DestinationOutcome, PipelineOptions};
pub use verify::{hash_destination, verify_destination, VerifyOutcome};
pub use hasher::{hash_bytes, HashAlgorithm, HashValue, Hasher};
