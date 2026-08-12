//! 稳拷 steadcopy 命令行驱动面。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/cli-driver/spec.md`
//!
//! 定位：**E2E 测试的唯一驱动面**，同时是自动化入口。
//! 与 GUI 消费同一套 core 门面——CLI 层零业务逻辑。

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

/// 一句需要插值的双语文案。
///
/// 中英语序差别很大（「X 的 Y 不足」vs "Not enough Y on X"），所以**整句各写一遍**，
/// 绝不拼片段——拼片段等于把语序知识散进调用点，改一句话要改两处。
///
/// 展开是对 `Locale` 的穷尽 `match`：新增一种语言，编译器会把每一处点出来。
/// 不带插值的短语用 [`output::w`]。
macro_rules! wf {
    ($zh:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        match $crate::output::lang() {
            steadcopy_core::i18n::Locale::Zh => format!($zh $(, $arg)*),
            steadcopy_core::i18n::Locale::En => format!($en $(, $arg)*),
        }
    };
}

mod format;
mod output;
mod setup;
mod watch;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use steadcopy_core::engine::{CancelToken, HashAlgorithm};
use steadcopy_core::error::{CoreError, TerminalKind};
use steadcopy_core::manifest::model::SourceRef;
use steadcopy_core::manifest::{audit, read_manifest, ObservedFile};
use steadcopy_core::organize::{scan_source, PathTemplate, ScanOptions};
use steadcopy_core::platform::{volume_io, Clock, SystemClock};
use steadcopy_core::ledger::{write_report, ReportInput};
use steadcopy_core::task::{plan_task, run_task, DestinationSpec, TaskSpec};

use output::{human_bytes, Emitter, ExitKind};

// 这一屏（`--help`）**恒为中文**，这是个取舍不是漏译。
//
// clap 的帮助文本由属性宏在编译期定死，运行期换不了语言——真要双语，
// 只能编译两份二进制，或者自己接管整套 help 渲染。两条都不值得为一屏文字付。
// 所以在 `about` 里明说：`--lang en` 管的是**运行期输出**（结论、报告、错误），
// 不管这一屏。用户看到英文结论而帮助是中文时，至少知道这是设计。
//
// 注意：这里用 `//` 而不是 `///`——doc 注释会被 clap 当成 `long_about` 印到
// 用户屏幕上，内部取舍说明不该出现在那儿。
#[derive(Parser)]
#[command(
    name = "steadcopy",
    about = "稳拷 · 插卡自动备份、双端校验、拷完给一张人话报告\n\
             （本帮助恒为中文；--lang en 只切换运行时输出与报告的语言）",
    version
)]
struct Cli {
    /// 机读输出：stdout 只出 JSON，人读日志走 stderr
    #[arg(long, global = true)]
    json: bool,

    /// 运行时输出用哪种语言。auto = 跟随配置里的 locale，配置也是 auto 则跟随系统
    #[arg(
        long,
        global = true,
        default_value = steadcopy_core::i18n::LOCALE_AUTO,
        value_parser = [
            steadcopy_core::i18n::LOCALE_AUTO,
            steadcopy_core::i18n::LOCALE_ZH,
            steadcopy_core::i18n::LOCALE_EN,
        ],
    )]
    lang: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 列出本机卷，标注哪些可作为拷贝源
    Devices,
    /// 守候插卡：认出设备 → 匹配预设 → 规划 → 确认后开跑
    Watch {
        /// 处理完第一个设备就退出（便于脚本与验收）
        #[arg(long)]
        once: bool,
        /// 跳过确认直接开跑（等同于本次进入无人值守）
        #[arg(long)]
        yes: bool,
        /// 测试专用：把一个普通目录当作到达的源设备（不绕过任何拷贝与校验逻辑）
        #[arg(long, hide = true)]
        simulate: Option<PathBuf>,
    },
    /// 查看配置文件位置与内容
    Config {
        #[command(subcommand)]
        action: setup::ConfigAction,
    },
    /// 项目管理
    Project {
        #[command(subcommand)]
        action: setup::ProjectAction,
    },
    /// 预设任务管理
    Preset {
        #[command(subcommand)]
        action: setup::PresetAction,
    },
    /// 设备记忆库管理（指认类型、改名、忽略）
    Device {
        #[command(subcommand)]
        action: setup::DeviceAction,
    },
    /// 扫描源，输出素材统计与将纳入的文件集合
    Scan {
        /// 源目录（读卡器盘符根或任意目录）
        source: PathBuf,
        /// 列出每个文件（默认只给统计）
        #[arg(long)]
        list: bool,
    },
    /// 输出任务计划而不执行：落地路径、增量集合、空间预检结论
    Plan(TaskArgs),
    /// 执行拷贝任务
    Copy(TaskArgs),
    /// 复验一份清单：产出一致 / 已移动 / 丢失 / 新增四态结果
    Audit {
        /// 清单文件路径（.json）
        manifest: PathBuf,
        /// 被复验的目录。省略时取清单所在目录的上一级
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// ⚠️ 格式化源卡。默认拒绝执行，需显式危险参数
    Format {
        /// 目标卷（盘符如 E: 或卷 GUID）
        target: String,
        /// 显式危险确认。不给就直接拒绝
        #[arg(long = "yes-i-know-this-erases-data")]
        confirmed: bool,
        /// 覆盖倒计时秒数（最小 10）
        #[arg(long)]
        countdown: Option<u32>,
    },
    /// 安全弹出一个卷（锁定 → 卸载 → 弹出，不依赖任何外部程序）
    Eject {
        /// 目标卷（盘符如 E: 或卷 GUID）
        target: String,
    },
    /// 由一份清单生成 HTML 报告（单文件、可离线打开、可打印为 PDF）
    Report {
        /// 清单文件路径（.json）
        manifest: PathBuf,
        /// 输出路径。省略时与清单同目录、同名 .html
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

/// 这里的中文缺省值**不随 `--lang` 变**，而且不能变。
///
/// 项目名与设备名会被路径模板的 `{项目}` `{设备}` 渲染进目录名，模板本身也是配置字面量。
/// 跟着界面语言换，同一条命令在中英两种环境下就会把素材落到不同目录、
/// 续传账本也对不上——那是判定随 locale 变，铁律不允许。
/// 它们是**数据**，不是文案。
#[derive(Args, Clone)]
struct TaskArgs {
    /// 源目录
    source: PathBuf,
    /// 目的地根目录，可重复给出（1..4 个）
    #[arg(short, long = "dest", required = true)]
    dests: Vec<PathBuf>,
    /// 项目名
    #[arg(short, long, default_value = "未命名项目")]
    project: String,
    /// 源设备显示名
    #[arg(long, default_value = "存储卡")]
    device: String,
    /// 源设备身份标识（续传账本的作用域键）
    #[arg(long)]
    source_id: Option<String>,
    /// 落地路径模板
    #[arg(long, default_value = "{项目}/{日期}/{设备}")]
    template: String,
    /// 关闭读回校验（**不推荐**：关掉就发现不了介质写入错误）
    #[arg(long)]
    no_verify: bool,
    /// 校验算法
    #[arg(long, default_value = "xxh64", value_parser = ["xxh64", "md5"])]
    algorithm: String,
    /// 校验失败后的重拷次数上限
    #[arg(long, default_value_t = 2)]
    retries: u32,
}

impl TaskArgs {
    fn to_spec(&self) -> Result<TaskSpec, String> {
        if self.dests.is_empty() || self.dests.len() > 4 {
            return Err(wf!(
                "目的地数量应为 1..4 个，实际 {} 个",
                "Expected 1 to 4 destinations, got {}",
                self.dests.len()
            ));
        }
        // 模板错的成句在 core（界面与命令行共用同一句），这里只负责取本次的语言
        let template =
            PathTemplate::parse(&self.template).map_err(|e| e.describe(output::lang()))?;
        let algorithm = match self.algorithm.as_str() {
            "md5" => HashAlgorithm::Md5,
            _ => HashAlgorithm::Xxh64,
        };
        // 身份标识缺省时由源路径派生，保证同一路径的多次运行能续上账本
        let source_id = self
            .source_id
            .clone()
            .unwrap_or_else(|| format!("path:{}", self.source.display()));

        Ok(TaskSpec {
            source_root: self.source.clone(),
            source: SourceRef {
                id: source_id,
                display_name: self.device.clone(),
            },
            project: self.project.clone(),
            destinations: self
                .dests
                .iter()
                .map(|d| DestinationSpec {
                    root: d.clone(),
                    template: template.clone(),
                    enabled: true,
                })
                .collect(),
            algorithm,
            verify: !self.no_verify,
            // 命令行不自动弹卡：脚本调用时把卡弹掉是意外副作用，
            // 要弹就显式跑 `steadcopy eject`
            eject_after: false,
            scan: ScanOptions::mirror(),
            retries: self.retries,
            at: SystemClock.now(),
        })
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // 语言在这里定一次，之后只读——命令行是短命进程，不需要也不该有可变的全局语言
    output::set_lang(&cli.lang);
    let mut out = Emitter::new(cli.json);
    let kind = match run(&cli, &mut out) {
        Ok(k) => k,
        Err(e) => {
            out.error(&e);
            ExitKind::Terminal
        }
    };
    ExitCode::from(kind as u8)
}

fn run(cli: &Cli, out: &mut Emitter) -> Result<ExitKind, String> {
    match &cli.command {
        Command::Devices => cmd_devices(out),
        Command::Watch {
            once,
            yes,
            simulate,
        } => watch::run(out, *once, *yes, simulate.as_deref()),
        Command::Config { action } => setup::config_cmd(action, out),
        Command::Project { action } => setup::project_cmd(action, out),
        Command::Preset { action } => setup::preset_cmd(action, out),
        Command::Device { action } => setup::device_cmd(action, out),
        Command::Scan { source, list } => cmd_scan(source, *list, out),
        Command::Plan(args) => cmd_plan(args, out),
        Command::Copy(args) => cmd_copy(args, out),
        Command::Audit { manifest, dir } => cmd_audit(manifest, dir.as_deref(), out),
        Command::Format {
            target,
            confirmed,
            countdown,
        } => format::run(out, target, *confirmed, *countdown),
        Command::Eject { target } => cmd_eject(target, out),
        Command::Report { manifest, output } => cmd_report(manifest, output.as_deref(), out),
    }
}

/// 安全弹出。命令行版没有「任务进行中」的概念（每次调用是独立进程），
/// 所以准入判据这里传空——真正的互斥由系统的卷锁定负责：
/// 有别的进程开着卡上的文件时，FSCTL_LOCK_VOLUME 会失败。
fn cmd_eject(target: &str, out: &mut Emitter) -> Result<ExitKind, String> {
    let lang = output::lang();
    let vols = steadcopy_core::device::enumerate_volumes().map_err(|e| e.describe(lang))?;
    let vol = vols
        .into_iter()
        .find(|v| {
            v.guid_path.eq_ignore_ascii_case(target)
                || v.drive_letter.as_deref().map(str::to_ascii_uppercase)
                    == Some(target.to_ascii_uppercase())
        })
        .ok_or_else(|| wf!("找不到这个卷：{}", "No such volume: {}", target))?;

    steadcopy_core::device::can_eject(&vol.composite_id(), &[]).map_err(|e| e.describe(lang))?;
    steadcopy_core::device::ejector()
        .eject(&vol.root_path())
        .map_err(|e| e.describe(lang))?;
    out.note(&wf!(
        "{} 已安全弹出，可以拔了",
        "{} was ejected safely — you can unplug it now",
        vol.display_name()
    ));
    Ok(ExitKind::Ok)
}

/// 由清单生成 HTML 报告，返回落地路径。
fn write_html_report(
    manifest_path: &Path,
    m: &steadcopy_core::manifest::Manifest,
    failures: &[(String, String, u32)],
    skipped: usize,
    notices: &[String],
    elapsed_secs: Option<u64>,
    output: Option<&Path>,
) -> Result<PathBuf, String> {
    let target = match output {
        Some(o) => o.to_path_buf(),
        None => manifest_path.with_extension("html"),
    };
    let input = ReportInput {
        manifest: m,
        failures,
        skipped,
        notices,
        elapsed_secs,
        generated_at: SystemClock.now(),
        audit: None,
        lang: output::lang(),
    };
    // io::Error 的文字由系统给，本身不受我们控制；外面这句是我们的，跟随语言
    write_report(&target, &input)
        .map_err(|e| wf!("报告写入失败：{}", "Could not write the report: {}", e))?;
    Ok(target)
}

fn cmd_report(
    manifest_path: &Path,
    output_path: Option<&Path>,
    out: &mut Emitter,
) -> Result<ExitKind, String> {
    let m = read_manifest(manifest_path).map_err(|e| e.describe(output::lang()))?;
    let p = write_html_report(manifest_path, &m, &[], 0, &[], None, output_path)?;
    out.report_written(&p);
    Ok(ExitKind::Ok)
}

fn cmd_devices(out: &mut Emitter) -> Result<ExitKind, String> {
    let vols =
        steadcopy_core::device::enumerate_volumes().map_err(|e| e.describe(output::lang()))?;
    out.devices(&vols);
    Ok(ExitKind::Ok)
}

fn cmd_scan(source: &Path, list: bool, out: &mut Emitter) -> Result<ExitKind, String> {
    if !source.exists() {
        return Err(wf!(
            "源目录不存在：{}",
            "The source directory does not exist: {}",
            source.display()
        ));
    }
    let r = scan_source(source, &ScanOptions::mirror());
    out.scan(&r, list);
    // 源上没素材是「终态族」结果：重跑一样，只需告知
    Ok(if r.file_count() == 0 {
        ExitKind::Terminal
    } else {
        ExitKind::Ok
    })
}

fn cmd_plan(args: &TaskArgs, out: &mut Emitter) -> Result<ExitKind, String> {
    let spec = args.to_spec()?;
    let io = volume_io();
    let plan = plan_task(&spec, io.as_ref()).map_err(|e| e.describe(output::lang()))?;
    out.plan(&plan);
    Ok(if plan.insufficient().count() > 0 {
        ExitKind::Terminal
    } else {
        ExitKind::Ok
    })
}

fn cmd_copy(args: &TaskArgs, out: &mut Emitter) -> Result<ExitKind, String> {
    let lang = output::lang();
    let spec = args.to_spec()?;
    let io = volume_io();
    let plan = plan_task(&spec, io.as_ref()).map_err(|e| e.describe(lang))?;

    // 这两句 core 已经产过成句（界面也用同一句），命令行不再重写一份
    if plan.is_no_source() {
        out.finished_empty(
            "no_source",
            &CoreError::terminal(TerminalKind::NoSource).describe(lang),
        );
        return Ok(ExitKind::Terminal);
    }
    if let Some(d) = plan.insufficient().next() {
        let unknown = output::w("未知", "unknown");
        return Err(wf!(
            "目的地空间不足：{} 需要 {}，可用 {}，还差 {}",
            "Not enough space at {}: needs {}, has {}, short by {}",
            d.landing_dir.display(),
            human_bytes(d.required_bytes),
            d.available_bytes
                .map(human_bytes)
                .unwrap_or_else(|| unknown.into()),
            d.shortfall()
                .map(human_bytes)
                .unwrap_or_else(|| unknown.into()),
        ));
    }
    if plan.is_no_new_source() {
        // 「无新素材」是正常结果不是错误 → 退出码 0
        out.finished_empty(
            "no_new_source",
            &CoreError::terminal(TerminalKind::NoNewSource).describe(lang),
        );
        return Ok(ExitKind::Ok);
    }

    out.plan(&plan);
    let started = std::time::Instant::now();
    let cancel = CancelToken::new();
    let clock = SystemClock;
    let report = {
        let mut sink = out.progress_sink();
        run_task(&spec, &plan, io.as_ref(), &clock, &cancel, &mut sink)
            .map_err(|e| e.describe(lang))?
    };
    // 拷完自动出一份报告——「证」是这个产品的一半价值，不该要用户再敲一条命令
    let failures: Vec<(String, String, u32)> = report
        .failed_files()
        .map(|f| {
            let reason = match &f.status {
                steadcopy_core::task::FileStatus::Failed(m) => m.clone(),
                _ => String::new(),
            };
            (f.relative_path.clone(), reason, f.retries)
        })
        .collect();
    let elapsed = started.elapsed().as_secs();
    let mut report_paths = Vec::new();
    for mp in &report.manifests {
        if let Ok(m) = read_manifest(mp) {
            match write_html_report(
                mp,
                &m,
                &failures,
                report.skipped_count(),
                &report.notices,
                Some(elapsed),
                None,
            ) {
                Ok(p) => report_paths.push(p),
                Err(e) => out.error(&e),
            }
        }
    }

    out.report(&report);
    for p in &report_paths {
        out.report_written(p);
    }

    if report.cancelled {
        return Ok(ExitKind::Cancelled);
    }
    // 部分失败 MUST 以非零退出
    Ok(if report.failed_files().count() > 0 {
        ExitKind::Retryable
    } else {
        ExitKind::Ok
    })
}

fn cmd_audit(
    manifest_path: &Path,
    dir: Option<&Path>,
    out: &mut Emitter,
) -> Result<ExitKind, String> {
    let lang = output::lang();
    let m = read_manifest(manifest_path).map_err(|e| e.describe(lang))?;
    // 清单落在 <落地目录>/steadcopy/ 下，默认取其上一级
    let target = match dir {
        Some(d) => d.to_path_buf(),
        None => manifest_path
            .parent()
            .and_then(|p| p.parent())
            .ok_or_else(|| {
                output::w(
                    "无法从清单路径推断被复验目录，请用 --dir 指定",
                    "Cannot infer the directory to re-verify from the manifest path — pass --dir",
                )
                .to_string()
            })?
            .to_path_buf(),
    };
    if !target.exists() {
        return Err(wf!(
            "被复验的目录不存在：{}",
            "The directory to re-verify does not exist: {}",
            target.display()
        ));
    }

    let io = volume_io();
    let mut observed = Vec::new();
    for f in scan_source(&target, &ScanOptions::mirror()).files {
        if steadcopy_core::manifest::is_manifest_path(&target, &f.absolute_path) {
            continue;
        }
        let hash =
            steadcopy_core::engine::hash_destination(io.as_ref(), &f.absolute_path, m.algorithm)
                .map_err(|e| {
                    wf!(
                        "{} 读取失败：{}",
                        "Could not read {}: {}",
                        f.relative_path,
                        e.describe(lang)
                    )
                })?;
        observed.push(ObservedFile::new(&f.relative_path, f.size, hash));
    }

    let r = audit(&m, &observed, true);
    out.audit(&r);
    // 有丢失才算失败；「新增」不构成失败。
    //
    // 归**终态族**而不是可重试族：拷贝期的校验失败重拷一次可能就好了，
    // 但复验是对已经落地的数据做的——同一份清单再跑一遍，答案只会一样。
    // 把它标成「可重试」会让脚本白白重试，也会误导人以为再试试就能好。
    Ok(if r.is_data_intact() {
        ExitKind::Ok
    } else {
        ExitKind::Terminal
    })
}
