//! 卷模型与**源卡准入判据**。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/device-registry/spec.md`
//! → Requirement: 源卡准入判据
//!
//! # 为什么不能用「可移动介质」标志位
//!
//! CFexpress 读卡器的桥接芯片（常见 Realtek RTL9210B）普遍透传 NVMe 身份、
//! **不置 removable 位**。按标志位过滤源，会让专业用户的卡整个消失——
//! Lightroom Classic 与 Adobe Bridge 都有这个实证。
//!
//! 所以判据改为**正向证明**：非系统盘 + 非目的地 + **总线类型属外接总线**。
//! 这一条同时容纳了「removable 位为假的 CFexpress」与「排除内置 NVMe/SATA 盘」。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 卷所在的总线类型。判定源卡准入的关键依据。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusType {
    Usb,
    /// Thunderbolt / 1394
    Thunderbolt,
    Sd,
    Mmc,
    /// 内置 NVMe
    Nvme,
    Sata,
    Scsi,
    /// 网络位置
    Network,
    /// 虚拟盘、光驱等
    Other,
    Unknown,
}

impl BusType {
    /// 是否属于**外接总线**。这是源卡准入的正向证据。
    pub const fn is_external(self) -> bool {
        matches!(
            self,
            BusType::Usb | BusType::Thunderbolt | BusType::Sd | BusType::Mmc
        )
    }

    pub const fn label(self) -> &'static str {
        match self {
            BusType::Usb => "USB",
            BusType::Thunderbolt => "Thunderbolt",
            BusType::Sd => "SD",
            BusType::Mmc => "MMC",
            BusType::Nvme => "NVMe",
            BusType::Sata => "SATA",
            BusType::Scsi => "SCSI",
            BusType::Network => "网络",
            BusType::Other => "其他",
            BusType::Unknown => "未知",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeState {
    Online,
    /// 卡槽存在但没插介质（多卡槽读卡器的空槽）
    NoMedia,
    Offline,
}

/// 一个已挂载的卷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Volume {
    /// 卷 GUID 路径（`\\?\Volume{...}\`）。**同机稳定，换机器会变。**
    pub guid_path: String,
    /// 盘符（如 `D:`）。可能没有——CFexpress 有「枚举成功但无盘符」的实例
    pub drive_letter: Option<String>,
    /// 卷标
    pub label: String,
    /// 卷序列号。**相机每次格式化都会变**，只能作为复合身份的一部分
    pub serial: Option<u32>,
    pub file_system: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub bus_type: BusType,
    pub is_system: bool,
    pub state: VolumeState,
    /// 卷根发现的设备指纹目录（`DCIM` / `PRIVATE/M4ROOT` 等），用于自动分类
    pub fingerprints: Vec<String>,
}

impl Volume {
    /// 用于访问该卷的根路径。优先盘符，没有则用卷 GUID 路径。
    pub fn root_path(&self) -> PathBuf {
        match &self.drive_letter {
            Some(d) => PathBuf::from(format!("{d}\\")),
            None => PathBuf::from(&self.guid_path),
        }
    }

    /// 界面上给用户看的名字。
    pub fn display_name(&self) -> String {
        let name = if self.label.trim().is_empty() {
            "未命名卷".to_string()
        } else {
            self.label.clone()
        };
        match &self.drive_letter {
            Some(d) => format!("{name} ({d})"),
            // 无盘符时用容量帮用户辨认
            None => format!("{name}（无盘符 · {}）", human(self.total_bytes)),
        }
    }

    /// **复合身份**：跨机器可用的稳定标识。
    ///
    /// 单独任何一项都不够——卷 GUID 换机器会变；卷序列号格式化就变；
    /// 物理序列号在读卡器后面报的是**读卡器**的号而不是卡的。
    /// 所以取多项拼合，容忍其中任一项变化时仍有较大概率认出同一张卡。
    pub fn composite_id(&self) -> String {
        format!(
            "vol:{}|label:{}|fs:{}|size:{}",
            self.serial
                .map(|s| format!("{s:08X}"))
                .unwrap_or_else(|| "-".into()),
            self.label.trim(),
            self.file_system,
            self.total_bytes
        )
    }

    /// 该卷**能否作为拷贝源**。
    ///
    /// 三个条件同时成立：非系统盘、非任一目的地所在卷、总线类型属外接总线。
    ///
    /// 注意：卷根有没有设备指纹目录**不是**准入条件——空卡与刚格式化的卡
    /// 上面什么都没有，但它们仍是合法的源（结果会是「无素材」终态）。
    pub fn can_be_source(&self, destination_roots: &[PathBuf]) -> bool {
        if self.is_system || self.state != VolumeState::Online {
            return false;
        }
        if !self.bus_type.is_external() {
            return false;
        }
        !self.is_any_destination(destination_roots)
    }

    /// 该卷是否是（或包含）任一已配置的目的地。
    pub fn is_any_destination(&self, destination_roots: &[PathBuf]) -> bool {
        let root = self.root_path();
        destination_roots.iter().any(|d| same_volume(&root, d))
    }
}

/// 粗略判断两个路径是否在同一个卷上（比较盘符 / 卷 GUID 前缀）。
fn same_volume(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let s = p.to_string_lossy().to_ascii_lowercase();
        // 取到第一个分隔符为止的前缀（盘符或 \\?\Volume{...}）
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            // split 至少给一段；取不到只可能是空串，显式写出来而不是靠 default
            format!(r"\\?\{}", rest.split('\\').next().unwrap_or(""))
        } else {
            s.split(['\\', '/']).next().unwrap_or("").to_string()
        }
    };
    !norm(a).is_empty() && norm(a) == norm(b)
}

fn human(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.0} {}", UNITS[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vol(bus: BusType, system: bool) -> Volume {
        Volume {
            guid_path: r"\\?\Volume{11111111-2222-3333-4444-555555555555}\".into(),
            drive_letter: Some("E:".into()),
            label: "A7M4".into(),
            serial: Some(0xDEAD_BEEF),
            file_system: "exFAT".into(),
            total_bytes: 128 * 1024 * 1024 * 1024,
            free_bytes: 30 * 1024 * 1024 * 1024,
            bus_type: bus,
            is_system: system,
            state: VolumeState::Online,
            fingerprints: vec!["影像设备卡".into()],
        }
    }

    // spec: device-registry → 源卡准入判据 → Scenario: 被判成固定磁盘的 CFexpress 仍可作为源
    #[test]
    fn scenario_device_registry_cfexpress_reported_as_fixed_is_still_a_source() {
        // 关键：这张卡的「可移动」标志位是假的（模型里根本没这个字段——
        // 我们刻意不采信它），但总线是 USB，所以准入
        let v = vol(BusType::Usb, false);
        assert!(v.can_be_source(&[]), "USB 总线的卷必须能作为源");
    }

    // spec: → Scenario: 内置固定盘不被当作源卡
    #[test]
    fn scenario_device_registry_internal_disk_is_not_a_source() {
        for bus in [BusType::Nvme, BusType::Sata, BusType::Scsi, BusType::Network] {
            assert!(
                !vol(bus, false).can_be_source(&[]),
                "{} 总线不该被当作源",
                bus.label()
            );
        }
    }

    // spec: → Scenario: 系统盘不被当作源卡
    #[test]
    fn scenario_device_registry_system_volume_is_never_a_source() {
        let mut v = vol(BusType::Usb, true);
        assert!(!v.can_be_source(&[]), "系统盘绝不可作为源");
        v.is_system = false;
        assert!(v.can_be_source(&[]));
    }

    #[test]
    fn scenario_device_registry_destination_volume_is_not_a_source() {
        let v = vol(BusType::Usb, false);
        let dests = vec![PathBuf::from(r"E:\素材\备份")];
        assert!(
            !v.can_be_source(&dests),
            "已配置为目的地的卷 MUST NOT 同时作为源"
        );
        assert!(v.can_be_source(&[PathBuf::from(r"F:\别处")]));
    }

    // spec: → Scenario: 空卡仍可作为源
    #[test]
    fn scenario_device_registry_blank_card_is_still_a_source() {
        let mut v = vol(BusType::Usb, false);
        v.fingerprints.clear();
        assert!(
            v.can_be_source(&[]),
            "没有设备指纹目录的空卡仍是合法的源"
        );
    }

    #[test]
    fn scenario_device_registry_offline_or_no_media_is_not_a_source() {
        for st in [VolumeState::Offline, VolumeState::NoMedia] {
            let mut v = vol(BusType::Usb, false);
            v.state = st;
            assert!(!v.can_be_source(&[]));
        }
    }

    // spec: → 无盘符卷的访问
    #[test]
    fn scenario_device_registry_volume_without_drive_letter_is_usable() {
        let mut v = vol(BusType::Usb, false);
        v.drive_letter = None;
        assert!(v.can_be_source(&[]), "无盘符的卷仍应可用");
        assert_eq!(v.root_path(), PathBuf::from(&v.guid_path));
        // 界面上要能辨认它
        let name = v.display_name();
        assert!(name.contains("无盘符"), "应提示无盘符：{name}");
        assert!(name.contains("A7M4"));
    }

    #[test]
    fn scenario_device_registry_composite_id_survives_drive_letter_change() {
        let a = vol(BusType::Usb, false);
        let mut b = vol(BusType::Usb, false);
        b.drive_letter = Some("H:".into()); // 换了盘符
        assert_eq!(a.composite_id(), b.composite_id(), "盘符变化不应改变身份");
    }

    #[test]
    fn scenario_device_registry_composite_id_differs_for_different_cards() {
        // 同容量、同卷标，但序列号不同 → 必须是两个设备
        let a = vol(BusType::Usb, false);
        let mut b = vol(BusType::Usb, false);
        b.serial = Some(0x1234_5678);
        assert_ne!(
            a.composite_id(),
            b.composite_id(),
            "同容量不同卡 MUST NOT 混淆"
        );
    }

    #[test]
    fn scenario_device_registry_bus_externality() {
        assert!(BusType::Usb.is_external());
        assert!(BusType::Thunderbolt.is_external());
        assert!(BusType::Sd.is_external());
        assert!(BusType::Mmc.is_external());
        for b in [
            BusType::Nvme,
            BusType::Sata,
            BusType::Scsi,
            BusType::Network,
            BusType::Other,
            BusType::Unknown,
        ] {
            assert!(!b.is_external(), "{} 不该被当作外接总线", b.label());
        }
    }
}
