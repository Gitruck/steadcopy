//! 格式化：安全前置检查链与编排。
//!
//! 规范：`openspec/changes/add-steadcopy-format-card/specs/format-card/spec.md`
//!
//! # 设计立场
//!
//! 这个功能的设计目标不是「好用」，是**难以误用**。任何在便利性与安全性之间的取舍，
//! 一律倒向安全性。判据：一个疲惫的人在凌晨三点收工时误点一下，不应该丢掉今天的素材。
//!
//! # 前身在 G1 出过的事
//!
//! 前身的写法是「查本机固定盘列表 → 目标不在其中则判定为可移动 → 放行」。
//! 它把 WMI 属性名拼错导致查询恒空，于是**任何盘都「不在固定盘列表中」，全部放行**。
//! README 里那句「可能把资料全扬了」不是假设性警告，是对一个现行 bug 的描述。
//!
//! 本模块的对策是把默认值反过来：判定返回 `Result` 而非 `bool`，
//! **积极证明**目标可移除，查不到证据一律拒绝。

use serde::{Deserialize, Serialize};

use crate::device::Volume;
use crate::manifest::Manifest;

/// 可移除性的判定结论。
///
/// 刻意**不是 bool**——「查不到」必须是一个独立于「是/否」的状态，
/// 而不是被悄悄折叠成其中之一。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removability {
    /// 已积极证明可移除（外接总线）
    Provable,
    /// 已证明**不可**移除（内置盘 / 系统盘）
    ProvablyFixed,
}

/// 判定失败的原因。**任何一种都走拒绝分支。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemovabilityError {
    /// 底层查询报错
    QueryFailed(String),
    /// 查询成功但没有可判定的信息
    Indeterminate,
}

/// 积极证明一个卷是否可移除。
///
/// **MUST NOT** 采用「不在固定盘列表中即视为可移动」的反向排除。
/// 查询失败、结果为空、无法明确判定，一律返回 `Err`，调用方一律拒绝。
pub fn removability(vol: &Volume) -> Result<Removability, RemovabilityError> {
    use crate::device::BusType;
    if vol.bus_type.is_external() {
        // 积极证据：外接总线
        return Ok(Removability::Provable);
    }
    match vol.bus_type {
        // 积极反证：内置总线
        BusType::Nvme | BusType::Sata | BusType::Scsi => Ok(Removability::ProvablyFixed),
        // 查不到 / 说不清 —— **不是「可能可以」，是「不知道」**
        BusType::Unknown => Err(RemovabilityError::QueryFailed(
            "无法查询该卷所在设备的总线类型".into(),
        )),
        // 外接分支已在上面返回，此处只剩不可判定的
        _ => Err(RemovabilityError::Indeterminate),
    }
}

/// 前置检查的编号与结论。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: String,
    pub passed: bool,
    pub detail: String,
}

/// 全部前置检查的结论。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyReport {
    pub checks: Vec<CheckResult>,
    /// 用于 G4 的备份任务标识（若通过）
    pub backup_task_id: Option<String>,
}

impl SafetyReport {
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }

    /// 第一条未通过的检查。界面据此告诉用户**具体卡在哪一步**。
    pub fn first_failure(&self) -> Option<&CheckResult> {
        self.checks.iter().find(|c| !c.passed)
    }

    /// 留痕用的紧凑字符串。
    pub fn compact(&self) -> String {
        self.checks
            .iter()
            .map(|c| format!("{}={}", c.id, if c.passed { "ok" } else { "fail" }))
            .collect::<Vec<_>>()
            .join(";")
    }
}

/// G4 的输入：该设备最近一次成功且全部校验通过的备份。
#[derive(Debug, Clone)]
pub struct BackupEvidence {
    pub task_id: String,
    /// 那次任务落下的清单（合并后的已校验条目路径集合）
    pub manifest: Manifest,
}

/// 跑一遍前置检查链。
///
/// `current_files` 是卡上**现在**的文件相对路径集合（由扫描得到）。
/// `destination_roots` 是全部已配置目的地的根。
pub fn check_safety(
    vol: &Volume,
    destination_roots: &[std::path::PathBuf],
    device_busy: bool,
    evidence: Option<&BackupEvidence>,
    current_files: &[String],
) -> SafetyReport {
    let mut checks = Vec::new();

    // G1：积极证明可移除。**查不到 = 拒绝。**
    let g1 = match removability(vol) {
        Ok(Removability::Provable) if !vol.is_system => CheckResult {
            id: "G1".into(),
            passed: true,
            detail: format!("已确认为可移除介质（{} 总线）", vol.bus_type.label()),
        },
        Ok(Removability::Provable) => CheckResult {
            id: "G1".into(),
            passed: false,
            detail: "这是系统盘".into(),
        },
        Ok(Removability::ProvablyFixed) => CheckResult {
            id: "G1".into(),
            passed: false,
            detail: format!("这是本机固定盘（{} 总线）", vol.bus_type.label()),
        },
        Err(e) => CheckResult {
            id: "G1".into(),
            passed: false,
            // 查不到就是不安全——不给「可能可以」留缝
            detail: match e {
                RemovabilityError::QueryFailed(m) => {
                    format!("无法确认这是不是可移除介质（{m}），出于安全拒绝")
                }
                RemovabilityError::Indeterminate => {
                    "无法确认这是不是可移除介质，出于安全拒绝".into()
                }
            },
        },
    };
    checks.push(g1);

    // G2：不能是任一目的地所在的卷
    let is_dest = vol.is_any_destination(destination_roots);
    checks.push(CheckResult {
        id: "G2".into(),
        passed: !is_dest,
        detail: if is_dest {
            "这个卷是已配置的备份目的地".into()
        } else {
            "不是任何已配置的目的地".into()
        },
    });

    // G3：设备上没有任务在跑
    checks.push(CheckResult {
        id: "G3".into(),
        passed: !device_busy,
        detail: if device_busy {
            "该设备上还有任务在进行".into()
        } else {
            "设备空闲".into()
        },
    });

    // G4：有覆盖当前全部内容的、已校验通过的备份
    let (g4_passed, g4_detail, backup_task_id) = match evidence {
        None => (
            false,
            "找不到这张卡已完成且校验通过的备份记录".to_string(),
            None,
        ),
        Some(ev) => {
            let covered: std::collections::HashSet<&str> = ev
                .manifest
                .entries
                .iter()
                .filter(|e| e.counts_as_done())
                .map(|e| e.relative_path.as_str())
                .collect();
            let uncovered: Vec<&String> = current_files
                .iter()
                .filter(|f| !covered.contains(f.as_str()))
                .collect();
            if uncovered.is_empty() {
                (
                    true,
                    format!(
                        "已备份并校验通过（{} 个文件，{}）",
                        covered.len(),
                        ev.task_id
                    ),
                    Some(ev.task_id.clone()),
                )
            } else {
                (
                    false,
                    format!(
                        "卡上有 {} 个文件不在已校验的备份里（例如 {}）",
                        uncovered.len(),
                        uncovered
                            .first()
                            .map(|s| s.as_str())
                            .unwrap_or("")
                    ),
                    None,
                )
            }
        }
    };
    checks.push(CheckResult {
        id: "G4".into(),
        passed: g4_passed,
        detail: g4_detail,
    });

    SafetyReport {
        checks,
        backup_task_id,
    }
}

/// 倒计时配置。P4 决策：默认 30 秒、可配、**最小 10 秒**。
pub const COUNTDOWN_DEFAULT_SECS: u32 = 30;
pub const COUNTDOWN_MIN_SECS: u32 = 10;

/// 校准用户配的倒计时秒数。低于下限 MUST 被拒绝。
pub fn validate_countdown(secs: u32) -> Result<u32, String> {
    if secs < COUNTDOWN_MIN_SECS {
        return Err(format!(
            "倒计时不能短于 {COUNTDOWN_MIN_SECS} 秒（你设的是 {secs} 秒）"
        ));
    }
    Ok(secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{BusType, VolumeState};
    use crate::engine::{hash_bytes, HashAlgorithm};
    use crate::manifest::model::{ManifestEntry, SourceRef, VerifyState};
    use std::path::PathBuf;
    use time::macros::datetime;

    fn vol(bus: BusType, system: bool) -> Volume {
        Volume {
            guid_path: r"\\?\Volume{1111}\".into(),
            drive_letter: Some("E:".into()),
            label: "A7M4".into(),
            serial: Some(1),
            file_system: "exFAT".into(),
            total_bytes: 128,
            free_bytes: 30,
            bus_type: bus,
            is_system: system,
            state: VolumeState::Online,
            fingerprints: vec![],
        }
    }

    fn evidence(files: &[&str], verified: bool) -> BackupEvidence {
        let at = datetime!(2026-08-10 09:00:00 UTC);
        let mut m = Manifest::new(
            SourceRef {
                id: "vol:1".into(),
                display_name: "A7M4".into(),
            },
            "婚礼",
            r"D:\素材",
            HashAlgorithm::Xxh64,
            at,
        );
        for f in files {
            let h = hash_bytes(HashAlgorithm::Xxh64, f.as_bytes());
            m.entries.push(ManifestEntry {
                relative_path: (*f).into(),
                size: 1,
                source_hash: h,
                verify: if verified {
                    VerifyState::Verified {
                        destination_hash: h,
                    }
                } else {
                    VerifyState::NotVerified
                },
                source_modified_at: None,
                completed_at: at,
                retries: 0,
            });
        }
        BackupEvidence {
            task_id: "task-1".into(),
            manifest: m,
        }
    }

    // spec: format-card → 可移动性判定采用正向证明且失败即拒绝
    // → Scenario: 查询失败判定为拒绝  ← **这是前身出事的那一行**
    #[test]
    fn scenario_format_card_query_failure_is_rejected() {
        let v = vol(BusType::Unknown, false);
        assert!(
            matches!(removability(&v), Err(RemovabilityError::QueryFailed(_))),
            "查不到总线类型 MUST 是 Err，不能被折叠成「可移动」"
        );
        let r = check_safety(&v, &[], false, None, &[]);
        let g1 = &r.checks[0];
        assert!(!g1.passed, "查询失败 MUST 判定为拒绝");
        assert!(g1.detail.contains("无法确认"), "{}", g1.detail);
        assert!(!r.passed());
    }

    // spec: → Scenario: 查询返回空结果判定为拒绝
    #[test]
    fn scenario_format_card_indeterminate_is_rejected() {
        for bus in [BusType::Other, BusType::Network] {
            let v = vol(bus, false);
            assert!(matches!(
                removability(&v),
                Err(RemovabilityError::Indeterminate)
            ));
            assert!(!check_safety(&v, &[], false, None, &[]).checks[0].passed);
        }
    }

    #[test]
    fn scenario_format_card_removability_returns_result_not_bool() {
        // 类型层面的证明：三种结论互不折叠
        assert_eq!(removability(&vol(BusType::Usb, false)), Ok(Removability::Provable));
        assert_eq!(
            removability(&vol(BusType::Nvme, false)),
            Ok(Removability::ProvablyFixed)
        );
        assert!(removability(&vol(BusType::Unknown, false)).is_err());
    }

    // spec: → Scenario: 系统盘被拒
    #[test]
    fn scenario_format_card_system_disk_rejected() {
        // 即便总线是 USB（外接系统盘也存在），系统盘一律拒
        let r = check_safety(&vol(BusType::Usb, true), &[], false, None, &[]);
        assert!(!r.checks[0].passed);
        assert!(r.checks[0].detail.contains("系统盘"));
    }

    #[test]
    fn scenario_format_card_internal_disk_rejected() {
        for bus in [BusType::Nvme, BusType::Sata, BusType::Scsi] {
            let r = check_safety(&vol(bus, false), &[], false, None, &[]);
            assert!(!r.checks[0].passed, "{} 总线应被拒", bus.label());
            assert!(r.checks[0].detail.contains("固定盘"));
        }
    }

    // spec: → 安全前置检查链 → Scenario: 目的地所在卷被拒
    #[test]
    fn scenario_format_card_destination_volume_rejected() {
        let v = vol(BusType::Usb, false);
        let dests = vec![PathBuf::from(r"E:\素材\备份")];
        let r = check_safety(&v, &dests, false, Some(&evidence(&[], true)), &[]);
        let g2 = r.checks.iter().find(|c| c.id == "G2").expect("G2");
        assert!(!g2.passed);
        assert!(g2.detail.contains("目的地"));
        assert_eq!(r.first_failure().map(|c| c.id.as_str()), Some("G2"));
    }

    #[test]
    fn scenario_format_card_busy_device_rejected() {
        let r = check_safety(&vol(BusType::Usb, false), &[], true, Some(&evidence(&[], true)), &[]);
        let g3 = r.checks.iter().find(|c| c.id == "G3").expect("G3");
        assert!(!g3.passed);
        assert!(g3.detail.contains("任务"));
    }

    // spec: → Scenario: 任一检查不过即拒绝（G4：备份未覆盖当前全部内容）
    #[test]
    fn scenario_format_card_backup_must_cover_current_content() {
        let v = vol(BusType::Usb, false);
        let ev = evidence(&["A001.MP4", "A002.MP4"], true);

        // 卡上内容正是备份过的 → 通过
        let ok = check_safety(
            &v,
            &[],
            false,
            Some(&ev),
            &["A001.MP4".into(), "A002.MP4".into()],
        );
        assert!(ok.passed(), "{:?}", ok.first_failure());
        assert_eq!(ok.backup_task_id.as_deref(), Some("task-1"));

        // 拷完之后又拍了一条 → G4 不通过
        let stale = check_safety(
            &v,
            &[],
            false,
            Some(&ev),
            &["A001.MP4".into(), "A002.MP4".into(), "A003.MP4".into()],
        );
        assert!(!stale.passed());
        let g4 = stale.checks.iter().find(|c| c.id == "G4").expect("G4");
        assert!(g4.detail.contains("A003.MP4"), "{}", g4.detail);
        assert!(stale.backup_task_id.is_none());
    }

    #[test]
    fn scenario_format_card_unverified_backup_does_not_count() {
        let v = vol(BusType::Usb, false);
        // 备份存在但当初没校验 → 不作数
        let ev = evidence(&["A001.MP4"], false);
        let r = check_safety(&v, &[], false, Some(&ev), &["A001.MP4".into()]);
        assert!(!r.passed(), "未校验的备份 MUST NOT 作为格式化依据");
    }

    #[test]
    fn scenario_format_card_no_backup_rejected() {
        let r = check_safety(&vol(BusType::Usb, false), &[], false, None, &["A001.MP4".into()]);
        let g4 = r.checks.iter().find(|c| c.id == "G4").expect("G4");
        assert!(!g4.passed);
        assert!(g4.detail.contains("备份"));
    }

    #[test]
    fn scenario_format_card_checks_run_in_order_and_report_which_failed() {
        let report = check_safety(&vol(BusType::Usb, false), &[], false, None, &[]);
        let ids: Vec<&str> = report.checks.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["G1", "G2", "G3", "G4"], "检查应按序全跑并各自留结论");
    }

    #[test]
    fn scenario_format_card_compact_trace_for_audit() {
        let r = check_safety(&vol(BusType::Unknown, false), &[], false, None, &[]);
        let c = r.compact();
        assert!(c.contains("G1=fail"), "{c}");
        assert!(c.contains("G2=ok"), "{c}");
    }

    // spec: → 倒计时参数
    #[test]
    fn scenario_format_card_countdown_bounds() {
        assert_eq!(COUNTDOWN_DEFAULT_SECS, 30);
        assert_eq!(COUNTDOWN_MIN_SECS, 10);
        assert_eq!(validate_countdown(30), Ok(30));
        assert_eq!(validate_countdown(10), Ok(10));
        assert_eq!(validate_countdown(300), Ok(300));
        let err = validate_countdown(5).expect_err("低于下限应被拒");
        assert!(err.contains("10"), "{err}");
    }
}
