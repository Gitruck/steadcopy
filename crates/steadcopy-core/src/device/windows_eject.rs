//! Windows 安全弹出实现。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/device-registry/spec.md`
//! → Requirement: 安全弹出
//!
//! 四步，缺一不可：
//!
//! ```text
//! FSCTL_LOCK_VOLUME        独占卷；被别的程序占着就在这一步失败（这是最常见的失败点）
//! FSCTL_DISMOUNT_VOLUME    卸载文件系统，把脏页刷下去
//! IOCTL_STORAGE_MEDIA_REMOVAL  解除「禁止取出介质」
//! IOCTL_STORAGE_EJECT_MEDIA    弹出
//! ```
//!
//! 只做到 dismount 就拔卡，写缓存可能还没落到介质上——「安全弹出」这四个字里
//! 「安全」的部分正是前两步。少做一步就成了摆设。

use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_NO_BUFFERING, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, IOCTL_STORAGE_EJECT_MEDIA,
    IOCTL_STORAGE_MEDIA_REMOVAL, PREVENT_MEDIA_REMOVAL,
};
use windows::Win32::System::IO::DeviceIoControl;

use crate::device::eject::{EjectError, Ejector};

#[derive(Debug, Default)]
pub struct WindowsEjector;

/// 打开卷的设备路径：`E:\` → `\\.\E:`，卷 GUID 路径去掉尾部反斜杠。
fn device_path(root: &Path) -> String {
    let s = root.to_string_lossy();
    let t = s.trim_end_matches(['\\', '/']);
    if t.len() == 2 && t.ends_with(':') {
        format!(r"\\.\{t}")
    } else {
        t.to_string()
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error() -> String {
    let e = unsafe { GetLastError() };
    format!("系统错误码 {}", e.0)
}

impl Ejector for WindowsEjector {
    fn eject(&self, root: &Path) -> Result<(), EjectError> {
        let path = wide(&device_path(root));
        let handle: HANDLE = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_NO_BUFFERING,
                None,
            )
        }
        .map_err(|e| EjectError::Failed(format!("打不开卷：{e}")))?;

        if handle == INVALID_HANDLE_VALUE {
            return Err(EjectError::Failed("打不开卷".into()));
        }

        let result = run_steps(handle);
        unsafe {
            let _ = CloseHandle(handle);
        }
        result
    }
}

fn run_steps(handle: HANDLE) -> Result<(), EjectError> {
    // 1. 独占。这一步失败几乎总是「有别的程序开着卡上的文件」
    if !ioctl(handle, FSCTL_LOCK_VOLUME, None) {
        return Err(EjectError::Busy(last_error()));
    }
    // 2. 卸载：把脏页刷下去。少了这一步，「弹出」只是把托盘图标去掉
    if !ioctl(handle, FSCTL_DISMOUNT_VOLUME, None) {
        return Err(EjectError::Failed(format!("卸载卷失败（{}）", last_error())));
    }
    // 3. 允许取出介质
    let mut pmr = PREVENT_MEDIA_REMOVAL {
        PreventMediaRemoval: false.into(),
    };
    let pmr_ptr = std::ptr::addr_of_mut!(pmr).cast::<std::ffi::c_void>();
    let pmr_len = std::mem::size_of::<PREVENT_MEDIA_REMOVAL>() as u32;
    if !ioctl(handle, IOCTL_STORAGE_MEDIA_REMOVAL, Some((pmr_ptr, pmr_len))) {
        return Err(EjectError::Failed(format!(
            "解除介质锁定失败（{}）",
            last_error()
        )));
    }
    // 4. 弹出。读卡器里的卡不一定有物理弹出动作，这一步失败不代表前三步白做——
    //    但也不能因此就报成功，如实返回
    if !ioctl(handle, IOCTL_STORAGE_EJECT_MEDIA, None) {
        return Err(EjectError::Failed(format!("弹出失败（{}）", last_error())));
    }
    Ok(())
}

fn ioctl(handle: HANDLE, code: u32, input: Option<(*mut std::ffi::c_void, u32)>) -> bool {
    let mut returned: u32 = 0;
    let (ptr, len) = input.unwrap_or((std::ptr::null_mut(), 0));
    unsafe {
        DeviceIoControl(
            handle,
            code,
            if ptr.is_null() { None } else { Some(ptr) },
            len,
            None,
            0,
            Some(&mut returned),
            None,
        )
    }
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: → Scenario: 安全弹出经系统接口实现
    #[test]
    fn scenario_device_registry_eject_device_path_shape() {
        // 盘符要转成设备路径，卷 GUID 路径只去尾部反斜杠
        assert_eq!(device_path(Path::new(r"E:\")), r"\\.\E:");
        assert_eq!(device_path(Path::new("E:")), r"\\.\E:");
        assert_eq!(
            device_path(Path::new(r"\\?\Volume{1234}\")),
            r"\\?\Volume{1234}"
        );
    }
}
