//! 任务执行：拷贝 → 校验 → 失败重拷 → 落清单。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/copy-engine/spec.md`
//! → Requirement: 校验失败自动重拷 / 任务阶段模型 / 错误双族分类

use std::path::PathBuf;
use std::time::Duration;

use crate::engine::{
    copy_file_to_many, verify_destination, CancelToken, HashValue, PipelineOptions, VerifyOutcome,
};
use crate::error::Result;
use crate::manifest::model::{ManifestEntry, VerifyState};
use crate::manifest::{write_manifest, Manifest};
use crate::platform::{Clock, VolumeIo};
use crate::task::plan::{TaskPlan, TaskSpec};
use crate::task::stage::{StageEvent, TaskStage};

/// 单个文件在某个目的地的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestFileResult {
    /// 目的地在 `plan.destinations` 中的下标
    pub slot: usize,
    pub path: PathBuf,
    pub verified: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    /// 本次拷贝并（若开启）校验通过
    Copied,
    /// 全部目的地都已有它，本次跳过
    Skipped,
    /// 重试耗尽仍失败
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOutcome {
    pub relative_path: String,
    pub size: u64,
    pub status: FileStatus,
    pub retries: u32,
    pub destinations: Vec<DestFileResult>,
}

impl FileOutcome {
    pub fn failed(&self) -> bool {
        matches!(self.status, FileStatus::Failed(_))
    }
}

/// 一次任务的执行报告。
#[derive(Debug, Clone)]
pub struct TaskReport {
    pub files: Vec<FileOutcome>,
    /// 每个目的地落下的 manifest 路径
    pub manifests: Vec<PathBuf>,
    pub cancelled: bool,
    /// 需要呈现给用户的提示（账本降级等）
    pub notices: Vec<String>,
    pub bytes_copied: u64,
}

impl TaskReport {
    pub fn failed_files(&self) -> impl Iterator<Item = &FileOutcome> {
        self.files.iter().filter(|f| f.failed())
    }

    pub fn copied_count(&self) -> usize {
        self.files
            .iter()
            .filter(|f| f.status == FileStatus::Copied)
            .count()
    }

    pub fn skipped_count(&self) -> usize {
        self.files
            .iter()
            .filter(|f| f.status == FileStatus::Skipped)
            .count()
    }

    /// 任务是否**全部成功**。有任何失败文件就不是——
    /// 界面 MUST NOT 在这种情况下呈现「完成」。
    pub fn all_succeeded(&self) -> bool {
        !self.cancelled && self.failed_files().count() == 0
    }
}

/// 执行一次已规划好的任务。
pub fn run_task(
    spec: &TaskSpec,
    plan: &TaskPlan,
    io: &dyn VolumeIo,
    clock: &dyn Clock,
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(StageEvent),
) -> Result<TaskReport> {
    let mut report = TaskReport {
        files: Vec::new(),
        manifests: Vec::new(),
        cancelled: false,
        notices: Vec::new(),
        bytes_copied: 0,
    };

    for d in &plan.destinations {
        for reason in &d.ledger_degraded {
            let msg = format!("历史清单不可读（{reason}），本次执行全量拷贝");
            on_event(StageEvent::Notice(msg.clone()));
            report.notices.push(msg);
        }
    }

    // 已跳过的文件如实记账
    for f in &plan.skipped {
        report.files.push(FileOutcome {
            relative_path: f.relative_path.clone(),
            size: f.size,
            status: FileStatus::Skipped,
            retries: 0,
            destinations: Vec::new(),
        });
    }

    on_event(StageEvent::Stage(TaskStage::Copying));

    let total_bytes = plan.total_bytes();
    let mut done_bytes: u64 = 0;
    let options = PipelineOptions {
        algorithm: spec.algorithm,
        ..Default::default()
    };

    // 每个目的地本次新增的 manifest 条目
    let mut entries: Vec<Vec<ManifestEntry>> = vec![Vec::new(); plan.destinations.len()];

    for planned in &plan.files {
        if cancel.is_cancelled() {
            report.cancelled = true;
            break;
        }

        let outcome = copy_one_file(
            spec,
            plan,
            planned,
            io,
            clock,
            cancel,
            &options,
            &mut entries,
            &mut |current, delta| {
                on_event(StageEvent::Progress {
                    stage: TaskStage::Copying,
                    done: done_bytes + delta,
                    total: total_bytes,
                    current: Some(current.to_string()),
                });
            },
        );

        if let FileStatus::Failed(reason) = &outcome.status {
            on_event(StageEvent::FileFailed {
                relative_path: outcome.relative_path.clone(),
                reason: reason.clone(),
            });
        }
        if outcome.status == FileStatus::Copied {
            done_bytes += outcome.size;
            report.bytes_copied += outcome.size;
        }
        report.files.push(outcome);
    }

    if cancel.is_cancelled() {
        report.cancelled = true;
    }

    on_event(StageEvent::Stage(TaskStage::Finishing));

    // 每个目的地落一份清单。**即使有失败文件也要落**——
    // 成功的部分是真实成果，凭证不该因为部分失败而整份丢掉。
    for (slot, dest) in plan.destinations.iter().enumerate() {
        if entries[slot].is_empty() {
            continue;
        }
        let mut m = Manifest::new(
            spec.source.clone(),
            spec.project.clone(),
            &dest.landing_dir,
            spec.algorithm,
            spec.at,
        );
        m.entries = std::mem::take(&mut entries[slot]);
        match write_manifest(&dest.landing_dir, &m) {
            Ok(p) => {
                // 同时产一份 MHL v1 兼容清单，让凭证能被商业工具复验
                if let Err(e) = crate::manifest::write_mhl(&p, &m) {
                    let msg = format!("MHL 清单写入失败（{}）：{e}", p.display());
                    on_event(StageEvent::Notice(msg.clone()));
                    report.notices.push(msg);
                }
                report.manifests.push(p);
            }
            Err(e) => {
                let msg = format!("清单写入失败（{}）：{e}", dest.landing_dir.display());
                on_event(StageEvent::Notice(msg.clone()));
                report.notices.push(msg);
            }
        }
    }

    on_event(StageEvent::Stage(TaskStage::Finished));
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn copy_one_file(
    spec: &TaskSpec,
    plan: &TaskPlan,
    planned: &crate::task::plan::PlannedFile,
    io: &dyn VolumeIo,
    clock: &dyn Clock,
    cancel: &CancelToken,
    options: &PipelineOptions,
    entries: &mut [Vec<ManifestEntry>],
    on_progress: &mut dyn FnMut(&str, u64),
) -> FileOutcome {
    let rel = &planned.file.relative_path;
    let rel_native = rel.replace('/', std::path::MAIN_SEPARATOR_STR);

    // 本轮仍需处理的目的地。重试时**只重试真正失败的那些**，不重拷全集。
    let mut pending: Vec<usize> = planned.targets.clone();
    let mut results: Vec<DestFileResult> = Vec::new();
    let mut retries: u32 = 0;
    let mut last_error: Option<String> = None;
    let mut source_hash: Option<HashValue> = None;

    loop {
        let dest_paths: Vec<PathBuf> = pending
            .iter()
            .map(|&slot| plan.destinations[slot].landing_dir.join(&rel_native))
            .collect();

        let copy = copy_file_to_many(
            &planned.file.absolute_path,
            &dest_paths,
            options,
            cancel,
            &mut |n| on_progress(rel, n),
        );

        let copy = match copy {
            Ok(c) => c,
            Err(e) => {
                last_error = Some(e.to_string());
                break;
            }
        };
        if copy.cancelled {
            last_error = Some("任务已取消".into());
            break;
        }
        source_hash = Some(copy.source_hash);

        let mut still_failing: Vec<usize> = Vec::new();
        for (i, &slot) in pending.iter().enumerate() {
            let dest_path = &dest_paths[i];
            let write = &copy.destinations[i];

            if let Some(err) = &write.error {
                still_failing.push(slot);
                last_error = Some(err.clone());
                continue;
            }

            if !spec.verify {
                results.push(DestFileResult {
                    slot,
                    path: dest_path.clone(),
                    verified: false,
                    error: None,
                });
                entries[slot].push(entry_for(planned, copy.source_hash, None, retries, spec));
                continue;
            }

            match verify_destination(io, dest_path, &copy.source_hash) {
                Ok(VerifyOutcome::Match) => {
                    results.push(DestFileResult {
                        slot,
                        path: dest_path.clone(),
                        verified: true,
                        error: None,
                    });
                    entries[slot].push(entry_for(
                        planned,
                        copy.source_hash,
                        Some(copy.source_hash),
                        retries,
                        spec,
                    ));
                }
                Ok(VerifyOutcome::Mismatch { actual }) => {
                    still_failing.push(slot);
                    last_error = Some(format!(
                        "校验不一致：期望 {}，实际 {}",
                        copy.source_hash.to_hex(),
                        actual.to_hex()
                    ));
                }
                Err(e) => {
                    // 校验**没做成**一律按失败处理，MUST NOT 当作通过
                    still_failing.push(slot);
                    last_error = Some(format!("校验未能完成：{e}"));
                }
            }
        }

        if still_failing.is_empty() {
            break;
        }
        if retries >= spec.retries || cancel.is_cancelled() {
            pending = still_failing;
            break;
        }

        retries += 1;
        // 指数退避：1s、2s、4s…（时钟可注入，测试里不真 sleep）
        clock.sleep(Duration::from_secs(1u64 << (retries - 1).min(6)));
        pending = still_failing;
    }

    let failed_slots: Vec<usize> = pending
        .iter()
        .copied()
        .filter(|slot| !results.iter().any(|r| r.slot == *slot))
        .collect();

    for slot in &failed_slots {
        results.push(DestFileResult {
            slot: *slot,
            path: plan.destinations[*slot].landing_dir.join(&rel_native),
            verified: false,
            error: last_error.clone(),
        });
    }

    let status = if failed_slots.is_empty() && source_hash.is_some() {
        FileStatus::Copied
    } else {
        FileStatus::Failed(last_error.unwrap_or_else(|| "未知原因".into()))
    };

    FileOutcome {
        relative_path: rel.clone(),
        size: planned.file.size,
        status,
        retries,
        destinations: results,
    }
}

fn entry_for(
    planned: &crate::task::plan::PlannedFile,
    source_hash: HashValue,
    dest_hash: Option<HashValue>,
    retries: u32,
    spec: &TaskSpec,
) -> ManifestEntry {
    ManifestEntry {
        relative_path: planned.file.relative_path.clone(),
        size: planned.file.size,
        source_hash,
        verify: match dest_hash {
            Some(h) => VerifyState::Verified {
                destination_hash: h,
            },
            None => VerifyState::NotVerified,
        },
        source_modified_at: planned.file.modified,
        completed_at: spec.at,
        retries,
    }
}
