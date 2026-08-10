//! manifest 落盘与读取。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/verify-manifest/spec.md`
//! → Requirement: manifest 落盘位置与作用域 / manifest 异常处理
//!
//! manifest 落在**它所描述数据的目录内**（`<落地目录>/steadcopy/`），
//! 因此用户把整个目的地目录搬到别处，凭证也跟着走。
//!
//! 目录取可见名（不加 `.` 前缀）是刻意的：这份东西是给用户的**凭证**，
//! 藏起来就失去了一半意义。同一目录多次拷贝按时间戳各留一份，互不覆盖。

use std::path::{Path, PathBuf};

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::error::{CoreError, ErrorContext, Result, TerminalKind};
use crate::manifest::model::{Manifest, MANIFEST_FORMAT_VERSION};

/// 落地目录下承载凭证的子目录名。
pub const MANIFEST_DIR: &str = "steadcopy";

/// 读取 manifest 时可能遇到的异常。
///
/// 全部以**显式错误**呈现——**MUST NOT** 静默当作空账本，
/// 那会让「漏拷」看起来像「没有新素材」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestReadIssue {
    /// 文件读不了
    Unreadable(String),
    /// JSON 非法或被截断
    Malformed(String),
    /// 格式版本高于本程序可识别的版本
    FutureVersion { found: u32, supported: u32 },
}

impl std::fmt::Display for ManifestReadIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestReadIssue::Unreadable(e) => write!(f, "清单文件读取失败：{e}"),
            ManifestReadIssue::Malformed(e) => {
                write!(f, "清单文件内容损坏或不完整：{e}")
            }
            ManifestReadIssue::FutureVersion { found, supported } => write!(
                f,
                "清单由更新版本的程序生成（格式版本 {found}，本程序支持到 {supported}），请升级后再读"
            ),
        }
    }
}

/// 一次目录扫描的结果：读到的 manifest + 读不了的那些（带原因）。
#[derive(Debug, Default)]
pub struct LoadedManifests {
    pub manifests: Vec<(PathBuf, Manifest)>,
    /// 有问题的清单文件。调用方 MUST 把它呈现给用户并降级为全量拷贝。
    pub issues: Vec<(PathBuf, ManifestReadIssue)>,
}

impl LoadedManifests {
    pub fn has_issues(&self) -> bool {
        !self.issues.is_empty()
    }
}

/// 落地目录下的凭证目录。
pub fn manifest_dir(landing_dir: &Path) -> PathBuf {
    landing_dir.join(MANIFEST_DIR)
}

/// 判断某路径是否位于凭证目录内。
///
/// 复验扫描时 MUST 跳过它——否则凭证自身会被报成「新增」文件。
pub fn is_manifest_path(landing_dir: &Path, path: &Path) -> bool {
    path.starts_with(manifest_dir(landing_dir))
}

fn timestamp_slug(at: OffsetDateTime) -> String {
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        at.year(),
        at.month() as u8,
        at.day(),
        at.hour(),
        at.minute(),
        at.second()
    )
}

/// 生成本次任务的 manifest 文件名（不含扩展名）。
///
/// 含时间戳与源设备标识片段——同一目录多次拷贝互不覆盖，且一眼能看出是哪张卡哪一次。
pub fn manifest_stem(manifest: &Manifest) -> String {
    let src: String = manifest
        .source
        .id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    let src = if src.is_empty() { "src".into() } else { src };
    format!("{}-{}", timestamp_slug(manifest.created_at), src)
}

/// 把 manifest 写进落地目录的凭证目录。返回写出的 JSON 文件路径。
pub fn write_manifest(landing_dir: &Path, manifest: &Manifest) -> Result<PathBuf> {
    let dir = manifest_dir(landing_dir);
    std::fs::create_dir_all(&dir).map_err(|e| {
        CoreError::Terminal(
            TerminalKind::InvalidConfig,
            ErrorContext::new()
                .path(&dir)
                .cause(format!("创建凭证目录失败：{e}")),
        )
    })?;

    let mut path = dir.join(format!("{}.json", manifest_stem(manifest)));
    // 极端情况下同一秒内两次任务：加序号避免覆盖
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{}-{n}.json", manifest_stem(manifest)));
        n += 1;
    }

    let json = serde_json::to_string_pretty(manifest).map_err(|e| {
        CoreError::Terminal(
            TerminalKind::InvalidConfig,
            ErrorContext::new().cause(format!("序列化清单失败：{e}")),
        )
    })?;
    std::fs::write(&path, json).map_err(|e| {
        CoreError::Terminal(
            TerminalKind::InvalidConfig,
            ErrorContext::new()
                .path(&path)
                .cause(format!("写入清单失败：{e}")),
        )
    })?;
    Ok(path)
}

/// 读一份 manifest。异常一律显式返回，**不静默**。
pub fn read_manifest(path: &Path) -> std::result::Result<Manifest, ManifestReadIssue> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ManifestReadIssue::Unreadable(e.to_string()))?;

    // 先只取版本号：版本比我们新时不要按当前结构强行解析
    #[derive(serde::Deserialize)]
    struct VersionProbe {
        format_version: u32,
    }
    let probe: VersionProbe = serde_json::from_str(&text)
        .map_err(|e| ManifestReadIssue::Malformed(e.to_string()))?;
    if probe.format_version > MANIFEST_FORMAT_VERSION {
        return Err(ManifestReadIssue::FutureVersion {
            found: probe.format_version,
            supported: MANIFEST_FORMAT_VERSION,
        });
    }

    serde_json::from_str(&text).map_err(|e| ManifestReadIssue::Malformed(e.to_string()))
}

/// 读出落地目录下的全部 manifest。
///
/// 目录不存在不算错误（第一次往这里拷）；能读的照读，读不了的记进 `issues`。
pub fn load_manifests(landing_dir: &Path) -> LoadedManifests {
    let dir = manifest_dir(landing_dir);
    let mut out = LoadedManifests::default();

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return out;
    };

    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")))
        .collect();
    paths.sort();

    for p in paths {
        match read_manifest(&p) {
            Ok(m) => out.manifests.push((p, m)),
            Err(issue) => out.issues.push((p, issue)),
        }
    }
    out
}

/// RFC3339 时间串（用于报告与日志）。
pub fn format_time(at: OffsetDateTime) -> String {
    at.format(&Rfc3339).unwrap_or_else(|_| String::from("-"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{hash_bytes, HashAlgorithm};
    use crate::manifest::model::{ManifestEntry, SourceRef, VerifyState};
    use time::macros::datetime;

    fn sample(source_id: &str, at: OffsetDateTime) -> Manifest {
        let mut m = Manifest::new(
            SourceRef {
                id: source_id.into(),
                display_name: "A7M4主卡".into(),
            },
            "婚礼",
            r"D:\素材\婚礼",
            HashAlgorithm::Xxh64,
            at,
        );
        let h = hash_bytes(HashAlgorithm::Xxh64, b"aaa");
        m.entries.push(ManifestEntry {
            relative_path: "A001.MP4".into(),
            size: 3,
            source_hash: h,
            verify: VerifyState::Verified {
                destination_hash: h,
            },
            source_modified_at: None,
            completed_at: at,
            retries: 0,
        });
        m
    }

    // spec: verify-manifest → manifest 落盘位置与作用域 → Scenario: manifest 随数据移动
    #[test]
    fn scenario_verify_manifest_lives_inside_landing_dir() {
        let dir = tempfile::tempdir().expect("临时目录");
        let landing = dir.path().join("婚礼/2026-08-08");
        std::fs::create_dir_all(&landing).expect("建落地目录");

        let m = sample("vol-1", datetime!(2026-08-08 09:30:00 UTC));
        let p = write_manifest(&landing, &m).expect("写清单");

        assert!(p.starts_with(&landing), "清单必须落在落地目录内：{p:?}");
        assert!(p.parent().is_some_and(|d| d.ends_with(MANIFEST_DIR)));

        // 整个落地目录搬走，清单跟着走且仍可读
        let moved = dir.path().join("搬到别处");
        std::fs::rename(&landing, &moved).expect("搬移");
        let loaded = load_manifests(&moved);
        assert_eq!(loaded.manifests.len(), 1);
        assert_eq!(loaded.manifests[0].1, m);
    }

    // spec: → Scenario: 多次拷贝不互相覆盖
    #[test]
    fn scenario_verify_manifest_multiple_runs_do_not_overwrite() {
        let dir = tempfile::tempdir().expect("临时目录");
        let landing = dir.path().to_path_buf();

        let m1 = sample("vol-1", datetime!(2026-08-08 09:30:00 UTC));
        let m2 = sample("vol-1", datetime!(2026-08-08 14:00:00 UTC));
        let p1 = write_manifest(&landing, &m1).expect("写第一份");
        let p2 = write_manifest(&landing, &m2).expect("写第二份");
        assert_ne!(p1, p2);

        let loaded = load_manifests(&landing);
        assert_eq!(loaded.manifests.len(), 2, "两份清单都应保留");
        assert!(!loaded.has_issues());
    }

    #[test]
    fn scenario_verify_manifest_same_second_runs_do_not_overwrite() {
        let dir = tempfile::tempdir().expect("临时目录");
        let at = datetime!(2026-08-08 09:30:00 UTC);
        let m = sample("vol-1", at);
        let p1 = write_manifest(dir.path(), &m).expect("第一份");
        let p2 = write_manifest(dir.path(), &m).expect("同一秒的第二份");
        assert_ne!(p1, p2, "同一秒内两次任务也不能互相覆盖");
        assert_eq!(load_manifests(dir.path()).manifests.len(), 2);
    }

    #[test]
    fn scenario_verify_manifest_stem_carries_time_and_source() {
        let m = sample("vol-guid-abcdef123", datetime!(2026-08-08 14:05:09 UTC));
        let stem = manifest_stem(&m);
        assert!(stem.starts_with("20260808-140509"), "文件名应含时间戳：{stem}");
        assert!(stem.contains("vol"), "文件名应含源设备片段：{stem}");
    }

    // spec: → manifest 异常处理 → Scenario: 损坏的 manifest 降级为全量
    #[test]
    fn scenario_verify_manifest_malformed_is_reported_not_silent() {
        let dir = tempfile::tempdir().expect("临时目录");
        let mdir = manifest_dir(dir.path());
        std::fs::create_dir_all(&mdir).expect("建目录");
        std::fs::write(mdir.join("broken.json"), b"{\"format_version\":1, \"entr")
            .expect("写半截文件");

        let loaded = load_manifests(dir.path());
        assert!(loaded.manifests.is_empty());
        assert_eq!(loaded.issues.len(), 1, "损坏的清单 MUST 被显式报告");
        assert!(matches!(
            loaded.issues[0].1,
            ManifestReadIssue::Malformed(_)
        ));
        assert!(loaded.has_issues());
    }

    // spec: → Scenario: 未来版本的 manifest
    #[test]
    fn scenario_verify_manifest_future_version_is_reported() {
        let dir = tempfile::tempdir().expect("临时目录");
        let mdir = manifest_dir(dir.path());
        std::fs::create_dir_all(&mdir).expect("建目录");
        let mut v: serde_json::Value =
            serde_json::to_value(sample("vol-1", datetime!(2026-08-08 09:30:00 UTC)))
                .expect("转 json");
        v["format_version"] = serde_json::json!(MANIFEST_FORMAT_VERSION + 5);
        std::fs::write(mdir.join("future.json"), v.to_string()).expect("写");

        let loaded = load_manifests(dir.path());
        assert!(loaded.manifests.is_empty(), "未来版本 MUST NOT 被强行解析");
        assert!(matches!(
            loaded.issues[0].1,
            ManifestReadIssue::FutureVersion { .. }
        ));
        // 错误信息要能指导用户
        assert!(loaded.issues[0].1.to_string().contains("升级"));
    }

    #[test]
    fn scenario_verify_manifest_missing_dir_is_not_an_error() {
        let dir = tempfile::tempdir().expect("临时目录");
        let loaded = load_manifests(&dir.path().join("从未拷过"));
        assert!(loaded.manifests.is_empty());
        assert!(!loaded.has_issues(), "第一次往这里拷不算异常");
    }

    #[test]
    fn scenario_verify_manifest_dir_is_excluded_from_audit_scan() {
        // 凭证自身 MUST NOT 在复验里被报成「新增」
        let dir = tempfile::tempdir().expect("临时目录");
        let m = sample("vol-1", datetime!(2026-08-08 09:30:00 UTC));
        let p = write_manifest(dir.path(), &m).expect("写");
        assert!(is_manifest_path(dir.path(), &p));
        assert!(!is_manifest_path(dir.path(), &dir.path().join("A001.MP4")));
    }

    #[test]
    fn scenario_verify_manifest_non_json_files_ignored() {
        let dir = tempfile::tempdir().expect("临时目录");
        let mdir = manifest_dir(dir.path());
        std::fs::create_dir_all(&mdir).expect("建目录");
        std::fs::write(mdir.join("readme.txt"), b"hi").expect("写");
        let loaded = load_manifests(dir.path());
        assert!(loaded.manifests.is_empty());
        assert!(!loaded.has_issues(), "非 json 文件不应被当成损坏清单");
    }
}
