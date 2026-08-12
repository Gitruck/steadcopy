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
use crate::i18n::Locale;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovabilityError {
    /// 底层查不出这个卷所在设备的总线类型
    QueryFailed,
    /// 查询成功但没有可判定的信息
    Indeterminate,
}

impl RemovabilityError {
    /// 拒绝理由，跟随语言。
    ///
    /// 两条都落在「查不到 = 不安全」上——措辞刻意不留「可能可以」的缝，
    /// 这是前身在 G1 上出事的那一处。
    pub const fn describe(self, lang: Locale) -> &'static str {
        match self {
            RemovabilityError::QueryFailed => lang.pick(
                "无法确认这是不是可移除介质（查不到总线类型），出于安全拒绝",
                "Could not confirm this is removable media (bus type unavailable) — refusing, to be safe",
            ),
            RemovabilityError::Indeterminate => lang.pick(
                "无法确认这是不是可移除介质，出于安全拒绝",
                "Could not confirm this is removable media — refusing, to be safe",
            ),
        }
    }
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
        BusType::Unknown => Err(RemovabilityError::QueryFailed),
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
///
/// `lang` **只决定 `detail` 怎么写**。每一条的 `passed` 都由上面那些判据算出来，
/// 没有一处读 `lang`——安全结论随界面语言变，是这个模块最不能出的事。
pub fn check_safety(
    vol: &Volume,
    destination_roots: &[std::path::PathBuf],
    device_busy: bool,
    evidence: Option<&BackupEvidence>,
    current_files: &[String],
    lang: Locale,
) -> SafetyReport {
    let mut checks = Vec::new();

    // G1：积极证明可移除。**查不到 = 拒绝。**
    let g1 = match removability(vol) {
        Ok(Removability::Provable) if !vol.is_system => CheckResult {
            id: "G1".into(),
            passed: true,
            detail: match lang {
                Locale::Zh => format!("已确认为可移除介质（{} 总线）", vol.bus_type.label(lang)),
                Locale::En => {
                    format!("Confirmed removable media ({} bus)", vol.bus_type.label(lang))
                }
            },
        },
        Ok(Removability::Provable) => CheckResult {
            id: "G1".into(),
            passed: false,
            detail: lang.pick("这是系统盘", "This is the system disk").into(),
        },
        Ok(Removability::ProvablyFixed) => CheckResult {
            id: "G1".into(),
            passed: false,
            detail: match lang {
                Locale::Zh => format!("这是本机固定盘（{} 总线）", vol.bus_type.label(lang)),
                Locale::En => format!(
                    "This is an internal fixed disk ({} bus)",
                    vol.bus_type.label(lang)
                ),
            },
        },
        // 查不到就是不安全——不给「可能可以」留缝
        Err(e) => CheckResult {
            id: "G1".into(),
            passed: false,
            detail: e.describe(lang).into(),
        },
    };
    checks.push(g1);

    // G2：不能是任一目的地所在的卷
    let is_dest = vol.is_any_destination(destination_roots);
    checks.push(CheckResult {
        id: "G2".into(),
        passed: !is_dest,
        detail: if is_dest {
            lang.pick(
                "这个卷是已配置的备份目的地",
                "This volume is a configured backup destination",
            )
        } else {
            lang.pick(
                "不是任何已配置的目的地",
                "Not one of the configured destinations",
            )
        }
        .into(),
    });

    // G3：设备上没有任务在跑
    checks.push(CheckResult {
        id: "G3".into(),
        passed: !device_busy,
        detail: if device_busy {
            lang.pick(
                "该设备上还有任务在进行",
                "A task is still running on this device",
            )
        } else {
            lang.pick("设备空闲", "The device is idle")
        }
        .into(),
    });

    // G4：有覆盖当前全部内容的、已校验通过的备份
    let (g4_passed, g4_detail, backup_task_id) = match evidence {
        None => (
            false,
            lang.pick(
                "找不到这张卡已完成且校验通过的备份记录",
                "No completed, verified backup of this card was found",
            )
            .to_string(),
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
                let n = covered.len();
                let task = &ev.task_id;
                (
                    true,
                    match lang {
                        Locale::Zh => format!("已备份并校验通过（{n} 个文件，{task}）"),
                        Locale::En => format!("Backed up and verified ({n} files, {task})"),
                    },
                    Some(ev.task_id.clone()),
                )
            } else {
                let n = uncovered.len();
                // 举一个例子就够——列全了反而看不出重点
                let sample = uncovered.first().map(|s| s.as_str()).unwrap_or("");
                (
                    false,
                    match lang {
                        Locale::Zh => format!("卡上有 {n} 个文件不在已校验的备份里（例如 {sample}）"),
                        Locale::En => format!(
                            "{n} file(s) on the card are not in the verified backup (e.g. {sample})"
                        ),
                    },
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
///
/// `lang` 只影响被拒时那句话；下限判定与它无关。
pub fn validate_countdown(secs: u32, lang: Locale) -> Result<u32, String> {
    if secs < COUNTDOWN_MIN_SECS {
        return Err(match lang {
            Locale::Zh => format!("倒计时不能短于 {COUNTDOWN_MIN_SECS} 秒（你设的是 {secs} 秒）"),
            Locale::En => format!(
                "The countdown cannot be shorter than {COUNTDOWN_MIN_SECS}s (you set {secs}s)"
            ),
        });
    }
    Ok(secs)
}

/// 无卷标的卡改用这个词做确认。
///
/// 直接拿空卷标当确认串，「请输入卷标」会退化成「直接回车」——
/// 摩擦归零，这一重确认等于不存在。无名卡在实际拍摄里很常见。
///
/// **这个词不随语言变**，哪怕界面是英文。它是 [`label_matches`] 的判据输入，
/// 跟着语言变就意味着同一张卡在两种语言下要输不同的词——那是判定随 locale 变。
/// 提示语会把它原样摆在屏幕上，照抄即可。
pub const BLANK_LABEL_PHRASE: &str = "格式化";

/// 这张卡要用户手输的确认串。有卷标就是卷标，没有就是固定词。
pub fn confirmation_phrase(label: &str) -> &str {
    let t = label.trim();
    if t.is_empty() {
        BLANK_LABEL_PHRASE
    } else {
        t
    }
}

/// 手输的确认串是否对得上。
///
/// 规则只有一处实现。命令行与界面都调它——同一个安全判据在两处各写一遍，
/// 迟早有一处漏掉 `trim` 或漏掉空卷标的情况，而那一处就是数据被误抹的入口。
///
/// 大小写**必须**严格匹配：卷标就在屏幕上摆着，抄一遍是刻意的摩擦，
/// 放宽到不分大小写等于把摩擦削掉一半。
pub fn label_matches(typed: &str, actual: &str) -> bool {
    typed.trim() == confirmation_phrase(actual)
}

/// 「拷完自动格式化」的判定结论。
///
/// 是枚举不是 bool——没触发的原因必须能说出来。用户开了开关却没被提议格式化时，
/// 「为什么没弹」比「弹不弹」更需要答案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoFormatDecision {
    /// 开关没开（默认状态）
    Disabled,
    /// 任务被取消
    Cancelled,
    /// 有文件失败
    HasFailures,
    /// 本次没开校验——没校验过就不算「确认拷好了」
    NotVerified,
    /// 有目的地没落下凭证，说明没写完
    DestinationIncomplete,
    /// 可以提议格式化。**仍然要走完整的安全链与倒计时确认。**
    Propose,
}

impl AutoFormatDecision {
    /// 「为什么没提议格式化」，跟随语言。
    pub const fn reason(&self, lang: Locale) -> &'static str {
        match self {
            AutoFormatDecision::Disabled => {
                lang.pick("「拷完自动格式化」未开启", "\"Format after copy\" is off")
            }
            AutoFormatDecision::Cancelled => lang.pick(
                "任务被取消，不提议格式化",
                "The task was cancelled, so no format is proposed",
            ),
            AutoFormatDecision::HasFailures => lang.pick(
                "有文件拷贝失败，不提议格式化",
                "Some files failed to copy, so no format is proposed",
            ),
            AutoFormatDecision::NotVerified => lang.pick(
                "本次未做读回校验，不提议格式化",
                "No read-back verification this run, so no format is proposed",
            ),
            AutoFormatDecision::DestinationIncomplete => lang.pick(
                "有目的地未落下凭证，不提议格式化",
                "A destination has no manifest, so no format is proposed",
            ),
            AutoFormatDecision::Propose => lang.pick(
                "全部目的地完成且全部校验通过",
                "Every destination finished and everything verified",
            ),
        }
    }
}

/// 一次任务跑完之后，判断能不能提议格式化源卡。
///
/// 判据一律取「实际发生了什么」，不取「本来打算怎么做」：
/// 开关开着但这次关了校验、或者少写了一个目的地，都不算数。
pub fn decide_auto_format(
    enabled: bool,
    verified_this_run: bool,
    cancelled: bool,
    failed_files: usize,
    manifests_written: usize,
    destinations_planned: usize,
) -> AutoFormatDecision {
    if !enabled {
        return AutoFormatDecision::Disabled;
    }
    if cancelled {
        return AutoFormatDecision::Cancelled;
    }
    if failed_files > 0 {
        return AutoFormatDecision::HasFailures;
    }
    if !verified_this_run {
        return AutoFormatDecision::NotVerified;
    }
    if destinations_planned == 0 || manifests_written < destinations_planned {
        return AutoFormatDecision::DestinationIncomplete;
    }
    AutoFormatDecision::Propose
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{BusType, VolumeState};
    use crate::i18n::Locale;
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
            matches!(removability(&v), Err(RemovabilityError::QueryFailed)),
            "查不到总线类型 MUST 是 Err，不能被折叠成「可移动」"
        );
        let r = check_safety(&v, &[], false, None, &[], Locale::Zh);
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
            assert!(!check_safety(&v, &[], false, None, &[], Locale::Zh).checks[0].passed);
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
        let r = check_safety(&vol(BusType::Usb, true), &[], false, None, &[], Locale::Zh);
        assert!(!r.checks[0].passed);
        assert!(r.checks[0].detail.contains("系统盘"));
    }

    #[test]
    fn scenario_format_card_internal_disk_rejected() {
        for bus in [BusType::Nvme, BusType::Sata, BusType::Scsi] {
            let r = check_safety(&vol(bus, false), &[], false, None, &[], Locale::Zh);
            assert!(!r.checks[0].passed, "{} 总线应被拒", bus.label(Locale::Zh));
            assert!(r.checks[0].detail.contains("固定盘"));
        }
    }

    // spec: → 安全前置检查链 → Scenario: 目的地所在卷被拒
    #[test]
    fn scenario_format_card_destination_volume_rejected() {
        let v = vol(BusType::Usb, false);
        let dests = vec![PathBuf::from(r"E:\素材\备份")];
        let r = check_safety(&v, &dests, false, Some(&evidence(&[], true)), &[], Locale::Zh);
        let g2 = r.checks.iter().find(|c| c.id == "G2").expect("G2");
        assert!(!g2.passed);
        assert!(g2.detail.contains("目的地"));
        assert_eq!(r.first_failure().map(|c| c.id.as_str()), Some("G2"));
    }

    #[test]
    fn scenario_format_card_busy_device_rejected() {
        let r = check_safety(&vol(BusType::Usb, false), &[], true, Some(&evidence(&[], true)), &[], Locale::Zh);
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
            Locale::Zh,
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
            Locale::Zh,
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
        let r = check_safety(&v, &[], false, Some(&ev), &["A001.MP4".into()], Locale::Zh);
        assert!(!r.passed(), "未校验的备份 MUST NOT 作为格式化依据");
    }

    #[test]
    fn scenario_format_card_no_backup_rejected() {
        let r = check_safety(&vol(BusType::Usb, false), &[], false, None, &["A001.MP4".into()], Locale::Zh);
        let g4 = r.checks.iter().find(|c| c.id == "G4").expect("G4");
        assert!(!g4.passed);
        assert!(g4.detail.contains("备份"));
    }

    #[test]
    fn scenario_format_card_checks_run_in_order_and_report_which_failed() {
        let report = check_safety(&vol(BusType::Usb, false), &[], false, None, &[], Locale::Zh);
        let ids: Vec<&str> = report.checks.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["G1", "G2", "G3", "G4"], "检查应按序全跑并各自留结论");
    }

    #[test]
    fn scenario_format_card_compact_trace_for_audit() {
        let r = check_safety(&vol(BusType::Unknown, false), &[], false, None, &[], Locale::Zh);
        let c = r.compact();
        assert!(c.contains("G1=fail"), "{c}");
        assert!(c.contains("G2=ok"), "{c}");
    }

    // spec: → 卷标输入不匹配则不可执行
    #[test]
    fn scenario_format_card_typed_label_must_match() {
        assert!(label_matches("A7M4", "A7M4"));
        // 前后空白无所谓——用户从界面复制常带空格
        assert!(label_matches("  A7M4 ", "A7M4"));
        assert!(label_matches("A7M4", " A7M4"));
        // 大小写必须严格：抄一遍卷标就是刻意的摩擦
        assert!(!label_matches("a7m4", "A7M4"));
        assert!(!label_matches("A7M", "A7M4"));
        assert!(!label_matches("", "A7M4"));
        assert!(label_matches("未命名", "未命名"));

        // 无卷标的卡：直接回车不算确认过，得输固定词
        assert_eq!(confirmation_phrase(""), BLANK_LABEL_PHRASE);
        assert_eq!(confirmation_phrase("  "), BLANK_LABEL_PHRASE);
        assert_eq!(confirmation_phrase(" A7M4 "), "A7M4");
        assert!(!label_matches("", "  "), "空对空放行等于这一重确认不存在");
        assert!(!label_matches("  ", ""));
        assert!(label_matches(BLANK_LABEL_PHRASE, ""));
        assert!(label_matches(BLANK_LABEL_PHRASE, "   "));
    }

    // spec: → 全绿后按时执行 / 部分失败不触发自动格式化 / 关闭校验时不触发
    #[test]
    fn scenario_format_card_auto_format_only_when_everything_is_green() {
        // 全绿：两个目的地都落了凭证、校验开着、零失败、没取消
        assert_eq!(
            decide_auto_format(true, true, false, 0, 2, 2),
            AutoFormatDecision::Propose
        );

        // 开关没开是默认状态
        assert_eq!(
            decide_auto_format(false, true, false, 0, 2, 2),
            AutoFormatDecision::Disabled
        );
        // 有失败文件
        assert_eq!(
            decide_auto_format(true, true, false, 1, 2, 2),
            AutoFormatDecision::HasFailures
        );
        // 关了校验——没校验过就不算「确认拷好了」
        assert_eq!(
            decide_auto_format(true, false, false, 0, 2, 2),
            AutoFormatDecision::NotVerified
        );
        // 被取消
        assert_eq!(
            decide_auto_format(true, true, true, 0, 2, 2),
            AutoFormatDecision::Cancelled
        );
        // 少写了一个目的地
        assert_eq!(
            decide_auto_format(true, true, false, 0, 1, 2),
            AutoFormatDecision::DestinationIncomplete
        );
        // 一个目的地都没有
        assert_eq!(
            decide_auto_format(true, true, false, 0, 0, 0),
            AutoFormatDecision::DestinationIncomplete
        );
    }

    // spec: → 不触发时原因可呈现（「为什么没弹」比「弹不弹」更需要答案）
    #[test]
    fn scenario_format_card_every_skip_reason_is_presentable() {
        for d in [
            AutoFormatDecision::Disabled,
            AutoFormatDecision::Cancelled,
            AutoFormatDecision::HasFailures,
            AutoFormatDecision::NotVerified,
            AutoFormatDecision::DestinationIncomplete,
        ] {
            assert!(
                !d.reason(Locale::Zh).is_empty(),
                "{d:?} 必须能说出没触发的原因"
            );
            assert_ne!(d, AutoFormatDecision::Propose);
        }
    }

    // spec: → 倒计时参数
    #[test]
    fn scenario_format_card_countdown_bounds() {
        assert_eq!(COUNTDOWN_DEFAULT_SECS, 30);
        assert_eq!(COUNTDOWN_MIN_SECS, 10);
        assert_eq!(validate_countdown(30, Locale::Zh), Ok(30));
        assert_eq!(validate_countdown(10, Locale::Zh), Ok(10));
        assert_eq!(validate_countdown(300, Locale::Zh), Ok(300));
        let err = validate_countdown(5, Locale::Zh).expect_err("低于下限应被拒");
        assert!(err.contains("10"), "{err}");
    }
}
