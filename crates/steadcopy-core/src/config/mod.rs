//! 配置持久化：项目、目的地、预设、设备记忆、应用设置。
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/config-store/spec.md`

pub mod model;
pub mod store;

pub use model::{
    new_id, ArrivalMode, Config, ConfigError, DestinationConfig, Project, Settings,
    CONFIG_VERSION, DEFAULT_TEMPLATE,
};
pub use store::{config_dir, config_path, load, save, ConfigLoadError};
