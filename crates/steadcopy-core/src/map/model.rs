//! 导图树模型：节点、落位、全部校验。
//!
//! 规范：`openspec/changes/add-steadcopy-copy-map/specs/copy-map/spec.md`
//! → Requirement: 树模型由 core 持有并校验
//!
//! # 为什么校验全在写入时（设计 D1）
//!
//! 前端不持任何可独立演化的树状态：增删改名换父全走这里，校验失败**当场**拒绝并
//! 返回双语原因。否决「前端持树、保存时提交」——那会出现「画布上看着合法、
//! 落盘时才报错」，错误出现的位置离造成它的操作越远越难懂。
//!
//! # 为什么节点名的合法性标准比「渲染时净化」更严
//!
//! 路径模板的渲染管线会把非法字符替换掉（`sanitize_segment`），但导图节点是
//! **要落成真实目录的所见即所得**：名字若靠净化才合法，画布上显示的就和磁盘上
//! 建出来的对不上——画面与将要发生的事实脱节，正是导图要消灭的东西。
//! 所以这里直接拒绝，而不是悄悄改写。

use serde::{Deserialize, Serialize};

use crate::config::model::new_id;
use crate::i18n::Locale;
use crate::organize::{sanitize_value, PathTemplate, TemplateError};
use crate::task::AdhocError;

/// 树的深度上限（根为第 1 层）。
///
/// 12 层已远超任何正经拷卡目录结构；不设限的话，一个失控的循环或恶意配置
/// 能让路径长度穿透 Windows 的 MAX_PATH，错误会在拷贝中途才炸出来。
pub const MAX_DEPTH: usize = 12;

/// 单个节点名的字符数上限。
pub const MAX_NAME_CHARS: usize = 100;

/// 导图里的一个目录节点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapNode {
    pub id: String,
    /// 目录名。允许 `{占位符}`（与字符串模板同一套词表），派发时才渲染
    pub name: String,
    /// 父节点 id。`None` 表示顶层——挂在项目目的地根之下
    pub parent: Option<String>,
    /// 子节点 id。顺序即画布展示与预览清单的顺序，**稳定**，不随操作重排
    #[serde(default)]
    pub children: Vec<String>,
}

/// 一条「设备 → 节点」的落位（画布上的一根连线）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub id: String,
    pub device_id: String,
    /// 设备显示名。连线上要挂它——颜色 MUST NOT 是唯一信息载体
    pub device_name: String,
    pub node_id: String,
}

/// 导图本体：一棵（或一片）目录树加全部落位。
///
/// 顶层节点的顺序 = 它们在 `nodes` 里的出现顺序；非顶层由父节点的 `children` 定序。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderMap {
    #[serde(default)]
    pub nodes: Vec<MapNode>,
    #[serde(default)]
    pub assignments: Vec<Assignment>,
}

/// 导图操作被拒绝的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    /// 名字是空的（或全是空白）
    EmptyName,
    /// 名字太长
    NameTooLong { name: String, max: usize, actual: usize },
    /// 名字含 Windows 非法字符或控制字符
    IllegalCharacter { name: String },
    /// 名字是 Windows 保留设备名（含带扩展名形态）
    ReservedName { name: String },
    /// 名字以空格开头，或以点、空格结尾——Windows 会静默截掉它们
    PaddedName { name: String },
    /// 名字里的占位符写法不合法。带 [`TemplateError`] 本体而不是渲染好的字，
    /// 理由同 `AdhocError::BadTemplate`：在这层渲染成字等于把语言定死
    BadPlaceholder { name: String, reason: TemplateError },
    /// 同一父节点下已有同名节点（不区分大小写——NTFS 不区分）
    DuplicateSibling { name: String },
    /// 超出深度上限
    TooDeep { max: usize },
    /// 节点不存在（前端状态过期，或配置被外部改过）
    NodeMissing { id: String },
    /// 换父会成环：不许把节点挂到它自己（或它的后代）下面
    WouldCycle { name: String },
    /// 落位不存在
    AssignmentMissing { id: String },
    /// 同一设备到同一节点的落位已存在——重复连线只会派出两份一样的任务
    DuplicateAssignment { device_name: String, node_name: String },
    /// 派发时找不到该设备的源卷（卡已拔出或从未提供）
    SourceOffline { device_name: String },
    /// 树不是链，导不成字符串模板。`at` 是分叉处的节点名，`None` 表示顶层就并列
    NotAChain { at: Option<String>, branches: usize },
    /// 导图是空的，没有可导出的结构
    EmptyMap,
    /// 字符串模板串不合法（导入/导出与字符串模板视图互通时的把关）
    BadTemplateString { template: String, reason: TemplateError },
    /// 派发被临时拷贝的构造路拒绝（正在跑的设备、项目丢失等）。带上游错误本体
    Dispatch { reason: AdhocError },
    /// 树结构不一致（配置被外部改坏）。`detail` 是给排查用的技术细节
    Inconsistent { detail: String },
    /// 刷新时项目根目录读不了
    Unreadable { path: std::path::PathBuf, reason: String },
}

impl MapError {
    /// 给用户看的一句话，跟随语言。
    pub fn describe(&self, lang: Locale) -> String {
        match (self, lang) {
            (MapError::EmptyName, Locale::Zh) => "目录名不能是空的".into(),
            (MapError::EmptyName, Locale::En) => "A folder name cannot be empty".into(),
            (MapError::NameTooLong { name, max, actual }, Locale::Zh) => {
                format!("目录名最长 {max} 个字符，「{name}」有 {actual} 个")
            }
            (MapError::NameTooLong { name, max, actual }, Locale::En) => {
                format!("A folder name can be at most {max} characters — \"{name}\" has {actual}")
            }
            (MapError::IllegalCharacter { name }, Locale::Zh) => {
                format!("目录名不能含 < > : \" / \\ | ? * 或控制字符：{name}")
            }
            (MapError::IllegalCharacter { name }, Locale::En) => {
                format!("A folder name cannot contain < > : \" / \\ | ? * or control characters: {name}")
            }
            (MapError::ReservedName { name }, Locale::Zh) => {
                format!("「{name}」是 Windows 保留设备名，不能当目录名")
            }
            (MapError::ReservedName { name }, Locale::En) => {
                format!("\"{name}\" is a reserved Windows device name and cannot be used as a folder name")
            }
            (MapError::PaddedName { name }, Locale::Zh) => format!(
                "目录名不能以空格开头，也不能以点或空格结尾（Windows 会悄悄截掉它们，画布上看到的就和磁盘上的对不上了）：{name}"
            ),
            (MapError::PaddedName { name }, Locale::En) => format!(
                "A folder name cannot start with a space or end with a dot or space — Windows silently trims them, so the canvas would no longer match the disk: {name}"
            ),
            (MapError::BadPlaceholder { name, reason }, Locale::Zh) => {
                format!("节点名「{name}」里的占位符不合法：{}", reason.describe(lang))
            }
            (MapError::BadPlaceholder { name, reason }, Locale::En) => format!(
                "The placeholder in node name \"{name}\" is not valid: {}",
                reason.describe(lang)
            ),
            (MapError::DuplicateSibling { name }, Locale::Zh) => format!(
                "同一层已有叫「{name}」的目录（Windows 不区分大小写），换个名字"
            ),
            (MapError::DuplicateSibling { name }, Locale::En) => format!(
                "A folder named \"{name}\" already exists at this level (Windows is case-insensitive) — pick another name"
            ),
            (MapError::TooDeep { max }, Locale::Zh) => {
                format!("目录最多嵌套 {max} 层，再深路径就要超出 Windows 的长度限制了")
            }
            (MapError::TooDeep { max }, Locale::En) => format!(
                "Folders can be nested at most {max} levels deep — any deeper and paths hit the Windows length limit"
            ),
            (MapError::NodeMissing { id }, Locale::Zh) => format!("找不到这个节点：{id}"),
            (MapError::NodeMissing { id }, Locale::En) => format!("No such node: {id}"),
            (MapError::WouldCycle { name }, Locale::Zh) => {
                format!("不能把「{name}」挂到它自己（或它的子目录）下面")
            }
            (MapError::WouldCycle { name }, Locale::En) => {
                format!("\"{name}\" cannot be moved under itself or one of its own subfolders")
            }
            (MapError::AssignmentMissing { id }, Locale::Zh) => format!("找不到这条落位：{id}"),
            (MapError::AssignmentMissing { id }, Locale::En) => {
                format!("No such assignment: {id}")
            }
            (MapError::DuplicateAssignment { device_name, node_name }, Locale::Zh) => format!(
                "「{device_name}」已经连到「{node_name}」了——重复连线只会拷出两份一样的东西"
            ),
            (MapError::DuplicateAssignment { device_name, node_name }, Locale::En) => format!(
                "\"{device_name}\" is already connected to \"{node_name}\" — a duplicate line would just copy the same thing twice"
            ),
            (MapError::SourceOffline { device_name }, Locale::Zh) => {
                format!("「{device_name}」现在不在机上，插回来再派发")
            }
            (MapError::SourceOffline { device_name }, Locale::En) => {
                format!("\"{device_name}\" is not connected right now — plug it back in before dispatching")
            }
            (MapError::NotAChain { at, branches }, Locale::Zh) => match at {
                Some(name) => format!(
                    "「{name}」下有 {branches} 个分支，一串字符串模板表达不了分叉——要保结构请存成导图模板"
                ),
                None => format!(
                    "顶层有 {branches} 个并列目录，一串字符串模板表达不了分叉——要保结构请存成导图模板"
                ),
            },
            (MapError::NotAChain { at, branches }, Locale::En) => match at {
                Some(name) => format!(
                    "\"{name}\" has {branches} branches — a single string template cannot express a fork. Save it as a map template to keep the structure"
                ),
                None => format!(
                    "There are {branches} folders side by side at the top level — a single string template cannot express a fork. Save it as a map template to keep the structure"
                ),
            },
            (MapError::EmptyMap, Locale::Zh) => "导图还是空的，没有可导出的结构".into(),
            (MapError::EmptyMap, Locale::En) => {
                "The map is empty — there is nothing to export yet".into()
            }
            (MapError::BadTemplateString { template, reason }, Locale::Zh) => {
                format!("模板串「{template}」不合法：{}", reason.describe(lang))
            }
            (MapError::BadTemplateString { template, reason }, Locale::En) => format!(
                "The template string \"{template}\" is not valid: {}",
                reason.describe(lang)
            ),
            (MapError::Dispatch { reason }, Locale::Zh) => {
                format!("这条落位没派出去：{}", reason.describe(lang))
            }
            (MapError::Dispatch { reason }, Locale::En) => {
                format!("This assignment was not dispatched: {}", reason.describe(lang))
            }
            (MapError::Inconsistent { detail }, Locale::Zh) => format!(
                "导图数据不一致（配置可能被外部改过）：{detail}"
            ),
            (MapError::Inconsistent { detail }, Locale::En) => format!(
                "The map data is inconsistent (the configuration may have been edited externally): {detail}"
            ),
            (MapError::Unreadable { path, reason }, Locale::Zh) => {
                format!("读不了项目目录 {}：{reason}", path.display())
            }
            (MapError::Unreadable { path, reason }, Locale::En) => {
                format!("Could not read the project folder {}: {reason}", path.display())
            }
        }
    }
}

impl std::fmt::Display for MapError {
    /// `Display` 恒为中文，理由同 [`crate::error::CoreError`]。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe(Locale::Zh))
    }
}

impl std::error::Error for MapError {}

/// 校验一个节点名。
///
/// 比渲染净化更严的理由见模块注释。检查顺序从「最一眼能看懂」到「最需要解释」，
/// 一次只报一个原因——一句话说一件事。
pub fn validate_node_name(name: &str) -> Result<(), MapError> {
    if name.trim().is_empty() {
        return Err(MapError::EmptyName);
    }
    let chars = name.chars().count();
    if chars > MAX_NAME_CHARS {
        return Err(MapError::NameTooLong {
            name: name.to_string(),
            max: MAX_NAME_CHARS,
            actual: chars,
        });
    }
    // 复用渲染管线的字符级定义（单一事实源），不再抄一份非法字符表
    if sanitize_value(name) != name {
        return Err(MapError::IllegalCharacter {
            name: name.to_string(),
        });
    }
    if name.starts_with(' ') || name.ends_with(' ') || name.ends_with('.') {
        return Err(MapError::PaddedName {
            name: name.to_string(),
        });
    }
    if crate::organize::is_reserved_segment(name) {
        return Err(MapError::ReservedName {
            name: name.to_string(),
        });
    }
    // 占位符按导图口径解析：未知占位符、花括号不配对在这里被拒；
    // 纯字面量的名字也从这条路走一遍，保证「名字能渲染出非空的一段」
    if let Err(reason) = PathTemplate::parse_map_path(name) {
        return Err(MapError::BadPlaceholder {
            name: name.to_string(),
            reason,
        });
    }
    Ok(())
}

/// NTFS 口径的同名判定：不区分大小写。
fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

impl FolderMap {
    pub fn node(&self, id: &str) -> Option<&MapNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    fn node_mut(&mut self, id: &str) -> Option<&mut MapNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// 某一层的子节点（按稳定顺序）。`None` 取顶层。
    pub fn children(&self, parent: Option<&str>) -> Vec<&MapNode> {
        match parent {
            None => self.nodes.iter().filter(|n| n.parent.is_none()).collect(),
            Some(pid) => match self.node(pid) {
                Some(p) => p.children.iter().filter_map(|c| self.node(c)).collect(),
                None => Vec::new(),
            },
        }
    }

    /// 按名字（不区分大小写）在某一层找子节点。
    pub fn find_child(&self, parent: Option<&str>, name: &str) -> Option<&MapNode> {
        self.children(parent)
            .into_iter()
            .find(|n| same_name(&n.name, name))
    }

    /// 节点深度（根为 1）。结构损坏（环）时报 [`MapError::Inconsistent`]。
    pub fn depth_of(&self, id: &str) -> Result<usize, MapError> {
        let mut depth = 1usize;
        let mut cur = self
            .node(id)
            .ok_or_else(|| MapError::NodeMissing { id: id.to_string() })?;
        // 步数上限 = 节点总数：走不完说明有环，别在损坏的数据上转圈
        for _ in 0..self.nodes.len() {
            match &cur.parent {
                None => return Ok(depth),
                Some(pid) => {
                    cur = self.node(pid).ok_or_else(|| MapError::Inconsistent {
                        detail: format!("节点 {id} 的祖先 {pid} 不存在"),
                    })?;
                    depth += 1;
                }
            }
        }
        Err(MapError::Inconsistent {
            detail: format!("节点 {id} 的祖先链成环"),
        })
    }

    /// 节点在树里的路径各段（根在前，含自身）。派发时它就是模板串的原料。
    pub fn path_segments(&self, id: &str) -> Result<Vec<String>, MapError> {
        let mut segs = Vec::new();
        let mut cur = self
            .node(id)
            .ok_or_else(|| MapError::NodeMissing { id: id.to_string() })?;
        for _ in 0..self.nodes.len() {
            segs.push(cur.name.clone());
            match &cur.parent {
                None => {
                    segs.reverse();
                    return Ok(segs);
                }
                Some(pid) => {
                    cur = self.node(pid).ok_or_else(|| MapError::Inconsistent {
                        detail: format!("节点 {id} 的祖先 {pid} 不存在"),
                    })?;
                }
            }
        }
        Err(MapError::Inconsistent {
            detail: format!("节点 {id} 的祖先链成环"),
        })
    }

    /// 子树高度（含自身；叶子为 1）。
    fn subtree_height(&self, id: &str) -> usize {
        let mut height = 0usize;
        let mut frontier = vec![id.to_string()];
        // 层数上限 = 节点总数，防止损坏数据里的环把这里拖成死循环
        for _ in 0..self.nodes.len() {
            if frontier.is_empty() {
                break;
            }
            height += 1;
            frontier = frontier
                .iter()
                .filter_map(|i| self.node(i))
                .flat_map(|n| n.children.iter().cloned())
                .collect();
        }
        height
    }

    /// 同层重名检查。`exclude` 排除自己（改名/换父时用）。
    fn ensure_no_sibling(
        &self,
        parent: Option<&str>,
        name: &str,
        exclude: Option<&str>,
    ) -> Result<(), MapError> {
        let clash = self
            .children(parent)
            .into_iter()
            .any(|n| Some(n.id.as_str()) != exclude && same_name(&n.name, name));
        if clash {
            return Err(MapError::DuplicateSibling {
                name: name.to_string(),
            });
        }
        Ok(())
    }

    /// 加一个节点，返回新节点 id。校验失败树保持原状。
    /// 整棵清空——节点与落位一起走。
    ///
    /// 界面上的「新建导图」走这里。磁盘完全不动（导图从不删用户文件），
    /// 这只是把画布归零；已存的模板不受影响，想找回结构可以套用模板。
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.assignments.clear();
    }

    pub fn add_node(&mut self, parent: Option<&str>, name: &str) -> Result<String, MapError> {
        validate_node_name(name)?;
        if let Some(pid) = parent {
            let depth = self.depth_of(pid)?;
            if depth + 1 > MAX_DEPTH {
                return Err(MapError::TooDeep { max: MAX_DEPTH });
            }
        }
        self.ensure_no_sibling(parent, name, None)?;

        let id = new_id("map");
        self.nodes.push(MapNode {
            id: id.clone(),
            name: name.to_string(),
            parent: parent.map(str::to_string),
            children: Vec::new(),
        });
        if let Some(pid) = parent {
            if let Some(p) = self.node_mut(pid) {
                p.children.push(id.clone());
            }
        }
        Ok(id)
    }

    /// 改名。校验失败树保持原状。
    pub fn rename_node(&mut self, id: &str, name: &str) -> Result<(), MapError> {
        let parent = self
            .node(id)
            .ok_or_else(|| MapError::NodeMissing { id: id.to_string() })?
            .parent
            .clone();
        validate_node_name(name)?;
        self.ensure_no_sibling(parent.as_deref(), name, Some(id))?;
        if let Some(n) = self.node_mut(id) {
            n.name = name.to_string();
        }
        Ok(())
    }

    /// 换父（拖拽调层级）。环检测：不许挂到自己或自己的后代下。
    pub fn move_node(&mut self, id: &str, new_parent: Option<&str>) -> Result<(), MapError> {
        let node = self
            .node(id)
            .ok_or_else(|| MapError::NodeMissing { id: id.to_string() })?;
        let name = node.name.clone();
        let old_parent = node.parent.clone();

        if let Some(np) = new_parent {
            // 先确认目标存在，再沿它的祖先链找自己——链上有自己就是环
            let mut cur = self
                .node(np)
                .ok_or_else(|| MapError::NodeMissing { id: np.to_string() })?;
            for _ in 0..=self.nodes.len() {
                if cur.id == id {
                    return Err(MapError::WouldCycle { name });
                }
                match &cur.parent {
                    None => break,
                    Some(pid) => {
                        cur = self.node(pid).ok_or_else(|| MapError::Inconsistent {
                            detail: format!("节点 {np} 的祖先 {pid} 不存在"),
                        })?;
                    }
                }
            }
        }

        self.ensure_no_sibling(new_parent, &name, Some(id))?;

        // 深度：整棵被移动的子树都不许超限，不是只看被拖的那一个
        let parent_depth = match new_parent {
            Some(np) => self.depth_of(np)?,
            None => 0,
        };
        if parent_depth + self.subtree_height(id) > MAX_DEPTH {
            return Err(MapError::TooDeep { max: MAX_DEPTH });
        }

        // 校验全过，才动结构
        if let Some(op) = &old_parent {
            let op = op.clone();
            if let Some(p) = self.node_mut(&op) {
                p.children.retain(|c| c != id);
            }
        }
        if let Some(np) = new_parent {
            if let Some(p) = self.node_mut(np) {
                p.children.push(id.to_string());
            }
        }
        if let Some(n) = self.node_mut(id) {
            n.parent = new_parent.map(str::to_string);
        }
        Ok(())
    }

    /// 删除节点：连带删掉整棵子树与其上的全部落位。
    ///
    /// **只动导图，绝不动磁盘**——本产品在拷贝路径上绝不删除用户文件，
    /// 这条铁律高于功能完整性（设计 D5）。返回被删的节点 id（含子树）。
    pub fn remove_node(&mut self, id: &str) -> Result<Vec<String>, MapError> {
        let node = self
            .node(id)
            .ok_or_else(|| MapError::NodeMissing { id: id.to_string() })?;
        let parent = node.parent.clone();

        // BFS 收整棵子树
        let mut doomed: Vec<String> = vec![id.to_string()];
        let mut i = 0usize;
        while i < doomed.len() {
            if let Some(n) = self.node(&doomed[i]) {
                doomed.extend(n.children.iter().cloned());
            }
            i += 1;
        }

        if let Some(pid) = &parent {
            let pid = pid.clone();
            if let Some(p) = self.node_mut(&pid) {
                p.children.retain(|c| c != id);
            }
        }
        self.nodes.retain(|n| !doomed.contains(&n.id));
        self.assignments.retain(|a| !doomed.contains(&a.node_id));
        Ok(doomed)
    }

    /// 整棵树的完整校验。给「配置被外部改过」兜底——正常操作路径上每一步
    /// 都已当场校验，这里防的是手改 config.json 绕过它们。
    pub fn validate(&self) -> Result<(), MapError> {
        // id 唯一
        for (i, n) in self.nodes.iter().enumerate() {
            if self.nodes.iter().skip(i + 1).any(|m| m.id == n.id) {
                return Err(MapError::Inconsistent {
                    detail: format!("节点 id 重复：{}", n.id),
                });
            }
        }
        for (i, a) in self.assignments.iter().enumerate() {
            if self.assignments.iter().skip(i + 1).any(|b| b.id == a.id) {
                return Err(MapError::Inconsistent {
                    detail: format!("落位 id 重复：{}", a.id),
                });
            }
            if self.node(&a.node_id).is_none() {
                return Err(MapError::Inconsistent {
                    detail: format!("落位 {} 指向不存在的节点 {}", a.id, a.node_id),
                });
            }
        }

        // 父子指针互相咬合
        for n in &self.nodes {
            if let Some(pid) = &n.parent {
                let p = self.node(pid).ok_or_else(|| MapError::Inconsistent {
                    detail: format!("节点 {} 的父节点 {pid} 不存在", n.id),
                })?;
                if p.children.iter().filter(|c| *c == &n.id).count() != 1 {
                    return Err(MapError::Inconsistent {
                        detail: format!("节点 {pid} 的子表与 {} 的父指针不一致", n.id),
                    });
                }
            }
            for c in &n.children {
                let child = self.node(c).ok_or_else(|| MapError::Inconsistent {
                    detail: format!("节点 {} 的子节点 {c} 不存在", n.id),
                })?;
                if child.parent.as_deref() != Some(n.id.as_str()) {
                    return Err(MapError::Inconsistent {
                        detail: format!("节点 {c} 的父指针没有指回 {}", n.id),
                    });
                }
            }
        }

        // 名字合法 + 深度上限 + 同层无重名（depth_of 顺带把环也查了）
        for n in &self.nodes {
            validate_node_name(&n.name)?;
            let depth = self.depth_of(&n.id)?;
            if depth > MAX_DEPTH {
                return Err(MapError::TooDeep { max: MAX_DEPTH });
            }
            let siblings = self.children(n.parent.as_deref());
            if siblings
                .iter()
                .any(|s| s.id != n.id && same_name(&s.name, &n.name))
            {
                return Err(MapError::DuplicateSibling {
                    name: n.name.clone(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(map: &mut FolderMap, names: &[&str]) -> Vec<String> {
        let mut parent: Option<String> = None;
        let mut ids = Vec::new();
        for n in names {
            let id = map.add_node(parent.as_deref(), n).expect("建链");
            parent = Some(id.clone());
            ids.push(id);
        }
        ids
    }

    // spec: copy-map → 树模型由 core 持有并校验 → Scenario: 非法节点名被当场拒绝
    #[test]
    fn scenario_copy_map_illegal_node_name_rejected_on_the_spot() {
        let mut map = FolderMap::default();
        let id = map.add_node(None, "素材").expect("合法名");
        let before = map.clone();

        // Windows 非法字符逐个试
        for bad in ["a<b", "a>b", "a:b", "a\"b", "a/b", "a\\b", "a|b", "a?b", "a*b", "a\u{7}b"] {
            let e = map.rename_node(&id, bad).expect_err("非法字符必须被拒");
            assert!(matches!(e, MapError::IllegalCharacter { .. }), "{bad} → {e:?}");
            assert_eq!(map, before, "改名被拒后树必须保持原状");
        }

        // 保留名：裸的、变大小写的、带扩展名的
        for bad in ["CON", "con", "PRN", "NUL", "AUX", "COM1", "lpt9", "CON.txt"] {
            let e = map.rename_node(&id, bad).expect_err("保留名必须被拒");
            assert!(matches!(e, MapError::ReservedName { .. }), "{bad} → {e:?}");
            assert_eq!(map, before);
        }

        // 双语原因：两种语言都有话说，且不是同一句
        let e = map.rename_node(&id, "a:b").expect_err("拒绝");
        assert!(!e.describe(Locale::Zh).trim().is_empty());
        assert!(!e.describe(Locale::En).trim().is_empty());
        assert_ne!(e.describe(Locale::Zh), e.describe(Locale::En));

        // 空名、超长、首尾点空格同样当场拒
        assert!(matches!(map.rename_node(&id, "  "), Err(MapError::EmptyName)));
        assert!(matches!(map.rename_node(&id, "素材."), Err(MapError::PaddedName { .. })));
        assert!(matches!(map.rename_node(&id, " 素材"), Err(MapError::PaddedName { .. })));
        assert_eq!(map, before);
    }

    // spec: copy-map → 树模型由 core 持有并校验 → Scenario: 兄弟重名被拒绝
    #[test]
    fn scenario_copy_map_duplicate_sibling_name_rejected() {
        let mut map = FolderMap::default();
        let root = map.add_node(None, "素材").expect("根");
        map.add_node(Some(&root), "DCIM").expect("首个");

        // 新增重名：不区分大小写——NTFS 不区分
        for dup in ["DCIM", "dcim", "Dcim"] {
            let e = map.add_node(Some(&root), dup).expect_err("重名必须被拒");
            assert!(matches!(e, MapError::DuplicateSibling { .. }), "{dup} → {e:?}");
        }
        // 改名撞车同样拒
        let other = map.add_node(Some(&root), "PRIVATE").expect("另一个");
        assert!(matches!(
            map.rename_node(&other, "dcim"),
            Err(MapError::DuplicateSibling { .. })
        ));
        // 顶层与不同父下互不干扰
        map.add_node(None, "DCIM").expect("顶层同名不算兄弟");
        let sub = map.add_node(Some(&other), "DCIM").expect("不同父下同名合法");
        // 改回自己的名字（大小写不变）不算撞自己
        map.rename_node(&sub, "DCIM").expect("改成自己现在的名字应放行");
    }

    // spec: copy-map → 树模型由 core 持有并校验 → Scenario: 不许挂到自己后代下
    #[test]
    fn scenario_copy_map_reparent_under_own_descendant_rejected() {
        let mut map = FolderMap::default();
        let ids = chain(&mut map, &["A", "B", "C"]);
        let before = map.clone();

        // A 挂到孙子 C 下：环
        let e = map.move_node(&ids[0], Some(&ids[2])).expect_err("挂到后代必须被拒");
        assert!(matches!(e, MapError::WouldCycle { .. }));
        assert_eq!(map, before, "被拒后树必须保持原状");
        // 挂到自己下面同样是环
        assert!(matches!(
            map.move_node(&ids[0], Some(&ids[0])),
            Err(MapError::WouldCycle { .. })
        ));
        assert_eq!(map, before);

        // 合法的换父照常走：C 提到顶层
        map.move_node(&ids[2], None).expect("提到顶层");
        assert!(map.node(&ids[2]).expect("C").parent.is_none());
        map.validate().expect("换父后结构完好");
    }

    // 深度上限：第 12 层能建，第 13 层被拒；换父把整棵子树带超限同样拒
    #[test]
    fn scenario_copy_map_depth_limit_enforced() {
        let mut map = FolderMap::default();
        let names: Vec<String> = (1..=MAX_DEPTH).map(|i| format!("层{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let ids = chain(&mut map, &refs);
        let e = map
            .add_node(ids.last().map(String::as_str), "多一层")
            .expect_err("第 13 层必须被拒");
        assert!(matches!(e, MapError::TooDeep { max } if max == MAX_DEPTH));

        // 换父：两层高的子树挂到第 11 层下 → 总深 13，拒
        let sub = map.add_node(None, "子树").expect("顶层");
        map.add_node(Some(&sub), "叶子").expect("两层");
        assert!(matches!(
            map.move_node(&sub, Some(&ids[MAX_DEPTH - 2])),
            Err(MapError::TooDeep { .. })
        ));
        // 挂到第 10 层下 → 总深 12，放行
        map.move_node(&sub, Some(&ids[MAX_DEPTH - 3])).expect("恰到上限应放行");
    }

    // 名长上限：100 字符放行，101 拒
    #[test]
    fn scenario_copy_map_name_length_limit() {
        let mut map = FolderMap::default();
        map.add_node(None, &"字".repeat(MAX_NAME_CHARS)).expect("100 字符应放行");
        let e = map
            .add_node(None, &"字".repeat(MAX_NAME_CHARS + 1))
            .expect_err("101 字符必须被拒");
        assert!(matches!(e, MapError::NameTooLong { actual: 101, .. }));
    }

    // 节点名允许占位符，与字符串模板同一套词表
    #[test]
    fn scenario_copy_map_node_names_accept_placeholders() {
        let mut map = FolderMap::default();
        for ok in ["{项目}", "{日期}", "{设备}", "{年}-{月}", "素材{卡}"] {
            map.add_node(None, ok).expect(ok);
        }
        // 未知占位符、花括号不配对被拒，且错误带的是 TemplateError 本体
        match map.add_node(None, "{不存在}").expect_err("未知占位符必须被拒") {
            MapError::BadPlaceholder { reason, .. } => {
                assert!(matches!(reason, TemplateError::UnknownPlaceholder(_)));
            }
            other => panic!("错误类型不对：{other:?}"),
        }
        assert!(matches!(
            map.add_node(None, "{项目"),
            Err(MapError::BadPlaceholder { .. })
        ));
    }

    // 删除节点连带删子树与相关落位（只动导图，fs 铁律的测试在 refresh.rs）
    #[test]
    fn scenario_copy_map_remove_node_removes_subtree_and_assignments() {
        let mut map = FolderMap::default();
        let ids = chain(&mut map, &["A", "B", "C"]);
        let other = map.add_node(None, "留下").expect("旁支");
        map.add_assignment("vol:1", "A7M4", &ids[1]).expect("落位 B");
        map.add_assignment("vol:1", "A7M4", &ids[2]).expect("落位 C");
        map.add_assignment("vol:2", "H6", &other).expect("落位旁支");

        let removed = map.remove_node(&ids[1]).expect("删 B");
        assert_eq!(removed.len(), 2, "B 与 C 都该在被删清单里");
        assert!(map.node(&ids[1]).is_none() && map.node(&ids[2]).is_none());
        assert_eq!(map.assignments.len(), 1, "B、C 上的落位要连带删掉");
        assert_eq!(map.assignments[0].device_id, "vol:2", "旁支的落位不受牵连");
        assert!(map.node(&ids[0]).expect("A").children.is_empty(), "父的子表要摘掉 B");
        map.validate().expect("删完结构完好");
    }

    // 手改配置绕过操作路径 → validate 兜底
    #[test]
    fn scenario_copy_map_validate_catches_hand_edited_corruption() {
        let mut map = FolderMap::default();
        let ids = chain(&mut map, &["A", "B"]);
        map.validate().expect("好树");

        // 父指针指向不存在的节点
        let mut broken = map.clone();
        if let Some(n) = broken.node_mut(&ids[1]) {
            n.parent = Some("map-不存在".into());
        }
        assert!(matches!(broken.validate(), Err(MapError::Inconsistent { .. })));

        // 落位指向不存在的节点
        let mut broken = map.clone();
        broken.assignments.push(Assignment {
            id: "lnk-1".into(),
            device_id: "vol:1".into(),
            device_name: "A7M4".into(),
            node_id: "map-不存在".into(),
        });
        assert!(matches!(broken.validate(), Err(MapError::Inconsistent { .. })));

        // 两个节点互为父子（环）
        let mut broken = map.clone();
        if let Some(n) = broken.node_mut(&ids[0]) {
            n.parent = Some(ids[1].clone());
        }
        assert!(broken.validate().is_err(), "环必须被识破");
    }
}
