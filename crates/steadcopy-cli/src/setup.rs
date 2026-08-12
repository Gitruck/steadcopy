//! 配置类子命令：项目、预设、设备记忆、配置文件。
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/config-store/spec.md`
//!
//! 子命令的 doc 注释是 `--help` 的文字，由 clap 在编译期定死，**恒为中文**
//! （取舍见 `main.rs` 上 `Cli` 的注释）；运行期回给用户的每一句都跟随 `--lang`。

use std::path::PathBuf;

use clap::Subcommand;
use steadcopy_core::config::{self, model::DestinationConfig, model::Project, Config};
use steadcopy_core::device::DeviceKind;
use steadcopy_core::platform::{Clock, SystemClock};
use steadcopy_core::preset::{Preset, PresetMatch};

use crate::output::{lang, w, Emitter, ExitKind};

#[derive(Subcommand)]
pub enum ConfigAction {
    /// 打印配置文件位置
    Path,
    /// 打印配置内容
    Show,
    /// 危险区：开关「跳过插卡确认」
    SkipConfirm {
        /// on = 插卡即拷不再询问；off = 恢复为每次确认
        #[arg(value_parser = ["on", "off"])]
        value: String,
    },
}

#[derive(Subcommand)]
pub enum ProjectAction {
    List,
    /// 新建项目
    Add {
        name: String,
        /// 目的地根目录，可重复给出（1..4 个）
        #[arg(short, long = "dest", required = true)]
        dests: Vec<PathBuf>,
        /// 落地路径模板
        #[arg(long, default_value = steadcopy_core::config::DEFAULT_TEMPLATE)]
        template: String,
    },
    /// 设为当前项目
    Use { id_or_name: String },
    Remove { id_or_name: String },
}

#[derive(Subcommand)]
pub enum PresetAction {
    List,
    /// 新建预设
    Add {
        name: String,
        /// 匹配：device:<设备id> | kind:camera|recorder|storage | any
        #[arg(long, default_value = "any")]
        matches: String,
        /// 用哪个项目（省略则用当前项目）
        #[arg(long)]
        project: Option<String>,
        /// 关闭读回校验（不推荐）
        #[arg(long)]
        no_verify: bool,
    },
    Remove { id_or_name: String },
    Enable { id_or_name: String },
    Disable { id_or_name: String },
}

#[derive(Subcommand)]
pub enum DeviceAction {
    List,
    /// 指认设备类型
    SetKind {
        id: String,
        #[arg(value_parser = ["camera", "recorder", "storage", "ignored"])]
        kind: String,
    },
    Rename { id: String, name: String },
    Forget { id: String },
}

fn load() -> Result<Config, String> {
    config::load().map_err(|e| e.describe(lang()))
}

fn save(c: &Config) -> Result<(), String> {
    config::save(c).map_err(|e| e.describe(lang()))
}

pub fn config_cmd(action: &ConfigAction, out: &mut Emitter) -> Result<ExitKind, String> {
    match action {
        ConfigAction::Path => {
            println!("{}", config::config_path().display());
        }
        ConfigAction::Show => {
            let c = load()?;
            // 配置本身是机读结构，不随语言变
            println!(
                "{}",
                serde_json::to_string_pretty(&c).map_err(|e| e.to_string())?
            );
        }
        ConfigAction::SkipConfirm { value } => {
            let mut c = load()?;
            let on = value == "on";
            c.settings.skip_confirmation = on;
            save(&c)?;
            if on {
                out.warn(w(
                    "危险区已开启：插入**已分类**的设备将直接开始拷贝，不再询问。\n\
                     （未分类的新设备仍会先要求指认——这条绕不过去）",
                    "Danger zone on: plugging in a **classified** device starts copying right away, \
                     with no prompt.\n\
                     (A new, unclassified device still has to be identified first — that one cannot \
                     be bypassed)",
                ));
            } else {
                out.note(w(
                    "已恢复为每次插卡都需确认",
                    "Back to asking for confirmation on every card",
                ));
            }
        }
    }
    Ok(ExitKind::Ok)
}

pub fn project_cmd(action: &ProjectAction, out: &mut Emitter) -> Result<ExitKind, String> {
    let mut c = load()?;
    match action {
        ProjectAction::List => out.projects(&c),
        ProjectAction::Add {
            name,
            dests,
            template,
        } => {
            if dests.is_empty() || dests.len() > 4 {
                return Err(wf!(
                    "目的地数量应为 1..4 个，实际 {} 个",
                    "Expected 1 to 4 destinations, got {}",
                    dests.len()
                ));
            }
            let mut p = Project::new(name, SystemClock.now());
            for d in dests {
                let mut dc = DestinationConfig::new(d);
                dc.template = template.clone();
                p.destinations.push(dc);
            }
            let id = p.id.clone();
            if c.current_project.is_none() {
                c.current_project = Some(id.clone());
            }
            c.projects.push(p);
            save(&c)?;
            out.note(&wf!(
                "已新建项目「{}」（{}）",
                "Created project \"{}\" ({})",
                name,
                id
            ));
        }
        ProjectAction::Use { id_or_name } => {
            let id = find_project(&c, id_or_name)?;
            c.current_project = Some(id.clone());
            save(&c)?;
            out.note(&wf!(
                "当前项目已设为 {}",
                "Current project is now {}",
                id
            ));
        }
        ProjectAction::Remove { id_or_name } => {
            let id = find_project(&c, id_or_name)?;
            c.projects.retain(|p| p.id != id);
            c.presets.retain(|p| p.project_id.as_deref() != Some(id.as_str()));
            if c.current_project.as_deref() == Some(id.as_str()) {
                c.current_project = c.projects.first().map(|p| p.id.clone());
            }
            save(&c)?;
            out.note(w(
                "已删除该项目（已拷素材与凭证不受影响）",
                "Project deleted (copied media and manifests are untouched)",
            ));
        }
    }
    Ok(ExitKind::Ok)
}

pub fn preset_cmd(action: &PresetAction, out: &mut Emitter) -> Result<ExitKind, String> {
    let mut c = load()?;
    match action {
        PresetAction::List => out.presets(&c),
        PresetAction::Add {
            name,
            matches,
            project,
            no_verify,
        } => {
            let matcher = parse_match(matches)?;
            let mut p = Preset::new(name).matching(matcher);
            p.verify = !no_verify;
            if let Some(pr) = project {
                p.project_id = Some(find_project(&c, pr)?);
            }
            c.presets.push(p);
            save(&c)?;
            out.note(&wf!("已新建预设「{}」", "Created preset \"{}\"", name));
        }
        PresetAction::Remove { id_or_name } => {
            let id = find_preset(&c, id_or_name)?;
            c.presets.retain(|p| p.id != id);
            save(&c)?;
            out.note(w("已删除该预设", "Preset deleted"));
        }
        PresetAction::Enable { id_or_name } | PresetAction::Disable { id_or_name } => {
            let on = matches!(action, PresetAction::Enable { .. });
            let id = find_preset(&c, id_or_name)?;
            if let Some(p) = c.presets.iter_mut().find(|p| p.id == id) {
                p.enabled = on;
            }
            save(&c)?;
            out.note(if on {
                w("已启用", "Enabled")
            } else {
                w("已停用", "Disabled")
            });
        }
    }
    Ok(ExitKind::Ok)
}

pub fn device_cmd(action: &DeviceAction, out: &mut Emitter) -> Result<ExitKind, String> {
    let mut c = load()?;
    match action {
        DeviceAction::List => out.device_records(&c),
        DeviceAction::SetKind { id, kind } => {
            let k = match kind.as_str() {
                "camera" => DeviceKind::Camera,
                "recorder" => DeviceKind::Recorder,
                "storage" => DeviceKind::Storage,
                _ => DeviceKind::Ignored,
            };
            let d = c.device_mut(id).ok_or_else(|| device_not_found(id))?;
            d.kind = k;
            let name = d.display_name();
            save(&c)?;
            out.note(&wf!(
                "「{}」已指认为{}",
                "\"{}\" is now identified as: {}",
                name,
                k.label(lang())
            ));
        }
        DeviceAction::Rename { id, name } => {
            let d = c.device_mut(id).ok_or_else(|| device_not_found(id))?;
            d.custom_name = name.clone();
            save(&c)?;
            out.note(&wf!("已改名为「{}」", "Renamed to \"{}\"", name));
        }
        DeviceAction::Forget { id } => {
            c.devices.retain(|d| d.id != *id);
            save(&c)?;
            out.note(w(
                "已删除该设备的记忆（下次插入将视为新设备）",
                "Forgot this device (next time it plugs in it counts as new)",
            ));
        }
    }
    Ok(ExitKind::Ok)
}

fn device_not_found(id: &str) -> String {
    wf!(
        "记忆库里没有这个设备：{}",
        "No such device in the registry: {}",
        id
    )
}

fn parse_match(s: &str) -> Result<PresetMatch, String> {
    if s == "any" {
        return Ok(PresetMatch::AnyClassifiedSource);
    }
    if let Some(id) = s.strip_prefix("device:") {
        return Ok(PresetMatch::Device {
            device_id: id.to_string(),
        });
    }
    if let Some(k) = s.strip_prefix("kind:") {
        // 匹配写法是命令行语法，两种语言下敲的都是这几个 ASCII 词
        let kind = match k {
            "camera" => DeviceKind::Camera,
            "recorder" => DeviceKind::Recorder,
            "storage" => DeviceKind::Storage,
            other => {
                return Err(wf!(
                    "不认识的设备类型：{}（camera/recorder/storage）",
                    "Unknown device kind: {} (camera/recorder/storage)",
                    other
                ))
            }
        };
        return Ok(PresetMatch::Kind { device_kind: kind });
    }
    Err(wf!(
        "不认识的匹配写法：{}。可用：any | kind:camera|recorder|storage | device:<设备id>",
        "Unknown matcher: {}. Available: any | kind:camera|recorder|storage | device:<device-id>",
        s
    ))
}

fn find_project(c: &Config, key: &str) -> Result<String, String> {
    c.projects
        .iter()
        .find(|p| p.id == key || p.name == key)
        .map(|p| p.id.clone())
        .ok_or_else(|| wf!("找不到项目：{}", "No such project: {}", key))
}

fn find_preset(c: &Config, key: &str) -> Result<String, String> {
    c.presets
        .iter()
        .find(|p| p.id == key || p.name == key)
        .map(|p| p.id.clone())
        .ok_or_else(|| wf!("找不到预设：{}", "No such preset: {}", key))
}
