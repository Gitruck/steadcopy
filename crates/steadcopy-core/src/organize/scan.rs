//! 源扫描：整卡镜像为默认，类型过滤为 opt-in。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/organize-rules/spec.md`
//! → Requirement: 整卡镜像为默认 / 文件类型过滤（opt-in）
//! 事实依据：`docs/source-devices.md` §F2、§四
//!
//! # 为什么默认整卡镜像
//!
//! 配套文件缺一个就可能让整条素材在剪辑软件里废掉：
//! Canon 的 `CANONXF/CLIPS001/INDEX.MIF` 是其应用挂载卡的硬性前提；
//! Insta360 官方明说同一条 clip 的双 `.insv` 与 `.lrv` 缺一个就认不出；
//! Panasonic 已有「只导 DCIM、忘了 PRIVATE/AVCHD、然后把卡格了」的真实事故。
//!
//! 所以过滤是 opt-in，且优先作为**视图层**能力，而不是默认改变拷贝范围。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use time::OffsetDateTime;
use walkdir::WalkDir;

use crate::manifest::model::normalize_relative;
use crate::organize::filter::{Classification, FilterConfig, MediaKind};

/// 操作系统与文件管理器留下的垃圾。这些**可以**安全排除。
///
/// 注意：**只排这些**。任何看不懂的文件都按素材对待——「看不懂」不是排除的理由。
const JUNK_NAMES: [&str; 9] = [
    "system volume information",
    "$recycle.bin",
    "recycler",
    "thumbs.db",
    ".trashes",
    ".spotlight-v100",
    ".fseventsd",
    ".temporaryitems",
    ".ds_store",
];

/// AppleDouble 伴随文件前缀（`._IMG_0001.JPG`）。
const APPLEDOUBLE_PREFIX: &str = "._";

/// 卡内设备指纹目录——用于**推测**这是什么设备，辅助自动分类。
///
/// **MUST NOT** 作为准入条件：空卡与刚格式化的卡上没有这些目录，但它们仍是合法的源。
const FINGERPRINT_DIRS: [(&str, &str); 10] = [
    ("private/m4root", "Sony 摄影卡"),
    ("private/xdroot", "Sony 专业机卡"),
    ("private/avchd", "AVCHD 摄影卡"),
    ("canonxf", "Canon 专业机卡"),
    ("contents", "Canon 摄录机卡"),
    ("dji_audio", "DJI 无线麦发射器"),
    ("avf_info", "AVCHD 摄影卡"),
    ("mp_root", "Sony 摄影卡"),
    ("movie", "行车记录仪"),
    ("dcim", "影像设备卡"),
];

/// 一个待拷贝的源文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// 相对源根的路径，`/` 分隔
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub size: u64,
    pub modified: Option<OffsetDateTime>,
}

/// 扫描选项。
#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    /// `None` = 整卡镜像（默认）；`Some` = 用户显式开启了类型过滤
    pub filter: Option<FilterConfig>,
}

impl ScanOptions {
    /// 整卡镜像（默认行为）。
    pub fn mirror() -> Self {
        Self { filter: None }
    }

    /// 显式开启类型过滤。调用方 MUST 已向用户出示过遗漏配套文件的警示。
    pub fn filtered(filter: FilterConfig) -> Self {
        Self {
            filter: Some(filter),
        }
    }

    pub fn is_mirror(&self) -> bool {
        self.filter.is_none()
    }
}

/// 扫描结果。
#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub files: Vec<SourceFile>,
    /// 被当作系统垃圾排除的条目数
    pub junk_excluded: usize,
    /// 被类型过滤排除的条目数（整卡镜像时恒为 0）
    pub filtered_out: usize,
    /// 推测出的设备指纹描述（如「Sony 摄影卡」）
    pub fingerprints: Vec<String>,
}

impl ScanResult {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// 按素材类别统计（用于确认卡片上的分类展示）。
    ///
    /// 分类只用于**展示**，不影响整卡镜像下的拷贝范围。
    pub fn by_category(&self, filter: &FilterConfig) -> BTreeMap<&'static str, (usize, u64)> {
        let mut out: BTreeMap<&'static str, (usize, u64)> = BTreeMap::new();
        for f in &self.files {
            let label = match filter.classify(Path::new(&f.relative_path)) {
                Classification::Media(MediaKind::Video) => "视频",
                Classification::Media(MediaKind::Photo) => "照片",
                Classification::Media(MediaKind::Audio) => "音频",
                Classification::Sidecar => "配套文件",
                Classification::Excluded => "其他",
            };
            let e = out.entry(label).or_insert((0, 0));
            e.0 += 1;
            e.1 += f.size;
        }
        out
    }
}

/// 判断某个路径分量是否属于可安全排除的系统垃圾。
pub fn is_junk_component(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    JUNK_NAMES.contains(&lower.as_str()) || lower.starts_with(APPLEDOUBLE_PREFIX)
}

/// 判断一条相对路径是否落在垃圾目录里或本身是垃圾文件。
pub fn is_junk_path(relative: &str) -> bool {
    relative.split('/').any(is_junk_component)
}

/// 从卷根的目录名推测设备类型。
pub fn detect_fingerprints(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let dirs: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().to_ascii_lowercase())
        .collect();

    let mut out: Vec<String> = Vec::new();
    for (marker, label) in FINGERPRINT_DIRS {
        let hit = if let Some((parent, child)) = marker.split_once('/') {
            dirs.iter().any(|d| d == parent) && root.join(parent).join(child).is_dir()
        } else {
            dirs.iter().any(|d| d == marker)
        };
        if hit && !out.contains(&label.to_string()) {
            out.push(label.to_string());
        }
    }
    out
}

/// 扫描源根，产出待拷贝文件集合。
///
/// 目录本身不进结果——目录会在写入目的地时按需创建。
pub fn scan_source(root: &Path, options: &ScanOptions) -> ScanResult {
    let mut result = ScanResult {
        fingerprints: detect_fingerprints(root),
        ..Default::default()
    };

    for entry in WalkDir::new(root).follow_links(false).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();
        let Ok(rel_raw) = abs.strip_prefix(root) else {
            continue;
        };
        let relative = normalize_relative(&rel_raw.to_string_lossy());
        if relative.is_empty() {
            continue;
        }

        if is_junk_path(&relative) {
            result.junk_excluded += 1;
            continue;
        }

        if let Some(filter) = &options.filter {
            if matches!(
                filter.classify(Path::new(&relative)),
                Classification::Excluded
            ) {
                result.filtered_out += 1;
                continue;
            }
        }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        result.files.push(SourceFile {
            relative_path: relative,
            absolute_path: abs.to_path_buf(),
            size: meta.len(),
            modified: meta.modified().ok().map(OffsetDateTime::from),
        });
    }

    // 稳定顺序：让进度、报告、manifest 的条目顺序可预期
    result.files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, rel: &str, bytes: usize) {
        let p = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR.as_ref()));
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("建目录");
        }
        std::fs::write(&p, vec![b'x'; bytes]).expect("写文件");
    }

    fn sony_card(root: &Path) {
        touch(root, "DCIM/100MSDCF/DSC00001.JPG", 100);
        touch(root, "DCIM/100MSDCF/DSC00002.ARW", 200);
        touch(root, "PRIVATE/M4ROOT/CLIP/C0001.MP4", 1000);
        touch(root, "PRIVATE/M4ROOT/CLIP/C0001M01.XML", 10);
        touch(root, "PRIVATE/M4ROOT/SUB/C0001S01.MP4", 50);
        touch(root, "PRIVATE/M4ROOT/GENERAL/无扩展名文件", 5);
        touch(root, "AVF_INFO/AVIN0001.BNP", 8);
        touch(root, "MISC/AUTPRINT.MRK", 3);
    }

    // spec: organize-rules → 整卡镜像为默认 → Scenario: 默认拷走全部内容
    #[test]
    fn scenario_organize_rules_mirror_copies_everything() {
        let dir = tempfile::tempdir().expect("临时目录");
        sony_card(dir.path());

        let r = scan_source(dir.path(), &ScanOptions::mirror());
        assert_eq!(r.file_count(), 8, "整卡镜像应拷走全部 8 个文件");
        assert_eq!(r.filtered_out, 0);

        let paths: Vec<&str> = r.files.iter().map(|f| f.relative_path.as_str()).collect();
        // 配套文件与「看不懂」的文件都必须在
        assert!(paths.contains(&"PRIVATE/M4ROOT/CLIP/C0001M01.XML"));
        assert!(paths.contains(&"AVF_INFO/AVIN0001.BNP"));
        assert!(paths.contains(&"MISC/AUTPRINT.MRK"));
        assert!(paths.contains(&"PRIVATE/M4ROOT/GENERAL/无扩展名文件"));
    }

    // spec: → Scenario: 系统垃圾被排除
    #[test]
    fn scenario_organize_rules_system_junk_excluded() {
        let dir = tempfile::tempdir().expect("临时目录");
        touch(dir.path(), "DCIM/A001.MP4", 100);
        touch(dir.path(), "System Volume Information/IndexerVolumeGuid", 10);
        touch(dir.path(), "$RECYCLE.BIN/S-1-5-21/x.dat", 10);
        touch(dir.path(), ".Trashes/501/y.dat", 10);
        touch(dir.path(), ".Spotlight-V100/z.dat", 10);
        touch(dir.path(), ".fseventsd/0000", 10);
        touch(dir.path(), "DCIM/Thumbs.db", 10);
        touch(dir.path(), "DCIM/.DS_Store", 10);
        touch(dir.path(), "DCIM/._A001.MP4", 10);

        let r = scan_source(dir.path(), &ScanOptions::mirror());
        assert_eq!(r.file_count(), 1, "只应剩下真素材：{:?}", r.files);
        assert_eq!(r.files[0].relative_path, "DCIM/A001.MP4");
        assert_eq!(r.junk_excluded, 8);
        assert_eq!(r.total_bytes(), 100, "垃圾不计入总量");
    }

    #[test]
    fn scenario_organize_rules_unknown_files_are_not_junk() {
        // 「看不懂」不是排除的理由
        let dir = tempfile::tempdir().expect("临时目录");
        touch(dir.path(), "WEIRD.XYZ", 10);
        touch(dir.path(), "没有扩展名", 10);
        touch(dir.path(), "厂商私有目录/data.bin", 10);
        let r = scan_source(dir.path(), &ScanOptions::mirror());
        assert_eq!(r.file_count(), 3);
        assert_eq!(r.junk_excluded, 0);
    }

    // spec: → 文件类型过滤（opt-in）
    #[test]
    fn scenario_organize_rules_filter_is_opt_in() {
        let dir = tempfile::tempdir().expect("临时目录");
        sony_card(dir.path());

        let mirrored = scan_source(dir.path(), &ScanOptions::mirror());
        let mut f = FilterConfig::default();
        f.photo.enabled = false;
        let filtered = scan_source(dir.path(), &ScanOptions::filtered(f));

        assert!(
            filtered.file_count() < mirrored.file_count(),
            "开启过滤后应少于整卡镜像"
        );
        assert!(filtered.filtered_out > 0);
        // 关掉照片后，JPG/ARW 不在了
        let paths: Vec<&str> = filtered.files.iter().map(|f| f.relative_path.as_str()).collect();
        assert!(!paths.contains(&"DCIM/100MSDCF/DSC00001.JPG"));
        // 视频还在
        assert!(paths.contains(&"PRIVATE/M4ROOT/CLIP/C0001.MP4"));
    }

    #[test]
    fn scenario_organize_rules_scan_is_stably_ordered() {
        let dir = tempfile::tempdir().expect("临时目录");
        sony_card(dir.path());
        let a = scan_source(dir.path(), &ScanOptions::mirror());
        let b = scan_source(dir.path(), &ScanOptions::mirror());
        assert_eq!(a.files, b.files, "两次扫描顺序 MUST 一致");
        let paths: Vec<&str> = a.files.iter().map(|f| f.relative_path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort_unstable();
        assert_eq!(paths, sorted);
    }

    #[test]
    fn scenario_organize_rules_fingerprint_detection() {
        let dir = tempfile::tempdir().expect("临时目录");
        sony_card(dir.path());
        let r = scan_source(dir.path(), &ScanOptions::mirror());
        assert!(
            r.fingerprints.iter().any(|f| f.contains("Sony")),
            "应识别出 Sony 卡：{:?}",
            r.fingerprints
        );
    }

    #[test]
    fn scenario_organize_rules_fingerprint_dji_mic() {
        let dir = tempfile::tempdir().expect("临时目录");
        touch(dir.path(), "DJI_AUDIO/DJI_01_20260808_100000.WAV", 100);
        let r = scan_source(dir.path(), &ScanOptions::mirror());
        assert!(
            r.fingerprints.iter().any(|f| f.contains("DJI")),
            "应识别出 DJI 无线麦：{:?}",
            r.fingerprints
        );
    }

    #[test]
    fn scenario_organize_rules_blank_card_has_no_fingerprint_but_is_scannable() {
        // 空卡没有指纹目录，但仍是合法的源——结果是「无素材」而不是设备不出现
        let dir = tempfile::tempdir().expect("临时目录");
        let r = scan_source(dir.path(), &ScanOptions::mirror());
        assert!(r.fingerprints.is_empty());
        assert_eq!(r.file_count(), 0);
    }

    #[test]
    fn scenario_organize_rules_category_stats_for_display() {
        let dir = tempfile::tempdir().expect("临时目录");
        sony_card(dir.path());
        let r = scan_source(dir.path(), &ScanOptions::mirror());
        let stats = r.by_category(&FilterConfig::default());
        assert_eq!(stats.get("视频").map(|s| s.0), Some(2)); // C0001.MP4 + 代理
        assert_eq!(stats.get("照片").map(|s| s.0), Some(2)); // JPG + ARW
        assert!(stats.contains_key("配套文件")); // XML
        // 统计总数与文件数一致——展示分类不改变拷贝范围
        let total: usize = stats.values().map(|s| s.0).sum();
        assert_eq!(total, r.file_count());
    }

    #[test]
    fn scenario_organize_rules_junk_component_matching() {
        assert!(is_junk_component("System Volume Information"));
        assert!(is_junk_component("system volume information"));
        assert!(is_junk_component("Thumbs.db"));
        assert!(is_junk_component("._IMG_0001.JPG"));
        assert!(!is_junk_component("DCIM"));
        assert!(!is_junk_component(".insv"));
        assert!(is_junk_path("DCIM/.DS_Store"));
        assert!(is_junk_path("System Volume Information/x"));
        assert!(!is_junk_path("DCIM/100MSDCF/A.MP4"));
    }
}
