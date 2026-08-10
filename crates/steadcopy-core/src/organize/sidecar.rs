//! sidecar 配对：把附属文件挂到它的主素材上。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/organize-rules/spec.md`
//! → Requirement: sidecar 配对
//!
//! 现实里 sidecar 的主干名**不一定**与主素材相同，所以不能只做 stem 相等匹配：
//!
//! | 设备 | 主素材 | sidecar | 关系 |
//! |---|---|---|---|
//! | Sony | `C0210.MP4` | `C0210M01.XML` | 主干 + `M01` 式后缀 |
//! | 通用 | `DJI_0001.MP4` | `DJI_0001.SRT` | 主干相同 |
//! | GoPro | `GX010001.MP4` | `GL010001.LRV` | 主干首段 `GX`→`GL` 同位替换 |
//!
//! 孤儿 sidecar（配不到主素材）默认**不拷贝**。

use std::collections::HashMap;
use std::path::Path;

/// sidecar 主干名与主素材主干名之间的关系规则。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StemRule {
    /// 主干完全相同（不区分大小写）
    Exact,
    /// sidecar 主干 = 主素材主干 + 「一个字母 + 若干数字」后缀，如 Sony 的 `M01`
    LetterDigitSuffix,
    /// 主干前缀同位替换，如 GoPro 的 `GL` ↔ `GX`
    PrefixSwap { sidecar: String, media: String },
}

/// 配对器。规则按顺序尝试，先命中先用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarMatcher {
    pub rules: Vec<StemRule>,
}

impl Default for SidecarMatcher {
    fn default() -> Self {
        Self {
            rules: vec![
                StemRule::Exact,
                StemRule::LetterDigitSuffix,
                StemRule::PrefixSwap {
                    sidecar: "GL".into(),
                    media: "GX".into(),
                },
                StemRule::PrefixSwap {
                    sidecar: "GL".into(),
                    media: "GH".into(),
                },
            ],
        }
    }
}

fn stem_of(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
}

/// 「一个字母 + 至少一位数字」且到此结束。
fn is_letter_digit_suffix(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    let rest: Vec<char> = chars.collect();
    !rest.is_empty() && rest.iter().all(char::is_ascii_digit)
}

impl SidecarMatcher {
    /// 判断 `sidecar_stem` 是否可以挂到 `media_stem` 上。
    fn stem_matches(&self, sidecar_stem: &str, media_stem: &str) -> bool {
        self.rules.iter().any(|rule| match rule {
            StemRule::Exact => sidecar_stem == media_stem,
            StemRule::LetterDigitSuffix => sidecar_stem
                .strip_prefix(media_stem)
                .is_some_and(is_letter_digit_suffix),
            StemRule::PrefixSwap { sidecar, media } => {
                let (sc, md) = (sidecar.to_ascii_lowercase(), media.to_ascii_lowercase());
                match (
                    sidecar_stem.strip_prefix(&sc),
                    media_stem.strip_prefix(&md),
                ) {
                    (Some(a), Some(b)) => a == b,
                    _ => false,
                }
            }
        })
    }

    /// 在候选主素材中为一个 sidecar 找归属。
    ///
    /// 多个候选命中时取**主干最长**的那个——避免 `C021.MP4` 抢走本属于 `C0210.MP4` 的 sidecar。
    pub fn owner_of<'a, P: AsRef<Path>>(
        &self,
        sidecar: &Path,
        media_candidates: &'a [P],
    ) -> Option<&'a Path> {
        let sc_stem = stem_of(sidecar)?;
        media_candidates
            .iter()
            .map(AsRef::as_ref)
            .filter(|m| {
                stem_of(m).is_some_and(|ms| !ms.is_empty() && self.stem_matches(&sc_stem, &ms))
            })
            .max_by_key(|m| stem_of(m).map(|s| s.len()).unwrap_or(0))
    }

    /// 批量配对：返回「sidecar 路径 → 其主素材路径」的映射。
    ///
    /// 配不到主素材的 sidecar（孤儿）**不会**出现在结果里，即默认不拷贝。
    pub fn pair<'a, P: AsRef<Path>, Q: AsRef<Path>>(
        &self,
        sidecars: &'a [P],
        media: &'a [Q],
    ) -> HashMap<&'a Path, &'a Path> {
        sidecars
            .iter()
            .map(AsRef::as_ref)
            .filter_map(|sc| self.owner_of(sc, media).map(|m| (sc, m)))
            .collect()
    }

    /// 孤儿 sidecar 清单（配不到主素材的）。
    pub fn orphans<'a, P: AsRef<Path>, Q: AsRef<Path>>(
        &self,
        sidecars: &'a [P],
        media: &'a [Q],
    ) -> Vec<&'a Path> {
        sidecars
            .iter()
            .map(AsRef::as_ref)
            .filter(|sc| self.owner_of(sc, media).is_none())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    // spec: organize-rules → sidecar 配对 → Scenario: sidecar 跟随主素材
    #[test]
    fn scenario_organize_rules_sidecar_follows_media_exact_stem() {
        let m = SidecarMatcher::default();
        let media = paths(&["DJI_0001.MP4"]);
        let owner = m.owner_of(Path::new("DJI_0001.SRT"), &media);
        assert_eq!(owner, Some(Path::new("DJI_0001.MP4")));
    }

    #[test]
    fn scenario_organize_rules_sidecar_sony_m01_suffix() {
        // Sony：C0210.MP4 的 sidecar 是 C0210M01.XML，主干并不相同
        let m = SidecarMatcher::default();
        let media = paths(&["C0210.MP4"]);
        assert_eq!(
            m.owner_of(Path::new("C0210M01.XML"), &media),
            Some(Path::new("C0210.MP4"))
        );
    }

    #[test]
    fn scenario_organize_rules_sidecar_gopro_prefix_swap() {
        // GoPro：GX010001.MP4 的低码率代理是 GL010001.LRV
        let m = SidecarMatcher::default();
        let media = paths(&["GX010001.MP4"]);
        assert_eq!(
            m.owner_of(Path::new("GL010001.LRV"), &media),
            Some(Path::new("GX010001.MP4"))
        );
    }

    #[test]
    fn scenario_organize_rules_sidecar_match_is_case_insensitive() {
        let m = SidecarMatcher::default();
        let media = paths(&["c0210.mp4"]);
        assert_eq!(
            m.owner_of(Path::new("C0210M01.XML"), &media),
            Some(Path::new("c0210.mp4"))
        );
    }

    // spec: organize-rules → sidecar 配对 → Scenario: 孤儿 sidecar 不拷贝
    #[test]
    fn scenario_organize_rules_orphan_sidecar_not_copied() {
        let m = SidecarMatcher::default();
        let media = paths(&["C0210.MP4"]);
        assert_eq!(m.owner_of(Path::new("orphan.XML"), &media), None);

        let sidecars = paths(&["C0210M01.XML", "orphan.XML"]);
        let paired = m.pair(&sidecars, &media);
        assert_eq!(paired.len(), 1, "孤儿 MUST NOT 进入配对结果");
        assert!(paired.contains_key(Path::new("C0210M01.XML")));

        let orphans = m.orphans(&sidecars, &media);
        assert_eq!(orphans, vec![Path::new("orphan.XML")]);
    }

    #[test]
    fn scenario_organize_rules_sidecar_prefers_longest_stem() {
        // C021.MP4 MUST NOT 抢走本属于 C0210.MP4 的 sidecar
        let m = SidecarMatcher::default();
        let media = paths(&["C021.MP4", "C0210.MP4"]);
        assert_eq!(
            m.owner_of(Path::new("C0210M01.XML"), &media),
            Some(Path::new("C0210.MP4"))
        );
    }

    #[test]
    fn scenario_organize_rules_sidecar_suffix_must_be_letter_then_digits() {
        let m = SidecarMatcher {
            rules: vec![StemRule::LetterDigitSuffix],
        };
        let media = paths(&["C0210.MP4"]);
        // 合法：M01 / m1 / X99
        for s in ["C0210M01.XML", "C0210m1.XML", "C0210X99.XML"] {
            assert!(m.owner_of(Path::new(s), &media).is_some(), "{s} 应配上");
        }
        // 不合法：纯数字后缀、纯字母后缀、字母数字交替
        for s in ["C021001.XML", "C0210AB.XML", "C0210M1A.XML"] {
            assert!(m.owner_of(Path::new(s), &media).is_none(), "{s} 不应配上");
        }
    }

    #[test]
    fn scenario_organize_rules_sidecar_empty_candidates() {
        let m = SidecarMatcher::default();
        let media: Vec<PathBuf> = vec![];
        assert_eq!(m.owner_of(Path::new("C0210M01.XML"), &media), None);
    }

    #[test]
    fn scenario_organize_rules_sidecar_pair_batch() {
        let m = SidecarMatcher::default();
        let media = paths(&["C0210.MP4", "C0211.MP4", "GX010001.MP4"]);
        let sidecars = paths(&[
            "C0210M01.XML",
            "C0211M01.XML",
            "GL010001.LRV",
            "GX010001.THM",
            "stray.XML",
        ]);
        let paired = m.pair(&sidecars, &media);
        assert_eq!(paired.len(), 4);
        assert_eq!(
            paired.get(Path::new("GL010001.LRV")),
            Some(&Path::new("GX010001.MP4"))
        );
        assert_eq!(
            paired.get(Path::new("GX010001.THM")),
            Some(&Path::new("GX010001.MP4"))
        );
        assert_eq!(m.orphans(&sidecars, &media), vec![Path::new("stray.XML")]);
    }
}
