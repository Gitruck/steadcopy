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

/// 配置所在目录。**用户数据目录**，不是程序安装目录——
/// 安装目录可能无写权限，且卸载会把用户的项目与预设一起带走。
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("STEADCOPY_CONFIG_DIR") {
        return PathBuf::from(dir);
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
}
