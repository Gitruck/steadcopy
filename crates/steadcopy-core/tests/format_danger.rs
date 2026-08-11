#![allow(clippy::unwrap_used, clippy::expect_used)]
//! ⚠️ **危险轨**：会真正格式化目标卷的测试。
//!
//! 规范：`openspec/changes/add-steadcopy-format-card/specs/format-card/spec.md`
//! → Requirement: 危险轨测试隔离
//! 制度：`openspec/README.md` §三 双轨约束制
//! 登记：`docs/danger-tests.md`
//!
//! # 铁律
//!
//! **这些测试 MUST NOT 在开发机上执行。** 它们在专用虚拟机里跑，靶标是一块
//! 上面没有任何需要保留的数据的专用虚拟磁盘。
//!
//! 三重闸门缺一不可：
//! 1. `#[ignore]` —— `cargo test` 默认不跑；
//! 2. `STEADCOPY_DANGER_TESTS=1` —— 未设则**跳过**；
//! 3. `STEADCOPY_DANGER_TARGET=<卷>` —— 未设则**中止**（不是跳过：设了闸门 2 却没设靶标，
//!    说明运行者以为自己在跑危险测试但配置有误，这种状态必须响）；
//!    靶标是系统盘或本机固定盘同样**中止**。

use steadcopy_core::device::{enumerate_volumes, formatter, Volume};

/// 三重闸门里的第 2、3 道。返回靶标卷。
///
/// 返回 `None` 表示闸门 2 未开——调用方应直接返回（跳过）。
/// 闸门 3 不满足时**直接 panic**，绝不静默放行。
fn danger_guard() -> Option<Volume> {
    if std::env::var("STEADCOPY_DANGER_TESTS").as_deref() != Ok("1") {
        eprintln!(
            "跳过：危险轨测试需要 STEADCOPY_DANGER_TESTS=1。\
             这些测试会格式化磁盘，只应在专用虚拟机中运行，见 docs/danger-tests.md"
        );
        return None;
    }

    let target = std::env::var("STEADCOPY_DANGER_TARGET").unwrap_or_else(|_| {
        panic!(
            "已开启 STEADCOPY_DANGER_TESTS 但没有指定 STEADCOPY_DANGER_TARGET。\
             这属于配置有误，绝不放行——请指定靶标卷（卷 GUID 或盘符）"
        )
    });

    let vols = enumerate_volumes().expect("枚举卷");
    let vol = vols
        .into_iter()
        .find(|v| {
            v.guid_path.eq_ignore_ascii_case(&target)
                || v.drive_letter.as_deref().map(str::to_ascii_uppercase)
                    == Some(target.to_ascii_uppercase())
        })
        .unwrap_or_else(|| panic!("找不到靶标卷：{target}"));

    // 闸门 3 的两条硬拦截
    assert!(
        !vol.is_system,
        "靶标 {target} 是系统盘。中止——这个测试会把它格掉"
    );
    assert!(
        vol.bus_type.is_external(),
        "靶标 {target} 不在外接总线上（{}），疑似本机固定盘。中止",
        vol.bus_type.label()
    );

    Some(vol)
}

/// 登记：docs/danger-tests.md · D-002
#[test]
#[ignore = "danger: 会格式化目标卷，仅限虚拟机，见 docs/danger-tests.md"]
fn scenario_format_card_wipes_target_volume() {
    let Some(vol) = danger_guard() else { return };
    let f = formatter();
    let root = vol.root_path();

    // 先放几个文件进去，格式化后必须消失
    let probe = root.join("稳拷危险轨探针.txt");
    std::fs::write(&probe, b"this file must be gone after format").expect("写探针文件");
    assert!(probe.exists());

    let params = f.read_params(&vol.root_path().display().to_string()).expect("读卷参数");
    f.quick_format(&params).expect("格式化");

    assert!(!probe.exists(), "格式化后探针文件 MUST 已不存在");
}

/// 登记：docs/danger-tests.md · D-003
#[test]
#[ignore = "danger: 会格式化目标卷，仅限虚拟机，见 docs/danger-tests.md"]
fn scenario_format_card_preserves_filesystem_and_label() {
    let Some(vol) = danger_guard() else { return };
    let f = formatter();
    let before = f
        .read_params(&vol.root_path().display().to_string())
        .expect("格式化前读参数");

    f.quick_format(&before).expect("格式化");

    let after = f
        .read_params(&vol.root_path().display().to_string())
        .expect("格式化后读参数");
    assert_eq!(
        after.file_system, before.file_system,
        "文件系统 MUST 原样重建——相机对它有要求"
    );
    assert_eq!(after.label, before.label, "卷标 MUST 原样重建");
}

/// 登记：docs/danger-tests.md · D-004
#[test]
#[ignore = "danger: 会操作物理卷，仅限虚拟机，见 docs/danger-tests.md"]
fn scenario_format_card_failure_is_reported_readably() {
    let Some(_vol) = danger_guard() else { return };
    let f = formatter();
    // 不存在的卷：MUST 报可读错误，MUST NOT panic、MUST NOT 静默成功
    let err = f
        .read_params(r"\\?\Volume{00000000-0000-0000-0000-000000000000}")
        .expect_err("不存在的卷 MUST 报错");
    let msg = err.to_string();
    assert!(!msg.is_empty());
    assert!(!msg.is_ascii(), "错误应为中文人话：{msg}");
}

// ---- 以下是**安全轨**：验证闸门本身的行为，不碰任何真实卷 ----

#[test]
fn scenario_format_card_gate_skips_without_env() {
    // 本机默认不设 STEADCOPY_DANGER_TESTS，闸门应返回 None（跳过）而不是放行
    if std::env::var("STEADCOPY_DANGER_TESTS").as_deref() == Ok("1") {
        eprintln!("跳过本条：当前环境已开启危险轨");
        return;
    }
    assert!(
        danger_guard().is_none(),
        "未开启危险轨时 MUST 返回 None，绝不放行"
    );
}
