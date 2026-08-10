//! 任务规划：算出落地路径、增量集合、空间预检结论。**零副作用。**
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/copy-engine/spec.md`
//! → Requirement: 空间预检 / 断点续传
//! 以及 `specs/cli-driver/spec.md` → Requirement: 干跑（plan）不产生副作用

use std::path::PathBuf;

use time::OffsetDateTime;

use crate::engine::HashAlgorithm;
use crate::error::Result;
use crate::manifest::model::SourceRef;
use crate::manifest::ResumeLedger;
use crate::organize::{PathTemplate, RenderContext, ScanOptions, ScanResult, SourceFile};
use crate::platform::VolumeIo;

/// 一个目的地的配置。
#[derive(Debug, Clone)]
pub struct DestinationSpec {
    /// 目的地根目录（用户选的盘或文件夹）
    pub root: PathBuf,
    pub template: PathTemplate,
    pub enabled: bool,
}

/// 一次任务的完整输入。
#[derive(Debug, Clone)]
pub struct TaskSpec {
    pub source_root: PathBuf,
    pub source: SourceRef,
    pub project: String,
    pub destinations: Vec<DestinationSpec>,
    pub algorithm: HashAlgorithm,
    /// 是否做无缓冲读回校验
    pub verify: bool,
    pub scan: ScanOptions,
    /// 校验失败后的重拷次数上限
    pub retries: u32,
    /// 任务时间（路径模板与 manifest 都用它，保证同一任务内取值一致）
    pub at: OffsetDateTime,
}

impl TaskSpec {
    pub fn render_context(&self) -> RenderContext {
        RenderContext {
            project: self.project.clone(),
            device: self.source.display_name.clone(),
            card: self.source.display_name.clone(),
            at: self.at,
        }
    }
}

/// 一个目的地的规划结论。
#[derive(Debug, Clone)]
pub struct DestinationPlan {
    pub root: PathBuf,
    /// 模板渲染后的**完整落地目录**——确认卡片上要显示的就是它
    pub landing_dir: PathBuf,
    /// 本次实际要写入的字节数（**增量**口径，已跳过的不计）
    pub required_bytes: u64,
    pub required_files: usize,
    pub available_bytes: Option<u64>,
    /// 账本降级原因（非空表示本目的地本次走全量）
    pub ledger_degraded: Vec<String>,
}

impl DestinationPlan {
    /// 空间是否充足。可用空间查不到时返回 `None`——**不假装充足**。
    pub fn sufficient(&self) -> Option<bool> {
        self.available_bytes.map(|a| a >= self.required_bytes)
    }

    /// 还差多少字节。
    pub fn shortfall(&self) -> Option<u64> {
        self.available_bytes
            .map(|a| self.required_bytes.saturating_sub(a))
    }
}

/// 一个待拷文件及其目标目的地。
///
/// `targets` 是**还需要它**的目的地下标——某文件可能已在目的地 A 完成、但目的地 B 还缺。
/// 这样仍然只读一遍源，只是写入的目的地少几个。
#[derive(Debug, Clone)]
pub struct PlannedFile {
    pub file: SourceFile,
    pub targets: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct TaskPlan {
    pub scan: ScanResult,
    /// 本次要拷的文件（至少有一个目的地需要它）
    pub files: Vec<PlannedFile>,
    /// 全部目的地都已完成、本次跳过的文件
    pub skipped: Vec<SourceFile>,
    pub destinations: Vec<DestinationPlan>,
    /// 启用的目的地在 `spec.destinations` 中的下标
    pub enabled_indices: Vec<usize>,
}

impl TaskPlan {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.file.size).sum()
    }

    /// 源上有素材、但全部已完成 → 「无新素材」终态。
    pub fn is_no_new_source(&self) -> bool {
        self.files.is_empty() && !self.skipped.is_empty()
    }

    /// 源上根本没有素材 → 「无素材」终态。
    pub fn is_no_source(&self) -> bool {
        self.files.is_empty() && self.skipped.is_empty()
    }

    /// 空间不足的目的地。
    pub fn insufficient(&self) -> impl Iterator<Item = &DestinationPlan> {
        self.destinations
            .iter()
            .filter(|d| d.sufficient() == Some(false))
    }

    /// 任一目的地的账本降级了。
    pub fn any_ledger_degraded(&self) -> bool {
        self.destinations.iter().any(|d| !d.ledger_degraded.is_empty())
    }
}

/// 规划一次任务。**只读**：不创建目录、不写文件、不动台账。
pub fn plan_task(spec: &TaskSpec, io: &dyn VolumeIo) -> Result<TaskPlan> {
    let scan = crate::organize::scan_source(&spec.source_root, &spec.scan);
    let ctx = spec.render_context();

    let enabled_indices: Vec<usize> = spec
        .destinations
        .iter()
        .enumerate()
        .filter(|(_, d)| d.enabled)
        .map(|(i, _)| i)
        .collect();

    let mut destinations: Vec<DestinationPlan> = Vec::with_capacity(enabled_indices.len());
    let mut ledgers: Vec<ResumeLedger> = Vec::with_capacity(enabled_indices.len());

    for &idx in &enabled_indices {
        let spec_dest = &spec.destinations[idx];
        let mut landing = spec_dest.root.clone();
        for seg in spec_dest.template.render_segments(&ctx) {
            landing.push(seg);
        }
        let ledger = ResumeLedger::load(&landing, &spec.source.id);
        destinations.push(DestinationPlan {
            root: spec_dest.root.clone(),
            landing_dir: landing,
            required_bytes: 0,
            required_files: 0,
            available_bytes: None,
            ledger_degraded: ledger.degraded_reasons.clone(),
        });
        ledgers.push(ledger);
    }

    let mut files = Vec::new();
    let mut skipped = Vec::new();

    for f in &scan.files {
        let mut targets = Vec::new();
        for (slot, ledger) in ledgers.iter().enumerate() {
            if !ledger.is_done(&f.relative_path, f.size) {
                targets.push(slot);
            }
        }
        if targets.is_empty() {
            skipped.push(f.clone());
        } else {
            for &slot in &targets {
                destinations[slot].required_bytes += f.size;
                destinations[slot].required_files += 1;
            }
            files.push(PlannedFile {
                file: f.clone(),
                targets,
            });
        }
    }

    // 可用空间查不到就留 None——**不假装充足**，界面上如实呈现「无法确认」
    for d in &mut destinations {
        d.available_bytes = io.available_space(&d.landing_dir).ok();
    }

    Ok(TaskPlan {
        scan,
        files,
        skipped,
        destinations,
        enabled_indices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::model::{ManifestEntry, VerifyState};
    use crate::manifest::{write_manifest, Manifest};
    use crate::organize::PathTemplate;
    use crate::platform::volume_io;
    use std::path::Path;
    use time::macros::datetime;

    fn touch(root: &Path, rel: &str, bytes: usize) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("建目录");
        }
        std::fs::write(&p, vec![b'x'; bytes]).expect("写");
    }

    fn spec_for(src: &Path, dests: &[&Path]) -> TaskSpec {
        TaskSpec {
            source_root: src.to_path_buf(),
            source: SourceRef {
                id: "vol-1".into(),
                display_name: "A7M4主卡".into(),
            },
            project: "婚礼".into(),
            destinations: dests
                .iter()
                .map(|d| DestinationSpec {
                    root: d.to_path_buf(),
                    template: PathTemplate::parse("{项目}/{日期}/{设备}").expect("模板"),
                    enabled: true,
                })
                .collect(),
            algorithm: HashAlgorithm::Xxh64,
            verify: true,
            scan: ScanOptions::mirror(),
            retries: 2,
            at: datetime!(2026-08-08 09:30:00 UTC),
        }
    }

    #[test]
    fn scenario_cli_driver_plan_has_no_side_effects() {
        let dir = tempfile::tempdir().expect("临时目录");
        let src = dir.path().join("src");
        let dst = dir.path().join("dst");
        touch(&src, "DCIM/A001.MP4", 100);
        std::fs::create_dir_all(&dst).expect("建目的地根");

        let before: Vec<_> = walkdir::WalkDir::new(&dst)
            .into_iter()
            .flatten()
            .map(|e| e.path().to_path_buf())
            .collect();

        let io = volume_io();
        let plan = plan_task(&spec_for(&src, &[&dst]), io.as_ref()).expect("规划");
        assert_eq!(plan.files.len(), 1);

        let after: Vec<_> = walkdir::WalkDir::new(&dst)
            .into_iter()
            .flatten()
            .map(|e| e.path().to_path_buf())
            .collect();
        assert_eq!(before, after, "plan MUST NOT 创建任何目录或文件");
    }

    #[test]
    fn scenario_copy_engine_plan_renders_landing_dir() {
        let dir = tempfile::tempdir().expect("临时目录");
        let src = dir.path().join("src");
        let dst = dir.path().join("dst");
        touch(&src, "A001.MP4", 10);

        let io = volume_io();
        let plan = plan_task(&spec_for(&src, &[&dst]), io.as_ref()).expect("规划");
        let landing = &plan.destinations[0].landing_dir;
        assert!(landing.ends_with("A7M4主卡"));
        assert!(landing.to_string_lossy().contains("婚礼"));
        assert!(landing.to_string_lossy().contains("2026-08-08"));
    }

    // spec: copy-engine → 空间预检 → Scenario: 增量口径而非全量口径
    #[test]
    fn scenario_copy_engine_precheck_uses_incremental_size() {
        let dir = tempfile::tempdir().expect("临时目录");
        let src = dir.path().join("src");
        let dst = dir.path().join("dst");
        touch(&src, "A001.MP4", 1000);
        touch(&src, "A002.MP4", 500);

        let spec = spec_for(&src, &[&dst]);
        let io = volume_io();
        // 先规划一次拿到落地目录
        let landing = plan_task(&spec, io.as_ref()).expect("规划").destinations[0]
            .landing_dir
            .clone();

        // 伪造「A001 已完成」：清单 + 落地文件
        let h = crate::engine::hash_bytes(HashAlgorithm::Xxh64, &vec![b'x'; 1000]);
        let mut m = Manifest::new(
            spec.source.clone(),
            "婚礼",
            &landing,
            HashAlgorithm::Xxh64,
            spec.at,
        );
        m.entries.push(ManifestEntry {
            relative_path: "A001.MP4".into(),
            size: 1000,
            source_hash: h,
            verify: VerifyState::Verified {
                destination_hash: h,
            },
            source_modified_at: None,
            completed_at: spec.at,
            retries: 0,
        });
        std::fs::create_dir_all(&landing).expect("建落地目录");
        write_manifest(&landing, &m).expect("写清单");
        std::fs::write(landing.join("A001.MP4"), vec![b'x'; 1000]).expect("落地文件");

        let plan = plan_task(&spec, io.as_ref()).expect("再规划");
        assert_eq!(plan.files.len(), 1, "只剩 A002 要拷");
        assert_eq!(plan.skipped.len(), 1);
        assert_eq!(
            plan.destinations[0].required_bytes, 500,
            "预检 MUST 用增量口径（500）而非全量（1500）"
        );
    }

    #[test]
    fn scenario_copy_engine_plan_no_source_vs_no_new_source() {
        let dir = tempfile::tempdir().expect("临时目录");
        let src = dir.path().join("empty_src");
        std::fs::create_dir_all(&src).expect("建空源");
        let dst = dir.path().join("dst");

        let io = volume_io();
        let plan = plan_task(&spec_for(&src, &[&dst]), io.as_ref()).expect("规划");
        assert!(plan.is_no_source(), "空源应判为「无素材」");
        assert!(!plan.is_no_new_source(), "空源不是「无新素材」——两者要能区分");
    }

    #[test]
    fn scenario_copy_engine_plan_per_destination_targets() {
        // 文件在目的地 A 已完成、目的地 B 还缺 → 仍要拷，但只写 B
        let dir = tempfile::tempdir().expect("临时目录");
        let src = dir.path().join("src");
        let da = dir.path().join("A");
        let db = dir.path().join("B");
        touch(&src, "A001.MP4", 100);

        let spec = spec_for(&src, &[&da, &db]);
        let io = volume_io();
        let landing_a = plan_task(&spec, io.as_ref()).expect("规划").destinations[0]
            .landing_dir
            .clone();

        let h = crate::engine::hash_bytes(HashAlgorithm::Xxh64, &[b'x'; 100]);
        let mut m = Manifest::new(
            spec.source.clone(),
            "婚礼",
            &landing_a,
            HashAlgorithm::Xxh64,
            spec.at,
        );
        m.entries.push(ManifestEntry {
            relative_path: "A001.MP4".into(),
            size: 100,
            source_hash: h,
            verify: VerifyState::Verified {
                destination_hash: h,
            },
            source_modified_at: None,
            completed_at: spec.at,
            retries: 0,
        });
        std::fs::create_dir_all(&landing_a).expect("建 A 落地目录");
        write_manifest(&landing_a, &m).expect("写清单");
        std::fs::write(landing_a.join("A001.MP4"), vec![b'x'; 100]).expect("落地");

        let plan = plan_task(&spec, io.as_ref()).expect("再规划");
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].targets, vec![1], "只有目的地 B 还需要它");
        assert_eq!(plan.destinations[0].required_bytes, 0);
        assert_eq!(plan.destinations[1].required_bytes, 100);
    }

    #[test]
    fn scenario_copy_engine_plan_reports_available_space() {
        let dir = tempfile::tempdir().expect("临时目录");
        let src = dir.path().join("src");
        let dst = dir.path().join("dst");
        touch(&src, "A001.MP4", 100);
        let io = volume_io();
        let plan = plan_task(&spec_for(&src, &[&dst]), io.as_ref()).expect("规划");
        let d = &plan.destinations[0];
        assert!(d.available_bytes.is_some(), "本机应能查到可用空间");
        assert_eq!(d.sufficient(), Some(true));
        assert_eq!(d.shortfall(), Some(0));
    }

    #[test]
    fn scenario_copy_engine_plan_disabled_destination_excluded() {
        let dir = tempfile::tempdir().expect("临时目录");
        let src = dir.path().join("src");
        touch(&src, "A001.MP4", 10);
        let mut spec = spec_for(&src, &[&dir.path().join("A"), &dir.path().join("B")]);
        spec.destinations[1].enabled = false;
        let io = volume_io();
        let plan = plan_task(&spec, io.as_ref()).expect("规划");
        assert_eq!(plan.destinations.len(), 1);
        assert_eq!(plan.enabled_indices, vec![0]);
    }
}
