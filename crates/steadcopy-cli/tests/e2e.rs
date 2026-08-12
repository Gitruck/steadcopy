#![allow(clippy::unwrap_used, clippy::expect_used)]
//! CLI 端到端测试。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/cli-driver/spec.md`
//!
//! 纪律：E2E **MUST 经 CLI 驱动**、**MUST 断言产物**（目的地字节、清单内容、
//! 报告文件、退出码），MUST NOT 只断言「命令没报错」。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    // cargo 会把被测二进制放在与测试同级的目录
    let mut p = std::env::current_exe().expect("测试二进制路径");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join(format!("steadcopy{}", std::env::consts::EXE_SUFFIX))
}

fn run(args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .output()
        .expect("执行 steadcopy 失败")
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn make_card(root: &Path) {
    let clip = root.join("DCIM").join("100MSDCF");
    std::fs::create_dir_all(&clip).expect("建目录");
    std::fs::write(clip.join("A001.MP4"), vec![b'v'; 120_000]).expect("写");
    std::fs::write(clip.join("A001.XML"), b"<meta/>").expect("写");
    let junk = root.join("System Volume Information");
    std::fs::create_dir_all(&junk).expect("建目录");
    std::fs::write(junk.join("x.dat"), b"junk").expect("写");
}

fn manifest_of(landing: &Path) -> PathBuf {
    std::fs::read_dir(landing.join("steadcopy"))
        .expect("凭证目录")
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "json"))
        .expect("清单文件")
}

struct Env {
    _dir: tempfile::TempDir,
    card: PathBuf,
    dest: PathBuf,
    landing: PathBuf,
}

fn env() -> Env {
    let dir = tempfile::tempdir().expect("临时目录");
    let card = dir.path().join("card");
    let dest = dir.path().join("dest");
    make_card(&card);
    let landing = dest.join("测试项目").join(today()).join("测试卡");
    Env {
        _dir: dir,
        card,
        dest,
        landing,
    }
}

fn today() -> String {
    use steadcopy_core::platform::{Clock, SystemClock};
    let n = SystemClock.now();
    format!("{:04}-{:02}-{:02}", n.year(), n.month() as u8, n.day())
}

/// 断言中文文案的用例都显式钉 `--lang zh`。
///
/// 不钉的话，这些断言在系统语言是英文的机器上会红——而那是**测试**不稳，
/// 不是产品坏了。语言是这批用例的输入之一，就该写进命令行。
fn copy_args(e: &Env) -> Vec<String> {
    vec![
        "copy".into(),
        e.card.display().to_string(),
        "-d".into(),
        e.dest.display().to_string(),
        "-p".into(),
        "测试项目".into(),
        "--device".into(),
        "测试卡".into(),
        "--lang".into(),
        "zh".into(),
    ]
}

fn run_owned(args: &[String]) -> Output {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&refs)
}

// spec: cli-driver → CLI 覆盖全部引擎能力 → Scenario: 无 GUI 完成完整流程
#[test]
fn scenario_cli_driver_full_flow_without_gui() {
    let e = env();

    // scan
    let o = run(&["scan", &e.card.display().to_string(), "--lang", "zh"]);
    assert_eq!(code(&o), 0, "扫描应成功");

    // plan：零副作用
    let plan_args: Vec<String> = {
        let mut a = copy_args(&e);
        a[0] = "plan".into();
        a
    };
    let o = run_owned(&plan_args);
    assert_eq!(code(&o), 0);
    assert!(!e.dest.exists(), "plan MUST NOT 创建任何目录");

    // copy
    let o = run_owned(&copy_args(&e));
    assert_eq!(code(&o), 0, "拷贝应成功：{}", String::from_utf8_lossy(&o.stderr));

    // 断言产物：目的地文件逐字节一致
    for rel in ["DCIM/A001.MP4", "DCIM/A001.XML"] {
        let rel = rel.replace("DCIM/", "DCIM/100MSDCF/");
        let landed = e.landing.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        let src = e.card.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        assert_eq!(
            std::fs::read(&landed).expect("读落地"),
            std::fs::read(&src).expect("读源"),
            "{rel} 内容不符"
        );
    }
    // 系统垃圾不该被拷
    assert!(!e.landing.join("System Volume Information").exists());

    // 断言清单
    let mpath = manifest_of(&e.landing);
    let text = std::fs::read_to_string(&mpath).expect("读清单");
    let m: serde_json::Value = serde_json::from_str(&text).expect("解析清单");
    assert_eq!(m["entries"].as_array().expect("条目数组").len(), 2);
    assert_eq!(m["algorithm"], "xxh64");
    assert_eq!(m["project"], "测试项目");

    // 断言报告文件
    let report = mpath.with_extension("html");
    assert!(report.exists(), "拷完应自动出一份报告");
    let html = std::fs::read_to_string(&report).expect("读报告");
    assert!(html.contains("全部 2 个文件校验通过"));
    assert!(!html.contains("<script"), "报告 MUST NOT 含脚本");

    // audit
    let o = run(&["audit", &mpath.display().to_string()]);
    assert_eq!(code(&o), 0, "复验应通过");
}

// spec: → 退出码契约 → Scenario: 无新素材是成功
#[test]
fn scenario_cli_driver_no_new_source_exits_zero() {
    let e = env();
    assert_eq!(code(&run_owned(&copy_args(&e))), 0);
    let o = run_owned(&copy_args(&e));
    assert_eq!(code(&o), 0, "「无新素材」是正常结果，MUST 以 0 退出");
    let combined = format!("{}{}", stdout(&o), String::from_utf8_lossy(&o.stderr));
    assert!(combined.contains("没有新素材"));
}

// spec: → Scenario: 校验失败非零退出
#[test]
fn scenario_cli_driver_data_loss_exits_nonzero() {
    let e = env();
    assert_eq!(code(&run_owned(&copy_args(&e))), 0);
    let mpath = manifest_of(&e.landing);

    // 删掉一个已拷文件，复验应报丢失并以非零退出
    std::fs::remove_file(
        e.landing
            .join("DCIM")
            .join("100MSDCF")
            .join("A001.XML"),
    )
    .expect("删文件");

    let o = run(&["audit", &mpath.display().to_string()]);
    // 终态族：复验是对已落地数据做的，重跑同一份清单答案只会一样，
    // 标成可重试会让脚本白白重试
    assert_eq!(code(&o), 1, "有数据丢失 MUST 以非零退出，且是终态族");
}

// spec: → 人读与机读双输出 → Scenario: JSON 模式 stdout 纯净
#[test]
fn scenario_cli_driver_json_stdout_is_pure() {
    let e = env();
    let o = run(&["scan", &e.card.display().to_string(), "--json"]);
    assert_eq!(code(&o), 0);
    let s = stdout(&o);
    let v: serde_json::Value = serde_json::from_str(s.trim()).expect("stdout 必须是合法 JSON");
    assert_eq!(v["files"], 2);
    assert_eq!(v["junk_excluded"], 1);
    assert!(v["fingerprints"]
        .as_array()
        .expect("指纹数组")
        .iter()
        .any(|f| f.as_str().is_some_and(|s| s.contains("影像"))));

    // copy 的 JSON 输出同样必须可解析
    let mut args = copy_args(&e);
    args.push("--json".into());
    let o = run_owned(&args);
    assert_eq!(code(&o), 0);
    for line in stdout(&o).lines().filter(|l| !l.trim().is_empty()) {
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|e| panic!("stdout 有非 JSON 行：{line}（{e}）"));
    }
}

// spec: → Scenario: 人读模式为中文
#[test]
fn scenario_cli_driver_human_output_is_chinese() {
    let e = env();
    let o = run(&["scan", &e.card.display().to_string(), "--lang", "zh"]);
    let s = stdout(&o);
    assert!(s.contains("扫描结果"));
    assert!(s.contains("文件"));
}

// spec: → Requirement: locale 的确定与切换 → Scenario: 命令行支持单次调用的覆盖参数
#[test]
fn scenario_cli_driver_lang_switch_changes_runtime_output() {
    let e = env();

    // 同一条命令，只换 --lang，运行期输出整段换语言
    let zh = stdout(&run(&["scan", &e.card.display().to_string(), "--lang", "zh"]));
    let en = stdout(&run(&["scan", &e.card.display().to_string(), "--lang", "en"]));
    assert!(zh.contains("扫描结果"), "{zh}");
    assert!(en.contains("Scan result"), "{en}");
    assert!(!en.contains("扫描结果"), "英文输出里还留着中文标题：{en}");

    // 拷贝这一路也要换：结论、报告都跟着走
    let mut args = copy_args(&e);
    // copy_args 尾部钉的是 zh，改成 en（--lang 是最后一项的值）
    let last = args.len() - 1;
    args[last] = "en".into();
    let o = run_owned(&args);
    assert_eq!(code(&o), 0, "{}", String::from_utf8_lossy(&o.stderr));
    let s = stdout(&o);
    assert!(s.contains("Copy finished"), "{s}");
    assert!(!s.contains("拷贝完成"), "英文输出里还留着中文结论：{s}");

    // 报告与命令行同一份语言设置——报告是要拿给客户看的
    let html =
        std::fs::read_to_string(manifest_of(&e.landing).with_extension("html")).expect("读报告");
    assert!(html.contains("Copy report"), "报告没跟着换语言");
    assert!(!html.contains("拷卡报告"));
    // 项目名与素材名是**数据**，本来就可能是中文，出现在英文报告里是对的
    assert!(html.contains("测试项目"));
}

#[test]
fn scenario_cli_driver_usage_error_on_bad_template() {
    let e = env();
    let mut args = copy_args(&e);
    args.push("--template".into());
    args.push("素材/{年}".into()); // 缺必需占位符
    let o = run_owned(&args);
    assert_ne!(code(&o), 0, "非法模板 MUST 以非零退出");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("项目") && err.contains("日期") && err.contains("设备"),
        "错误信息应说清楚缺什么：{err}"
    );
}

#[test]
fn scenario_cli_driver_missing_source_is_reported() {
    let o = run(&["scan", "Z:/绝对不存在的路径", "--lang", "zh"]);
    assert_ne!(code(&o), 0);
    assert!(String::from_utf8_lossy(&o.stderr).contains("不存在"));
}

#[test]
fn scenario_cli_driver_report_can_be_regenerated_from_manifest() {
    let e = env();
    assert_eq!(code(&run_owned(&copy_args(&e))), 0);
    let mpath = manifest_of(&e.landing);
    let target = e.landing.join("重新生成的报告.html");
    let o = run(&[
        "report",
        &mpath.display().to_string(),
        "-o",
        &target.display().to_string(),
        "--lang",
        "zh",
    ]);
    assert_eq!(code(&o), 0);
    assert!(target.exists());
    let html = std::fs::read_to_string(&target).expect("读");
    assert!(html.contains("拷卡报告"));
    assert!(html.contains("A001.MP4"));
}

#[test]
fn scenario_cli_driver_two_destinations_read_source_once() {
    let e = env();
    let dest2 = e.dest.with_file_name("dest2");
    let mut args = copy_args(&e);
    args.push("-d".into());
    args.push(dest2.display().to_string());
    assert_eq!(code(&run_owned(&args)), 0);

    let landing2 = dest2.join("测试项目").join(today()).join("测试卡");
    for landing in [&e.landing, &landing2] {
        let f = landing
            .join("DCIM")
            .join("100MSDCF")
            .join("A001.MP4");
        assert_eq!(std::fs::metadata(&f).expect("元数据").len(), 120_000);
        assert!(manifest_of(landing).exists());
    }
}
