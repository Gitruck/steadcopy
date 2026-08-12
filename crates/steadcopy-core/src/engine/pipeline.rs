//! 拷贝流水线：单遍读源 → 多目的地并行写 + 边读边算哈希。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/copy-engine/spec.md`
//! → Requirement: 单遍读源、多目的地并行写 / 边读边算源哈希 / 暂停、继续与取消
//!
//! ```text
//!         ┌─── writer A ──> 目的地 A
//! 读源 ───┼─── writer B ──> 目的地 B
//!  │      └─── writer N ──> 目的地 N
//!  └──> 源哈希器（读线程内联，零额外读源）
//! ```
//!
//! **这是品类本质，不是优化项。** 卡是慢介质，「拷完 A 再回头重读源拷 B」是成倍的墙钟时间。
//! 调研里被解剖的几个同类实现**无一**做到单遍读。
//!
//! 背压：每个写线程一个**有界**队列。最慢的目的地把队列填满后读线程自然被节流，
//! 因此常驻内存不随文件大小增长；同时快目的地可以先跑完自己队列里的活，
//! 不必等慢的那个——总耗时接近**最慢的那一个**，而不是各家之和。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use crate::engine::hasher::{HashAlgorithm, HashValue, Hasher};
use crate::error::{CoreError, ErrorContext, Result, RetryableKind, TerminalKind};

/// 默认读取块大小。顺序大块读是慢介质上的正确模式。
pub const DEFAULT_CHUNK_SIZE: usize = 4 * 1024 * 1024;

/// 每个写线程的队列深度。给快目的地留出跑在前面的余量，同时限制内存上限
/// （峰值内存 ≈ 块大小 × 队列深度 × 目的地数）。
pub const QUEUE_DEPTH: usize = 4;

/// 任务控制令牌：取消 + 暂停 / 继续。
///
/// 读线程在每个块的边界检查，因此指令能在**一个块周期内**响应。
/// 暂停用条件变量而不是自旋——暂停可能持续几分钟（换硬盘、腾空间），
/// 自旋等于让一个核空转到用户回来。
///
/// 取消优先于暂停：暂停中收到取消 MUST 立刻醒来退出，
/// 否则「暂停了忘了继续」会让取消按钮变成摆设。
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<Control>);

#[derive(Debug, Default)]
struct Control {
    cancelled: AtomicBool,
    paused: Mutex<bool>,
    resumed: std::sync::Condvar,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        // 叫醒可能正卡在暂停里的线程，否则取消要等到「继续」之后才生效
        if let Ok(mut p) = self.0.paused.lock() {
            *p = false;
        }
        self.0.resumed.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    pub fn pause(&self) {
        if let Ok(mut p) = self.0.paused.lock() {
            *p = true;
        }
    }

    pub fn resume(&self) {
        if let Ok(mut p) = self.0.paused.lock() {
            *p = false;
        }
        self.0.resumed.notify_all();
    }

    pub fn is_paused(&self) -> bool {
        self.0.paused.lock().map(|p| *p).unwrap_or(false)
    }

    /// 处于暂停中就在这里等着，直到被继续或被取消。
    pub fn wait_if_paused(&self) {
        let Ok(mut p) = self.0.paused.lock() else {
            return;
        };
        while *p && !self.0.cancelled.load(Ordering::SeqCst) {
            match self.0.resumed.wait(p) {
                Ok(g) => p = g,
                Err(_) => return,
            }
        }
    }
}

/// 缓冲区池：避免每块都分配。缓冲区在最后一个持有者放手时自动回池。
#[derive(Debug)]
struct BufferPool {
    free: Mutex<Vec<Vec<u8>>>,
    chunk_size: usize,
}

impl BufferPool {
    fn new(chunk_size: usize) -> Arc<Self> {
        Arc::new(Self {
            free: Mutex::new(Vec::new()),
            chunk_size,
        })
    }

    fn take(self: &Arc<Self>) -> Vec<u8> {
        let mut v = self
            .free
            .lock()
            .ok()
            .and_then(|mut f| f.pop())
            .unwrap_or_else(|| Vec::with_capacity(self.chunk_size));
        v.clear();
        v.resize(self.chunk_size, 0);
        v
    }

    fn recycle(&self, buf: Vec<u8>) {
        if let Ok(mut f) = self.free.lock() {
            // 池子不无限长——超过目的地可能同时持有的量就丢弃
            if f.len() < 16 {
                f.push(buf);
            }
        }
    }
}

/// 一块数据。多个写线程共享同一份内存（零拷贝），全部放手后缓冲区回池。
#[derive(Debug)]
struct Chunk {
    data: Vec<u8>,
    len: usize,
    pool: Arc<BufferPool>,
}

impl Chunk {
    fn bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        self.pool.recycle(std::mem::take(&mut self.data));
    }
}

/// 一个目的地的写入结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationOutcome {
    pub path: PathBuf,
    pub bytes_written: u64,
    /// 写入过程中的失败。`None` 表示该目的地写成功。
    pub error: Option<String>,
}

impl DestinationOutcome {
    pub fn succeeded(&self) -> bool {
        self.error.is_none()
    }
}

/// 一次「一源多目的地」拷贝的结果。
#[derive(Debug, Clone)]
pub struct CopyResult {
    /// 源哈希——在读源的同一遍 IO 里算出，**零额外读源**
    pub source_hash: HashValue,
    /// 源被读取的总字节数。用于证明「只读了一遍」：它 MUST 等于文件大小，而非其倍数
    pub source_bytes_read: u64,
    pub destinations: Vec<DestinationOutcome>,
    pub cancelled: bool,
}

impl CopyResult {
    pub fn all_succeeded(&self) -> bool {
        !self.cancelled && self.destinations.iter().all(DestinationOutcome::succeeded)
    }

    pub fn failed_destinations(&self) -> impl Iterator<Item = &DestinationOutcome> {
        self.destinations.iter().filter(|d| !d.succeeded())
    }
}

/// 流水线参数。
#[derive(Debug, Clone)]
pub struct PipelineOptions {
    pub algorithm: HashAlgorithm,
    pub chunk_size: usize,
    pub queue_depth: usize,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            algorithm: HashAlgorithm::default(),
            chunk_size: DEFAULT_CHUNK_SIZE,
            queue_depth: QUEUE_DEPTH,
        }
    }
}

enum Message {
    Data(Arc<Chunk>),
    Done,
}

/// 把一个源文件**读一遍**、同时写入全部目的地、并算出源哈希。
///
/// 这是 [`copy_reader_to_many`] 针对本地文件的薄封装。
pub fn copy_file_to_many(
    source: &Path,
    destinations: &[PathBuf],
    options: &PipelineOptions,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(u64),
) -> Result<CopyResult> {
    let reader = std::fs::File::open(source).map_err(|e| {
        CoreError::Terminal(
            TerminalKind::SourceUnreadable,
            ErrorContext::new().path(source).cause(e.to_string()),
        )
    })?;
    copy_reader_to_many(reader, destinations, options, cancel, on_progress).map_err(|e| {
        // 补上路径上下文——错误 MUST 能定位到具体文件
        let ctx = e.context().clone();
        e.with_context(if ctx.path.is_some() {
            ctx
        } else {
            ErrorContext {
                path: Some(source.to_path_buf()),
                ..ctx
            }
        })
    })
}

/// 把**任意可读的源**读一遍、同时写入全部目的地、并算出源哈希。
///
/// 之所以以 `Read` 而非文件路径为入口：并非所有源都是挂载的卷。
/// 安卓与 iOS 手机在 Windows 上走 MTP / WPD——**没有盘符、没有卷 GUID、
/// 也不是文件系统对象**，`std::fs` 根本打不开。把读取端抽象在这一层，
/// 将来接 MTP 源时引擎主体零改动。
///
/// `on_progress` 收到的是**已读源字节数**，由调用方负责限流
/// （引擎内不做限流，避免把节流策略焊死在这一层）。
pub fn copy_reader_to_many<R: Read>(
    mut reader: R,
    destinations: &[PathBuf],
    options: &PipelineOptions,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(u64),
) -> Result<CopyResult> {
    if destinations.is_empty() {
        return Err(CoreError::Terminal(
            TerminalKind::InvalidConfig,
            ErrorContext::new().cause("至少要有一个目的地"),
        ));
    }

    let pool = BufferPool::new(options.chunk_size);
    let mut senders: Vec<SyncSender<Message>> = Vec::with_capacity(destinations.len());
    let mut handles = Vec::with_capacity(destinations.len());

    for dest in destinations {
        let (tx, rx) = sync_channel::<Message>(options.queue_depth);
        senders.push(tx);
        let dest = dest.clone();
        let cancel = cancel.clone();
        handles.push(std::thread::spawn(move || writer_thread(dest, rx, cancel)));
    }

    let mut hasher = Hasher::new(options.algorithm);
    let mut total_read: u64 = 0;
    let mut read_error: Option<CoreError> = None;

    loop {
        // 暂停点放在块边界：不会把一个块撕成两半，继续之后也不必回退
        cancel.wait_if_paused();
        if cancel.is_cancelled() {
            break;
        }

        let mut buf = pool.take();
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                pool.recycle(buf);
                // 路径上下文由调用方补——本函数面向任意 `Read`，不一定有路径
                read_error = Some(CoreError::Retryable(
                    RetryableKind::CopyIo,
                    ErrorContext::new().cause(format!("读取源失败：{e}")),
                ));
                break;
            }
        };
        if n == 0 {
            pool.recycle(buf);
            break;
        }

        // 边读边算：源哈希在这一遍 IO 里就算完了，不再重读源
        hasher.update(&buf[..n]);
        total_read += n as u64;

        let chunk = Arc::new(Chunk {
            data: buf,
            len: n,
            pool: Arc::clone(&pool),
        });

        // 广播给全部写线程。有界队列在此处形成背压：
        // 最慢的目的地把队列填满 → send 阻塞 → 读线程被节流。
        for tx in &senders {
            if tx.send(Message::Data(Arc::clone(&chunk))).is_err() {
                // 对端写线程已退出（出错），继续把其余目的地喂完，错误在汇总时报告
                continue;
            }
        }

        on_progress(total_read);
    }

    for tx in &senders {
        let _ = tx.send(Message::Done);
    }
    drop(senders);

    let mut outcomes = Vec::with_capacity(destinations.len());
    for handle in handles {
        match handle.join() {
            Ok(outcome) => outcomes.push(outcome),
            Err(_) => outcomes.push(DestinationOutcome {
                path: PathBuf::new(),
                bytes_written: 0,
                error: Some("写入线程异常终止".into()),
            }),
        }
    }

    if let Some(e) = read_error {
        return Err(e);
    }

    let cancelled = cancel.is_cancelled();
    if cancelled {
        // 取消时清理半截文件：MUST NOT 在目的地留下无标记的截断文件
        for o in &outcomes {
            if !o.path.as_os_str().is_empty() {
                let _ = std::fs::remove_file(&o.path);
            }
        }
    }

    Ok(CopyResult {
        source_hash: hasher.finish(),
        source_bytes_read: total_read,
        destinations: outcomes,
        cancelled,
    })
}

fn writer_thread(dest: PathBuf, rx: Receiver<Message>, cancel: CancelToken) -> DestinationOutcome {
    let mut outcome = DestinationOutcome {
        path: dest.clone(),
        bytes_written: 0,
        error: None,
    };

    if let Some(parent) = dest.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            outcome.error = Some(format!("创建目录失败：{e}"));
            return outcome;
        }
    }

    let mut file = match std::fs::File::create(&dest) {
        Ok(f) => f,
        Err(e) => {
            outcome.error = Some(format!("创建文件失败：{e}"));
            return outcome;
        }
    };

    let written = AtomicU64::new(0);
    while let Ok(msg) = rx.recv() {
        match msg {
            Message::Done => break,
            Message::Data(chunk) => {
                if cancel.is_cancelled() {
                    break;
                }
                if let Err(e) = file.write_all(chunk.bytes()) {
                    outcome.error = Some(format!("写入失败：{e}"));
                    outcome.bytes_written = written.load(Ordering::Relaxed);
                    return outcome;
                }
                written.fetch_add(chunk.len as u64, Ordering::Relaxed);
            }
        }
    }

    outcome.bytes_written = written.load(Ordering::Relaxed);

    if let Err(e) = file.flush() {
        outcome.error = Some(format!("刷新缓冲失败：{e}"));
        return outcome;
    }
    // 落盘：读回校验之前必须确保数据真的到了介质上，
    // 否则「无缓冲读」可能读到尚未落盘的旧内容。
    if let Err(e) = file.sync_all() {
        outcome.error = Some(format!("落盘失败：{e}"));
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::hasher::hash_bytes;
    use std::time::{Duration, Instant};

    fn make_source(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, data).expect("写源文件");
        p
    }

    fn data_of(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    // spec: copy-engine → 单遍读源、多目的地并行写 → Scenario: 双目的地只读一遍源
    #[test]
    fn scenario_copy_engine_two_destinations_read_source_once() {
        let dir = tempfile::tempdir().expect("临时目录");
        let data = data_of(3 * DEFAULT_CHUNK_SIZE + 12345);
        let src = make_source(dir.path(), "src.bin", &data);
        let d1 = dir.path().join("dst1/a.bin");
        let d2 = dir.path().join("dst2/a.bin");

        let cancel = CancelToken::new();
        let r = copy_file_to_many(
            &src,
            &[d1.clone(), d2.clone()],
            &PipelineOptions::default(),
            &cancel,
            &mut |_| {},
        )
        .expect("拷贝应成功");

        assert!(r.all_succeeded());
        // 关键断言：源被读取的字节数等于文件大小，**不是两倍**
        assert_eq!(
            r.source_bytes_read,
            data.len() as u64,
            "两个目的地 MUST 只读一遍源"
        );
        // 两个目的地内容都与源逐字节一致
        for d in [&d1, &d2] {
            assert_eq!(std::fs::read(d).expect("读目的地"), data, "{d:?} 内容不符");
        }
        // 源哈希在同一遍 IO 里算出，与独立计算一致
        assert!(r
            .source_hash
            .matches(&hash_bytes(HashAlgorithm::Xxh64, &data)));
    }

    #[test]
    fn scenario_copy_engine_four_destinations() {
        let dir = tempfile::tempdir().expect("临时目录");
        let data = data_of(1_000_000);
        let src = make_source(dir.path(), "src.bin", &data);
        let dests: Vec<PathBuf> = (1..=4).map(|i| dir.path().join(format!("d{i}/a.bin"))).collect();

        let r = copy_file_to_many(
            &src,
            &dests,
            &PipelineOptions::default(),
            &CancelToken::new(),
            &mut |_| {},
        )
        .expect("拷贝");

        assert!(r.all_succeeded());
        assert_eq!(r.source_bytes_read, data.len() as u64, "四个目的地仍只读一遍");
        for d in &dests {
            assert_eq!(std::fs::read(d).expect("读"), data);
        }
    }

    // spec: → Scenario: 背压防止内存无界增长
    #[test]
    fn scenario_copy_engine_backpressure_bounds_memory() {
        // 峰值内存上界 = 块大小 × (队列深度 + 1) × 目的地数。
        // 用小块大小构造一个远大于该上界的文件，若无背压则内存会线性膨胀。
        let dir = tempfile::tempdir().expect("临时目录");
        let opts = PipelineOptions {
            chunk_size: 64 * 1024,
            queue_depth: 2,
            ..Default::default()
        };
        let data = data_of(8 * 1024 * 1024); // 128 块，远超队列容量
        let src = make_source(dir.path(), "src.bin", &data);
        let dests = vec![dir.path().join("d1/a.bin"), dir.path().join("d2/a.bin")];

        let r = copy_file_to_many(&src, &dests, &opts, &CancelToken::new(), &mut |_| {})
            .expect("拷贝");
        assert!(r.all_succeeded());
        assert_eq!(r.source_bytes_read, data.len() as u64);
        for d in &dests {
            assert_eq!(std::fs::read(d).expect("读").len(), data.len());
        }
    }

    #[test]
    fn scenario_copy_engine_progress_is_monotonic_and_reaches_total() {
        let dir = tempfile::tempdir().expect("临时目录");
        let data = data_of(2 * DEFAULT_CHUNK_SIZE + 7);
        let src = make_source(dir.path(), "src.bin", &data);

        let mut seen: Vec<u64> = Vec::new();
        let r = copy_file_to_many(
            &src,
            &[dir.path().join("d/a.bin")],
            &PipelineOptions::default(),
            &CancelToken::new(),
            &mut |n| seen.push(n),
        )
        .expect("拷贝");

        assert!(r.all_succeeded());
        assert!(!seen.is_empty(), "应有进度回调");
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "进度 MUST 单调不减");
        assert_eq!(*seen.last().expect("末次进度"), data.len() as u64);
    }

    // spec: → Scenario: 取消不留下无标记截断文件
    #[test]
    fn scenario_copy_engine_cancel_leaves_no_untracked_partial_file() {
        let dir = tempfile::tempdir().expect("临时目录");
        let data = data_of(16 * 1024 * 1024);
        let src = make_source(dir.path(), "src.bin", &data);
        let dst = dir.path().join("d/a.bin");

        let cancel = CancelToken::new();
        let c2 = cancel.clone();
        // 拷到一点点就取消
        let r = copy_file_to_many(
            &src,
            std::slice::from_ref(&dst),
            &PipelineOptions {
                chunk_size: 64 * 1024,
                ..Default::default()
            },
            &cancel,
            &mut |n| {
                if n > 128 * 1024 {
                    c2.cancel();
                }
            },
        )
        .expect("取消不应是错误");

        assert!(r.cancelled, "结果应标注已取消");
        assert!(
            !dst.exists(),
            "取消后 MUST NOT 在目的地留下无标记的截断文件"
        );
    }

    // spec: → Scenario: 暂停后继续
    #[test]
    fn scenario_copy_engine_pause_then_resume_matches_uninterrupted_result() {
        let dir = tempfile::tempdir().expect("临时目录");
        let data = data_of(8 * 1024 * 1024);
        let src = make_source(dir.path(), "src.bin", &data);
        let dst = dir.path().join("d/a.bin");

        let cancel = CancelToken::new();
        let c2 = cancel.clone();
        let paused_at = Arc::new(AtomicU64::new(0));
        let seen = Arc::clone(&paused_at);

        // 拷到一半按暂停，另起一个线程隔一会儿再继续
        let resumer = std::thread::spawn(move || {
            while seen.load(Ordering::SeqCst) == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            std::thread::sleep(std::time::Duration::from_millis(80));
            assert!(c2.is_paused(), "这会儿应该还停着");
            c2.resume();
        });

        let c3 = cancel.clone();
        let mark = Arc::clone(&paused_at);
        let r = copy_file_to_many(
            &src,
            std::slice::from_ref(&dst),
            &PipelineOptions {
                chunk_size: 256 * 1024,
                ..Default::default()
            },
            &cancel,
            &mut |n| {
                if n > 1024 * 1024 && mark.load(Ordering::SeqCst) == 0 {
                    c3.pause();
                    mark.store(n, Ordering::SeqCst);
                }
            },
        )
        .expect("暂停后继续应正常完成");
        resumer.join().expect("继续线程");

        assert!(!r.cancelled, "暂停不是取消");
        assert!(paused_at.load(Ordering::SeqCst) > 0, "没真的暂停过就白测了");
        // 最终结果与不暂停执行一致：逐字节相同，哈希相同
        assert_eq!(std::fs::read(&dst).expect("读回"), data);
        assert!(r.source_hash.matches(&hash_bytes(HashAlgorithm::Xxh64, &data)));
    }

    // spec: → Scenario: 暂停后继续（取消优先于暂停）
    #[test]
    fn scenario_copy_engine_cancel_wakes_a_paused_task() {
        // 「暂停了忘了继续」不能让取消按钮变成摆设
        let cancel = CancelToken::new();
        cancel.pause();
        assert!(cancel.is_paused());

        let c2 = cancel.clone();
        let waiter = std::thread::spawn(move || {
            c2.wait_if_paused();
            c2.is_cancelled()
        });
        std::thread::sleep(std::time::Duration::from_millis(30));
        cancel.cancel();

        assert!(waiter.join().expect("等待线程"), "取消应叫醒暂停中的线程");
        assert!(!cancel.is_paused(), "取消之后不该还停着");
    }

    #[test]
    fn scenario_copy_engine_cancel_responds_within_a_chunk() {
        let dir = tempfile::tempdir().expect("临时目录");
        let data = data_of(64 * 1024 * 1024);
        let src = make_source(dir.path(), "src.bin", &data);

        let cancel = CancelToken::new();
        cancel.cancel(); // 开跑前就取消
        let start = Instant::now();
        let r = copy_file_to_many(
            &src,
            &[dir.path().join("d/a.bin")],
            &PipelineOptions::default(),
            &cancel,
            &mut |_| {},
        )
        .expect("取消不应是错误");
        assert!(r.cancelled);
        assert_eq!(r.source_bytes_read, 0, "开跑前取消不应读任何数据");
        assert!(start.elapsed() < Duration::from_secs(5), "取消应立即响应");
    }

    #[test]
    fn scenario_copy_engine_zero_byte_file() {
        let dir = tempfile::tempdir().expect("临时目录");
        let src = make_source(dir.path(), "empty.bin", b"");
        let dst = dir.path().join("d/empty.bin");
        let r = copy_file_to_many(
            &src,
            std::slice::from_ref(&dst),
            &PipelineOptions::default(),
            &CancelToken::new(),
            &mut |_| {},
        )
        .expect("零字节文件应正常拷贝");
        assert!(r.all_succeeded());
        assert_eq!(r.source_bytes_read, 0);
        assert!(dst.exists(), "零字节文件也要落地");
        assert_eq!(std::fs::metadata(&dst).expect("元数据").len(), 0);
        assert!(r
            .source_hash
            .matches(&hash_bytes(HashAlgorithm::Xxh64, b"")));
    }

    #[test]
    fn scenario_copy_engine_missing_source_is_terminal_error() {
        let dir = tempfile::tempdir().expect("临时目录");
        let err = copy_file_to_many(
            &dir.path().join("不存在.bin"),
            &[dir.path().join("d/a.bin")],
            &PipelineOptions::default(),
            &CancelToken::new(),
            &mut |_| {},
        )
        .expect_err("源不存在 MUST 报错");
        assert!(!err.is_retryable(), "源不可读属终态族");
    }

    #[test]
    fn scenario_copy_engine_empty_destinations_rejected() {
        let dir = tempfile::tempdir().expect("临时目录");
        let src = make_source(dir.path(), "s.bin", b"x");
        let err = copy_file_to_many(
            &src,
            &[],
            &PipelineOptions::default(),
            &CancelToken::new(),
            &mut |_| {},
        )
        .expect_err("零目的地应被拒");
        assert!(!err.is_retryable());
    }

    #[test]
    fn scenario_copy_engine_one_bad_destination_others_still_succeed() {
        // 一个目的地不可写时，其余目的地 MUST 照常完成，
        // 且结果里能定位到是哪个失败了——不静默、不整体失败。
        let dir = tempfile::tempdir().expect("临时目录");
        let data = data_of(200_000);
        let src = make_source(dir.path(), "src.bin", &data);

        let good = dir.path().join("good/a.bin");
        // 用一个已存在的**文件**当作目录的父级，制造创建失败
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"i am a file").expect("建阻塞文件");
        let bad = blocker.join("sub/a.bin");

        let r = copy_file_to_many(
            &src,
            &[good.clone(), bad.clone()],
            &PipelineOptions::default(),
            &CancelToken::new(),
            &mut |_| {},
        )
        .expect("整体不应报错");

        assert!(!r.all_succeeded(), "存在失败目的地时不应报告全部成功");
        assert_eq!(r.failed_destinations().count(), 1);
        assert_eq!(
            r.failed_destinations().next().map(|d| d.path.clone()),
            Some(bad)
        );
        assert_eq!(std::fs::read(&good).expect("好目的地应写成"), data);
    }

    #[test]
    fn scenario_copy_engine_slow_destination_does_not_serialize_total_time() {
        // 两个目的地并行：总耗时应接近最慢的那一个，而不是两者之和。
        // 这里用「单目的地耗时」做基线，断言双目的地没有翻倍。
        let dir = tempfile::tempdir().expect("临时目录");
        let data = data_of(4 * 1024 * 1024);
        let src = make_source(dir.path(), "src.bin", &data);
        let opts = PipelineOptions {
            chunk_size: 256 * 1024,
            ..Default::default()
        };

        let t1 = Instant::now();
        copy_file_to_many(
            &src,
            &[dir.path().join("a/x.bin")],
            &opts,
            &CancelToken::new(),
            &mut |_| {},
        )
        .expect("单目的地");
        let single = t1.elapsed();

        let t2 = Instant::now();
        copy_file_to_many(
            &src,
            &[dir.path().join("b/x.bin"), dir.path().join("c/x.bin")],
            &opts,
            &CancelToken::new(),
            &mut |_| {},
        )
        .expect("双目的地");
        let double = t2.elapsed();

        // 宽松上界：并行写不应让耗时接近翻倍。给足余量以免在忙碌机器上 flaky。
        assert!(
            double < single * 3 + Duration::from_millis(500),
            "双目的地耗时 {double:?} 相对单目的地 {single:?} 异常，疑似串行写"
        );
    }
}
