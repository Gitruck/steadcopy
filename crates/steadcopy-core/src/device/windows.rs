//! Windows 卷枚举。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/device-registry/spec.md`
//! 事实依据：`docs/source-devices.md` §F1、§六
//!
//! 三条实现纪律：
//! 1. **枚举前抑制系统错误弹窗**——多卡槽读卡器的空槽也占盘符，
//!    直接打开会弹「请插入磁盘」模态框打断用户；
//! 2. **总线类型经 `IOCTL_STORAGE_QUERY_PROPERTY` 查**，不采信「可移动」标志位；
//! 3. **支持无盘符的卷**——CFexpress 有「枚举成功但无盘符」的实例。

use std::os::windows::ffi::OsStringExt;
use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, MAX_PATH};
use windows::Win32::Storage::FileSystem::{
    BusType1394, BusTypeMmc, BusTypeNvme, BusTypeSas, BusTypeSata, BusTypeScsi, BusTypeSd,
    BusTypeUsb, CreateFileW, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose,
    GetDiskFreeSpaceExW, GetVolumeInformationW, GetVolumePathNamesForVolumeNameW,
    FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Diagnostics::Debug::{
    SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX, THREAD_ERROR_MODE,
};
use windows::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageAdapterProperty, IOCTL_STORAGE_QUERY_PROPERTY,
    STORAGE_ADAPTER_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
};
use windows::Win32::System::IO::DeviceIoControl;

use crate::device::volume::{BusType, Volume, VolumeState};
use crate::error::Result;
use crate::organize::detect_fingerprints;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    std::ffi::OsString::from_wide(&buf[..end])
        .to_string_lossy()
        .into_owned()
}

/// RAII：进入时抑制系统严重错误弹窗，离开时恢复。
struct QuietErrors(THREAD_ERROR_MODE);

impl QuietErrors {
    fn enter() -> Self {
        // SAFETY: SetErrorMode 是进程级设置，这里保存旧值并在 Drop 中恢复。
        let prev = unsafe {
            SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX)
        };
        Self(prev)
    }
}

impl Drop for QuietErrors {
    fn drop(&mut self) {
        // SAFETY: 恢复进入时保存的旧值。
        unsafe {
            SetErrorMode(self.0);
        }
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: 句柄由 CreateFileW 得到，仅在此关闭一次。
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// 查询卷所在设备的总线类型。
///
/// 这是源卡准入的**正向证据**——取代不可靠的「可移动」标志位。
fn query_bus_type(guid_path: &str) -> BusType {
    // 打开卷设备要去掉结尾的反斜杠
    let dev = guid_path.trim_end_matches('\\');
    let w = wide(dev);
    // SAFETY: w 以 NUL 结尾且在调用期间存活。以零访问权限打开，只做属性查询。
    let handle = unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    };
    let Ok(handle) = handle else {
        return BusType::Unknown;
    };
    let handle = OwnedHandle(handle);

    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageAdapterProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut desc = STORAGE_ADAPTER_DESCRIPTOR::default();
    let mut returned: u32 = 0;

    // SAFETY: 入参与出参均指向本栈上的结构，大小如实传入。
    let ok = unsafe {
        DeviceIoControl(
            handle.0,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const _),
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(&mut desc as *mut _ as *mut _),
            std::mem::size_of::<STORAGE_ADAPTER_DESCRIPTOR>() as u32,
            Some(&mut returned),
            None,
        )
    };
    if ok.is_err() || returned == 0 {
        return BusType::Unknown;
    }

    // 描述符里的 BusType 是 u8，常量是 STORAGE_BUS_TYPE(i32)，统一到 i32 比
    let t = i32::from(desc.BusType);
    match t {
        _ if t == BusTypeUsb.0 => BusType::Usb,
        _ if t == BusType1394.0 => BusType::Thunderbolt,
        _ if t == BusTypeSd.0 => BusType::Sd,
        _ if t == BusTypeMmc.0 => BusType::Mmc,
        _ if t == BusTypeNvme.0 => BusType::Nvme,
        _ if t == BusTypeSata.0 => BusType::Sata,
        _ if t == BusTypeScsi.0 || t == BusTypeSas.0 => BusType::Scsi,
        _ => BusType::Other,
    }
}

/// 取卷对应的盘符（可能没有）。
fn drive_letter_of(guid_path: &str) -> Option<String> {
    let w = wide(guid_path);
    let mut buf = vec![0u16; 512];
    let mut len: u32 = 0;
    // SAFETY: 入参以 NUL 结尾，出参缓冲区大小如实传入。
    let ok = unsafe {
        GetVolumePathNamesForVolumeNameW(PCWSTR(w.as_ptr()), Some(&mut buf), &mut len)
    };
    if ok.is_err() {
        return None;
    }
    let first = from_wide(&buf);
    // 形如 "D:\"，取前两个字符
    if first.len() >= 2 && first.as_bytes()[1] == b':' {
        Some(first[..2].to_string())
    } else {
        None
    }
}

/// 系统盘所在卷的根路径（如 `C:\`）。
fn system_drive() -> String {
    std::env::var("SystemDrive")
        .map(|d| format!("{d}\\"))
        .unwrap_or_else(|_| r"C:\".to_string())
}

/// 枚举本机全部卷。
pub fn enumerate() -> Result<Vec<Volume>> {
    // 抑制「请插入磁盘」弹窗——多卡槽读卡器的空槽必踩
    let _quiet = QuietErrors::enter();

    let mut out = Vec::new();
    let mut name = vec![0u16; MAX_PATH as usize];

    // SAFETY: 缓冲区大小如实传入。
    let find = unsafe { FindFirstVolumeW(&mut name) };
    let Ok(find) = find else {
        return Ok(out);
    };

    let sys = system_drive().to_ascii_uppercase();

    loop {
        let guid_path = from_wide(&name);
        if !guid_path.is_empty() {
            if let Some(v) = describe_volume(&guid_path, &sys) {
                out.push(v);
            }
        }
        name.iter_mut().for_each(|c| *c = 0);
        // SAFETY: find 由 FindFirstVolumeW 得到且尚未关闭。
        if unsafe { FindNextVolumeW(find, &mut name) }.is_err() {
            break;
        }
    }
    // SAFETY: 句柄仅在此关闭一次。
    unsafe {
        let _ = FindVolumeClose(find);
    }

    out.sort_by(|a, b| a.drive_letter.cmp(&b.drive_letter));
    Ok(out)
}

fn describe_volume(guid_path: &str, system_root: &str) -> Option<Volume> {
    let letter = drive_letter_of(guid_path);
    let probe = letter
        .as_ref()
        .map(|d| format!("{d}\\"))
        .unwrap_or_else(|| guid_path.to_string());
    let w = wide(&probe);

    let mut label_buf = vec![0u16; 261];
    let mut fs_buf = vec![0u16; 261];
    let mut serial: u32 = 0;

    // SAFETY: 各缓冲区大小如实传入；probe 以 NUL 结尾。
    let info_ok = unsafe {
        GetVolumeInformationW(
            PCWSTR(w.as_ptr()),
            Some(&mut label_buf),
            Some(&mut serial),
            None,
            None,
            Some(&mut fs_buf),
        )
    };

    // 读不到卷信息通常意味着卡槽空着（多卡槽读卡器的常态），
    // 如实记为「无介质」而不是当作错误或直接丢弃。
    let state = if info_ok.is_ok() {
        VolumeState::Online
    } else {
        VolumeState::NoMedia
    };

    let mut total: u64 = 0;
    let mut free: u64 = 0;
    if state == VolumeState::Online {
        // SAFETY: 出参指向本栈上的变量。
        let _ = unsafe {
            GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut free), Some(&mut total), None)
        };
    }

    let is_system = letter
        .as_ref()
        .is_some_and(|d| format!("{d}\\").to_ascii_uppercase() == system_root);

    let fingerprints = if state == VolumeState::Online {
        detect_fingerprints(Path::new(&probe))
    } else {
        Vec::new()
    };

    Some(Volume {
        guid_path: guid_path.to_string(),
        drive_letter: letter,
        label: from_wide(&label_buf),
        serial: (serial != 0).then_some(serial),
        file_system: from_wide(&fs_buf),
        total_bytes: total,
        free_bytes: free,
        bus_type: query_bus_type(guid_path),
        is_system,
        state,
        fingerprints,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 真机枚举：本机至少有一个系统盘，且它 MUST NOT 被判为可用源。
    #[test]
    fn scenario_device_registry_enumerate_finds_system_volume() {
        let vols = enumerate().expect("枚举卷");
        assert!(!vols.is_empty(), "本机应至少枚举到一个卷");

        let sys: Vec<_> = vols.iter().filter(|v| v.is_system).collect();
        assert_eq!(sys.len(), 1, "应恰好有一个系统盘：{:?}",
            vols.iter().map(|v| (&v.drive_letter, v.is_system)).collect::<Vec<_>>());
        assert!(!sys[0].can_be_source(&[]), "系统盘 MUST NOT 可作为源");
        assert!(sys[0].total_bytes > 0, "系统盘应能读到容量");
    }

    #[test]
    fn scenario_device_registry_enumerate_does_not_panic_on_empty_slots() {
        // 空卡槽会让 GetVolumeInformationW 失败——必须被记成 NoMedia 而非崩溃
        let vols = enumerate().expect("枚举卷");
        for v in &vols {
            assert!(!v.guid_path.is_empty());
            if v.state == VolumeState::NoMedia {
                assert_eq!(v.total_bytes, 0);
                assert!(!v.can_be_source(&[]));
            }
        }
    }

    #[test]
    fn scenario_device_registry_bus_type_is_queried() {
        let vols = enumerate().expect("枚举卷");
        let sys = vols.iter().find(|v| v.is_system).expect("系统盘");
        // 本机系统盘应查得出总线类型且不是外接总线
        assert_ne!(
            sys.bus_type,
            BusType::Unknown,
            "系统盘的总线类型应能查到，实际 {:?}",
            sys.bus_type
        );
        assert!(!sys.bus_type.is_external(), "系统盘不该在外接总线上");
    }
}
