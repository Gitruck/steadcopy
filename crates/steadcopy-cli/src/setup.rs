//! 配置类子命令：项目、预设、设备记忆、配置文件。
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/config-store/spec.md`

use std::path::PathBuf;

use clap::Subcommand;
use steadcopy_core::config::{self, model::DestinationConfig, model::Project, Config};
use steadcopy_core::device::DeviceKind;
use steadcopy_core::platform::{Clock, SystemClock};
use steadcopy_core::preset::{Preset, PresetMatch};

use crate::output::{Emitter, ExitKind};

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
    config::load().map_err(|e| e.to_string())
}

fn save(c: &Config) -> Result<(), String> {
    config::save(c).map_err(|e| e.to_string())
}

pub fn config_cmd(action: &ConfigAction, out: &mut Emitter) -> Result<ExitKind, String> {
    match action {
        ConfigAction::Path => {
            println!("{}", config::config_path().display());
        }
        ConfigAction::Show => {
            let c = load()?;
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
                out.warn(
                    "危险区已开启：插入**已分类**的设备将直接开始拷贝，不再询问。\n\
                     （未分类的新设备仍会先要求指认——这条绕不过去）",
                );
            } else {
                out.note("已恢复为每次插卡都需确认");
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
                return Err(format!("目的地数量应为 1..4 个，实际 {}", dests.len()));
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
            out.note(&format!("已新建项目「{name}」（{id}）"));
        }
        ProjectAction::Use { id_or_name } => {
            let id = find_project(&c, id_or_name)?;
            c.current_project = Some(id.clone());
            save(&c)?;
            out.note(&format!("当前项目已设为 {id}"));
        }
        ProjectAction::Remove { id_or_name } => {
            let id = find_project(&c, id_or_name)?;
            c.projects.retain(|p| p.id != id);
            c.presets.retain(|p| p.project_id.as_deref() != Some(id.as_str()));
            if c.current_project.as_deref() == Some(id.as_str()) {
                c.current_project = c.projects.first().map(|p| p.id.clone());
            }
            save(&c)?;
            out.note("已删除该项目（已拷素材与凭证不受影响）");
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
            out.note(&format!("已新建预设「{name}」"));
        }
        PresetAction::Remove { id_or_name } => {
            let id = find_preset(&c, id_or_name)?;
            c.presets.retain(|p| p.id != id);
            save(&c)?;
            out.note("已删除该预设");
        }
        PresetAction::Enable { id_or_name } | PresetAction::Disable { id_or_name } => {
            let on = matches!(action, PresetAction::Enable { .. });
            let id = find_preset(&c, id_or_name)?;
            if let Some(p) = c.presets.iter_mut().find(|p| p.id == id) {
                p.enabled = on;
            }
            save(&c)?;
            out.note(if on { "已启用" } else { "已停用" });
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
            let d = c
                .device_mut(id)
                .ok_or_else(|| format!("记忆库里没有这个设备：{id}"))?;
            d.kind = k;
            let name = d.display_name();
            save(&c)?;
            out.note(&format!("「{name}」已指认为{}", k.label(crate::output::lang())));
        }
        DeviceAction::Rename { id, name } => {
            let d = c
                .device_mut(id)
                .ok_or_else(|| format!("记忆库里没有这个设备：{id}"))?;
            d.custom_name = name.clone();
            save(&c)?;
            out.note(&format!("已改名为「{name}」"));
        }
        DeviceAction::Forget { id } => {
            c.devices.retain(|d| d.id != *id);
            save(&c)?;
            out.note("已删除该设备的记忆（下次插入将视为新设备）");
        }
    }
    Ok(ExitKind::Ok)
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
        let kind = match k {
            "camera" => DeviceKind::Camera,
            "recorder" => DeviceKind::Recorder,
            "storage" => DeviceKind::Storage,
            other => return Err(format!("不认识的设备类型：{other}（camera/recorder/storage）")),
        };
        return Ok(PresetMatch::Kind { device_kind: kind });
    }
    Err(format!(
        "不认识的匹配写法：{s}。可用：any | kind:camera|recorder|storage | device:<设备id>"
    ))
}

fn find_project(c: &Config, key: &str) -> Result<String, String> {
    c.projects
        .iter()
        .find(|p| p.id == key || p.name == key)
        .map(|p| p.id.clone())
        .ok_or_else(|| format!("找不到项目：{key}"))
}

fn find_preset(c: &Config, key: &str) -> Result<String, String> {
    c.presets
        .iter()
        .find(|p| p.id == key || p.name == key)
        .map(|p| p.id.clone())
        .ok_or_else(|| format!("找不到预设：{key}"))
}
