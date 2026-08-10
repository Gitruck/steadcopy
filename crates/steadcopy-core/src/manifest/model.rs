//! manifest 数据模型：一次任务在一个目的地落下的校验清单。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/verify-manifest/spec.md`
//! → Requirement: manifest 格式与内容
//!
//! manifest 同时承担三个角色：**校验凭证**（用户可据以复验）、**续传账本**
//! （下次任务据以跳过已完成文件）、**交付物**（随数据走，脱离本应用也能被读懂）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::engine::{HashAlgorithm, HashValue};

/// manifest 格式版本。读到更高版本时 MUST 降级为全量拷贝而非强行解析。
pub const MANIFEST_FORMAT_VERSION: u32 = 1;

/// 生成本 manifest 的工具。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generator {
    pub name: String,
    pub version: String,
}

impl Default for Generator {
    fn default() -> Self {
        Self {
            name: "steadcopy".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        }
    }
}

/// 源设备引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    /// 设备身份（本机记忆库主键的稳定字符串形态）
    pub id: String,
    /// 用户看到的名字
    pub display_name: String,
}

/// 一个条目的校验状态。
///
/// 用枚举而非「可空的目的地哈希字段」承载，是为了让
/// 「拿源哈希填充目的地哈希」这件事在类型层面写不出来。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VerifyState {
    /// 本次任务关闭了校验——**不可**作为续传时「已完成」的依据
    NotVerified,
    /// 已从目的地无缓冲读回并与源哈希比对通过
    Verified { destination_hash: HashValue },
}

impl VerifyState {
    pub const fn is_verified(&self) -> bool {
        matches!(self, VerifyState::Verified { .. })
    }

    pub const fn destination_hash(&self) -> Option<&HashValue> {
        match self {
            VerifyState::Verified { destination_hash } => Some(destination_hash),
            VerifyState::NotVerified => None,
        }
    }
}

/// 一个文件条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// 相对于目的地根的路径，**一律用 `/` 分隔**，便于跨平台阅读与比对
    pub relative_path: String,
    pub size: u64,
    pub source_hash: HashValue,
    #[serde(flatten)]
    pub verify: VerifyState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "time::serde::rfc3339::option")]
    pub source_modified_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub completed_at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub retries: u32,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

impl ManifestEntry {
    /// 该条目是否可作为续传时「已完成」的依据。
    ///
    /// 仅有记录不够——**必须校验通过**。未校验条目在后续开启校验的任务中要重拷。
    pub const fn counts_as_done(&self) -> bool {
        self.verify.is_verified()
    }
}

/// 一份 manifest。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub generator: Generator,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub source: SourceRef,
    pub project: String,
    pub destination_root: PathBuf,
    pub algorithm: HashAlgorithm,
    pub entries: Vec<ManifestEntry>,
}

impl Manifest {
    pub fn new(
        source: SourceRef,
        project: impl Into<String>,
        destination_root: impl Into<PathBuf>,
        algorithm: HashAlgorithm,
        created_at: OffsetDateTime,
    ) -> Self {
        Self {
            format_version: MANIFEST_FORMAT_VERSION,
            generator: Generator::default(),
            created_at,
            source,
            project: project.into(),
            destination_root: destination_root.into(),
            algorithm,
            entries: Vec::new(),
        }
    }

    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().map(|e| e.size).sum()
    }

    pub fn verified_count(&self) -> usize {
        self.entries.iter().filter(|e| e.counts_as_done()).count()
    }

    /// 按相对路径查条目。
    pub fn entry(&self, relative_path: &str) -> Option<&ManifestEntry> {
        let key = normalize_relative(relative_path);
        self.entries.iter().find(|e| e.relative_path == key)
    }
}

/// 把一个相对路径归一为 manifest 里存的形态：`/` 分隔、去掉首尾分隔符。
pub fn normalize_relative(path: &str) -> String {
    path.split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// 由 `Path` 得到归一后的相对路径串。
pub fn relative_of(base: &Path, full: &Path) -> Option<String> {
    let rel = full.strip_prefix(base).ok()?;
    Some(normalize_relative(&rel.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{hash_bytes, HashAlgorithm};
    use time::macros::datetime;

    fn src() -> SourceRef {
        SourceRef {
            id: "vol-guid-1234".into(),
            display_name: "A7M4主卡".into(),
        }
    }

    fn entry(name: &str, verified: bool) -> ManifestEntry {
        let h = hash_bytes(HashAlgorithm::Xxh64, name.as_bytes());
        ManifestEntry {
            relative_path: name.into(),
            size: 1024,
            source_hash: h,
            verify: if verified {
                VerifyState::Verified {
                    destination_hash: h,
                }
            } else {
                VerifyState::NotVerified
            },
            source_modified_at: Some(datetime!(2026-08-08 09:00:00 UTC)),
            completed_at: datetime!(2026-08-08 09:30:00 UTC),
            retries: 0,
        }
    }

    fn manifest() -> Manifest {
        let mut m = Manifest::new(
            src(),
            "婚礼",
            r"D:\素材\婚礼",
            HashAlgorithm::Xxh64,
            datetime!(2026-08-08 09:30:00 UTC),
        );
        m.entries.push(entry("A001.MP4", true));
        m.entries.push(entry("A002.MP4", true));
        m
    }

    // spec: verify-manifest → manifest 格式与内容 → Scenario: manifest 字段完整
    #[test]
    fn scenario_verify_manifest_fields_are_complete() {
        let m = manifest();
        let json = serde_json::to_string_pretty(&m).expect("序列化");
        for field in [
            "format_version",
            "generator",
            "created_at",
            "source",
            "project",
            "destination_root",
            "algorithm",
            "entries",
            "relative_path",
            "size",
            "source_hash",
            "completed_at",
        ] {
            assert!(json.contains(field), "manifest 缺字段 {field}：{json}");
        }
        assert_eq!(m.format_version, MANIFEST_FORMAT_VERSION);
        assert_eq!(m.generator.name, "steadcopy");
        assert!(!m.generator.version.is_empty());
    }

    #[test]
    fn scenario_verify_manifest_roundtrips() {
        let m = manifest();
        let json = serde_json::to_string(&m).expect("序列化");
        let back: Manifest = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(back, m);
    }

    #[test]
    fn scenario_verify_manifest_timestamps_carry_offset() {
        // 时间 MUST 带时区，否则跨时区读 manifest 会错判
        let m = manifest();
        let json = serde_json::to_string(&m).expect("序列化");
        assert!(
            json.contains("2026-08-08T09:30:00Z") || json.contains("+00:00"),
            "时间应为带偏移的 RFC3339：{json}"
        );
    }

    // spec: verify-manifest → manifest 格式与内容 → Scenario: 未校验时明确标注
    #[test]
    fn scenario_verify_manifest_unverified_is_marked() {
        let e = entry("A003.MP4", false);
        assert!(!e.verify.is_verified());
        assert_eq!(e.verify.destination_hash(), None);

        let json = serde_json::to_string(&e).expect("序列化");
        assert!(json.contains("not_verified"), "应显式标注未校验：{json}");
        assert!(
            !json.contains("destination_hash"),
            "未校验时 MUST NOT 出现目的地哈希字段：{json}"
        );
    }

    #[test]
    fn scenario_verify_manifest_unverified_does_not_count_as_done() {
        // 续传的关键：仅有记录不够，必须校验通过
        assert!(entry("A001.MP4", true).counts_as_done());
        assert!(!entry("A002.MP4", false).counts_as_done());

        let mut m = manifest();
        m.entries.push(entry("A003.MP4", false));
        assert_eq!(m.entries.len(), 3);
        assert_eq!(m.verified_count(), 2);
    }

    #[test]
    fn scenario_verify_manifest_source_hash_is_not_reused_as_destination() {
        // 类型层面：NotVerified 分支根本没有目的地哈希字段可填。
        // 这里断言未校验条目取不到目的地哈希，不存在「用源哈希顶上」的路径。
        let e = entry("A004.MP4", false);
        assert!(e.verify.destination_hash().is_none());
        assert!(matches!(e.verify, VerifyState::NotVerified));
    }

    #[test]
    fn scenario_verify_manifest_relative_paths_are_normalized() {
        assert_eq!(normalize_relative(r"\DCIM\100MSDCF\A001.MP4"), "DCIM/100MSDCF/A001.MP4");
        assert_eq!(normalize_relative("/a//b/"), "a/b");
        assert_eq!(normalize_relative("a/b"), "a/b");
        assert_eq!(normalize_relative(""), "");
    }

    #[test]
    fn scenario_verify_manifest_relative_of_base() {
        let base = Path::new(r"D:\素材\婚礼");
        let full = Path::new(r"D:\素材\婚礼\DCIM\A001.MP4");
        assert_eq!(relative_of(base, full).as_deref(), Some("DCIM/A001.MP4"));
        // 不在 base 之下时返回 None，不静默产生怪路径
        assert_eq!(relative_of(base, Path::new(r"E:\别处\x.mp4")), None);
    }

    #[test]
    fn scenario_verify_manifest_entry_lookup_is_separator_agnostic() {
        let mut m = manifest();
        m.entries.push(ManifestEntry {
            relative_path: "DCIM/100MSDCF/A003.MP4".into(),
            ..entry("A003.MP4", true)
        });
        assert!(m.entry(r"DCIM\100MSDCF\A003.MP4").is_some());
        assert!(m.entry("DCIM/100MSDCF/A003.MP4").is_some());
        assert!(m.entry("不存在.MP4").is_none());
    }

    #[test]
    fn scenario_verify_manifest_totals() {
        let m = manifest();
        assert_eq!(m.total_bytes(), 2048);
    }
}
