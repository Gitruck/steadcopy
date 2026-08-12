//! 节点刷新：把文件系统里手动建的目录同步回导图。**单向、只读、只增不删。**
//!
//! 规范：`openspec/changes/add-steadcopy-copy-map/specs/copy-map/spec.md`
//! → Requirement: 节点刷新单向且零删除
//!
//! # 三条底线（设计 D5）
//!
//! - **fs → 图，单向**：不做「删了节点就删目录」——本产品在拷贝路径上绝不删除
//!   用户文件，这条铁律高于功能完整性。
//! - **绝不写 fs**：刷新是 diff + 确认 + 合并，全程只读目录项，一个字节都不写。
//! - **先预览再合并**：diff 产出的候选清单给用户确认之后才进树，防止把回收站
//!   这类杂物目录一键收编。
//!
//! # 占位符节点不做反向匹配
//!
//! 节点叫 `{日期}` 时，磁盘上的 `2026-08-12` 不会被认成它——反向匹配要么猜
//! （猜错把无关目录吞进占位符），要么带一套模糊规则（用户解释不了）。渲染出来的
//! 目录会老实出现在候选清单里，收不收由用户在确认那一步定夺，这正是预览存在的意义。

use std::path::Path;

use crate::map::model::{validate_node_name, FolderMap, MapError, MAX_DEPTH};

/// 一条将新增的目录（相对项目根，父在前）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshAddition {
    pub segments: Vec<String>,
}

impl RefreshAddition {
    /// 预览清单上显示的相对路径。
    pub fn display_path(&self) -> String {
        self.segments.join("/")
    }
}

/// 一条**无法并入**的目录：名字过不了节点校验（保留名 / 非法字符 / 坏占位符等）。
///
/// 带 [`MapError`] 本体而不是渲染好的句子——在这层渲染成字等于把语言定死，
/// 由调用方在呈现时 `describe(lang)`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshSkipped {
    pub segments: Vec<String>,
    pub reason: MapError,
}

impl RefreshSkipped {
    /// 确认面板「无法并入」区显示的相对路径。
    pub fn display_path(&self) -> String {
        self.segments.join("/")
    }
}

/// 刷新预览：确认之后才合并，diff 本身零副作用。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefreshPlan {
    /// 父目录一定排在子目录前面（合并按序即可）
    pub additions: Vec<RefreshAddition>,
    /// 名字不合法、进不了树的目录，逐条带原因。它们不进候选、apply 不碰它们——
    /// 磁盘上一个 `{GUID}` 目录（Windows 上各种运行库常拉的）曾把整批刷新永久堵死：
    /// 进候选 → apply 校验炸 → 原子合并整批不动 → 且没有逐条剔除口。
    /// 单列出来是「拒了要说清为什么」，不是吞掉
    pub skipped: Vec<RefreshSkipped>,
}

impl RefreshPlan {
    pub fn is_empty(&self) -> bool {
        self.additions.is_empty() && self.skipped.is_empty()
    }

    /// 按用户确认的相对路径清单裁剪候选：只留确认集里的条目。
    ///
    /// 预览与落地之间磁盘可能又变了（导图派发自己就会在目的地建目录）——
    /// 落地前重算的 diff 里若冒出**确认集之外**的新条目，一律不并，留给下一次刷新：
    /// 用户确认的是 N 条，落进去的就只能是那 N 条的子集，多一条都是没确认过的。
    /// 确认过、如今已不在 diff 里的条目（目录被删了）自然消失，不报错。
    /// `skipped` 原样保留——它只用于呈现，本来就不参与合并。
    #[must_use]
    pub fn confirmed_only(&self, confirmed: &[String]) -> RefreshPlan {
        RefreshPlan {
            additions: self
                .additions
                .iter()
                .filter(|a| {
                    let p = a.display_path();
                    confirmed.iter().any(|c| c == &p)
                })
                .cloned()
                .collect(),
            skipped: self.skipped.clone(),
        }
    }
}

/// 刷新时默认排除的目录：隐藏（`.` 开头）与系统杂物。
///
/// 这些目录从来不是用户的素材结构，列进候选只会污染确认清单——
/// 用户真想收编它们，手动在画布上建同名节点就是了。
pub fn is_excluded_dir(name: &str) -> bool {
    name.starts_with('.')
        || name.eq_ignore_ascii_case("$RECYCLE.BIN")
        || name.eq_ignore_ascii_case("System Volume Information")
}

/// 找出 fs 里存在而树里没有的子目录。**只读**，一个字节都不写。
pub fn diff_refresh(map: &FolderMap, project_root: &Path) -> Result<RefreshPlan, MapError> {
    let entries = std::fs::read_dir(project_root).map_err(|e| MapError::Unreadable {
        path: project_root.to_path_buf(),
        reason: e.to_string(),
    })?;
    drop(entries); // 只为确认根目录可读；真正的遍历在下面统一走

    let mut plan = RefreshPlan::default();
    walk(map, Some(NodeAnchor::Top), project_root, &mut Vec::new(), &mut plan);
    Ok(plan)
}

/// 遍历锚点：当前 fs 目录在树里对应什么。
enum NodeAnchor<'a> {
    /// 项目根（树的顶层）
    Top,
    /// 一个既有节点
    Node(&'a str),
}

fn walk(
    map: &FolderMap,
    anchor: Option<NodeAnchor<'_>>,
    dir: &Path,
    prefix: &mut Vec<String>,
    plan: &mut RefreshPlan,
) {
    // 深度上限：树里挂不下的层级，列进候选也合并不进去，不如不列
    if prefix.len() >= MAX_DEPTH {
        return;
    }
    // 子目录读不了（权限、独占）就跳过它的下层——刷新是只读辅助，不是审计，
    // 一个拒绝访问的目录不该让整个刷新失败。目录本身已在上一层处理过
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    // 排序保证候选清单顺序稳定：read_dir 的顺序随文件系统实现变，预览不能跟着抖
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !is_excluded_dir(n))
        .collect();
    names.sort();

    for name in names {
        let matched = match &anchor {
            Some(NodeAnchor::Top) => map.find_child(None, &name),
            Some(NodeAnchor::Node(id)) => map.find_child(Some(id), &name),
            // 父目录本身就是新增：它下面不可能有既有节点
            None => None,
        };
        prefix.push(name.clone());
        match matched {
            Some(node) => {
                let id = node.id.clone();
                walk(map, Some(NodeAnchor::Node(&id)), &dir.join(&name), prefix, plan);
            }
            // 名字过不了节点校验的目录在 diff 阶段就单列，不进候选。
            // 它的下层也不再展开：父进不了树，子挂无可挂——列出来只会
            // 产生一串注定合并失败的候选。校验规则与 add_node 是同一个函数，
            // 不会出现「diff 放行、apply 又拒」的两套口径
            None => match validate_node_name(&name) {
                Ok(()) => {
                    plan.additions.push(RefreshAddition {
                        segments: prefix.clone(),
                    });
                    walk(map, None, &dir.join(&name), prefix, plan);
                }
                Err(reason) => {
                    plan.skipped.push(RefreshSkipped {
                        segments: prefix.clone(),
                        reason,
                    });
                }
            },
        }
        prefix.pop();
    }
}

/// 用户确认后，把候选合并进树。返回新建节点的 id。
///
/// 只消费 `additions`——`skipped` 是呈现用的说明，永远不参与合并；
/// 调用方落地前应先用 [`RefreshPlan::confirmed_only`] 裁到用户确认过的交集。
///
/// **原子**：先在副本上全部走通，再一次换入——任何一条不合法
/// （比如绕过 diff 手拼的 plan 里混进 `CON`）就整批不动，不留半合并状态。
pub fn apply_refresh(map: &mut FolderMap, plan: &RefreshPlan) -> Result<Vec<String>, MapError> {
    let mut work = map.clone();
    let mut created = Vec::new();

    for add in &plan.additions {
        let mut parent: Option<String> = None;
        for seg in &add.segments {
            let existing = work.find_child(parent.as_deref(), seg).map(|n| n.id.clone());
            parent = Some(match existing {
                Some(id) => id,
                None => {
                    let id = work.add_node(parent.as_deref(), seg)?;
                    created.push(id.clone());
                    id
                }
            });
        }
    }

    *map = work;
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn mkdirs(root: &Path, rels: &[&str]) {
        for r in rels {
            std::fs::create_dir_all(root.join(r)).expect("建目录");
        }
    }

    /// 一棵目录树的完整快照：路径 + 类型 + 修改时间 + 文件内容。
    /// 光看「目录还在不在」不够——刷新要证明的是**什么都没动**。
    ///
    /// 目录**不比 mtime**：NTFS 对目录 last-write 的更新是惰性的，测前搭环境的
    /// 写入会在两次快照之间才落定，比它只会把「测前写入」误报成「刷新写了盘」。
    /// 目录的真实变化（增删条目）由路径清单兜住；文件照旧比 mtime 与内容。
    fn snapshot(root: &Path) -> Vec<(PathBuf, bool, Option<std::time::SystemTime>, Vec<u8>)> {
        let mut out = Vec::new();
        for e in walkdir::WalkDir::new(root).sort_by_file_name().into_iter().flatten() {
            let is_dir = e.file_type().is_dir();
            let (mtime, content) = if is_dir {
                (None, Vec::new())
            } else {
                (
                    e.metadata().ok().and_then(|m| m.modified().ok()),
                    std::fs::read(e.path()).expect("读文件"),
                )
            };
            out.push((e.path().to_path_buf(), is_dir, mtime, content));
        }
        out
    }

    // spec: copy-map → 节点刷新单向且零删除 → Scenario: 外部新建目录经确认后并入
    #[test]
    fn scenario_copy_map_external_dirs_merge_after_confirm() {
        let dir = tempfile::tempdir().expect("临时目录");
        let root = dir.path();
        mkdirs(root, &["素材/视频", "素材/照片", "素材/照片/精选", "备份"]);

        // 树里已有「素材/视频」；「照片（含精选）」与「备份」是外部新建的
        let mut map = FolderMap::default();
        let su = map.add_node(None, "素材").expect("素材");
        map.add_node(Some(&su), "视频").expect("视频");

        let plan = diff_refresh(&map, root).expect("diff");
        let paths: Vec<String> = plan.additions.iter().map(|a| a.display_path()).collect();
        assert_eq!(paths, vec!["备份", "素材/照片", "素材/照片/精选"]);

        // 确认后合并：新目录成为对应父节点的子节点
        let created = apply_refresh(&mut map, &plan).expect("合并");
        assert_eq!(created.len(), 3);
        map.validate().expect("合并后结构完好");
        let zhaopian = map
            .find_child(Some(&su), "照片")
            .expect("照片挂在素材下")
            .id
            .clone();
        assert!(map.find_child(Some(&zhaopian), "精选").is_some(), "精选挂在照片下");
        assert!(map.find_child(None, "备份").is_some(), "备份挂在顶层");

        // 已并入的不再出现在下一次 diff 里
        assert!(diff_refresh(&map, root).expect("再 diff").is_empty());
    }

    // spec: copy-map → 节点刷新单向且零删除 → Scenario: 刷新不写文件系统
    #[test]
    fn scenario_copy_map_refresh_never_writes_filesystem() {
        let dir = tempfile::tempdir().expect("临时目录");
        let root = dir.path();
        mkdirs(root, &["素材/视频", "外来目录"]);
        std::fs::write(root.join("素材/清单.txt"), "内容不许变").expect("写文件");

        let mut map = FolderMap::default();
        map.add_node(None, "素材").expect("素材");

        let before = snapshot(root);
        let plan = diff_refresh(&map, root).expect("diff");
        assert!(!plan.is_empty(), "确认 diff 真的扫到了东西，测试才有效");
        // 合并也只动内存里的树，不动磁盘
        apply_refresh(&mut map, &plan).expect("合并");
        let after = snapshot(root);
        assert_eq!(before, after, "刷新 MUST NOT 新增、修改或删除任何文件系统条目");
    }

    // spec: copy-map → 节点刷新单向且零删除 → Scenario: 图里删节点不删目录
    #[test]
    fn scenario_copy_map_deleting_node_keeps_directory_on_disk() {
        let dir = tempfile::tempdir().expect("临时目录");
        let root = dir.path();
        mkdirs(root, &["素材/视频"]);
        std::fs::write(root.join("素材/视频/A001.MP4"), "素材本体").expect("写文件");

        // 树与磁盘一一对应
        let mut map = FolderMap::default();
        let su = map.add_node(None, "素材").expect("素材");
        map.add_node(Some(&su), "视频").expect("视频");

        let before = snapshot(root);
        map.remove_node(&su).expect("删掉整棵");
        assert!(map.nodes.is_empty(), "导图这边真的删干净了");
        assert_eq!(snapshot(root), before, "磁盘目录与文件 MUST 原样保留");
    }

    // 隐藏与系统目录不进候选
    #[test]
    fn scenario_copy_map_refresh_excludes_hidden_and_system_dirs() {
        let dir = tempfile::tempdir().expect("临时目录");
        let root = dir.path();
        mkdirs(
            root,
            &[".thumbnails", "$RECYCLE.BIN/S-1-5-21", "System Volume Information", "正经目录"],
        );

        let plan = diff_refresh(&FolderMap::default(), root).expect("diff");
        let paths: Vec<String> = plan.additions.iter().map(|a| a.display_path()).collect();
        assert_eq!(paths, vec!["正经目录"], "杂物目录 MUST NOT 出现在候选清单里");

        // 判定本身
        assert!(is_excluded_dir(".hidden"));
        assert!(is_excluded_dir("$RECYCLE.BIN"));
        assert!(is_excluded_dir("$recycle.bin"));
        assert!(is_excluded_dir("System Volume Information"));
        assert!(!is_excluded_dir("素材"));
        assert!(!is_excluded_dir("DCIM"));
    }

    // 匹配不区分大小写：磁盘上叫 dcim、树里叫 DCIM，是同一个
    #[test]
    fn scenario_copy_map_refresh_matches_case_insensitively() {
        let dir = tempfile::tempdir().expect("临时目录");
        let root = dir.path();
        mkdirs(root, &["dcim/子层"]);

        let mut map = FolderMap::default();
        map.add_node(None, "DCIM").expect("DCIM");

        let plan = diff_refresh(&map, root).expect("diff");
        let paths: Vec<String> = plan.additions.iter().map(|a| a.display_path()).collect();
        assert_eq!(paths, vec!["dcim/子层"], "dcim 本身不该被当成新目录");
    }

    // 坏名不再进 plan（diff 阶段就单列，见 scenario_copy_map_refresh_reports_unmappable_dirs）；
    // 这里钉的是 apply 的最后防线：**绕过 diff 的调用方**（手拼 / 过期的 plan）
    // 把保留名混进确认集时，合并对确认集仍然原子——整批不动，不留半合并状态
    #[test]
    fn scenario_copy_map_refresh_merge_is_atomic_on_bad_names() {
        let mut map = FolderMap::default();
        map.add_node(None, "素材").expect("素材");
        let before = map.clone();

        // 手工拼一个含保留名的候选清单（真实磁盘上确实建得出 \\?\ 前缀的 CON 目录）
        let plan = RefreshPlan {
            additions: vec![
                RefreshAddition { segments: vec!["好目录".into()] },
                RefreshAddition { segments: vec!["CON".into()] },
            ],
            skipped: Vec::new(),
        };
        let e = apply_refresh(&mut map, &plan).expect_err("保留名必须被拒");
        assert!(matches!(e, MapError::ReservedName { .. }));
        assert_eq!(map, before, "任何一条不合法就整批不动");
    }

    // 名字进不了树的目录（{GUID} 这类）：单列并带双语原因，不堵死别人（复核修复 F3）
    #[test]
    fn scenario_copy_map_refresh_reports_unmappable_dirs() {
        let dir = tempfile::tempdir().expect("临时目录");
        let root = dir.path();
        // {GUID} 目录是 Windows 上各种运行库常在盘根拉的；花括号会被占位符解析拒绝。
        // 它下面再放一层，验证「进不了树的目录不再展开子层」
        mkdirs(
            root,
            &["正经目录", "{3EA8D3CF-0000-0000-0000-000000000000}/子层"],
        );

        let plan = diff_refresh(&FolderMap::default(), root).expect("diff");
        let paths: Vec<String> = plan.additions.iter().map(|a| a.display_path()).collect();
        assert_eq!(paths, vec!["正经目录"], "坏名不进候选");
        assert_eq!(plan.skipped.len(), 1, "坏名单列一条；它的子层不再展开");
        assert_eq!(
            plan.skipped[0].display_path(),
            "{3EA8D3CF-0000-0000-0000-000000000000}"
        );
        assert!(
            matches!(plan.skipped[0].reason, MapError::BadPlaceholder { .. }),
            "原因要带 MapError 本体：{:?}",
            plan.skipped[0].reason
        );
        // 原因双语都能成句，且不是同一句
        let zh = plan.skipped[0].reason.describe(crate::i18n::Locale::Zh);
        let en = plan.skipped[0].reason.describe(crate::i18n::Locale::En);
        assert!(!zh.trim().is_empty() && !en.trim().is_empty());
        assert_ne!(zh, en);

        // 合并只吃合法项，整批照走——一条坏名不再堵死整批刷新
        let mut map = FolderMap::default();
        apply_refresh(&mut map, &plan).expect("合并");
        assert!(map.find_child(None, "正经目录").is_some());
        assert_eq!(map.nodes.len(), 1, "skipped 的目录不进树");
    }

    // 落地只并用户确认过的交集：预览与确认之间磁盘冒出的新目录不并（复核修复 F2）
    #[test]
    fn scenario_copy_map_refresh_apply_merges_only_confirmed() {
        let dir = tempfile::tempdir().expect("临时目录");
        let root = dir.path();
        mkdirs(root, &["备份"]);

        let mut map = FolderMap::default();
        let plan = diff_refresh(&map, root).expect("预览 diff");
        let confirmed: Vec<String> = plan.additions.iter().map(|a| a.display_path()).collect();
        assert_eq!(confirmed, vec!["备份"], "用户看到并确认的就这一条");

        // 确认到落地之间磁盘又变了：导图派发自己就会在目的地建目录
        mkdirs(root, &["派发新建的目录"]);
        let recomputed = diff_refresh(&map, root).expect("落地前重算 diff");
        assert_eq!(recomputed.additions.len(), 2, "重算确实看见了新目录，测试才有效");

        let filtered = recomputed.confirmed_only(&confirmed);
        apply_refresh(&mut map, &filtered).expect("合并");
        assert!(map.find_child(None, "备份").is_some(), "确认过的要并入");
        assert!(
            map.find_child(None, "派发新建的目录").is_none(),
            "确认集之外的新条目 MUST NOT 并入——留给下一次刷新"
        );

        // 确认过但如今已消失的条目：交集为空，不报错、不并入
        let ghost = recomputed.confirmed_only(&["早已不存在".to_string()]);
        assert!(ghost.additions.is_empty());
    }
}
