//! 续传账本：由历史 manifest 合并出「已完成集合」。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/verify-manifest/spec.md`
//! → Requirement: 续传账本
//!
//! # 前身在这里做错了什么
//!
//! 前身用一张**全局**哈希表当账本：只按 (哈希, 阶段) 记录，不关联项目与目的地。
//! 后果有三个，都是真实的漏拷：
//!
//! 1. 同一文件拷进项目 A 之后，为项目 B 再拷同一张卡会被判「无新素材」直接跳过；
//! 2. 目的盘上的文件被用户删了，账本里还记着，照样跳过；
//! 3. 「校验通过」被当成「文件还在」，两者根本不是一回事。
//!
//! 本模块的账本作用域是 **目的地 × 源卡**，且判定「已完成」要**三个条件同时成立**：
//! 清单里有该条目、该条目**校验通过**、目的地上文件**实际存在且大小相符**。
//!
//! 另外：**MUST NOT 凭修改时间判定**。exFAT / FAT32 的时间戳是 2 秒粒度且受时区影响，
//! 拿它做增量判断会既漏又误。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::manifest::model::Manifest;
use crate::manifest::store::{load_manifests, ManifestReadIssue};

/// 账本里的一条「已完成」记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoneEntry {
    pub relative_path: String,
    pub size: u64,
}

/// 某个「落地目录 × 源卡」作用域下的续传账本。
#[derive(Debug, Default)]
pub struct ResumeLedger {
    landing_dir: PathBuf,
    done: HashMap<String, DoneEntry>,
    /// 非空表示读清单时出了问题，本次**已降级为全量拷贝**，原因需呈现给用户。
    pub degraded_reasons: Vec<String>,
}

impl ResumeLedger {
    /// 从落地目录加载属于指定源卡的历史清单。
    ///
    /// 任何清单异常都会让本次**降级为全量拷贝**（`done` 清空），
    /// 并把原因记进 `degraded_reasons`——**MUST NOT** 静默当作空账本，
    /// 否则「漏拷」会看起来像「本来就没拷过」。
    pub fn load(landing_dir: &Path, source_id: &str) -> Self {
        let loaded = load_manifests(landing_dir);
        let mut ledger = Self {
            landing_dir: landing_dir.to_path_buf(),
            done: HashMap::new(),
            degraded_reasons: Vec::new(),
        };

        for (path, issue) in &loaded.issues {
            ledger.degraded_reasons.push(format!(
                "{}：{}",
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string()),
                issue_text(issue)
            ));
        }

        if !ledger.degraded_reasons.is_empty() {
            // 有读不了的清单 → 无法确定账本完整 → 本次全量
            return ledger;
        }

        for (_, m) in &loaded.manifests {
            ledger.absorb(m, source_id);
        }
        ledger
    }

    /// 从空账本开始（用于测试与「强制全量」）。
    pub fn empty(landing_dir: &Path) -> Self {
        Self {
            landing_dir: landing_dir.to_path_buf(),
            done: HashMap::new(),
            degraded_reasons: Vec::new(),
        }
    }

    fn absorb(&mut self, m: &Manifest, source_id: &str) {
        // 作用域过滤：只认同一张卡的记录。别的卡拷进同一目录不构成「这张卡已拷过」。
        if m.source.id != source_id {
            return;
        }
        for e in &m.entries {
            // 未校验的条目不作为「已完成」依据
            if !e.counts_as_done() {
                continue;
            }
            self.done.insert(
                e.relative_path.clone(),
                DoneEntry {
                    relative_path: e.relative_path.clone(),
                    size: e.size,
                },
            );
        }
    }

    pub fn is_degraded(&self) -> bool {
        !self.degraded_reasons.is_empty()
    }

    /// 账本里记了多少个已完成文件。
    pub fn len(&self) -> usize {
        self.done.len()
    }

    pub fn is_empty(&self) -> bool {
        self.done.is_empty()
    }

    /// 该文件本次是否可以跳过。
    ///
    /// 三个条件同时成立才算「已完成」：
    /// 1. 账本里有这条记录（且当初**校验通过**）；
    /// 2. 记录的大小与本次源文件大小一致；
    /// 3. 目的地上该文件**现在确实存在**且大小相符。
    ///
    /// 第 3 条是对「校验通过 ≠ 文件还在」的正面回应。
    pub fn is_done(&self, relative_path: &str, source_size: u64) -> bool {
        let key = crate::manifest::model::normalize_relative(relative_path);
        let Some(entry) = self.done.get(&key) else {
            return false;
        };
        if entry.size != source_size {
            return false;
        }
        let landed = self.landing_dir.join(key.replace('/', std::path::MAIN_SEPARATOR_STR));
        match std::fs::metadata(&landed) {
            Ok(meta) => meta.is_file() && meta.len() == source_size,
            Err(_) => false,
        }
    }
}

fn issue_text(issue: &ManifestReadIssue) -> String {
    issue.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{hash_bytes, HashAlgorithm};
    use crate::manifest::model::{ManifestEntry, SourceRef, VerifyState};
    use crate::manifest::store::{manifest_dir, write_manifest};
    use time::macros::datetime;

    fn manifest_for(source_id: &str, project: &str, files: &[(&str, u64, bool)]) -> Manifest {
        let at = datetime!(2026-08-08 09:30:00 UTC);
        let mut m = Manifest::new(
            SourceRef {
                id: source_id.into(),
                display_name: "A7M4主卡".into(),
            },
            project,
            r"D:\素材",
            HashAlgorithm::Xxh64,
            at,
        );
        for (name, size, verified) in files {
            let h = hash_bytes(HashAlgorithm::Xxh64, name.as_bytes());
            m.entries.push(ManifestEntry {
                relative_path: (*name).into(),
                size: *size,
                source_hash: h,
                verify: if *verified {
                    VerifyState::Verified {
                        destination_hash: h,
                    }
                } else {
                    VerifyState::NotVerified
                },
                source_modified_at: None,
                completed_at: at,
                retries: 0,
            });
        }
        m
    }

    /// 在落地目录里造一个真实存在的目标文件
    fn land(dir: &Path, rel: &str, size: usize) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("建目录");
        }
        std::fs::write(&p, vec![0u8; size]).expect("写落地文件");
    }

    // spec: verify-manifest → 续传账本 → Scenario: 拔卡重插不重拷
    #[test]
    fn scenario_verify_manifest_resume_skips_completed() {
        let dir = tempfile::tempdir().expect("临时目录");
        let landing = dir.path();
        write_manifest(landing, &manifest_for("vol-1", "婚礼", &[("A001.MP4", 100, true)]))
            .expect("写清单");
        land(landing, "A001.MP4", 100);

        let ledger = ResumeLedger::load(landing, "vol-1");
        assert!(!ledger.is_degraded());
        assert!(ledger.is_done("A001.MP4", 100), "已完成的文件应被跳过");
        assert!(!ledger.is_done("A002.MP4", 100), "没拷过的不该被跳过");
    }

    // spec: → Scenario: 目的地文件被删除则重拷
    #[test]
    fn scenario_verify_manifest_resume_recopies_when_destination_file_deleted() {
        let dir = tempfile::tempdir().expect("临时目录");
        let landing = dir.path();
        write_manifest(landing, &manifest_for("vol-1", "婚礼", &[("A001.MP4", 100, true)]))
            .expect("写清单");
        // 刻意不创建落地文件——模拟用户把它删了
        let ledger = ResumeLedger::load(landing, "vol-1");
        assert!(
            !ledger.is_done("A001.MP4", 100),
            "账本有记录但文件不在，MUST 重拷"
        );
    }

    #[test]
    fn scenario_verify_manifest_resume_recopies_when_size_mismatch() {
        let dir = tempfile::tempdir().expect("临时目录");
        let landing = dir.path();
        write_manifest(landing, &manifest_for("vol-1", "婚礼", &[("A001.MP4", 100, true)]))
            .expect("写清单");
        land(landing, "A001.MP4", 40); // 只写了一半
        let ledger = ResumeLedger::load(landing, "vol-1");
        assert!(!ledger.is_done("A001.MP4", 100), "大小不符 MUST 重拷");
    }

    // spec: → Scenario: 未校验条目不作为已完成依据
    #[test]
    fn scenario_verify_manifest_resume_ignores_unverified_entries() {
        let dir = tempfile::tempdir().expect("临时目录");
        let landing = dir.path();
        write_manifest(
            landing,
            &manifest_for("vol-1", "婚礼", &[("A001.MP4", 100, false)]),
        )
        .expect("写清单");
        land(landing, "A001.MP4", 100);
        let ledger = ResumeLedger::load(landing, "vol-1");
        assert!(
            !ledger.is_done("A001.MP4", 100),
            "拷贝时没校验过的条目 MUST NOT 作为已完成依据"
        );
    }

    // spec: manifest 落盘位置与作用域 → Scenario: 跨项目不误判
    #[test]
    fn scenario_verify_manifest_resume_scope_is_per_source_card() {
        let dir = tempfile::tempdir().expect("临时目录");
        let landing = dir.path();
        // 另一张卡往同一目录拷过同名文件
        write_manifest(landing, &manifest_for("vol-OTHER", "别的项目", &[("A001.MP4", 100, true)]))
            .expect("写清单");
        land(landing, "A001.MP4", 100);

        let ledger = ResumeLedger::load(landing, "vol-1");
        assert!(
            !ledger.is_done("A001.MP4", 100),
            "别的卡拷过 MUST NOT 让本张卡被判已完成"
        );
        assert_eq!(ledger.len(), 0);
    }

    #[test]
    fn scenario_verify_manifest_resume_merges_multiple_manifests() {
        let dir = tempfile::tempdir().expect("临时目录");
        let landing = dir.path();
        write_manifest(landing, &manifest_for("vol-1", "婚礼", &[("A001.MP4", 10, true)]))
            .expect("第一次");
        let mut second = manifest_for("vol-1", "婚礼", &[("A002.MP4", 20, true)]);
        second.created_at = datetime!(2026-08-08 14:00:00 UTC);
        write_manifest(landing, &second).expect("第二次");
        land(landing, "A001.MP4", 10);
        land(landing, "A002.MP4", 20);

        let ledger = ResumeLedger::load(landing, "vol-1");
        assert_eq!(ledger.len(), 2);
        assert!(ledger.is_done("A001.MP4", 10));
        assert!(ledger.is_done("A002.MP4", 20));
    }

    // spec: manifest 异常处理 → Scenario: 损坏的 manifest 降级为全量
    #[test]
    fn scenario_verify_manifest_resume_degrades_to_full_on_broken_manifest() {
        let dir = tempfile::tempdir().expect("临时目录");
        let landing = dir.path();
        write_manifest(landing, &manifest_for("vol-1", "婚礼", &[("A001.MP4", 100, true)]))
            .expect("好清单");
        land(landing, "A001.MP4", 100);
        // 再塞一个坏清单
        std::fs::write(manifest_dir(landing).join("broken.json"), b"{ not json")
            .expect("写坏清单");

        let ledger = ResumeLedger::load(landing, "vol-1");
        assert!(ledger.is_degraded(), "有坏清单时 MUST 标记降级");
        assert!(
            !ledger.is_done("A001.MP4", 100),
            "降级后应全量拷贝，MUST NOT 沿用可能不完整的账本"
        );
        assert!(!ledger.degraded_reasons.is_empty());
        assert!(
            ledger.degraded_reasons[0].contains("broken.json"),
            "降级原因应指明是哪个文件：{:?}",
            ledger.degraded_reasons
        );
    }

    #[test]
    fn scenario_verify_manifest_resume_no_history_is_empty_not_degraded() {
        let dir = tempfile::tempdir().expect("临时目录");
        let ledger = ResumeLedger::load(dir.path(), "vol-1");
        assert!(ledger.is_empty());
        assert!(!ledger.is_degraded(), "第一次拷不算降级");
    }

    #[test]
    fn scenario_verify_manifest_resume_handles_nested_paths() {
        let dir = tempfile::tempdir().expect("临时目录");
        let landing = dir.path();
        write_manifest(
            landing,
            &manifest_for("vol-1", "婚礼", &[("DCIM/100MSDCF/A001.MP4", 50, true)]),
        )
        .expect("写清单");
        land(landing, "DCIM/100MSDCF/A001.MP4", 50);
        let ledger = ResumeLedger::load(landing, "vol-1");
        assert!(ledger.is_done("DCIM/100MSDCF/A001.MP4", 50));
        // 分隔符风格不影响判定
        assert!(ledger.is_done(r"DCIM\100MSDCF\A001.MP4", 50));
    }

    #[test]
    fn scenario_verify_manifest_resume_does_not_use_mtime() {
        // 落地文件的 mtime 与源不同（exFAT 2 秒粒度 / 时区偏移的常态），
        // 只要大小与哈希记录对得上就应判为已完成。
        let dir = tempfile::tempdir().expect("临时目录");
        let landing = dir.path();
        write_manifest(landing, &manifest_for("vol-1", "婚礼", &[("A001.MP4", 100, true)]))
            .expect("写清单");
        land(landing, "A001.MP4", 100);
        // 不去动 mtime——判定逻辑里本来就不该读它。这里断言判定成立即证明未依赖 mtime。
        let ledger = ResumeLedger::load(landing, "vol-1");
        assert!(ledger.is_done("A001.MP4", 100));
    }
}
