//! 素材类别与扩展名过滤。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/organize-rules/spec.md`
//! → Requirement: 文件类型过滤
//!
//! 铁律：匹配基于**扩展名**而非文件名子串（`mp4_note.txt` 不是视频），且不区分大小写
//! （`A001.MOV` 与 `A002.mov` 都算）。

use std::collections::BTreeSet;
use std::path::Path;

/// 素材大类。每类可独立启用、配置扩展名与落地目的地。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MediaKind {
    Video,
    Photo,
    Audio,
}

impl MediaKind {
    pub const ALL: [MediaKind; 3] = [MediaKind::Video, MediaKind::Photo, MediaKind::Audio];

    pub const fn label(self) -> &'static str {
        match self {
            MediaKind::Video => "视频",
            MediaKind::Photo => "照片",
            MediaKind::Audio => "音频",
        }
    }
}

/// 一类文件的规则：是否启用 + 扩展名集合（小写、不含点）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryRule {
    pub enabled: bool,
    extensions: BTreeSet<String>,
}

impl CategoryRule {
    pub fn new(enabled: bool, exts: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        Self {
            enabled,
            extensions: exts.into_iter().map(|e| normalize_ext(e.as_ref())).collect(),
        }
    }

    pub fn extensions(&self) -> impl Iterator<Item = &str> {
        self.extensions.iter().map(String::as_str)
    }

    pub fn insert(&mut self, ext: &str) -> bool {
        self.extensions.insert(normalize_ext(ext))
    }

    pub fn remove(&mut self, ext: &str) -> bool {
        self.extensions.remove(&normalize_ext(ext))
    }

    /// 是否命中——**只看扩展名**，且不区分大小写。
    pub fn matches_ext(&self, ext: &str) -> bool {
        self.extensions.contains(&normalize_ext(ext))
    }
}

/// 把用户输入的扩展名归一：去掉前导点、去空白、转小写。
pub fn normalize_ext(raw: &str) -> String {
    raw.trim().trim_start_matches('.').to_ascii_lowercase()
}

/// 取路径的扩展名（小写、不含点）。无扩展名返回 `None`。
///
/// 注意：`Path::extension` 对 `.gitignore` 这类点开头无扩展名的文件返回 `None`，正是期望行为。
pub fn file_ext(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

/// 一个文件在过滤规则下的归类结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// 命中某个已启用的素材类别
    Media(MediaKind),
    /// 命中 sidecar 扩展名——是否拷贝取决于能否配到主素材（见 `sidecar` 模块）
    Sidecar,
    /// 不纳入
    Excluded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterConfig {
    pub video: CategoryRule,
    pub photo: CategoryRule,
    pub audio: CategoryRule,
    /// 与主素材配对的附属文件（相机产生的 XML、THM 等）
    pub sidecar: CategoryRule,
    /// 序列帧扩展名——用于把整目录识别为一个单元（见 `sequence` 模块）
    pub sequence: CategoryRule,
}

impl FilterConfig {
    pub fn rule(&self, kind: MediaKind) -> &CategoryRule {
        match kind {
            MediaKind::Video => &self.video,
            MediaKind::Photo => &self.photo,
            MediaKind::Audio => &self.audio,
        }
    }

    pub fn rule_mut(&mut self, kind: MediaKind) -> &mut CategoryRule {
        match kind {
            MediaKind::Video => &mut self.video,
            MediaKind::Photo => &mut self.photo,
            MediaKind::Audio => &mut self.audio,
        }
    }

    /// 归类一个文件。
    ///
    /// 顺序：先看已启用的素材类别，再看 sidecar。**未启用的类别一律不纳入**，
    /// 即便其扩展名同时出现在 sidecar 集合里也不例外。
    pub fn classify(&self, path: &Path) -> Classification {
        let Some(ext) = file_ext(path) else {
            return Classification::Excluded;
        };
        for kind in MediaKind::ALL {
            let rule = self.rule(kind);
            if rule.enabled && rule.matches_ext(&ext) {
                return Classification::Media(kind);
            }
        }
        if self.sidecar.enabled && self.sidecar.matches_ext(&ext) {
            return Classification::Sidecar;
        }
        Classification::Excluded
    }

    /// 该扩展名是否属于序列帧。
    pub fn is_sequence_ext(&self, ext: &str) -> bool {
        self.sequence.enabled && self.sequence.matches_ext(ext)
    }
}

impl Default for FilterConfig {
    /// 内置默认值。覆盖常见相机 / 手机 / 运动相机 / 录音设备的产出。
    ///
    /// 说明：`lrv` 与 `thm` 归 sidecar 而非视频——它们是低码率代理与缩略图，
    /// 应跟主文件走而不是被当成独立素材统计。
    fn default() -> Self {
        Self {
            video: CategoryRule::new(
                true,
                [
                    "mov", "mp4", "m4v", "avi", "mkv", "mxf", "mts", "m2ts", "mpg", "mpeg", "wmv",
                    "3gp", "webm", "braw", "r3d", "insv",
                ],
            ),
            photo: CategoryRule::new(
                true,
                [
                    "jpg", "jpeg", "png", "heic", "heif", "webp", "tif", "tiff", "bmp", "gif",
                    "dng", "arw", "cr2", "cr3", "crw", "nef", "nrw", "raf", "orf", "rw2", "pef",
                    "srw", "gpr", "insp",
                ],
            ),
            audio: CategoryRule::new(
                true,
                [
                    "wav", "mp3", "flac", "aac", "m4a", "aif", "aiff", "ogg", "opus", "wma",
                ],
            ),
            sidecar: CategoryRule::new(
                true,
                [
                    "xml", "thm", "lrv", "cpi", "bim", "ppn", "smi", "modd", "moff", "xmp", "gpx",
                    "srt", "wpl", "sec",
                ],
            ),
            sequence: CategoryRule::new(true, ["dpx", "ari", "exr", "cin", "dng"]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    // spec: organize-rules → 文件类型过滤 → Scenario: 大小写不敏感匹配
    #[test]
    fn scenario_organize_rules_ext_match_is_case_insensitive() {
        let f = FilterConfig::default();
        assert_eq!(
            f.classify(&p("A001.MOV")),
            Classification::Media(MediaKind::Video)
        );
        assert_eq!(
            f.classify(&p("A002.mov")),
            Classification::Media(MediaKind::Video)
        );
        assert_eq!(
            f.classify(&p("A003.MoV")),
            Classification::Media(MediaKind::Video)
        );
    }

    #[test]
    fn scenario_organize_rules_user_input_ext_is_normalized() {
        let mut r = CategoryRule::new(true, [".MP4", "  mov  ", "AVI"]);
        assert!(r.matches_ext("mp4"));
        assert!(r.matches_ext(".MP4"));
        assert!(r.matches_ext("MOV"));
        assert!(r.insert(".BRAW"));
        assert!(r.matches_ext("braw"));
        assert!(!r.insert("braw"), "重复插入应返回 false");
        assert!(r.remove("BRAW"));
        assert!(!r.matches_ext("braw"));
    }

    // spec: organize-rules → 文件类型过滤 → Scenario: 扩展名非子串匹配
    #[test]
    fn scenario_organize_rules_ext_is_not_substring_match() {
        let f = FilterConfig::default();
        // 文件名里含 "mp4" 但扩展名是 txt —— MUST NOT 当作视频
        assert_eq!(f.classify(&p("mp4_note.txt")), Classification::Excluded);
        assert_eq!(f.classify(&p("my.mov.txt")), Classification::Excluded);
        // 反过来：扩展名对了就算，不管主干名叫什么
        assert_eq!(
            f.classify(&p("txt.mov")),
            Classification::Media(MediaKind::Video)
        );
    }

    #[test]
    fn scenario_organize_rules_no_extension_is_excluded() {
        let f = FilterConfig::default();
        assert_eq!(f.classify(&p("README")), Classification::Excluded);
        assert_eq!(f.classify(&p(".gitignore")), Classification::Excluded);
        assert_eq!(f.classify(&p("noext.")), Classification::Excluded);
    }

    // spec: organize-rules → 文件类型过滤 → Scenario: 未启用类别不拷贝
    #[test]
    fn scenario_organize_rules_disabled_category_not_included() {
        let mut f = FilterConfig::default();
        f.photo.enabled = false;
        assert_eq!(f.classify(&p("DSC_0001.JPG")), Classification::Excluded);
        // 其他类别不受影响
        assert_eq!(
            f.classify(&p("A001.MOV")),
            Classification::Media(MediaKind::Video)
        );
    }

    #[test]
    fn scenario_organize_rules_disabled_sidecar_not_included() {
        let mut f = FilterConfig::default();
        assert_eq!(f.classify(&p("C0210M01.XML")), Classification::Sidecar);
        f.sidecar.enabled = false;
        assert_eq!(f.classify(&p("C0210M01.XML")), Classification::Excluded);
    }

    // spec: organize-rules → 文件类型过滤 → Scenario: 分类别分目的地
    // （分目的地的编排在 plan 层，这里断言归类结果足以驱动分流）
    #[test]
    fn scenario_organize_rules_classification_drives_routing() {
        let f = FilterConfig::default();
        let cases = [
            ("A001.MP4", Classification::Media(MediaKind::Video)),
            ("IMG_0001.HEIC", Classification::Media(MediaKind::Photo)),
            ("ZOOM0001.WAV", Classification::Media(MediaKind::Audio)),
            ("C0210M01.XML", Classification::Sidecar),
            ("desktop.ini", Classification::Excluded),
        ];
        for (name, want) in cases {
            assert_eq!(f.classify(&p(name)), want, "归类 {name} 出错");
        }
    }

    #[test]
    fn scenario_organize_rules_proxy_and_thumb_are_sidecar_not_media() {
        // LRV（低码率代理）与 THM（缩略图）跟主文件走，不该被当成独立素材统计
        let f = FilterConfig::default();
        assert_eq!(f.classify(&p("DJI_0001.LRV")), Classification::Sidecar);
        assert_eq!(f.classify(&p("DJI_0001.THM")), Classification::Sidecar);
    }

    #[test]
    fn scenario_organize_rules_sequence_ext_recognized() {
        let f = FilterConfig::default();
        assert!(f.is_sequence_ext("dpx"));
        assert!(f.is_sequence_ext("DPX"));
        assert!(!f.is_sequence_ext("mp4"));
    }
}
