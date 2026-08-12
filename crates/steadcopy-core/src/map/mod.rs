//! 拷贝导图：目标目录树画成节点图，连线即任务。
//!
//! 规范：`openspec/changes/add-steadcopy-copy-map/specs/copy-map/spec.md`
//!
//! 四块职责，各居其文件：
//! - `model`：树模型与全部校验（前端只画，不持状态——设计 D1）
//! - `dispatch`：落位增删；派发时逐条翻译成 `TaskSpec`，走临时拷贝同一条构造路（设计 D2）
//! - `template`：导图模板；与字符串模板互为视图，不搞两套存储（设计 D4）
//! - `refresh`：fs → 图单向同步，只读、只增不删（设计 D5）

pub mod dispatch;
pub mod model;
pub mod refresh;
pub mod template;

pub use dispatch::{
    dispatch_assignments, DispatchPlan, DispatchSource, MapDispatch, MapRejection,
};
pub use model::{
    validate_node_name, Assignment, FolderMap, MapError, MapNode, MAX_DEPTH, MAX_NAME_CHARS,
};
pub use refresh::{
    apply_refresh, diff_refresh, is_excluded_dir, RefreshAddition, RefreshPlan, RefreshSkipped,
};
pub use template::{export_template_string, import_template_string, MapTemplate};
