//! 任务台账与报告。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/task-ledger/spec.md`

pub mod db;
pub mod record;
pub mod report;

pub use db::{
    ledger_path, FileRecord, FormatAttempt, HistoryQuery, Ledger, LedgerError, TaskRecord,
    TaskStatus, SCHEMA_VERSION,
};
pub use record::{record_run, status_of};
pub use report::{render_report, write_report, ReportInput};
