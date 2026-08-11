//! 把一次任务的执行结果记进台账。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/task-ledger/spec.md`
//!
//! 这是「拷完之后」的收口：引擎产出 `TaskReport`，这里把它翻译成台账记录。
//! 台账**只记事实**，不做判断——状态由报告本身决定。

use time::OffsetDateTime;

use crate::config::model::new_id;
use crate::ledger::db::{FileRecord, Ledger, LedgerError, TaskRecord, TaskStatus};
use crate::manifest::store::format_time;
use crate::task::{FileStatus, TaskReport, TaskSpec};

/// 由报告推出最终状态。
///
/// **有任何失败就不是「全部通过」**——界面据此呈现，绝不粉饰。
pub fn status_of(report: &TaskReport) -> TaskStatus {
    if report.cancelled {
        return TaskStatus::Cancelled;
    }
    let failed = report.failed_files().count();
    if failed == 0 {
        TaskStatus::Ok
    } else if report.copied_count() == 0 {
        TaskStatus::Failed
    } else {
        TaskStatus::Partial
    }
}

/// 记一次任务。返回台账里的任务标识。
pub fn record_run(
    ledger: &Ledger,
    spec: &TaskSpec,
    report: &TaskReport,
    started_at: OffsetDateTime,
    finished_at: OffsetDateTime,
) -> Result<String, LedgerError> {
    let id = new_id("task");
    let elapsed = (finished_at - started_at).whole_seconds().max(0) as u64;

    let files: Vec<FileRecord> = report
        .files
        .iter()
        .map(|f| {
            let (status, reason) = match &f.status {
                FileStatus::Copied => ("copied", None),
                FileStatus::Skipped => ("skipped", None),
                FileStatus::Failed(r) => ("failed", Some(r.clone())),
            };
            FileRecord {
                relative_path: f.relative_path.clone(),
                size: f.size,
                // 目的地结果里带的是路径与校验状态；哈希在 manifest 里，
                // 台账只留一个可读标记，避免与 manifest 重复承载真相
                hash: if f.destinations.iter().any(|d| d.verified) {
                    "verified".into()
                } else {
                    String::new()
                },
                status: status.into(),
                reason,
                retries: f.retries,
            }
        })
        .collect();

    let record = TaskRecord {
        id: id.clone(),
        started_at: format_time(started_at),
        finished_at: format_time(finished_at),
        source_id: spec.source.id.clone(),
        source_name: spec.source.display_name.clone(),
        project: spec.project.clone(),
        algorithm: spec.algorithm.id().to_string(),
        verified: spec.verify,
        total_files: report.files.len() as u64,
        total_bytes: report.bytes_copied,
        copied: report.copied_count() as u64,
        skipped: report.skipped_count() as u64,
        failed: report.failed_files().count() as u64,
        status: status_of(report),
        elapsed_secs: elapsed,
        manifests: report
            .manifests
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
    };

    ledger.record_task(&record, &files)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::HashAlgorithm;
    use crate::manifest::model::SourceRef;
    use crate::organize::{PathTemplate, ScanOptions};
    use crate::task::{DestinationSpec, FileOutcome};
    use time::macros::datetime;

    fn spec() -> TaskSpec {
        TaskSpec {
            source_root: std::path::PathBuf::from(r"E:\"),
            source: SourceRef {
                id: "vol:1".into(),
                display_name: "A7M4主卡".into(),
            },
            project: "婚礼".into(),
            destinations: vec![DestinationSpec {
                root: std::path::PathBuf::from(r"D:\素材"),
                template: PathTemplate::parse("{项目}/{日期}/{设备}").expect("模板"),
                enabled: true,
            }],
            algorithm: HashAlgorithm::Xxh64,
            verify: true,
            scan: ScanOptions::mirror(),
            retries: 2,
            at: datetime!(2026-08-10 09:00:00 UTC),
        }
    }

    fn outcome(path: &str, status: FileStatus) -> FileOutcome {
        FileOutcome {
            relative_path: path.into(),
            size: 100,
            status,
            retries: 0,
            destinations: Vec::new(),
        }
    }

    fn report(files: Vec<FileOutcome>, cancelled: bool) -> TaskReport {
        TaskReport {
            files,
            manifests: vec![std::path::PathBuf::from(r"D:\素材\steadcopy\m.json")],
            cancelled,
            notices: Vec::new(),
            bytes_copied: 200,
        }
    }

    #[test]
    fn scenario_task_ledger_status_derivation() {
        assert_eq!(
            status_of(&report(vec![outcome("a", FileStatus::Copied)], false)),
            TaskStatus::Ok
        );
        assert_eq!(
            status_of(&report(
                vec![
                    outcome("a", FileStatus::Copied),
                    outcome("b", FileStatus::Failed("坏了".into()))
                ],
                false
            )),
            TaskStatus::Partial,
            "有失败就不是「全部通过」"
        );
        assert_eq!(
            status_of(&report(
                vec![outcome("a", FileStatus::Failed("坏了".into()))],
                false
            )),
            TaskStatus::Failed
        );
        assert_eq!(
            status_of(&report(vec![outcome("a", FileStatus::Copied)], true)),
            TaskStatus::Cancelled,
            "取消优先于其他判定"
        );
    }

    #[test]
    fn scenario_task_ledger_record_run_writes_task_and_files() {
        let l = Ledger::open_in_memory().expect("建库");
        let r = report(
            vec![
                outcome("A001.MP4", FileStatus::Copied),
                outcome("A002.MP4", FileStatus::Skipped),
                outcome("A003.MP4", FileStatus::Failed("校验不一致".into())),
            ],
            false,
        );
        let id = record_run(
            &l,
            &spec(),
            &r,
            datetime!(2026-08-10 09:00:00 UTC),
            datetime!(2026-08-10 09:02:05 UTC),
        )
        .expect("记账");

        let t = l.task(&id).expect("查").expect("应存在");
        assert_eq!(t.status, TaskStatus::Partial);
        assert_eq!(t.copied, 1);
        assert_eq!(t.skipped, 1);
        assert_eq!(t.failed, 1);
        assert_eq!(t.elapsed_secs, 125);
        assert_eq!(t.source_name, "A7M4主卡");
        assert_eq!(t.project, "婚礼");
        assert_eq!(t.manifests.len(), 1);

        let failed = l.task_files(&id, Some("failed")).expect("明细");
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].relative_path, "A003.MP4");
        assert_eq!(failed[0].reason.as_deref(), Some("校验不一致"));
        assert_eq!(l.task_files(&id, None).expect("全部").len(), 3);
    }

    #[test]
    fn scenario_task_ledger_two_runs_get_distinct_ids() {
        let l = Ledger::open_in_memory().expect("建库");
        let r = report(vec![outcome("a", FileStatus::Copied)], false);
        let at = datetime!(2026-08-10 09:00:00 UTC);
        let a = record_run(&l, &spec(), &r, at, at).expect("记");
        let b = record_run(&l, &spec(), &r, at, at).expect("记");
        assert_ne!(a, b, "两次任务 MUST 是两条记录");
        assert_eq!(l.count().expect("计数"), 2);
    }
}
