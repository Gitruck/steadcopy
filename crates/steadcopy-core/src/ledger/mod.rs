//! 任务台账与报告。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/task-ledger/spec.md`

pub mod report;

pub use report::{render_report, write_report, ReportInput};
