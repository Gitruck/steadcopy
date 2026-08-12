//! 导图模板：树的保存与套用，及与字符串模板的双向转换。
//!
//! 规范：`openspec/changes/add-steadcopy-copy-map/specs/copy-map/spec.md`
//! → Requirement: 导图模板与字符串模板是同一份结构的两个视图
//!
//! # 为什么不搞两套存储（设计 D4）
//!
//! `MapTemplate` 序列化的就是节点树；字符串模板可**导入**成链状树，
//! 链状树可**导出**回模板串。两套存储必然漂，漂了之后谁是权威没有答案——
//! 所以转换是纯函数，数据只有树这一份。
//!
//! 非链状树导出被拒（字符串表达不了分叉），拒绝时说清楚分叉在哪、
//! 以及「要保结构请存成导图模板」——错误信息要给出路，不是只给判决。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::config::model::new_id;
use crate::map::model::{FolderMap, MapError, MapNode};
use crate::organize::PathTemplate;

/// 存起来复用的导图模板。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapTemplate {
    pub id: String,
    pub name: String,
    /// 结构本体。模板只带树，不带设备落位——落位是工位现场的事，
    /// 换个项目、换一天，卡都不是同一批
    pub map: FolderMap,
}

impl MapTemplate {
    /// 由现有导图存成模板。落位被剥掉，理由见 `map` 字段注释。
    pub fn from_map(name: impl Into<String>, map: &FolderMap) -> Self {
        let mut m = map.clone();
        m.assignments.clear();
        Self {
            id: new_id("mtp"),
            name: name.into(),
            map: m,
        }
    }

    /// 套用：产出一棵**全新 id** 的树。
    ///
    /// id 重新生成是为了让「模板」与「项目里的实例」永远不可能被误认成同一棵——
    /// 同 id 不同物是最难查的一类混淆。结构损坏（手改过的模板）在这里被识破。
    pub fn instantiate(&self) -> Result<FolderMap, MapError> {
        let mut remap: HashMap<&str, String> = HashMap::new();
        for n in &self.map.nodes {
            remap.insert(n.id.as_str(), new_id("map"));
        }
        let lookup = |id: &str| -> Result<String, MapError> {
            remap.get(id).cloned().ok_or_else(|| MapError::Inconsistent {
                detail: format!("模板「{}」引用了不存在的节点 {id}", self.name),
            })
        };

        let mut nodes = Vec::with_capacity(self.map.nodes.len());
        for n in &self.map.nodes {
            let parent = match &n.parent {
                Some(p) => Some(lookup(p)?),
                None => None,
            };
            let children = n
                .children
                .iter()
                .map(|c| lookup(c))
                .collect::<Result<Vec<_>, _>>()?;
            nodes.push(MapNode {
                id: lookup(&n.id)?,
                name: n.name.clone(),
                parent,
                children,
            });
        }
        let out = FolderMap {
            nodes,
            // 模板不该带落位；就算配置被手改塞了进来，套用时也不带出去
            assignments: Vec::new(),
        };
        out.validate()?;
        Ok(out)
    }
}

/// 字符串模板 → 链状树。
///
/// 先按字符串模板的**全套**规矩过一遍（含必需占位符）——从那个视图进来
/// 就守那个视图的门；然后逐段建链，每段再过一遍节点名校验。
pub fn import_template_string(raw: &str) -> Result<FolderMap, MapError> {
    PathTemplate::parse(raw).map_err(|e| MapError::BadTemplateString {
        template: raw.to_string(),
        reason: e,
    })?;

    let mut map = FolderMap::default();
    let mut parent: Option<String> = None;
    for seg in raw.split(['/', '\\']).filter(|s| !s.is_empty()) {
        let id = map.add_node(parent.as_deref(), seg)?;
        parent = Some(id);
    }
    Ok(map)
}

/// 链状树 → 字符串模板。仅当树退化为链；分叉导不出去，拒绝并指出分叉在哪。
pub fn export_template_string(map: &FolderMap) -> Result<String, MapError> {
    if map.nodes.is_empty() {
        return Err(MapError::EmptyMap);
    }
    let roots = map.children(None);
    if roots.len() != 1 {
        return Err(MapError::NotAChain {
            at: None,
            branches: roots.len(),
        });
    }

    let mut segs = Vec::new();
    let mut cur = roots[0];
    loop {
        segs.push(cur.name.clone());
        match cur.children.len() {
            0 => break,
            1 => {
                cur = map.node(&cur.children[0]).ok_or_else(|| MapError::Inconsistent {
                    detail: format!("节点 {} 的子节点不存在", cur.id),
                })?;
            }
            n => {
                return Err(MapError::NotAChain {
                    at: Some(cur.name.clone()),
                    branches: n,
                })
            }
        }
    }
    let out = segs.join("/");

    // 导出的串要能在字符串模板视图立足（含必需占位符），否则导出去也用不了——
    // 与其让它在目的地设置里再失败一次，不如现在就说清
    PathTemplate::parse(&out).map_err(|e| MapError::BadTemplateString {
        template: out.clone(),
        reason: e,
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organize::TemplateError;

    // spec: copy-map → 导图模板与字符串模板是同一份结构的两个视图
    //       → Scenario: 字符串模板导入为链状树
    #[test]
    fn scenario_copy_map_string_template_imports_as_chain() {
        let map = import_template_string("{项目}/{日期}/{设备}").expect("导入");
        assert_eq!(map.nodes.len(), 3, "三层链");

        // 占位符原样保留为节点名，层级关系是链
        let roots = map.children(None);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].name, "{项目}");
        let l2 = map.children(Some(&roots[0].id));
        assert_eq!(l2.len(), 1);
        assert_eq!(l2[0].name, "{日期}");
        let l3 = map.children(Some(&l2[0].id));
        assert_eq!(l3.len(), 1);
        assert_eq!(l3[0].name, "{设备}");
        assert!(map.children(Some(&l3[0].id)).is_empty());
        map.validate().expect("导入的树结构完好");

        // 导入守字符串模板视图的门：非法串被拒，错误带上游本体
        match import_template_string("素材/{年}").expect_err("缺必需占位符必须被拒") {
            MapError::BadTemplateString { reason, .. } => {
                assert_eq!(reason, TemplateError::MissingRequiredPlaceholder);
            }
            other => panic!("错误类型不对：{other:?}"),
        }
        // Windows 风格分隔符同样成链
        let map = import_template_string(r"{项目}\{设备}").expect("反斜杠");
        assert_eq!(map.nodes.len(), 2);
    }

    // spec: copy-map → 导图模板与字符串模板是同一份结构的两个视图
    //       → Scenario: 非链状树导出被拒绝
    #[test]
    fn scenario_copy_map_branching_tree_export_rejected() {
        // 链能导出，且与导入往返一致
        let chain = import_template_string("{项目}/{日期}/{设备}").expect("链");
        assert_eq!(
            export_template_string(&chain).expect("链应能导出"),
            "{项目}/{日期}/{设备}"
        );

        // 中途分叉：拒绝并点名分叉节点、说明字符串表达不了分叉
        let mut fork = import_template_string("{项目}/{日期}").expect("底链");
        let mid = fork.children(None)[0].id.clone();
        fork.add_node(Some(&mid), "{设备}").expect("岔枝");
        let e = export_template_string(&fork).expect_err("分叉必须被拒");
        match &e {
            MapError::NotAChain { at: Some(name), branches: 2 } => {
                assert_eq!(name, "{项目}");
            }
            other => panic!("错误类型不对：{other:?}"),
        }
        let msg = e.describe(crate::i18n::Locale::Zh);
        assert!(msg.contains("表达不了分叉"), "{msg}");

        // 顶层并列同样是分叉
        let mut flat = FolderMap::default();
        flat.add_node(None, "{项目}").expect("一");
        flat.add_node(None, "{设备}").expect("二");
        assert!(matches!(
            export_template_string(&flat),
            Err(MapError::NotAChain { at: None, branches: 2 })
        ));

        // 空树导不出去
        assert!(matches!(
            export_template_string(&FolderMap::default()),
            Err(MapError::EmptyMap)
        ));

        // 纯字面量链在字符串模板视图立不住脚：导出时就说清，不拖到目的地设置再失败
        let mut plain = FolderMap::default();
        let a = plain.add_node(None, "素材").expect("素材");
        plain.add_node(Some(&a), "视频").expect("视频");
        assert!(matches!(
            export_template_string(&plain),
            Err(MapError::BadTemplateString { .. })
        ));
    }

    // 模板保存与套用：落位剥离、id 全新、结构不变
    #[test]
    fn scenario_copy_map_template_save_and_apply() {
        let mut map = import_template_string("{项目}/{日期}/{设备}").expect("链");
        let leaf = map
            .children(None)
            .first()
            .map(|r| r.id.clone())
            .expect("根");
        map.add_assignment("vol:1", "A7M4", &leaf).expect("现场落位");

        let tpl = MapTemplate::from_map("婚礼模板", &map);
        assert!(tpl.map.assignments.is_empty(), "模板 MUST NOT 带走现场落位");
        assert_eq!(tpl.name, "婚礼模板");

        let inst = tpl.instantiate().expect("套用");
        inst.validate().expect("套出来的树结构完好");
        assert_eq!(inst.nodes.len(), tpl.map.nodes.len());
        assert!(inst.assignments.is_empty());
        // 名字与层级原样，id 全新——模板与实例不可能被误认成同一棵
        let names = |m: &FolderMap| -> Vec<String> {
            m.nodes.iter().map(|n| n.name.clone()).collect()
        };
        assert_eq!(names(&inst), names(&tpl.map));
        for n in &inst.nodes {
            assert!(tpl.map.node(&n.id).is_none(), "实例的 id 不该与模板重合：{}", n.id);
        }
    }
}
