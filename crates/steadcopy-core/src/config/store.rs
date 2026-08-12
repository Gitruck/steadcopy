//! 配置读写：用户数据目录、原子写入、版本化、损坏保留。
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/config-store/spec.md`
//!
//! **为什么不上 SQLite**：配置是「几十个项目 + 几十条预设 + 几百个设备记忆」的量级，
//! JSON 读写一次是毫秒级，还能让用户直接打开看、出问题时手工救。
//! SQLite 留给任务台账（数万行、要按多维筛选），那是另一个量级的问题。

use std::path::{Path, PathBuf};

use crate::config::model::{Config, ConfigError, CONFIG_VERSION};

const FILE_NAME: &str = "config.json";
const APP_DIR: &str = "steadcopy";

#[derive(Debug)]
pub enum ConfigLoadError {
    /// 文件读不了（权限等）
    Unreadable(String),
    /// 内容损坏。`backup` 是原文件被改名保留的位置——**绝不静默重建**
    Corrupt {
        reason: String,
        backup: Option<PathBuf>,
    },
    /// 版本高于本程序可识别
    FutureVersion { found: u32, supported: u32 },
}

impl std::fmt::Display for ConfigLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigLoadError::Unreadable(e) => write!(f, "配置文件读取失败：{e}"),
            ConfigLoadError::Corrupt { reason, backup } => {
                write!(f, "配置文件内容损坏：{reason}")?;
                if let Some(b) = backup {
                    write!(f, "。原文件已保留在 {}，可据以人工恢复", b.display())?;
                }
                Ok(())
            }
            ConfigLoadError::FutureVersion { found, supported } => write!(
                f,
                "配置由更新版本的程序写入（格式版本 {found}，本程序支持到 {supported}），请升级后再打开"
            ),
        }
    }
}

impl std::error::Error for ConfigLoadError {}

/// 便携版标记文件。放在程序旁边即启用便携模式。
const PORTABLE_MARKER: &str = "steadcopy.portable";
/// 便携模式下的数据子目录名。
const PORTABLE_DATA: &str = "data";

/// 给定程序所在目录，判断是否处于便携模式并给出数据目录。
///
/// 判据是**程序旁边有没有标记文件**。安装版不带这个标记，
/// 所以两者的数据天然隔离——同机并存也不会互相读写。
fn portable_dir_at(exe_dir: &Path) -> Option<PathBuf> {
    exe_dir
        .join(PORTABLE_MARKER)
        .exists()
        .then(|| exe_dir.join(PORTABLE_DATA))
}

/// 便携模式的数据目录；非便携模式返回 `None`。
pub fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    portable_dir_at(exe.parent()?)
}

/// 当前是否以便携版运行。
pub fn is_portable() -> bool {
    portable_dir().is_some()
}

/// 配置所在目录。**用户数据目录**，不是程序安装目录——
/// 安装目录可能无写权限，且卸载会把用户的项目与预设一起带走。
///
/// 例外只有便携版：那是用户主动要求「数据跟着程序走」的形态，
/// 靠程序旁边的标记文件显式开启，不会被误触发。
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("STEADCOPY_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(d) = portable_dir() {
        return d;
    }
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("XDG_CONFIG_HOME"))
        .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.config")))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join(APP_DIR)
}

pub fn config_path() -> PathBuf {
    config_dir().join(FILE_NAME)
}

/// 读配置。文件不存在时返回默认配置（首次运行是正常情况，不是错误）。
pub fn load() -> Result<Config, ConfigLoadError> {
    load_from(&config_path())
}

pub fn load_from(path: &Path) -> Result<Config, ConfigLoadError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(ConfigLoadError::Unreadable(e.to_string())),
    };

    // 先只取版本号：版本比我们新时不要按当前结构强行解析
    #[derive(serde::Deserialize)]
    struct VersionProbe {
        version: u32,
    }
    match serde_json::from_str::<VersionProbe>(&text) {
        Ok(v) if v.version > CONFIG_VERSION => {
            return Err(ConfigLoadError::FutureVersion {
                found: v.version,
                supported: CONFIG_VERSION,
            })
        }
        Ok(_) => {}
        Err(e) => {
            return Err(ConfigLoadError::Corrupt {
                reason: e.to_string(),
                backup: preserve_corrupt(path),
            })
        }
    }

    serde_json::from_str(&text).map_err(|e| ConfigLoadError::Corrupt {
        reason: e.to_string(),
        backup: preserve_corrupt(path),
    })
}

/// 把损坏的配置改名保留，返回备份路径。
///
/// **绝不静默重建**——用户的项目与预设无声消失，比报错难受得多。
fn preserve_corrupt(path: &Path) -> Option<PathBuf> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let backup = path.with_file_name(format!("config.corrupt-{ms:x}.json"));
    std::fs::rename(path, &backup).ok().map(|()| backup)
}

/// 保存配置。**先校验再原子写入。**
pub fn save(config: &Config) -> Result<(), SaveError> {
    save_to(&config_path(), config)
}

#[derive(Debug)]
pub enum SaveError {
    Invalid(ConfigError),
    Io(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Invalid(e) => write!(f, "{e}"),
            SaveError::Io(e) => write!(f, "配置写入失败：{e}"),
        }
    }
}

impl std::error::Error for SaveError {}

pub fn save_to(path: &Path, config: &Config) -> Result<(), SaveError> {
    config.validate().map_err(SaveError::Invalid)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| SaveError::Io(e.to_string()))?;
    }

    let mut out = config.clone();
    out.version = CONFIG_VERSION;
    let json = serde_json::to_string_pretty(&out).map_err(|e| SaveError::Io(e.to_string()))?;

    // 原子写入：先写临时文件再替换。同卷内 rename 是原子的——
    // 中途断电只会丢掉这一次修改，不会留下半截文件。
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| SaveError::Io(e.to_string()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        SaveError::Io(e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{DestinationConfig, Project};
    use crate::device::{DeviceKind, DeviceRecord};
    use crate::preset::{Preset, PresetMatch};
    use time::macros::datetime;

    fn sample() -> Config {
        let at = datetime!(2026-08-10 09:00:00 UTC);
        let mut c = Config::default();
        let mut p = Project::new("婚礼-张先生", at);
        p.destinations.push(DestinationConfig::new(r"D:\素材"));
        p.destinations.push(DestinationConfig::new(r"F:\备份"));
        c.current_project = Some(p.id.clone());
        let pid = p.id.clone();
        c.projects.push(p);

        let mut pr = Preset::new("摄影卡进婚礼项目").matching(PresetMatch::Kind {
            device_kind: DeviceKind::Camera,
        });
        pr.project_id = Some(pid);
        c.presets.push(pr);

        let mut d = DeviceRecord::new("vol:1", "A7M4", 128, at);
        d.kind = DeviceKind::Camera;
        c.devices.push(d);
        c
    }

    /// 递归检查：凡是名字像时间的键，值都必须是字符串。
    ///
    /// 这是**跨层契约的守夜人**。TS 那边把时间声明成 `string`，但 JSON 边界
    /// 没有类型检查——`tsc --noEmit` 全绿，界面照样会在 `.replace()` 上炸。
    /// `last_seen` 就是这么漏过去的：manifest 那边加了 rfc3339，config 这边忘了。
    /// 所以不逐个字段断言，而是把规则本身钉住：**以后新增任何时间字段都自动被盯上。**
    fn assert_time_fields_are_strings(v: &serde_json::Value, path: &str) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, val) in m {
                    let here = format!("{path}.{k}");
                    if k.ends_with("_at") || k.ends_with("_seen") || k == "at" {
                        assert!(
                            val.is_string(),
                            "{here} 必须序列化成字符串，实际是 {val}。\
                             前端声明的是 string，写成别的形状会在运行时炸"
                        );
                    }
                    assert_time_fields_are_strings(val, &here);
                }
            }
            serde_json::Value::Array(a) => {
                for (i, val) in a.iter().enumerate() {
                    assert_time_fields_are_strings(val, &format!("{path}[{i}]"));
                }
            }
            _ => {}
        }
    }

    // spec: config-store → 时间字段序列化口径 → Scenario: 时间字段序列化为 ISO 字符串
    #[test]
    fn scenario_config_store_every_timestamp_crosses_as_a_string() {
        let v = serde_json::to_value(sample()).expect("序列化整份配置");
        // 先确认样本里真的有时间字段，否则这个测试等于没测
        assert!(v["devices"][0]["last_seen"].is_string(), "{v}");
        assert!(v["projects"][0]["created_at"].is_string(), "{v}");
        assert_time_fields_are_strings(&v, "config");
    }

    // spec: config-store → 配置内容与位置 → Scenario: 配置往返一致
    #[test]
    fn scenario_config_store_roundtrip() {
        let dir = tempfile::tempdir().expect("临时目录");
        let p = dir.path().join("config.json");
        let c = sample();
        save_to(&p, &c).expect("保存");
        let back = load_from(&p).expect("读回");
        assert_eq!(back, c);
    }

    // spec: → Scenario: 首次运行给出可用默认值
    #[test]
    fn scenario_config_store_missing_file_gives_defaults() {
        let dir = tempfile::tempdir().expect("临时目录");
        let c = load_from(&dir.path().join("从未存在.json")).expect("首次运行不该报错");
        assert_eq!(c.version, CONFIG_VERSION);
        assert!(c.projects.is_empty());
        // 默认设置必须是安全的
        assert!(c.settings.auto_prefill);
        assert!(!c.settings.skip_confirmation, "跳过确认默认必须是关");
        assert!(c.settings.verify_default);
    }

    // spec: → 原子写入 → Scenario: 写入不产生半截文件
    #[test]
    fn scenario_config_store_atomic_write() {
        let dir = tempfile::tempdir().expect("临时目录");
        let p = dir.path().join("config.json");
        for i in 0..20 {
            let mut c = sample();
            c.projects[0].name = format!("项目{i}");
            save_to(&p, &c).expect("保存");
            // 每次保存后立刻读，必须完整可解析
            let back = load_from(&p).expect("读回");
            assert_eq!(back.projects[0].name, format!("项目{i}"));
        }
        // 临时文件不该残留
        assert!(!p.with_extension("json.tmp").exists());
    }

    // spec: → 版本化与损坏处理 → Scenario: 损坏配置被保留而非覆盖
    #[test]
    fn scenario_config_store_corrupt_is_preserved_not_rebuilt() {
        let dir = tempfile::tempdir().expect("临时目录");
        let p = dir.path().join("config.json");
        std::fs::write(&p, br#"{"version":1,"projects":[{"id":"#).expect("写半截文件");

        let err = load_from(&p).expect_err("损坏配置 MUST 报错");
        match err {
            ConfigLoadError::Corrupt { backup, .. } => {
                let b = backup.expect("原文件 MUST 被改名保留");
                assert!(b.exists(), "备份文件应存在");
                assert!(!p.exists(), "原路径已让位，下次启动才会重建");
                // 备份内容原样保留，用户能据以恢复
                let kept = std::fs::read_to_string(&b).expect("读备份");
                assert!(kept.contains("projects"));
            }
            other => panic!("错误类型不对：{other:?}"),
        }
    }

    // spec: → Scenario: 未来版本被拒绝
    #[test]
    fn scenario_config_store_future_version_rejected() {
        let dir = tempfile::tempdir().expect("临时目录");
        let p = dir.path().join("config.json");
        let mut v = serde_json::to_value(sample()).expect("转 json");
        v["version"] = serde_json::json!(CONFIG_VERSION + 3);
        std::fs::write(&p, v.to_string()).expect("写");

        let err = load_from(&p).expect_err("未来版本 MUST 被拒");
        assert!(matches!(err, ConfigLoadError::FutureVersion { .. }));
        assert!(err.to_string().contains("升级"));
        assert!(p.exists(), "未来版本不是损坏，原文件 MUST NOT 被改名");
    }

    #[test]
    fn scenario_config_store_invalid_config_is_not_written() {
        let dir = tempfile::tempdir().expect("临时目录");
        let p = dir.path().join("config.json");
        let mut c = sample();
        c.projects[0].destinations.clear(); // 零目的地：非法
        let err = save_to(&p, &c).expect_err("非法配置 MUST 拒绝写入");
        assert!(matches!(err, SaveError::Invalid(_)));
        assert!(!p.exists(), "非法配置 MUST NOT 落盘");
    }

    // spec: → 设备记忆库持久化 → Scenario: 删除记忆后视为新设备
    #[test]
    fn scenario_config_store_forget_device_makes_it_new_again() {
        let dir = tempfile::tempdir().expect("临时目录");
        let p = dir.path().join("config.json");
        let mut c = sample();
        assert!(c.device("vol:1").is_some());

        c.devices.retain(|d| d.id != "vol:1");
        save_to(&p, &c).expect("保存");
        let back = load_from(&p).expect("读回");
        assert!(back.device("vol:1").is_none(), "删除记忆后不该还在");
    }

    #[test]
    fn scenario_config_store_dir_is_overridable_for_tests() {
        // 测试与便携版都需要能指定配置位置，而不是写死 APPDATA
        std::env::set_var("STEADCOPY_CONFIG_DIR", r"X:\某处");
        assert_eq!(config_dir(), PathBuf::from(r"X:\某处"));
        std::env::remove_var("STEADCOPY_CONFIG_DIR");
        // 默认落在用户数据目录，绝不是程序目录
        let d = config_dir();
        assert!(d.ends_with(APP_DIR), "配置目录应以应用名结尾：{d:?}");
    }

    #[test]
    fn scenario_build_release_portable_data_is_isolated_from_installed() {
        let dir = tempfile::tempdir().expect("临时目录");
        let exe_dir = dir.path();

        // 没有标记文件 = 安装版形态，数据不落在程序旁边
        assert!(
            portable_dir_at(exe_dir).is_none(),
            "没有标记文件就不该进便携模式"
        );

        std::fs::write(exe_dir.join(PORTABLE_MARKER), "").expect("写标记");
        let data = portable_dir_at(exe_dir).expect("有标记就该进便携模式");
        assert_eq!(data, exe_dir.join(PORTABLE_DATA));

        // 便携版的数据目录与安装版的用户数据目录不可能重合
        let installed = PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into()))
            .join(APP_DIR);
        assert_ne!(data, installed, "便携版与安装版必须各存各的");
    }
}
