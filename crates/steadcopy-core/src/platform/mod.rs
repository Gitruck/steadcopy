//! 平台能力边界。
//!
//! 规范：`openspec/changes/add-steadcopy-core/design.md` §3
//!
//! 业务核只依赖本模块的 trait，Windows 实现在 `windows` 子模块，
//! 其余平台返回 `TerminalKind::Unsupported`——**编译得过、跑不了**，
//! 这样 mac 版启动时能给出明确的「本平台尚未支持」而不是崩溃（P5 决策：架构留门）。
//!
//! `Clock` 单列一个 trait 不是为了跨平台，是为了**测试可控时间**（倒计时、退避、超时）。
//! 它与 `DeviceWatcher` 是 TDD 纪律 T3 允许的仅有两类替身。

use std::path::{Path, PathBuf};
use std::time::Duration;

use time::OffsetDateTime;

use crate::error::Result;

#[cfg(windows)]
pub mod windows;

/// 卷级 IO：无缓冲读、扇区大小查询、落盘、长路径归一。
pub trait VolumeIo: Send + Sync {
    /// 查询该路径所在卷的扇区大小。
    ///
    /// **MUST NOT 硬编码 512**——4Kn 盘是 4096。查询不到时返回一个安全的保守值
    /// （4096 是常见扇区大小的公倍数，按它对齐对 512 字节扇区同样合法）。
    fn sector_size(&self, path: &Path) -> Result<usize>;

    /// **绕过操作系统页缓存**读取整个文件，分块喂给 `sink`。
    ///
    /// 这是校验有效性的根基：不绕过缓存，读回的可能是刚写入时留在内存里的副本，
    /// 介质写坏完全测不出来——那样的校验比不校验更危险，因为它给用户假的确定性。
    ///
    /// 实现 MUST 处理扇区对齐（缓冲区地址、读取长度、文件偏移三者），
    /// 并把文件尾部不足一扇区的部分**按真实长度截断**后再喂给 `sink`。
    ///
    /// 返回实际读取的字节数（等于文件大小）。
    fn read_unbuffered(&self, path: &Path, sink: &mut dyn FnMut(&[u8])) -> Result<u64>;

    /// 把已写入的数据真正落到介质上。
    ///
    /// 读回校验之前 MUST 调用，否则「无缓冲读」可能读到尚未落盘的旧内容。
    fn flush_to_disk(&self, file: &std::fs::File) -> Result<()>;

    /// 长路径归一（Windows 上加 `\\?\` 前缀绕开 260 限制）。
    fn long_path(&self, path: &Path) -> PathBuf;
}

/// 时钟。抽出来是为了让倒计时与退避可以在测试里被精确控制，
/// 而不是靠真实 sleep 把测试拖慢、拖成 flaky。
pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
    fn sleep(&self, duration: Duration);
}

/// 系统时钟。
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// 取本平台的 `VolumeIo` 实现。
pub fn volume_io() -> Box<dyn VolumeIo> {
    #[cfg(windows)]
    {
        Box::new(windows::WindowsVolumeIo)
    }
    #[cfg(not(windows))]
    {
        Box::new(UnsupportedVolumeIo)
    }
}

/// 非 Windows 平台的空壳。**刻意不做「退化为带缓冲读」的降级**——
/// 那会让校验静默失去意义，违背「绝不静默降级」的铁律。
#[cfg(not(windows))]
#[derive(Debug, Clone, Copy, Default)]
pub struct UnsupportedVolumeIo;

#[cfg(not(windows))]
impl VolumeIo for UnsupportedVolumeIo {
    fn sector_size(&self, _path: &Path) -> Result<usize> {
        Err(crate::error::CoreError::terminal(
            crate::error::TerminalKind::Unsupported,
        ))
    }

    fn read_unbuffered(&self, _path: &Path, _sink: &mut dyn FnMut(&[u8])) -> Result<u64> {
        Err(crate::error::CoreError::terminal(
            crate::error::TerminalKind::Unsupported,
        ))
    }

    fn flush_to_disk(&self, _file: &std::fs::File) -> Result<()> {
        Err(crate::error::CoreError::terminal(
            crate::error::TerminalKind::Unsupported,
        ))
    }

    fn long_path(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }
}

/// 保守的扇区大小回退值。
///
/// 4096 是 512 与 4096 两种常见扇区大小的公倍数，按它对齐在两种盘上都合法。
pub const FALLBACK_SECTOR_SIZE: usize = 4096;
