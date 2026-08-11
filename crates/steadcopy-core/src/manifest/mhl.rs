//! MHL v1 兼容输出。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/verify-manifest/spec.md`
//! → Requirement: MHL v1 兼容输出
//!
//! MHL（Media Hash List）是拷卡行业的事实清单格式，Silverstack、ShotPut Pro、
//! OffShoot、DaVinci Resolve 都认它。产一份出来，用户的凭证就能被这些工具复验，
//! 而不是锁死在稳拷自己的 JSON 里。
//!
//! **JSON manifest 是本应用正本，MHL XML 是互认输出**，两者内容必须一致。
//!
//! 已知边界：MHL v1 的作用域只到单个文件夹，不记历史；ASC MHL（v2）的世代链
//! 是 V2 候选，本期不做（见 `project.md` §6 非目标）。

use std::fmt::Write as _;

use time::format_description::well_known::Rfc3339;

use crate::engine::HashAlgorithm;
use crate::manifest::model::Manifest;

/// MHL v1 里的算法标签。
fn algo_tag(a: HashAlgorithm) -> &'static str {
    match a {
        // MHL 规范里 xxHash64 的大端形态标签
        HashAlgorithm::Xxh64 => "xxhash64be",
        HashAlgorithm::Md5 => "md5",
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn rfc3339(t: time::OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_else(|_| String::from("-"))
}

/// 由一份 manifest 生成 MHL v1 兼容的 XML。
pub fn render_mhl(m: &Manifest) -> String {
    let mut x = String::with_capacity(4096 + m.entries.len() * 256);
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<hashlist version=\"1.1\">\n");

    // creatorinfo：谁、什么时候、用什么工具做的
    x.push_str("  <creatorinfo>\n");
    let _ = writeln!(x, "    <name>{}</name>", esc(&m.source.display_name));
    let _ = writeln!(x, "    <startdate>{}</startdate>", rfc3339(m.created_at));
    let _ = writeln!(x, "    <finishdate>{}</finishdate>", rfc3339(m.created_at));
    let _ = writeln!(
        x,
        "    <tool>{} {}</tool>",
        esc(&m.generator.name),
        esc(&m.generator.version)
    );
    // 项目名不是 MHL 的标准字段，放进 location 便于人读
    let _ = writeln!(x, "    <location>{}</location>", esc(&m.project));
    x.push_str("  </creatorinfo>\n");

    let tag = algo_tag(m.algorithm);
    for e in &m.entries {
        x.push_str("  <hash>\n");
        let _ = writeln!(x, "    <file>{}</file>", esc(&e.relative_path));
        let _ = writeln!(x, "    <size>{}</size>", e.size);
        if let Some(t) = e.source_modified_at {
            let _ = writeln!(x, "    <lastmodificationdate>{}</lastmodificationdate>", rfc3339(t));
        }
        let _ = writeln!(x, "    <{tag}>{}</{tag}>", e.source_hash.to_hex());
        let _ = writeln!(x, "    <hashdate>{}</hashdate>", rfc3339(e.completed_at));
        x.push_str("  </hash>\n");
    }

    x.push_str("</hashlist>\n");
    x
}

/// 把 MHL 写到 JSON 清单旁边（同名 `.mhl`）。
pub fn write_mhl(json_path: &std::path::Path, m: &Manifest) -> std::io::Result<std::path::PathBuf> {
    let target = json_path.with_extension("mhl");
    std::fs::write(&target, render_mhl(m))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::hash_bytes;
    use crate::manifest::model::{ManifestEntry, SourceRef, VerifyState};
    use time::macros::datetime;

    fn manifest(algo: HashAlgorithm) -> Manifest {
        let at = datetime!(2026-08-10 09:30:00 UTC);
        let mut m = Manifest::new(
            SourceRef {
                id: "vol-1".into(),
                display_name: "A7M4主卡".into(),
            },
            "婚礼<张先生>",
            r"D:\素材",
            algo,
            at,
        );
        for name in ["DCIM/A001.MP4", "DCIM/A001.XML"] {
            let h = hash_bytes(algo, name.as_bytes());
            m.entries.push(ManifestEntry {
                relative_path: name.into(),
                size: 1234,
                source_hash: h,
                verify: VerifyState::Verified {
                    destination_hash: h,
                },
                source_modified_at: Some(at),
                completed_at: at,
                retries: 0,
            });
        }
        m
    }

    // spec: verify-manifest → MHL v1 兼容输出 → Scenario: XML 结构合法
    #[test]
    fn scenario_verify_manifest_mhl_is_well_formed_xml() {
        let xml = render_mhl(&manifest(HashAlgorithm::Xxh64));
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<hashlist"));
        assert!(xml.trim_end().ends_with("</hashlist>"));

        // 用真正的 XML 解析器验证结构合法，而不是靠字符串包含
        let mut reader = quick_xml::Reader::from_str(&xml);
        let mut buf = Vec::new();
        let mut depth = 0i32;
        let mut hashes = 0;
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(quick_xml::events::Event::Start(e)) => {
                    depth += 1;
                    if e.name().as_ref() == b"hash" {
                        hashes += 1;
                    }
                }
                Ok(quick_xml::events::Event::End(_)) => depth -= 1,
                Ok(quick_xml::events::Event::Eof) => break,
                Err(e) => panic!("XML 解析失败：{e}"),
                _ => {}
            }
            buf.clear();
        }
        assert_eq!(depth, 0, "标签应全部闭合");
        assert_eq!(hashes, 2, "两个文件应产生两条 hash 记录");
    }

    // spec: → Scenario: 同时产出两份清单（内容一致）
    #[test]
    fn scenario_verify_manifest_mhl_matches_json_content() {
        let m = manifest(HashAlgorithm::Xxh64);
        let xml = render_mhl(&m);
        for e in &m.entries {
            assert!(xml.contains(&e.relative_path), "缺文件 {}", e.relative_path);
            assert!(
                xml.contains(&e.source_hash.to_hex()),
                "缺哈希 {}",
                e.source_hash.to_hex()
            );
            assert!(xml.contains(&e.size.to_string()));
        }
    }

    #[test]
    fn scenario_verify_manifest_mhl_algorithm_tag() {
        assert!(render_mhl(&manifest(HashAlgorithm::Xxh64)).contains("<xxhash64be>"));
        assert!(render_mhl(&manifest(HashAlgorithm::Md5)).contains("<md5>"));
    }

    #[test]
    fn scenario_verify_manifest_mhl_escapes_user_content() {
        // 项目名带尖括号，MUST NOT 破坏 XML 结构
        let xml = render_mhl(&manifest(HashAlgorithm::Xxh64));
        assert!(xml.contains("婚礼&lt;张先生&gt;"));
        assert!(!xml.contains("婚礼<张先生>"));
        // 转义后仍是合法 XML
        let mut r = quick_xml::Reader::from_str(&xml);
        let mut buf = Vec::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(quick_xml::events::Event::Eof) => break,
                Err(e) => panic!("转义后 XML 应仍合法：{e}"),
                _ => {}
            }
            buf.clear();
        }
    }

    #[test]
    fn scenario_verify_manifest_mhl_written_next_to_json() {
        let dir = tempfile::tempdir().expect("临时目录");
        let json = dir.path().join("m-20260810.json");
        std::fs::write(&json, "{}").expect("写占位");
        let p = write_mhl(&json, &manifest(HashAlgorithm::Xxh64)).expect("写 mhl");
        assert_eq!(p, dir.path().join("m-20260810.mhl"));
        assert!(std::fs::read_to_string(&p).expect("读").contains("<hashlist"));
    }

    #[test]
    fn scenario_verify_manifest_mhl_empty_manifest() {
        let mut m = manifest(HashAlgorithm::Xxh64);
        m.entries.clear();
        let xml = render_mhl(&m);
        assert!(xml.contains("<hashlist") && xml.contains("</hashlist>"));
        assert!(!xml.contains("<hash>"));
    }
}
