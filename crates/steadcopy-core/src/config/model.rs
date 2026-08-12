//! 配置模型：项目、目的地、预设、设备记忆、应用设置。
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/config-store/spec.md`

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::device::DeviceRecord;
use crate::engine::HashAlgorithm;
use crate::organize::{PathTemplate, TemplateError};
use crate::preset::Preset;

/// 配置格式版本。读到更高版本 MUST 拒绝解析。
pub const CONFIG_VERSION: u32 = 1;

/// 默认路径模板。
pub const DEFAULT_TEMPLATE: &str = "{项目}/{日期}/{设备}";

/// 一个目的地。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationConfig {
    pub id: String,
    pub root: PathBuf,
    /// 路径模板串。**保存时即校验**，不拖到拷贝时才失败
    pub template: String,
    pub enabled: bool,
}

impl DestinationConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            id: new_id("dst"),
            root: root.into(),
            template: DEFAULT_TEMPLATE.to_string(),
            enabled: true,
        }
    }

    pub fn parsed_template(&self) -> Result<PathTemplate, TemplateError> {
        PathTemplate::parse(&self.template)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    #[serde(with = "crate::serde_time")]
    pub created_at: OffsetDateTime,
    pub destinations: Vec<DestinationConfig>,
}

impl Project {
    pub fn new(name: impl Into<String>, at: OffsetDateTime) -> Self {
        Self {
            id: new_id("prj"),
            name: name.into(),
            created_at: at,
            destinations: Vec::new(),
        }
    }

    pub fn enabled_destinations(&self) -> impl Iterator<Item = &DestinationConfig> {
        self.destinations.iter().filter(|d| d.enabled)
    }
}

/// 应用设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 插卡后是否自动预填项目与目的地。默认开——不预填就谈不上「一次点击」
    pub auto_prefill: bool,
    /// **危险区**：跳过插卡确认，插卡即拷。默认关
    pub skip_confirmation: bool,
    pub verify_default: bool,
    pub algorithm: HashAlgorithm,
    pub retries: u32,
    pub notify_on_finish: bool,
    /// 拷完并校验通过后自动安全弹出
    pub eject_after: bool,
    /// **危险区**：拷完并全部校验通过后自动格式化源卡。默认关
    pub format_after_copy: bool,
    /// 不可逆操作的确认倒计时（秒）。默认 30，最小 10
    pub countdown_secs: u32,
    /// 界面与命令行的语言：`auto` / `zh` / `en`。默认跟随系统，判不出来落中文
    #[serde(default = "default_locale")]
    pub locale: String,
}

fn default_locale() -> String {
    crate::i18n::LOCALE_AUTO.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_prefill: true,
            skip_confirmation: false,
            verify_default: true,
            algorithm: HashAlgorithm::Xxh64,
            retries: 2,
            notify_on_finish: true,
            eject_after: false,
            format_after_copy: false,
            locale: default_locale(),
            countdown_secs: crate::device::COUNTDOWN_DEFAULT_SECS,
        }
    }
}

/// 插卡后的行为档位。由两个正交设置推导，不单独存。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrivalMode {
    /// 确认档（默认）：预填 + 等用户点一次
    Confirm,
    /// 手动档：提示到达，但项目与目的地留待用户选
    Manual,
    /// 无人值守档（危险区）：匹配到预设即直接开跑
    Unattended,
}

impl Settings {
    pub fn arrival_mode(&self) -> ArrivalMode {
        match (self.auto_prefill, self.skip_confirmation) {
            (true, true) => ArrivalMode::Unattended,
            (true, false) => ArrivalMode::Confirm,
            (false, _) => ArrivalMode::Manual,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub current_project: Option<String>,
    #[serde(default)]
    pub presets: Vec<Preset>,
    #[serde(default)]
    pub devices: Vec<DeviceRecord>,
    #[serde(default)]
    pub settings: Settings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            projects: Vec::new(),
            current_project: None,
            presets: Vec::new(),
            devices: Vec::new(),
            settings: Settings::default(),
        }
    }
}

/// 配置校验失败的原因。**保存时**就拒绝，不拖到拷贝时。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// 目的地数量不在 1..=4
    DestinationCount { project: String, count: usize },
    /// 该项目没有任何启用的目的地
    NoEnabledDestination { project: String },
    /// 路径模板非法
    BadTemplate {
        project: String,
        destination: String,
        reason: String,
    },
    /// 预设指向了不存在的项目
    PresetProjectMissing { preset: String, project_id: String },
    /// 倒计时低于安全下限
    CountdownTooShort { secs: u32, min: u32 },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::DestinationCount { project, count } => write!(
                f,
                "项目「{project}」的目的地数量应为 1 到 4 个，实际 {count} 个"
            ),
            ConfigError::NoEnabledDestination { project } => {
                write!(f, "项目「{project}」至少要有一个启用的目的地")
            }
            ConfigError::BadTemplate {
                project,
                destination,
                reason,
            } => write!(f, "项目「{project}」的目的地「{destination}」路径模板不合法：{reason}"),
            ConfigError::PresetProjectMissing { preset, project_id } => {
                write!(f, "预设「{preset}」指向的项目（{project_id}）已不存在")
            }
            ConfigError::CountdownTooShort { secs, min } => {
                write!(f, "不可逆操作的确认倒计时不能短于 {min} 秒（你设的是 {secs} 秒）")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// 保存前的完整校验。
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.settings.countdown_secs < crate::device::COUNTDOWN_MIN_SECS {
            return Err(ConfigError::CountdownTooShort {
                secs: self.settings.countdown_secs,
                min: crate::device::COUNTDOWN_MIN_SECS,
            });
        }
        for p in &self.projects {
            if p.destinations.is_empty() || p.destinations.len() > 4 {
                return Err(ConfigError::DestinationCount {
                    project: p.name.clone(),
                    count: p.destinations.len(),
                });
            }
            if p.enabled_destinations().count() == 0 {
                return Err(ConfigError::NoEnabledDestination {
                    project: p.name.clone(),
                });
            }
            for d in &p.destinations {
                if let Err(e) = d.parsed_template() {
                    return Err(ConfigError::BadTemplate {
                        project: p.name.clone(),
                        destination: d.root.display().to_string(),
                        reason: e.to_string(),
                    });
                }
            }
        }
        for pr in &self.presets {
            if let Some(pid) = &pr.project_id {
                if !self.projects.iter().any(|p| &p.id == pid) {
                    return Err(ConfigError::PresetProjectMissing {
                        preset: pr.name.clone(),
                        project_id: pid.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    pub fn project_mut(&mut self, id: &str) -> Option<&mut Project> {
        self.projects.iter_mut().find(|p| p.id == id)
    }

    /// 当前项目。没设或已被删时回落到第一个项目。
    pub fn effective_project(&self) -> Option<&Project> {
        self.current_project
            .as_ref()
            .and_then(|id| self.project(id))
            .or_else(|| self.projects.first())
    }

    pub fn device(&self, id: &str) -> Option<&DeviceRecord> {
        self.devices.iter().find(|d| d.id == id)
    }

    pub fn device_mut(&mut self, id: &str) -> Option<&mut DeviceRecord> {
        self.devices.iter_mut().find(|d| d.id == id)
    }

    /// 记住一个设备。已存在则只更新「最近见到」类信息，**不覆盖用户设的名字与类型**。
    pub fn remember_device(&mut self, mut record: DeviceRecord) -> &DeviceRecord {
        if let Some(idx) = self.devices.iter().position(|d| d.id == record.id) {
            let existing = &mut self.devices[idx];
            existing.last_seen = record.last_seen;
            existing.last_label = record.last_label;
            existing.total_bytes = record.total_bytes;
            return &self.devices[idx];
        }
        record.instance = crate::device::kind::next_instance(&self.devices, &record.custom_name);
        self.devices.push(record);
        self.devices.last().unwrap_or_else(|| unreachable!())
    }
}

/// 生成一个本机唯一的短标识。
///
/// 用「时间 + 进程内递增计数」而不是随机数：不引依赖，且同一毫秒内多次创建也不会撞。
pub fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{prefix}-{ms:x}-{n:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceKind;
    use time::macros::datetime;

    fn at() -> OffsetDateTime {
        datetime!(2026-08-10 09:00:00 UTC)
    }

    fn project_with(dests: usize) -> Project {
        let mut p = Project::new("婚礼", at());
        for i in 0..dests {
            p.destinations
                .push(DestinationConfig::new(format!(r"D:\素材{i}")));
        }
        p
    }

    // spec: config-store → 项目与目的地
    #[test]
    fn scenario_config_store_destination_count_bounds() {
        for n in [1usize, 2, 4] {
            let mut c = Config::default();
            c.projects.push(project_with(n));
            assert!(c.validate().is_ok(), "{n} 个目的地应合法");
        }
        for n in [0usize, 5] {
            let mut c = Config::default();
            c.projects.push(project_with(n));
            assert!(matches!(
                c.validate(),
                Err(ConfigError::DestinationCount { .. })
            ));
        }
    }

    // spec: → Scenario: 不能关闭最后一个启用的目的地
    #[test]
    fn scenario_config_store_needs_one_enabled_destination() {
        let mut c = Config::default();
        let mut p = project_with(2);
        p.destinations[0].enabled = false;
        p.destinations[1].enabled = false;
        c.projects.push(p);
        let err = c.validate().expect_err("全禁用应被拒");
        assert!(matches!(err, ConfigError::NoEnabledDestination { .. }));
        assert!(err.to_string().contains("至少要有一个启用"));
    }

    // spec: → Scenario: 非法模板保存时即被拒
    #[test]
    fn scenario_config_store_bad_template_rejected_at_save() {
        let mut c = Config::default();
        let mut p = project_with(1);
        p.destinations[0].template = "素材/{年}/{月}".into(); // 缺必需占位符
        c.projects.push(p);
        let err = c.validate().expect_err("非法模板应被拒");
        match err {
            ConfigError::BadTemplate { reason, .. } => {
                assert!(reason.contains("项目") && reason.contains("日期") && reason.contains("设备"));
            }
            other => panic!("错误类型不对：{other:?}"),
        }
    }

    #[test]
    fn scenario_config_store_preset_pointing_at_missing_project_rejected() {
        let mut c = Config::default();
        c.projects.push(project_with(1));
        let mut pr = Preset::new("摄影卡预设");
        pr.project_id = Some("prj-不存在".into());
        c.presets.push(pr);
        assert!(matches!(
            c.validate(),
            Err(ConfigError::PresetProjectMissing { .. })
        ));
    }

    // spec: → 档位模型（由两个正交设置推导）
    #[test]
    fn scenario_preset_autorun_arrival_mode_derivation() {
        let mut s = Settings::default();
        assert_eq!(s.arrival_mode(), ArrivalMode::Confirm, "默认必须是确认档");
        assert!(!s.skip_confirmation, "跳过确认默认必须是关");

        s.skip_confirmation = true;
        assert_eq!(s.arrival_mode(), ArrivalMode::Unattended);

        s.auto_prefill = false;
        assert_eq!(s.arrival_mode(), ArrivalMode::Manual);
        s.skip_confirmation = false;
        assert_eq!(s.arrival_mode(), ArrivalMode::Manual);
    }

    #[test]
    fn scenario_config_store_effective_project_falls_back() {
        let mut c = Config::default();
        assert!(c.effective_project().is_none());
        c.projects.push(project_with(1));
        // 没设当前项目 → 回落到第一个
        assert_eq!(c.effective_project().map(|p| p.id.clone()), Some(c.projects[0].id.clone()));
        // 当前项目指向已删除的 → 同样回落，不返回 None
        c.current_project = Some("prj-已删".into());
        assert!(c.effective_project().is_some());
    }

    // spec: → 设备记忆库持久化 → Scenario: 已记忆设备沿用设置
    #[test]
    fn scenario_config_store_remember_device_keeps_user_settings() {
        let mut c = Config::default();
        let mut rec = DeviceRecord::new("vol:1", "SD卡", 128, at());
        c.remember_device(rec.clone());

        // 用户改名并指认类型
        let d = c.device_mut("vol:1").expect("设备");
        d.custom_name = "A7M4主卡".into();
        d.kind = DeviceKind::Camera;

        // 再次到达：只更新「最近见到」，MUST NOT 覆盖用户设的名字与类型
        rec.last_label = "SD卡改了卷标".into();
        rec.total_bytes = 256;
        rec.last_seen = datetime!(2026-08-11 09:00:00 UTC);
        c.remember_device(rec);

        let d = c.device("vol:1").expect("设备");
        assert_eq!(d.custom_name, "A7M4主卡", "自定义名 MUST NOT 被覆盖");
        assert_eq!(d.kind, DeviceKind::Camera, "类型 MUST NOT 被覆盖");
        assert_eq!(d.last_label, "SD卡改了卷标", "卷标应更新");
        assert_eq!(d.total_bytes, 256);
        assert_eq!(c.devices.len(), 1, "不应重复登记");
    }

    #[test]
    fn scenario_config_store_remember_device_assigns_instance() {
        let mut c = Config::default();
        c.remember_device(DeviceRecord::new("vol:1", "SD卡", 128, at()));
        c.remember_device(DeviceRecord::new("vol:2", "SD卡", 128, at()));
        assert_eq!(c.devices[0].instance, 1);
        assert_eq!(c.devices[1].instance, 2, "重名应自动编号");
        assert_ne!(c.devices[0].display_name(), c.devices[1].display_name());
    }

    #[test]
    fn scenario_format_card_countdown_defaults_and_floor() {
        let c = Config::default();
        assert_eq!(c.settings.countdown_secs, 30, "倒计时默认 30 秒");
        assert!(!c.settings.format_after_copy, "拷后自动格式化默认必须关");

        let mut bad = Config::default();
        bad.settings.countdown_secs = 5;
        assert!(matches!(
            bad.validate(),
            Err(ConfigError::CountdownTooShort { .. })
        ));
        let mut ok = Config::default();
        ok.settings.countdown_secs = 10;
        assert!(ok.validate().is_ok(), "下限 10 秒本身应合法");
    }

    #[test]
    fn scenario_config_store_ids_are_unique() {
        let ids: Vec<String> = (0..500).map(|_| new_id("x")).collect();
        let uniq: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(uniq.len(), ids.len(), "同一毫秒内批量创建也不应撞 id");
    }
}
