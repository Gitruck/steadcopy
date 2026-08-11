//! 设备：枚举、身份、分类、准入判据。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/device-registry/spec.md`
//! 事实依据：`docs/source-devices.md`

pub mod format;
pub mod kind;
pub mod volume;
pub mod watch;

pub mod windows_format;

#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub mod windows_watch;

pub use kind::{next_instance, DeviceKind, DeviceRecord};
pub use volume::{BusType, Volume, VolumeState};
pub use windows_format::{formatter, FormatParams, Formatter};
pub use format::{
    check_safety, removability, validate_countdown, BackupEvidence, CheckResult, Removability,
    RemovabilityError, SafetyReport, COUNTDOWN_DEFAULT_SECS, COUNTDOWN_MIN_SECS,
};
pub use watch::{drive_letters_from_mask, DeviceEvent, DeviceWatcher, MockDeviceWatcher};

/// 取本平台的设备监听器。
pub fn device_watcher() -> Box<dyn DeviceWatcher> {
    #[cfg(windows)]
    {
        Box::new(windows_watch::WindowsDeviceWatcher::new())
    }
    #[cfg(not(windows))]
    {
        Box::new(MockDeviceWatcher::new())
    }
}

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
