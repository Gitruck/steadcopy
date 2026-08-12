//! 任务编排：把扫描、预检、拷贝、校验、清单串成一次完整的拷卡。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/copy-engine/spec.md`
//! → Requirement: 任务阶段模型 / 空间预检 / 断点续传 / 校验失败自动重拷

pub mod adhoc;
pub mod plan;
pub mod run;
pub mod stage;

pub use plan::{plan_task, DestinationPlan, TaskPlan, TaskSpec};
pub use run::{run_task, FileOutcome, FileStatus, TaskReport};
pub use stage::{StageEvent, TaskStage};

pub use plan::{DestinationSpec, PlannedFile};
pub use adhoc::{
    adhoc_defaults, build_adhoc_spec, AdhocDefaults, AdhocError, AdhocRequest, ProjectChoice,
    DEFAULT_PROJECT_NAME,
};
