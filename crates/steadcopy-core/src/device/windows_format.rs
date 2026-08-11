//! Windows 快速格式化实现。
//!
//! 规范：`openspec/changes/add-steadcopy-format-card/specs/format-card/spec.md`
//! → Requirement: 格式化行为限定
//!
//! # ⚠️ 本文件的代码在开发机上从不执行
//!
//! 它由危险轨测试覆盖，且危险轨**只在虚拟机中跑**（见 `docs/danger-tests.md`）。
//! 调用方 MUST 先过完 `device::format::check_safety` 的 G1–G5，本模块**不重复判断安全性**——
//! 它是执行器，不是守门人。守门在 `format.rs`，两者职责分开是刻意的：
//! 守门逻辑要能在安全轨被完整测试，执行器不能。
//!
//! 用 `fmifs.dll` 的 `FormatEx`（format.com 走的就是它），不拼 PowerShell 命令行。

use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{CoreError, ErrorContext, Result, RetryableKind, TerminalKind};

/// 格式化参数。**只做快速格式化，保留原文件系统与卷标。**
///
/// 相机对文件系统与卷标有要求，改掉会导致卡不被相机识别，
/// 所以这两项 MUST 从系统读取后原样重建，MUST NOT 由用户指定或程序臆断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatParams {
    /// 形如 `E:` 或 `\\?\Volume{...}`
    pub volume: String,
    /// 原文件系统（exFAT / FAT32 / NTFS），从系统读来
    pub file_system: String,
    /// 原卷标，从系统读来
    pub label: String,
}

/// 格式化执行器。
pub trait Formatter: Send + Sync {
    /// 读取该卷当前的文件系统与卷标（格式化前必须先读，之后原样重建）。
    fn read_params(&self, volume: &str) -> Result<FormatParams>;
    /// 执行快速格式化。**调用方 MUST 已通过 G1–G5。**
    fn quick_format(&self, params: &FormatParams) -> Result<()>;
}

/// 取本平台的执行器。
pub fn formatter() -> Box<dyn Formatter> {
    #[cfg(windows)]
    {
        Box::new(WindowsFormatter)
    }
    #[cfg(not(windows))]
    {
        Box::new(UnsupportedFormatter)
    }
}

#[cfg(not(windows))]
#[derive(Debug, Default)]
pub struct UnsupportedFormatter;

#[cfg(not(windows))]
impl Formatter for UnsupportedFormatter {
    fn read_params(&self, _volume: &str) -> Result<FormatParams> {
        Err(CoreError::terminal(TerminalKind::Unsupported))
    }
    fn quick_format(&self, _params: &FormatParams) -> Result<()> {
        Err(CoreError::terminal(TerminalKind::Unsupported))
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;

    #[derive(Debug, Default)]
    pub struct WindowsFormatter;

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn from_wide(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    /// 归一为 `X:\` 或以反斜杠结尾的卷 GUID 路径。
    fn root_of(volume: &str) -> String {
        if volume.ends_with('\\') {
            volume.to_string()
        } else {
            format!("{volume}\\")
        }
    }

    /// `fmifs.dll` 的 `FormatEx` 回调状态码。我们只关心「完成」与「进度」。
    const FMIFS_DONE: u32 = 11;

    static CALLBACK_OK: AtomicBool = AtomicBool::new(false);

    /// FormatEx 的回调。约定返回 TRUE 表示继续。
    unsafe extern "system" fn format_callback(
        command: u32,
        _modifier: u32,
        argument: *mut core::ffi::c_void,
    ) -> windows::Win32::Foundation::BOOL {
        if command == FMIFS_DONE && !argument.is_null() {
            // SAFETY: DONE 回调的 argument 指向一个 BOOLEAN（成功与否）
            let ok = unsafe { *(argument as *const u8) } != 0;
            CALLBACK_OK.store(ok, Ordering::SeqCst);
        }
        windows::Win32::Foundation::TRUE
    }

    type FormatExFn = unsafe extern "system" fn(
        PCWSTR, // DriveRoot
        u32,    // MediaFlag (0 = FMIFS_UNKNOWN, 12 = FMIFS_HARDDISK)
        PCWSTR, // Format (文件系统名)
        PCWSTR, // Label
        windows::Win32::Foundation::BOOL, // QuickFormat
        u32,    // ClusterSize（0 = 默认）
        Option<
            unsafe extern "system" fn(
                u32,
                u32,
                *mut core::ffi::c_void,
            ) -> windows::Win32::Foundation::BOOL,
        >,
    );

    impl Formatter for WindowsFormatter {
        fn read_params(&self, volume: &str) -> Result<FormatParams> {
            let root = root_of(volume);
            let w = wide(&root);
            let mut label = vec![0u16; 261];
            let mut fs = vec![0u16; 261];
            // SAFETY: 缓冲区大小如实传入；root 以 NUL 结尾
            unsafe {
                GetVolumeInformationW(
                    PCWSTR(w.as_ptr()),
                    Some(&mut label),
                    None,
                    None,
                    None,
                    Some(&mut fs),
                )
            }
            .map_err(|e| {
                CoreError::Terminal(
                    TerminalKind::InvalidConfig,
                    ErrorContext::new().cause(format!("读取卷信息失败（{root}）：{e}")),
                )
            })?;

            let file_system = from_wide(&fs);
            if file_system.trim().is_empty() {
                // 读不到文件系统就不敢格——不臆断
                return Err(CoreError::Terminal(
                    TerminalKind::InvalidConfig,
                    ErrorContext::new()
                        .cause(format!("读不到 {root} 的文件系统类型，出于安全拒绝格式化")),
                ));
            }
            Ok(FormatParams {
                volume: root,
                file_system,
                label: from_wide(&label),
            })
        }

        fn quick_format(&self, params: &FormatParams) -> Result<()> {
            let lib = wide("fmifs.dll");
            // SAFETY: 加载系统 DLL
            let module = unsafe {
                windows::Win32::System::LibraryLoader::LoadLibraryW(PCWSTR(lib.as_ptr()))
            }
            .map_err(|e| {
                CoreError::Terminal(
                    TerminalKind::Unsupported,
                    ErrorContext::new().cause(format!("加载 fmifs.dll 失败：{e}")),
                )
            })?;

            // SAFETY: 按名取导出函数
            let proc = unsafe {
                windows::Win32::System::LibraryLoader::GetProcAddress(
                    module,
                    windows::core::PCSTR(c"FormatEx".as_ptr() as *const u8),
                )
            }
            .ok_or_else(|| {
                CoreError::Terminal(
                    TerminalKind::Unsupported,
                    ErrorContext::new().cause("fmifs.dll 里找不到 FormatEx"),
                )
            })?;

            // SAFETY: FormatEx 的签名由 fmifs 约定，见 FormatExFn
            let format_ex: FormatExFn = unsafe { std::mem::transmute(proc) };

            let root = wide(&params.volume);
            let fs = wide(&params.file_system);
            let label = wide(&params.label);
            CALLBACK_OK.store(false, Ordering::SeqCst);

            // SAFETY: 三个宽串均以 NUL 结尾且在调用期间存活；回调为本模块的静态函数
            unsafe {
                format_ex(
                    PCWSTR(root.as_ptr()),
                    0, // FMIFS_UNKNOWN：让驱动自己判断介质类型
                    PCWSTR(fs.as_ptr()),
                    PCWSTR(label.as_ptr()),
                    windows::Win32::Foundation::TRUE, // 只做快速格式化
                    0,                                 // 默认簇大小
                    Some(format_callback),
                );
            }

            if CALLBACK_OK.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(CoreError::Retryable(
                    RetryableKind::DestinationUnwritable,
                    ErrorContext::new().cause(format!(
                        "格式化 {} 未成功完成（可能被占用或介质写保护）",
                        params.volume
                    )),
                ))
            }
        }
    }
}

#[cfg(windows)]
pub use imp::WindowsFormatter;

#[cfg(test)]
mod tests {
    use super::*;

    // 安全轨：只验参数结构与「读不到就不敢格」的立场，**不调用任何格式化 API**。
    #[test]
    fn scenario_format_card_params_carry_original_fs_and_label() {
        let p = FormatParams {
            volume: r"E:\".into(),
            file_system: "exFAT".into(),
            label: "A7M4-1".into(),
        };
        // 文件系统与卷标是从系统读来的，不是用户填的——这条靠类型与调用顺序保证，
        // 这里断言结构里确实带着它们，供执行时原样重建
        assert_eq!(p.file_system, "exFAT");
        assert_eq!(p.label, "A7M4-1");
    }

    #[cfg(windows)]
    #[test]
    fn scenario_format_card_read_params_on_missing_volume_errors() {
        // 读一个不存在的卷 MUST 报错，而不是返回一个瞎猜的默认值
        let f = WindowsFormatter;
        let err = f
            .read_params(r"\\?\Volume{00000000-0000-0000-0000-000000000000}")
            .expect_err("不存在的卷 MUST 报错");
        assert!(!err.is_retryable());
    }
}
