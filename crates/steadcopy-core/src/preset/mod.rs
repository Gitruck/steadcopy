//! 预设任务与插卡自动流程。
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/preset-autorun/spec.md`

pub mod arrival;
pub mod matching;
pub mod model;

pub use arrival::{build_spec, on_arrival, ArrivalOutcome};
pub use matching::select_preset;
pub use model::{Preset, PresetMatch};
