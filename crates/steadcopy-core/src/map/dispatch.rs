//! 落位与派发：连线即任务，翻译发生在按下「全部开始」那一刻。
//!
//! 规范：`openspec/changes/add-steadcopy-copy-map/specs/copy-map/spec.md`
//! → Requirement: 连线派发与临时拷贝走同一条构造路径
//!
//! # 为什么必须走 `build_adhoc_spec`（设计 D2）
//!
//! 每条落位翻译成 `TaskSpec` 只有一条路：把「节点在树里的路径」当模板串塞进
//! [`AdhocRequest::template_override`]，其余全交给临时拷贝的构造函数。
//! 下游——队列、引擎、清单、台账、报告——**分不出任务来自导图**，这是刻意的：
//! 下游能分辨来源，迟早会有人为「导图任务」写捷径，而捷径总是从跳过校验开始。
//! 「正在跑的设备拒绝」「校验不可跳过」这些不变量因此自动继承，一条不用重写。

use std::path::PathBuf;

use time::OffsetDateTime;

use crate::config::model::Config;
use crate::map::model::{Assignment, FolderMap, MapError};
use crate::task::{build_adhoc_spec, AdhocRequest, ProjectChoice, TaskSpec};

impl FolderMap {
    /// 建一条「设备 → 节点」落位（画布上拖一根连线）。
    ///
    /// 一张卡可以连多个节点、一个节点可收多张卡；唯独**完全相同**的
    /// 设备-节点对拒绝重复——那只会派出两份一样的任务。
    pub fn add_assignment(
        &mut self,
        device_id: &str,
        device_name: &str,
        node_id: &str,
    ) -> Result<String, MapError> {
        let node = self
            .node(node_id)
            .ok_or_else(|| MapError::NodeMissing {
                id: node_id.to_string(),
            })?;
        if self
            .assignments
            .iter()
            .any(|a| a.device_id == device_id && a.node_id == node_id)
        {
            return Err(MapError::DuplicateAssignment {
                device_name: device_name.to_string(),
                node_name: node.name.clone(),
            });
        }
        let id = crate::config::model::new_id("lnk");
        self.assignments.push(Assignment {
            id: id.clone(),
            device_id: device_id.to_string(),
            device_name: device_name.to_string(),
            node_id: node_id.to_string(),
        });
        Ok(id)
    }

    /// 摘掉一条落位。返回被摘的那条，调用方要给用户复述摘的是什么。
    pub fn remove_assignment(&mut self, assignment_id: &str) -> Result<Assignment, MapError> {
        let idx = self
            .assignments
            .iter()
            .position(|a| a.id == assignment_id)
            .ok_or_else(|| MapError::AssignmentMissing {
                id: assignment_id.to_string(),
            })?;
        Ok(self.assignments.remove(idx))
    }
}

/// 派发时每台设备的源卷位置。由调用方（壳层）从设备注册表拿，core 不碰枚举。
#[derive(Debug, Clone)]
pub struct DispatchSource {
    pub device_id: String,
    pub source_root: PathBuf,
}

/// 一条翻译成功的落位。
#[derive(Debug, Clone)]
pub struct MapDispatch {
    pub assignment_id: String,
    pub device_name: String,
    /// 节点在树里的路径（`/` 相连），进度沿连线显示时的锚
    pub node_path: String,
    pub spec: TaskSpec,
}

/// 一条被拒的落位。拒了要说清为什么——不是吞掉。
#[derive(Debug, Clone)]
pub struct MapRejection {
    pub assignment_id: String,
    pub device_name: String,
    pub reason: MapError,
}

/// 「全部开始」的翻译结果：能走的与被拒的，泾渭分明。
///
/// 不做 all-or-nothing：三张卡里一张正在跑，另两张没理由陪绑——
/// 被拒的逐条带原因呈现，用户自己决定等不等。
#[derive(Debug, Clone, Default)]
pub struct DispatchPlan {
    pub ready: Vec<MapDispatch>,
    pub rejected: Vec<MapRejection>,
}

/// 把导图上的全部落位翻译成任务规格。**零副作用**：不落盘、不入队，
/// 产出的规格由调用方在用户确认后逐个起跑。
pub fn dispatch_assignments(
    config: &Config,
    map: &FolderMap,
    project_id: &str,
    sources: &[DispatchSource],
    running_device_ids: &[String],
    now: OffsetDateTime,
) -> DispatchPlan {
    let mut plan = DispatchPlan::default();

    for a in &map.assignments {
        // 节点路径各段拼成模板串——两个视图共用同一套占位符词表（设计 D2/D4）
        let template = match map.path_segments(&a.node_id) {
            Ok(segs) => segs.join("/"),
            Err(reason) => {
                plan.rejected.push(MapRejection {
                    assignment_id: a.id.clone(),
                    device_name: a.device_name.clone(),
                    reason,
                });
                continue;
            }
        };

        // 卡不在机上就拒，不猜路径——猜错的代价是拷去不存在的源
        let Some(src) = sources.iter().find(|s| s.device_id == a.device_id) else {
            plan.rejected.push(MapRejection {
                assignment_id: a.id.clone(),
                device_name: a.device_name.clone(),
                reason: MapError::SourceOffline {
                    device_name: a.device_name.clone(),
                },
            });
            continue;
        };

        let req = AdhocRequest {
            source_root: src.source_root.clone(),
            device_id: a.device_id.clone(),
            device_name: a.device_name.clone(),
            // 导图长在项目上，派发只可能指向已有项目——不存在「现建一个」的分支
            project: ProjectChoice::Existing(project_id.to_string()),
            destinations: Vec::new(),
            // 校验不可跳过是导图的既定不变量：连「跟随全局默认」都不给——
            // 默认值可能被用户关过，而导图派发面板上根本没有这个旋钮
            verify: Some(true),
            algorithm: None,
            eject_after: config.settings.eject_after,
            template_override: Some(template.clone()),
        };

        match build_adhoc_spec(config, &req, running_device_ids, now) {
            Ok((spec, pending)) => {
                // ProjectChoice::Existing 不可能产出待建项目；真产出说明构造路变了，
                // 这里要第一时间炸出来而不是悄悄丢掉
                debug_assert!(pending.is_none(), "沿用已有项目不该产出待建项目");
                plan.ready.push(MapDispatch {
                    assignment_id: a.id.clone(),
                    device_name: a.device_name.clone(),
                    node_path: template,
                    spec,
                });
            }
            Err(reason) => plan.rejected.push(MapRejection {
                assignment_id: a.id.clone(),
                device_name: a.device_name.clone(),
                reason: MapError::Dispatch { reason },
            }),
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{DestinationConfig, Project};
    use crate::organize::RenderContext;
    use crate::task::AdhocError;
    use time::macros::datetime;

    fn at() -> OffsetDateTime {
        datetime!(2026-08-12 09:00:00 UTC)
    }

    fn config_with_project() -> (Config, String) {
        let mut c = Config::default();
        let mut p = Project::new("婚礼", at());
        p.destinations.push(DestinationConfig::new(r"D:\素材"));
        let id = p.id.clone();
        c.current_project = Some(id.clone());
        c.projects.push(p);
        (c, id)
    }

    fn sources() -> Vec<DispatchSource> {
        vec![DispatchSource {
            device_id: "vol:1".into(),
            source_root: PathBuf::from(r"E:\"),
        }]
    }

    // spec: copy-map → 连线派发与临时拷贝走同一条构造路径
    //       → Scenario: 导图任务与临时拷贝不可区分
    #[test]
    fn scenario_copy_map_dispatch_indistinguishable_from_adhoc() {
        let (mut c, pid) = config_with_project();
        // 导图：把默认字符串模板画成三层链，两个视图指同一个目标
        let mut map = FolderMap::default();
        let mut parent: Option<String> = None;
        for seg in ["{项目}", "{日期}", "{设备}"] {
            parent = Some(map.add_node(parent.as_deref(), seg).expect("建链"));
        }
        let leaf = parent.expect("叶子");
        map.add_assignment("vol:1", "A7M4", &leaf).expect("落位");
        c.project_mut(&pid).expect("项目").map = Some(map.clone());

        let plan = dispatch_assignments(&c, &map, &pid, &sources(), &[], at());
        assert!(plan.rejected.is_empty(), "{:?}", plan.rejected);
        let from_map = &plan.ready[0].spec;

        // 临时拷贝：同设备、同项目、同参数
        let (from_adhoc, pending) = build_adhoc_spec(
            &c,
            &AdhocRequest {
                source_root: PathBuf::from(r"E:\"),
                device_id: "vol:1".into(),
                device_name: "A7M4".into(),
                project: ProjectChoice::Existing(pid),
                destinations: vec![],
                verify: Some(true),
                algorithm: None,
                eject_after: c.settings.eject_after,
                template_override: None,
            },
            &[],
            at(),
        )
        .expect("临时路径");
        assert!(pending.is_none());

        // 逐字段比对：下游能分辨来源，迟早有人为「导图」写捷径
        assert_eq!(from_map.source_root, from_adhoc.source_root);
        assert_eq!(from_map.source.id, from_adhoc.source.id);
        assert_eq!(from_map.source.display_name, from_adhoc.source.display_name);
        assert_eq!(from_map.project, from_adhoc.project);
        assert_eq!(from_map.algorithm, from_adhoc.algorithm);
        assert_eq!(from_map.verify, from_adhoc.verify);
        assert_eq!(from_map.retries, from_adhoc.retries);
        assert_eq!(from_map.eject_after, from_adhoc.eject_after);
        assert_eq!(from_map.at, from_adhoc.at);
        assert_eq!(from_map.destinations.len(), from_adhoc.destinations.len());
        for (a, b) in from_map.destinations.iter().zip(from_adhoc.destinations.iter()) {
            assert_eq!(a.root, b.root);
            assert_eq!(a.enabled, b.enabled);
            // 三层链拼回的模板串与项目里的默认模板一字不差
            assert_eq!(a.template, b.template);
        }
    }

    // spec: copy-map → 连线派发与临时拷贝走同一条构造路径 → Scenario: 派发不可跳过校验
    #[test]
    fn scenario_copy_map_dispatch_cannot_skip_verification() {
        let (mut c, pid) = config_with_project();
        // 就算用户把全局默认校验关了，导图派发也不跟——它没有这个旋钮
        c.settings.verify_default = false;
        let mut map = FolderMap::default();
        let n = map.add_node(None, "素材").expect("节点");
        map.add_assignment("vol:1", "A7M4", &n).expect("落位");

        let plan = dispatch_assignments(&c, &map, &pid, &sources(), &[], at());
        assert_eq!(plan.ready.len(), 1);
        assert!(plan.ready[0].spec.verify, "导图派发的校验开关恒为开");
        assert!(
            plan.ready[0].spec.scan.filter.is_none(),
            "同样是整卡镜像，MUST NOT 偷偷过滤"
        );
    }

    // spec: copy-map → 连线派发与临时拷贝走同一条构造路径
    //       → Scenario: 正在跑的设备拒绝重复派发
    #[test]
    fn scenario_copy_map_running_device_rejects_dispatch() {
        let (c, pid) = config_with_project();
        let mut map = FolderMap::default();
        let n = map.add_node(None, "素材").expect("节点");
        map.add_assignment("vol:1", "A7M4", &n).expect("落位 1");
        map.add_assignment("vol:2", "H6", &n).expect("落位 2");

        let mut srcs = sources();
        srcs.push(DispatchSource {
            device_id: "vol:2".into(),
            source_root: PathBuf::from(r"F:\"),
        });
        let running = vec!["vol:1".to_string()];
        let plan = dispatch_assignments(&c, &map, &pid, &srcs, &running, at());

        // 在跑的被拒且说清原因；没在跑的不陪绑
        assert_eq!(plan.rejected.len(), 1);
        assert_eq!(plan.rejected[0].device_name, "A7M4");
        match &plan.rejected[0].reason {
            MapError::Dispatch { reason } => {
                assert!(matches!(reason, AdhocError::AlreadyRunning { .. }));
            }
            other => panic!("拒绝理由不对：{other:?}"),
        }
        let msg = plan.rejected[0].reason.describe(crate::i18n::Locale::Zh);
        assert!(msg.contains("A7M4"), "原因里要点名是哪台设备：{msg}");
        assert_eq!(plan.ready.len(), 1);
        assert_eq!(plan.ready[0].device_name, "H6");
    }

    // 排队中的设备同样拒绝重复派发（复核修复 F1）。
    //
    // `running_device_ids` 的契约是「**已被任务占用**的设备」，不只是「引擎此刻正在拷的」：
    // 壳层在派发被接受那一刻就把设备记进去（而不是等抢到串行闸才记），
    // 否则多卡派发时排队几十分钟的卡对第二次「全部开始」不可见——
    // 再点一次就重复起任务，大卡白拷数小时、台账双份。
    // 临时拷贝 / 插卡到达检查的也是同一个集合（build_adhoc_spec / on_arrival），
    // 占位提前后它们自动继承「已排队 = 占用」，不需要各自再写一遍。
    #[test]
    fn scenario_copy_map_queued_device_rejects_second_dispatch() {
        let (c, pid) = config_with_project();
        let mut map = FolderMap::default();
        let a = map.add_node(None, "素材").expect("节点 A");
        let b = map.add_node(None, "备份").expect("节点 B");
        // 同一张卡连两个节点：**同一批**里派两个任务是既定功能，占位不许误杀它
        map.add_assignment("vol:1", "A7M4", &a).expect("落位 A");
        map.add_assignment("vol:1", "A7M4", &b).expect("落位 B");

        // 第一次派发：没有占用，同批两条都放行
        let first = dispatch_assignments(&c, &map, &pid, &sources(), &[], at());
        assert_eq!(first.ready.len(), 2, "{:?}", first.rejected);

        // 壳层把被接受的任务立即记为占用（每任务一个名额）——
        // 它们可能还在串行闸后排队，没真正开拷
        let queued: Vec<String> = first.ready.iter().map(|d| d.spec.source.id.clone()).collect();

        // 第二次「全部开始」：同一设备的两条落位都必须被拒，且说清原因
        let second = dispatch_assignments(&c, &map, &pid, &sources(), &queued, at());
        assert!(second.ready.is_empty(), "排队中的设备不许再派");
        assert_eq!(second.rejected.len(), 2);
        for r in &second.rejected {
            match &r.reason {
                MapError::Dispatch { reason } => {
                    assert!(matches!(reason, AdhocError::AlreadyRunning { .. }));
                }
                other => panic!("拒绝理由不对：{other:?}"),
            }
        }
    }

    // 目标路径 = 项目根 + 节点路径，占位符复用 PathTemplate 渲染（tasks 2.3）
    #[test]
    fn scenario_copy_map_dispatch_renders_node_path_with_placeholders() {
        let (c, pid) = config_with_project();
        let mut map = FolderMap::default();
        let root = map.add_node(None, "素材").expect("根");
        let day = map.add_node(Some(&root), "{日期}").expect("日期层");
        let leaf = map.add_node(Some(&day), "{设备}").expect("设备层");
        map.add_assignment("vol:1", "A7M4", &leaf).expect("落位");

        let plan = dispatch_assignments(&c, &map, &pid, &sources(), &[], at());
        assert_eq!(plan.ready.len(), 1, "{:?}", plan.rejected);
        let d = &plan.ready[0];
        assert_eq!(d.node_path, "素材/{日期}/{设备}");

        // 渲染出的落地段与字符串模板视图一字不差——同一个渲染函数
        let spec = &d.spec;
        assert_eq!(spec.destinations[0].root, PathBuf::from(r"D:\素材"));
        let segs = spec.destinations[0].template.render_segments(&RenderContext {
            project: spec.project.clone(),
            device: spec.source.display_name.clone(),
            card: spec.source.display_name.clone(),
            at: spec.at,
        });
        assert_eq!(segs, vec!["素材", "2026-08-12", "A7M4"]);
    }

    // 卡不在机上：拒且说明，不猜路径
    #[test]
    fn scenario_copy_map_dispatch_offline_device_rejected_with_reason() {
        let (c, pid) = config_with_project();
        let mut map = FolderMap::default();
        let n = map.add_node(None, "素材").expect("节点");
        map.add_assignment("vol:404", "拔走的卡", &n).expect("落位");

        let plan = dispatch_assignments(&c, &map, &pid, &[], &[], at());
        assert!(plan.ready.is_empty());
        assert!(matches!(
            plan.rejected[0].reason,
            MapError::SourceOffline { .. }
        ));
    }

    // 落位增删：重复连线拒绝、摘线要有回执
    #[test]
    fn scenario_copy_map_assignment_add_remove_guarded() {
        let mut map = FolderMap::default();
        let a = map.add_node(None, "视频").expect("A");
        let b = map.add_node(None, "照片").expect("B");

        let id = map.add_assignment("vol:1", "A7M4", &a).expect("首条");
        // 一张卡连多个节点、一个节点收多张卡都合法
        map.add_assignment("vol:1", "A7M4", &b).expect("同一张卡连第二个节点");
        map.add_assignment("vol:2", "H6", &a).expect("同节点第二张卡");
        // 完全相同的设备-节点对拒绝
        assert!(matches!(
            map.add_assignment("vol:1", "A7M4", &a),
            Err(MapError::DuplicateAssignment { .. })
        ));
        // 指向不存在节点的落位拒绝
        assert!(matches!(
            map.add_assignment("vol:1", "A7M4", "map-不存在"),
            Err(MapError::NodeMissing { .. })
        ));

        let removed = map.remove_assignment(&id).expect("摘线");
        assert_eq!(removed.node_id, a);
        assert!(matches!(
            map.remove_assignment(&id),
            Err(MapError::AssignmentMissing { .. })
        ));
        assert_eq!(map.assignments.len(), 2);
    }
}
