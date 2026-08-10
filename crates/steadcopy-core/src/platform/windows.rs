//! Windows 的卷级 IO 实现。
//!
//! 核心是 `FILE_FLAG_NO_BUFFERING` 读回——见 `platform::VolumeIo::read_unbuffered` 的文档。
//!
//! 无缓冲 IO 的三条对齐硬要求（Windows 的硬性约束，不满足会直接失败）：
//! 1. 缓冲区**起始地址**按扇区大小对齐；
//! 2. 单次读取**长度**是扇区大小的整数倍；
//! 3. 文件**偏移**是扇区大小的整数倍。

use std::alloc::{alloc, dealloc, Layout};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, GetDiskFreeSpaceExW, GetDiskFreeSpaceW, ReadFile,
    FILE_FLAG_NO_BUFFERING, FILE_FLAG_SEQUENTIAL_SCAN, FILE_GENERIC_READ, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};

use super::{VolumeIo, FALLBACK_SECTOR_SIZE};
use crate::error::{CoreError, ErrorContext, Result, TerminalKind};

/// 单次读取的块大小（会被向上取整到扇区倍数）。顺序大块读是慢介质上的正确模式。
const READ_CHUNK: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsVolumeIo;

/// 按指定对齐分配的缓冲区。无缓冲读要求缓冲区**起始地址**对齐到扇区边界，
/// 普通 `Vec<u8>` 不保证这一点。
struct AlignedBuffer {
    ptr: *mut u8,
    len: usize,
    layout: Layout,
}

impl AlignedBuffer {
    fn new(len: usize, align: usize) -> Result<Self> {
        let layout = Layout::from_size_align(len, align).map_err(|e| {
            CoreError::Terminal(
                TerminalKind::SourceUnreadable,
                ErrorContext::new().cause(format!("无法构造对齐布局：{e}")),
            )
        })?;
        // SAFETY: layout 的 size 非零（调用方保证 len > 0），align 是 2 的幂。
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            return Err(CoreError::Terminal(
                TerminalKind::SourceUnreadable,
                ErrorContext::new().cause("对齐缓冲区分配失败"),
            ));
        }
        Ok(Self { ptr, len, layout })
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: ptr 由 alloc 得到且非空，长度为 layout.size()。
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        // SAFETY: ptr 与 layout 成对，仅在此处释放一次。
        unsafe { dealloc(self.ptr, self.layout) }
    }
}

/// RAII 句柄，保证任何早退路径都会关闭。
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: 句柄由 CreateFileW 得到且仅在此处关闭一次。
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

fn to_wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn io_err(kind: TerminalKind, path: &Path, cause: impl std::fmt::Display) -> CoreError {
    CoreError::Terminal(
        kind,
        ErrorContext::new().path(path).cause(cause.to_string()),
    )
}

/// 取路径所在卷的根（如 `D:\`）。取不到时返回 `None`。
fn volume_root(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut comps = path.components();
    match comps.next() {
        Some(Component::Prefix(p)) => {
            let mut root = PathBuf::from(p.as_os_str());
            root.push(std::path::MAIN_SEPARATOR_STR);
            Some(root)
        }
        _ => None,
    }
}

impl VolumeIo for WindowsVolumeIo {
    fn sector_size(&self, path: &Path) -> Result<usize> {
        // 查不到扇区大小时用保守回退值，而不是硬编码 512 —— 4096 对两种常见扇区都合法。
        // 这里的降级是**安全方向**的（对齐要求变严），因此不违反「绝不静默降级」。
        let Some(root) = volume_root(path) else {
            return Ok(FALLBACK_SECTOR_SIZE);
        };
        let wide = to_wide(&root);
        let mut bytes_per_sector: u32 = 0;
        // SAFETY: wide 以 NUL 结尾且在调用期间存活；输出指针指向本栈上的变量。
        let ok = unsafe {
            GetDiskFreeSpaceW(
                PCWSTR(wide.as_ptr()),
                None,
                Some(&mut bytes_per_sector),
                None,
                None,
            )
        };
        match ok {
            Ok(()) if bytes_per_sector > 0 => Ok(bytes_per_sector as usize),
            _ => Ok(FALLBACK_SECTOR_SIZE),
        }
    }

    fn read_unbuffered(&self, path: &Path, sink: &mut dyn FnMut(&[u8])) -> Result<u64> {
        let file_size = std::fs::metadata(path)
            .map_err(|e| io_err(TerminalKind::SourceUnreadable, path, e))?
            .len();

        if file_size == 0 {
            return Ok(0);
        }

        let sector = self.sector_size(path)?.max(1);
        let chunk = READ_CHUNK.div_ceil(sector) * sector;

        let long = self.long_path(path);
        let wide = to_wide(&long);

        // SAFETY: wide 以 NUL 结尾且在调用期间存活。
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_NO_BUFFERING | FILE_FLAG_SEQUENTIAL_SCAN,
                None,
            )
        }
        .map_err(|e| io_err(TerminalKind::SourceUnreadable, path, e))?;
        let handle = OwnedHandle(handle);

        let mut buf = AlignedBuffer::new(chunk, sector)?;
        let mut total: u64 = 0;

        loop {
            let mut read: u32 = 0;
            // SAFETY: 缓冲区按扇区对齐、长度为扇区整数倍，均满足无缓冲读的约束。
            unsafe {
                ReadFile(handle.0, Some(buf.as_mut_slice()), Some(&mut read), None)
            }
            .map_err(|e| io_err(TerminalKind::SourceUnreadable, path, e))?;

            if read == 0 {
                break;
            }

            // 文件尾部：无缓冲读会读满整扇区，多出来的填充字节必须按真实长度截断，
            // 否则算出的哈希会带上垃圾数据，与源哈希永远对不上。
            let remaining = file_size - total;
            let usable = (read as u64).min(remaining) as usize;
            sink(&buf.as_mut_slice()[..usable]);
            total += usable as u64;

            if total >= file_size {
                break;
            }
        }

        if total != file_size {
            return Err(io_err(
                TerminalKind::SourceUnreadable,
                path,
                format!("无缓冲读回字节数不符：读到 {total}，文件大小 {file_size}"),
            ));
        }

        Ok(total)
    }

    fn flush_to_disk(&self, file: &std::fs::File) -> Result<()> {
        let handle = HANDLE(file.as_raw_handle());
        // SAFETY: 句柄来自仍然存活的 File，未被本函数接管所有权。
        unsafe { FlushFileBuffers(handle) }.map_err(|e| {
            CoreError::Retryable(
                crate::error::RetryableKind::CopyIo,
                ErrorContext::new().cause(format!("落盘失败：{e}")),
            )
        })
    }

    fn available_space(&self, path: &Path) -> Result<u64> {
        // 目录可能尚未创建（首次拷贝），向上找到第一个存在的祖先来问
        let mut probe = path.to_path_buf();
        while !probe.exists() {
            match probe.parent() {
                Some(p) if p != probe => probe = p.to_path_buf(),
                _ => break,
            }
        }
        let mut wide = to_wide(&probe);
        // GetDiskFreeSpaceExW 要求目录路径，结尾补分隔符更稳
        if !probe.to_string_lossy().ends_with(['\\', '/']) {
            wide.pop();
            wide.push(u16::from(b'\\'));
            wide.push(0);
        }
        let mut free_to_caller: u64 = 0;
        // SAFETY: wide 以 NUL 结尾且在调用期间存活；输出指针指向本栈上的变量。
        unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR(wide.as_ptr()),
                Some(&mut free_to_caller),
                None,
                None,
            )
        }
        .map_err(|e| io_err(TerminalKind::InvalidConfig, path, format!("查询可用空间失败：{e}")))?;
        Ok(free_to_caller)
    }

    fn long_path(&self, path: &Path) -> PathBuf {
        let s = path.to_string_lossy();
        // 已经带前缀、或是 UNC、或是相对路径，都原样返回。
        if s.starts_with(r"\\?\") || s.starts_with(r"\\.\") || !path.is_absolute() {
            return path.to_path_buf();
        }
        if s.starts_with(r"\\") {
            // UNC：\\server\share → \\?\UNC\server\share
            return PathBuf::from(format!(r"\\?\UNC\{}", &s[2..]));
        }
        PathBuf::from(format!(r"\\?\{s}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{hash_bytes, HashAlgorithm, Hasher};
    use std::io::Write;

    fn write_temp(dir: &tempfile::TempDir, name: &str, data: &[u8]) -> PathBuf {
        let p = dir.path().join(name);
        let mut f = std::fs::File::create(&p).expect("建文件");
        f.write_all(data).expect("写入");
        f.sync_all().expect("落盘");
        p
    }

    #[test]
    fn scenario_copy_engine_sector_size_is_queried_not_hardcoded() {
        let io = WindowsVolumeIo;
        let dir = tempfile::tempdir().expect("临时目录");
        let size = io.sector_size(dir.path()).expect("查扇区大小");
        assert!(size > 0, "扇区大小必须为正");
        assert!(size.is_power_of_two(), "扇区大小应是 2 的幂，实际 {size}");
        assert!(
            (512..=65536).contains(&size),
            "扇区大小超出合理范围：{size}"
        );
    }

    #[test]
    fn scenario_copy_engine_unbuffered_read_matches_content() {
        let io = WindowsVolumeIo;
        let dir = tempfile::tempdir().expect("临时目录");
        // 刻意用非扇区整数倍的大小，覆盖「尾部不满一扇区」的截断逻辑
        let data: Vec<u8> = (0..(1024 * 1024 + 777u32)).map(|i| (i % 251) as u8).collect();
        let p = write_temp(&dir, "big.bin", &data);

        let mut got = Vec::new();
        let n = io
            .read_unbuffered(&p, &mut |chunk| got.extend_from_slice(chunk))
            .expect("无缓冲读");

        assert_eq!(n, data.len() as u64, "读取字节数应等于文件大小");
        assert_eq!(got.len(), data.len(), "尾部填充字节必须被截断");
        assert_eq!(got, data, "无缓冲读到的内容必须与写入完全一致");
    }

    #[test]
    fn scenario_copy_engine_unbuffered_read_hash_matches() {
        let io = WindowsVolumeIo;
        let dir = tempfile::tempdir().expect("临时目录");
        let data = b"steadcopy unbuffered verify".repeat(1000);
        let p = write_temp(&dir, "h.bin", &data);

        let mut hasher = Hasher::new(HashAlgorithm::Xxh64);
        io.read_unbuffered(&p, &mut |c| hasher.update(c))
            .expect("无缓冲读");
        let got = hasher.finish();
        assert!(got.matches(&hash_bytes(HashAlgorithm::Xxh64, &data)));
    }

    #[test]
    fn scenario_copy_engine_unbuffered_read_empty_file() {
        let io = WindowsVolumeIo;
        let dir = tempfile::tempdir().expect("临时目录");
        let p = write_temp(&dir, "empty.bin", b"");
        let mut called = false;
        let n = io
            .read_unbuffered(&p, &mut |_| called = true)
            .expect("零字节文件应正常返回");
        assert_eq!(n, 0);
        assert!(!called, "零字节文件不应产生数据块");
    }

    #[test]
    fn scenario_copy_engine_unbuffered_read_exact_sector_multiple() {
        let io = WindowsVolumeIo;
        let dir = tempfile::tempdir().expect("临时目录");
        let sector = io.sector_size(dir.path()).expect("扇区");
        let data = vec![0xABu8; sector * 3];
        let p = write_temp(&dir, "aligned.bin", &data);
        let mut got = Vec::new();
        io.read_unbuffered(&p, &mut |c| got.extend_from_slice(c))
            .expect("读");
        assert_eq!(got, data);
    }

    #[test]
    fn scenario_copy_engine_unbuffered_read_missing_file_errors() {
        let io = WindowsVolumeIo;
        let dir = tempfile::tempdir().expect("临时目录");
        let err = io
            .read_unbuffered(&dir.path().join("不存在.bin"), &mut |_| {})
            .expect_err("不存在的文件必须报错，MUST NOT 静默返回 0");
        assert!(!err.is_retryable());
    }

    #[test]
    fn scenario_organize_rules_long_path_prefix() {
        let io = WindowsVolumeIo;
        assert_eq!(
            io.long_path(Path::new(r"D:\a\b.txt")),
            PathBuf::from(r"\\?\D:\a\b.txt")
        );
        // 已带前缀的不重复加
        assert_eq!(
            io.long_path(Path::new(r"\\?\D:\a")),
            PathBuf::from(r"\\?\D:\a")
        );
        // UNC 转换
        assert_eq!(
            io.long_path(Path::new(r"\\server\share\x")),
            PathBuf::from(r"\\?\UNC\server\share\x")
        );
        // 相对路径原样
        assert_eq!(io.long_path(Path::new(r"a\b")), PathBuf::from(r"a\b"));
    }

    #[test]
    fn scenario_copy_engine_unbuffered_read_handles_long_path() {
        let io = WindowsVolumeIo;
        let dir = tempfile::tempdir().expect("临时目录");
        // 造一条超过 260 字符的路径
        let mut deep = dir.path().to_path_buf();
        for _ in 0..12 {
            deep.push("一个相当长的目录名用来把路径撑过二百六十个字符的限制");
        }
        std::fs::create_dir_all(&deep).expect("建深层目录");
        let p = deep.join("x.bin");
        std::fs::write(&p, b"deep").expect("写入");
        assert!(
            p.to_string_lossy().chars().count() > 260,
            "路径长度应超过 260"
        );

        let mut got = Vec::new();
        io.read_unbuffered(&p, &mut |c| got.extend_from_slice(c))
            .expect("超长路径应能无缓冲读");
        assert_eq!(got, b"deep");
    }
}
