//! 设备：枚举、身份、分类、准入判据。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/device-registry/spec.md`
//! 事实依据：`docs/source-devices.md`

pub mod kind;
pub mod volume;

#[cfg(windows)]
pub mod windows;

pub use kind::{DeviceKind, DeviceRecord};
pub use volume::{BusType, Volume, VolumeState};

/// 枚举本机当前挂载的全部卷。
pub fn enumerate_volumes() -> crate::error::Result<Vec<Volume>> {
    #[cfg(windows)]
    {
        windows::enumerate()
    }
    #[cfg(not(windows))]
    {
        Err(crate::error::CoreError::terminal(
            crate::error::TerminalKind::Unsupported,
        ))
    }
}
