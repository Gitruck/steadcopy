//! 复验四态：一致 / 已移动 / 丢失 / 新增。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/verify-manifest/spec.md`
//! → Requirement: 复验产出四态结果
//!
//! 取 hashdeep 的 audit 抽象——比布尔式的「通过 / 失败」高一个层级。
//! 用户真正想知道的不是「有没有问题」，而是「**哪些**文件出了**什么**问题」。
//!
//! 语义约定：
//! - **新增**用警示语义而非危险语义——多出文件本身不是错误，只是告知；
//! - **丢失**才是危险语义——它意味着 manifest 记录的内容在目录里找不到了；
//! - 内容被就地篡改的文件会**同时**出现在「丢失」（原哈希无对应）与「新增」（当前形态无记录）里，
//!   这不是重复计数，而是如实描述发生了什么。
//!
//! 本模块是**纯算法**：输入是已算好的 (路径, 大小, 哈希) 列表，不碰 IO，
//! 因此可以完全用单元测试覆盖。哈希的实际计算由调用方负责（且 MUST 无缓冲读）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::engine::{HashAlgorithm, HashValue};
use crate::manifest::model::{normalize_relative, Manifest};

/// 复验时在目录里实际观察到的一个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedFile {
    pub relative_path: String,
    pub size: u64,
    pub hash: HashValue,
}

impl ObservedFile {
    pub fn new(relative_path: impl AsRef<str>, size: u64, hash: HashValue) -> Self {
        Self {
            relative_path: normalize_relative(relative_path.as_ref()),
            size,
            hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntactItem {
    pub relative_path: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovedItem {
    /// manifest 里记录的原路径
    pub from: String,
    /// 目录里现在的位置
    pub to: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingItem {
    pub relative_path: String,
    pub size: u64,
    /// manifest 记录的期望哈希（十六进制）
    pub expected_hash: String,
    /// 该条目在拷贝时是否做过校验。未校验的条目本身可信度就低，UI 应区别呈现。
    pub was_verified_at_copy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedItem {
    pub relative_path: String,
    pub size: u64,
    pub hash: String,
}

/// 复验报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditReport {
    pub algorithm: HashAlgorithm,
    pub intact: Vec<IntactItem>,
    pub moved: Vec<MovedItem>,
    pub missing: Vec<MissingItem>,
    pub added: Vec<AddedItem>,
    /// 复验是否完整跑完。用户中途取消时为 false，结果 MUST 标注不完整。
    pub complete: bool,
    /// manifest 中拷贝时未做校验的条目数（供 UI 提示可信度）
    pub unverified_at_copy: usize,
}

impl AuditReport {
    /// 数据是否完好。
    ///
    /// 判据只看**丢失**——「新增」不构成失败（用户往目录里放别的东西是正常的）。
    pub fn is_data_intact(&self) -> bool {
        self.missing.is_empty()
    }

    /// 四态计数，供 UI 并列呈现。
    pub fn counts(&self) -> AuditCounts {
        AuditCounts {
            intact: self.intact.len(),
            moved: self.moved.len(),
            missing: self.missing.len(),
            added: self.added.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditCounts {
    pub intact: usize,
    pub moved: usize,
    pub missing: usize,
    pub added: usize,
}

/// 执行四态比对。
///
/// 算法分四遍，用「认领」机制保证一个观察到的文件只归属一处，
/// 从而在存在重复内容（同哈希多份）时也不会重复计数：
///
/// 1. 路径与哈希都对上 → **一致**
/// 2. 剩余条目里，哈希能在未被认领的观察文件中找到 → **已移动**
/// 3. 仍未匹配的 manifest 条目 → **丢失**
/// 4. 未被任何条目认领的观察文件 → **新增**
pub fn audit(manifest: &Manifest, observed: &[ObservedFile], complete: bool) -> AuditReport {
    let mut by_path: HashMap<&str, usize> = HashMap::new();
    for (i, o) in observed.iter().enumerate() {
        by_path.insert(o.relative_path.as_str(), i);
    }

    let mut claimed = vec![false; observed.len()];
    let mut intact = Vec::new();
    let mut moved = Vec::new();
    let mut missing = Vec::new();
    let mut pending: Vec<&crate::manifest::model::ManifestEntry> = Vec::new();

    // 第 1 遍：路径 + 哈希都对上
    for e in &manifest.entries {
        match by_path.get(e.relative_path.as_str()) {
            Some(&idx) if observed[idx].hash.matches(&e.source_hash) => {
                claimed[idx] = true;
                intact.push(IntactItem {
                    relative_path: e.relative_path.clone(),
                    size: e.size,
                });
            }
            _ => pending.push(e),
        }
    }

    // 第 2 遍：哈希相同但路径变了 → 已移动
    for e in pending {
        let found = observed
            .iter()
            .enumerate()
            .find(|(i, o)| !claimed[*i] && o.hash.matches(&e.source_hash));
        match found {
            Some((idx, o)) => {
                claimed[idx] = true;
                moved.push(MovedItem {
                    from: e.relative_path.clone(),
                    to: o.relative_path.clone(),
                    size: e.size,
                });
            }
            // 第 3 遍：找不到 → 丢失
            None => missing.push(MissingItem {
                relative_path: e.relative_path.clone(),
                size: e.size,
                expected_hash: e.source_hash.to_hex(),
                was_verified_at_copy: e.counts_as_done(),
            }),
        }
    }

    // 第 4 遍：没人认领的观察文件 → 新增
    let added = observed
        .iter()
        .enumerate()
        .filter(|(i, _)| !claimed[*i])
        .map(|(_, o)| AddedItem {
            relative_path: o.relative_path.clone(),
            size: o.size,
            hash: o.hash.to_hex(),
        })
        .collect();

    AuditReport {
        algorithm: manifest.algorithm,
        intact,
        moved,
        missing,
        added,
        complete,
        unverified_at_copy: manifest
            .entries
            .iter()
            .filter(|e| !e.counts_as_done())
            .count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{hash_bytes, HashAlgorithm};
    use crate::manifest::model::{ManifestEntry, SourceRef, VerifyState};
    use time::macros::datetime;

    fn h(content: &str) -> HashValue {
        hash_bytes(HashAlgorithm::Xxh64, content.as_bytes())
    }

    fn entry(path: &str, content: &str, verified: bool) -> ManifestEntry {
        let hv = h(content);
        ManifestEntry {
            relative_path: path.into(),
            size: content.len() as u64,
            source_hash: hv,
            verify: if verified {
                VerifyState::Verified {
                    destination_hash: hv,
                }
            } else {
                VerifyState::NotVerified
            },
            source_modified_at: None,
            completed_at: datetime!(2026-08-08 09:30:00 UTC),
            retries: 0,
        }
    }

    fn manifest_with(entries: Vec<ManifestEntry>) -> Manifest {
        let mut m = Manifest::new(
            SourceRef {
                id: "vol-1".into(),
                display_name: "A7M4主卡".into(),
            },
            "婚礼",
            r"D:\素材\婚礼",
            HashAlgorithm::Xxh64,
            datetime!(2026-08-08 09:30:00 UTC),
        );
        m.entries = entries;
        m
    }

    fn observed(path: &str, content: &str) -> ObservedFile {
        ObservedFile::new(path, content.len() as u64, h(content))
    }

    // spec: verify-manifest → 复验产出四态结果 → Scenario: 全部一致
    #[test]
    fn scenario_verify_manifest_audit_all_intact() {
        let m = manifest_with(vec![
            entry("A001.MP4", "aaa", true),
            entry("A002.MP4", "bbb", true),
        ]);
        let obs = vec![observed("A001.MP4", "aaa"), observed("A002.MP4", "bbb")];
        let r = audit(&m, &obs, true);
        assert_eq!(r.counts(), AuditCounts { intact: 2, moved: 0, missing: 0, added: 0 });
        assert!(r.is_data_intact());
    }

    // spec: → Scenario: 文件被改名识别为已移动
    #[test]
    fn scenario_verify_manifest_audit_renamed_is_moved() {
        let m = manifest_with(vec![entry("A001.MP4", "aaa", true)]);
        let obs = vec![observed("归档/A001-final.MP4", "aaa")];
        let r = audit(&m, &obs, true);
        assert_eq!(r.counts(), AuditCounts { intact: 0, moved: 1, missing: 0, added: 0 });
        // 已移动 MUST NOT 同时计入丢失与新增
        assert_eq!(r.moved[0].from, "A001.MP4");
        assert_eq!(r.moved[0].to, "归档/A001-final.MP4");
        assert!(r.is_data_intact(), "内容还在，只是位置变了，不算失败");
    }

    // spec: → Scenario: 文件内容被篡改识别为丢失
    #[test]
    fn scenario_verify_manifest_audit_tampered_is_missing_and_added() {
        let m = manifest_with(vec![entry("A001.MP4", "aaa", true)]);
        // 同一路径，内容变了
        let obs = vec![observed("A001.MP4", "xxx")];
        let r = audit(&m, &obs, true);
        assert_eq!(r.counts(), AuditCounts { intact: 0, moved: 0, missing: 1, added: 1 });
        assert_eq!(r.missing[0].relative_path, "A001.MP4");
        assert_eq!(r.added[0].relative_path, "A001.MP4");
        assert!(!r.is_data_intact());
    }

    // spec: → Scenario: 文件被删除识别为丢失
    #[test]
    fn scenario_verify_manifest_audit_deleted_is_missing() {
        let m = manifest_with(vec![
            entry("A001.MP4", "aaa", true),
            entry("A002.MP4", "bbb", true),
        ]);
        let obs = vec![observed("A001.MP4", "aaa")];
        let r = audit(&m, &obs, true);
        assert_eq!(r.counts(), AuditCounts { intact: 1, moved: 0, missing: 1, added: 0 });
        assert_eq!(r.missing[0].relative_path, "A002.MP4");
        assert!(!r.is_data_intact());
    }

    // spec: → Scenario: 无关文件识别为新增
    #[test]
    fn scenario_verify_manifest_audit_extra_file_is_added_not_failure() {
        let m = manifest_with(vec![entry("A001.MP4", "aaa", true)]);
        let obs = vec![observed("A001.MP4", "aaa"), observed("我的笔记.txt", "note")];
        let r = audit(&m, &obs, true);
        assert_eq!(r.counts(), AuditCounts { intact: 1, moved: 0, missing: 0, added: 1 });
        assert!(
            r.is_data_intact(),
            "仅有新增 MUST NOT 判定为失败——多出文件不是错误"
        );
    }

    #[test]
    fn scenario_verify_manifest_audit_duplicate_content_not_double_claimed() {
        // 两个条目内容相同：一个原地在、一个被删。不能让在的那个同时满足两个条目。
        let m = manifest_with(vec![
            entry("A001.MP4", "same", true),
            entry("A002.MP4", "same", true),
        ]);
        let obs = vec![observed("A001.MP4", "same")];
        let r = audit(&m, &obs, true);
        assert_eq!(r.counts(), AuditCounts { intact: 1, moved: 0, missing: 1, added: 0 });
        assert_eq!(r.missing[0].relative_path, "A002.MP4");
    }

    #[test]
    fn scenario_verify_manifest_audit_swapped_paths() {
        // 两个文件互换了位置：都算「已移动」，不该有丢失或新增
        let m = manifest_with(vec![
            entry("A.MP4", "aaa", true),
            entry("B.MP4", "bbb", true),
        ]);
        let obs = vec![observed("A.MP4", "bbb"), observed("B.MP4", "aaa")];
        let r = audit(&m, &obs, true);
        assert_eq!(r.counts(), AuditCounts { intact: 0, moved: 2, missing: 0, added: 0 });
        assert!(r.is_data_intact());
    }

    #[test]
    fn scenario_verify_manifest_audit_reports_unverified_at_copy() {
        let m = manifest_with(vec![
            entry("A001.MP4", "aaa", true),
            entry("A002.MP4", "bbb", false),
        ]);
        let obs = vec![observed("A001.MP4", "aaa")];
        let r = audit(&m, &obs, true);
        assert_eq!(r.unverified_at_copy, 1);
        // 未校验条目丢失时，报告里要标出来它当初就没校验过
        assert_eq!(r.missing.len(), 1);
        assert!(!r.missing[0].was_verified_at_copy);
    }

    #[test]
    fn scenario_verify_manifest_audit_incomplete_is_marked() {
        let m = manifest_with(vec![entry("A001.MP4", "aaa", true)]);
        let r = audit(&m, &[], false);
        assert!(!r.complete, "取消后的结果 MUST 标注不完整");
    }

    #[test]
    fn scenario_verify_manifest_audit_path_separators_are_normalized() {
        let m = manifest_with(vec![entry("DCIM/A001.MP4", "aaa", true)]);
        let obs = vec![observed(r"DCIM\A001.MP4", "aaa")];
        let r = audit(&m, &obs, true);
        assert_eq!(r.counts().intact, 1);
    }

    #[test]
    fn scenario_verify_manifest_audit_empty_manifest_all_added() {
        let m = manifest_with(vec![]);
        let obs = vec![observed("x.mp4", "aaa")];
        let r = audit(&m, &obs, true);
        assert_eq!(r.counts(), AuditCounts { intact: 0, moved: 0, missing: 0, added: 1 });
        assert!(r.is_data_intact());
    }

    #[test]
    fn scenario_verify_manifest_audit_report_serializes() {
        let m = manifest_with(vec![entry("A001.MP4", "aaa", true)]);
        let r = audit(&m, &[observed("A001.MP4", "aaa")], true);
        let json = serde_json::to_string(&r).expect("序列化");
        for k in ["intact", "moved", "missing", "added", "complete"] {
            assert!(json.contains(k), "复验报告缺字段 {k}");
        }
    }
}
