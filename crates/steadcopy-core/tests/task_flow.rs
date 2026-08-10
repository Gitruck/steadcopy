#![allow(clippy::unwrap_used, clippy::expect_used)]
//! 任务闭环集成测试：扫描 → 规划 → 拷贝 → 校验 → 清单 → 续传 → 复验。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/copy-engine/spec.md`
//!       `openspec/changes/add-steadcopy-core/specs/verify-manifest/spec.md`
//!
//! 纪律 T3：全部走 `tempfile::TempDir` 真实 IO，只 mock 时钟（避免重试退避真 sleep）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use steadcopy_core::engine::{hash_bytes, CancelToken, HashAlgorithm};
use steadcopy_core::manifest::model::SourceRef;
use steadcopy_core::manifest::{audit, load_manifests, ObservedFile, ResumeLedger};
use steadcopy_core::organize::{scan_source, PathTemplate, ScanOptions};
use steadcopy_core::platform::{volume_io, Clock, VolumeIo};
use steadcopy_core::task::{plan_task, run_task, DestinationSpec, StageEvent, TaskSpec};
use time::macros::datetime;
use time::OffsetDateTime;

/// 可控时钟：不真 sleep，只累计「被要求睡了多久」。
#[derive(Default)]
struct MockClock {
    slept_ms: AtomicU64,
}

impl Clock for MockClock {
    fn now(&self) -> OffsetDateTime {
        datetime!(2026-08-08 09:30:00 UTC)
    }
    fn sleep(&self, d: Duration) {
        self.slept_ms.fetch_add(d.as_millis() as u64, Ordering::SeqCst);
    }
}

fn touch(root: &Path, rel: &str, content: &[u8]) {
    let p = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("建目录");
    }
    std::fs::write(&p, content).expect("写文件");
}

fn spec_for(src: &Path, dests: &[&Path], verify: bool) -> TaskSpec {
    TaskSpec {
        source_root: src.to_path_buf(),
        source: SourceRef {
            id: "vol-test-1".into(),
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
        verify,
        scan: ScanOptions::mirror(),
        retries: 2,
        at: datetime!(2026-08-08 09:30:00 UTC),
    }
}

fn sample_card(root: &Path) {
    touch(root, "DCIM/100MSDCF/DSC00001.JPG", b"photo-one");
    touch(root, "PRIVATE/M4ROOT/CLIP/C0001.MP4", &vec![b'v'; 200_000]);
    touch(root, "PRIVATE/M4ROOT/CLIP/C0001M01.XML", b"<meta/>");
    touch(root, "MISC/AUTPRINT.MRK", b"mark");
    touch(root, "System Volume Information/IndexerVolumeGuid", b"junk");
}

struct Harness {
    _dir: tempfile::TempDir,
    src: PathBuf,
    dest_a: PathBuf,
    dest_b: PathBuf,
    io: Box<dyn VolumeIo>,
    clock: MockClock,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().expect("临时目录");
    let src = dir.path().join("card");
    let dest_a = dir.path().join("工作盘");
    let dest_b = dir.path().join("备份盘");
    sample_card(&src);
    Harness {
        _dir: dir,
        src,
        dest_a,
        dest_b,
        io: volume_io(),
        clock: MockClock::default(),
    }
}

fn run(h: &Harness, spec: &TaskSpec) -> (steadcopy_core::task::TaskReport, Vec<StageEvent>) {
    let plan = plan_task(spec, h.io.as_ref()).expect("规划");
    let mut events = Vec::new();
    let report = run_task(
        spec,
        &plan,
        h.io.as_ref(),
        &h.clock,
        &CancelToken::new(),
        &mut |e| events.push(e),
    )
    .expect("执行");
    (report, events)
}

#[test]
fn scenario_copy_engine_full_flow_two_destinations_verified() {
    let h = harness();
    let spec = spec_for(&h.src, &[&h.dest_a, &h.dest_b], true);
    let (report, events) = run(&h, &spec);

    assert!(report.all_succeeded(), "应全部成功：{:?}", report.failed_files().collect::<Vec<_>>());
    assert_eq!(report.copied_count(), 4, "整卡镜像应拷 4 个（垃圾已排除）");
    assert_eq!(report.manifests.len(), 2, "两个目的地各落一份清单");

    // 阶段按序上报
    let stages: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            StageEvent::Stage(s) => Some(*s),
            _ => None,
        })
        .collect();
    assert_eq!(
        stages,
        vec![
            steadcopy_core::task::TaskStage::Copying,
            steadcopy_core::task::TaskStage::Finishing,
            steadcopy_core::task::TaskStage::Finished,
        ]
    );

    // 两个目的地内容都与源一致
    for dest in [&h.dest_a, &h.dest_b] {
        let landing = dest.join("婚礼").join("2026-08-08").join("A7M4主卡");
        for rel in [
            "DCIM/100MSDCF/DSC00001.JPG",
            "PRIVATE/M4ROOT/CLIP/C0001.MP4",
            "PRIVATE/M4ROOT/CLIP/C0001M01.XML",
            "MISC/AUTPRINT.MRK",
        ] {
            let landed = landing.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            let original = h.src.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            assert_eq!(
                std::fs::read(&landed).expect("读落地文件"),
                std::fs::read(&original).expect("读源文件"),
                "{rel} 在 {dest:?} 内容不符"
            );
        }
        // 系统垃圾不该被拷过去
        assert!(!landing.join("System Volume Information").exists());
    }
}

#[test]
fn scenario_verify_manifest_written_with_verified_entries() {
    let h = harness();
    let spec = spec_for(&h.src, &[&h.dest_a], true);
    let (report, _) = run(&h, &spec);
    assert!(report.all_succeeded());

    let landing = h.dest_a.join("婚礼").join("2026-08-08").join("A7M4主卡");
    let loaded = load_manifests(&landing);
    assert!(!loaded.has_issues());
    assert_eq!(loaded.manifests.len(), 1);

    let m = &loaded.manifests[0].1;
    assert_eq!(m.entries.len(), 4);
    assert_eq!(m.verified_count(), 4, "开启校验时每条都应是已校验");
    assert_eq!(m.project, "婚礼");
    assert_eq!(m.source.id, "vol-test-1");
    assert_eq!(m.algorithm, HashAlgorithm::Xxh64);

    // 清单里的哈希与真实内容对得上
    let e = m.entry("MISC/AUTPRINT.MRK").expect("条目");
    assert!(e.source_hash.matches(&hash_bytes(HashAlgorithm::Xxh64, b"mark")));
}

#[test]
fn scenario_copy_engine_second_run_is_no_new_source() {
    let h = harness();
    let spec = spec_for(&h.src, &[&h.dest_a], true);
    let (first, _) = run(&h, &spec);
    assert_eq!(first.copied_count(), 4);

    // 同一张卡再跑一次：应全部跳过
    let plan = plan_task(&spec, h.io.as_ref()).expect("再规划");
    assert!(plan.is_no_new_source(), "第二次应判为「无新素材」");
    assert_eq!(plan.files.len(), 0);
    assert_eq!(plan.skipped.len(), 4);
    assert_eq!(plan.destinations[0].required_bytes, 0);
}

#[test]
fn scenario_copy_engine_resume_after_partial_copy() {
    let h = harness();
    let spec = spec_for(&h.src, &[&h.dest_a], true);
    let (_, _) = run(&h, &spec);

    let landing = h.dest_a.join("婚礼").join("2026-08-08").join("A7M4主卡");
    // 用户删掉了一个已拷文件
    std::fs::remove_file(landing.join("MISC").join("AUTPRINT.MRK")).expect("删文件");

    let plan = plan_task(&spec, h.io.as_ref()).expect("再规划");
    assert_eq!(plan.files.len(), 1, "被删的那个 MUST 重拷");
    assert_eq!(plan.files[0].file.relative_path, "MISC/AUTPRINT.MRK");
    assert_eq!(plan.skipped.len(), 3);
}

#[test]
fn scenario_verify_manifest_audit_after_copy_is_all_intact() {
    let h = harness();
    let spec = spec_for(&h.src, &[&h.dest_a], true);
    run(&h, &spec);

    let landing = h.dest_a.join("婚礼").join("2026-08-08").join("A7M4主卡");
    let m = &load_manifests(&landing).manifests[0].1.clone();

    // 复验：扫描落地目录（跳过凭证目录），算哈希，四态比对
    let observed = observe(&landing);
    let r = audit(m, &observed, true);
    assert_eq!(r.counts().intact, 4);
    assert_eq!(r.counts().missing, 0);
    assert_eq!(r.counts().added, 0, "凭证目录 MUST NOT 被报成新增");
    assert!(r.is_data_intact());
}

#[test]
fn scenario_verify_manifest_audit_detects_deletion_and_move() {
    let h = harness();
    let spec = spec_for(&h.src, &[&h.dest_a], true);
    run(&h, &spec);

    let landing = h.dest_a.join("婚礼").join("2026-08-08").join("A7M4主卡");
    let m = load_manifests(&landing).manifests[0].1.clone();

    // 删一个、移一个
    std::fs::remove_file(landing.join("MISC").join("AUTPRINT.MRK")).expect("删");
    let from = landing
        .join("DCIM")
        .join("100MSDCF")
        .join("DSC00001.JPG");
    let to = landing.join("DSC00001.JPG");
    std::fs::rename(&from, &to).expect("移动");

    let r = audit(&m, &observe(&landing), true);
    assert_eq!(r.counts().missing, 1, "删掉的应报丢失");
    assert_eq!(r.counts().moved, 1, "移动的应报已移动，而非丢失+新增");
    assert_eq!(r.moved[0].to, "DSC00001.JPG");
    assert!(!r.is_data_intact());
}

#[test]
fn scenario_copy_engine_verify_disabled_marks_entries_unverified() {
    let h = harness();
    let spec = spec_for(&h.src, &[&h.dest_a], false);
    let (report, events) = run(&h, &spec);
    assert!(report.all_succeeded());

    // 关闭校验时不应出现校验阶段
    assert!(!events.iter().any(|e| matches!(
        e,
        StageEvent::Stage(steadcopy_core::task::TaskStage::Verifying)
    )));

    let landing = h.dest_a.join("婚礼").join("2026-08-08").join("A7M4主卡");
    let m = &load_manifests(&landing).manifests[0].1;
    assert_eq!(m.verified_count(), 0, "关闭校验时条目应全部标为未校验");

    // 未校验条目 MUST NOT 作为续传的已完成依据
    let ledger = ResumeLedger::load(&landing, "vol-test-1");
    assert!(!ledger.is_done("MISC/AUTPRINT.MRK", 4));
}

#[test]
fn scenario_copy_engine_source_card_is_untouched() {
    let h = harness();
    let before = snapshot(&h.src);
    let spec = spec_for(&h.src, &[&h.dest_a, &h.dest_b], true);
    run(&h, &spec);
    let after = snapshot(&h.src);
    assert_eq!(before, after, "任务完成后源卡内容 MUST 完全未变");
}

#[test]
fn scenario_copy_engine_no_new_source_writes_no_manifest() {
    let h = harness();
    let spec = spec_for(&h.src, &[&h.dest_a], true);
    run(&h, &spec);
    let landing = h.dest_a.join("婚礼").join("2026-08-08").join("A7M4主卡");
    let before = load_manifests(&landing).manifests.len();

    // 第二次跑：无新素材，不该再落一份空清单
    let (report2, _) = run(&h, &spec);
    assert_eq!(report2.copied_count(), 0);
    assert_eq!(report2.skipped_count(), 4);
    assert_eq!(
        load_manifests(&landing).manifests.len(),
        before,
        "无新素材时 MUST NOT 落空清单"
    );
}

// ---- 辅助 ----

/// 扫描落地目录得到观察集合（跳过凭证目录）。
fn observe(landing: &Path) -> Vec<ObservedFile> {
    let io = volume_io();
    scan_source(landing, &ScanOptions::mirror())
        .files
        .into_iter()
        .filter(|f| !steadcopy_core::manifest::is_manifest_path(landing, &f.absolute_path))
        .map(|f| {
            let hash = steadcopy_core::engine::hash_destination(
                io.as_ref(),
                &f.absolute_path,
                HashAlgorithm::Xxh64,
            )
            .expect("算哈希");
            ObservedFile::new(&f.relative_path, f.size, hash)
        })
        .collect()
}

/// 目录内容快照（路径 + 大小 + 内容哈希），用于证明源卡未被改动。
fn snapshot(root: &Path) -> Vec<(String, u64, String)> {
    let mut out: Vec<(String, u64, String)> = walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            let rel = e
                .path()
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            let data = std::fs::read(e.path()).unwrap_or_default();
            let len = data.len() as u64;
            (rel, len, hash_bytes(HashAlgorithm::Xxh64, &data).to_hex())
        })
        .collect();
    out.sort();
    out
}
