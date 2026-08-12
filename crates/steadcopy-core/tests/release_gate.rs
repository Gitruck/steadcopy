//! 发布门控里**能自动化的那部分**。
//!
//! 规范：`openspec/changes/add-steadcopy-release/specs/build-release/spec.md`
//!
//! 这里检查的是「仓库现在的状态」而不是「代码的行为」——离网安装、非管理员安装、
//! 三处校验码同步这些只能人去走，登记在 `docs/release-checklist.md`。
//! 但下面这几条是能钉死的，钉死了就不必每次发版靠人记：
//!
//! - 文档里绝不出现「关掉杀软」这类表述
//! - 依赖清单里没有传染性许可
//! - 代码里没有 HTTP 客户端（零遥测靠的是**根本没有联网能力**，不是靠自觉）

#![allow(clippy::needless_raw_string_hashes, clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = crates/steadcopy-core
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("定位仓库根目录")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读不到 {}：{e}", p.display()))
}

/// 维护者自己看的文档。它们**讨论**误报与防护策略，必然会出现那些词，
/// 不属于「面向用户的表述」。名单是显式的白名单而不是模式匹配——
/// 想让一份新文档免检，得先把它明明白白加进来。
const MAINTAINER_DOCS: &[&str] = &[
    "antivirus-whitelist.md",
    "release-checklist.md",
    "danger-tests.md",
];

/// 面向用户的全部文本：README + docs/ 下的 md（维护者文档除外）。
fn user_facing_docs() -> Vec<(String, String)> {
    let mut out = vec![("README.md".to_string(), read("README.md"))];
    let docs = repo_root().join("docs");
    let mut entries: Vec<_> = std::fs::read_dir(&docs)
        .expect("读 docs/")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter(|p| {
            !p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| MAINTAINER_DOCS.contains(&n))
        })
        .collect();
    entries.sort();
    for p in entries {
        let name = format!("docs/{}", p.file_name().unwrap().to_string_lossy());
        let text = std::fs::read_to_string(&p).expect("读文档");
        out.push((name, text));
    }
    out
}

/// 应用内面向用户的文案。它和文档一样受约束——
/// 只在文档里守规矩、在界面上教用户关杀软，等于没守。
fn app_user_text() -> String {
    let mut s = read("app/src/App.tsx");
    s.push_str(&read("app/src/components.tsx"));
    s.push_str(&read("scripts/build-release.py"));
    s
}

// spec: → 不教用户关杀软
#[test]
fn scenario_build_release_docs_never_tell_users_to_disable_protection() {
    // 教用户关防护，是把安全成本转嫁给用户去换自己的安装成功率。
    // 这条不靠自觉——它是可机检的。
    const BANNED: &[&str] = &[
        "关闭杀毒",
        "关掉杀毒",
        "关闭防护",
        "关掉防护",
        "关闭安全软件",
        "关掉安全软件",
        "退出杀毒",
        "禁用 Defender",
        "关闭 Defender",
        "关闭实时保护",
        "临时关闭",
    ];
    // 「不要关杀软」这类正面表述里也会出现这些词，带明确禁止标记的行跳过。
    // 这是启发式：它挡得住「顺手写了一句关杀软」，挡不住蓄意绕过——
    // 后者本来就该靠人审，机检只负责不让它悄悄溜进来。
    const ALLOWED_CONTEXT: &[&str] = &[
        "不要", "无需", "不需要", "MUST NOT", "不建议", "不存在", "不出现", "不得", "禁止",
        "本项目不做",
    ];

    let mut hits = Vec::new();
    let mut surfaces = user_facing_docs();
    surfaces.push(("应用内文案".to_string(), app_user_text()));
    for (name, text) in surfaces {
        for line in text.lines() {
            let l = line.trim();
            if ALLOWED_CONTEXT.iter().any(|c| l.contains(c)) {
                continue;
            }
            for b in BANNED {
                if l.contains(b) {
                    hits.push(format!("{name}: {l}"));
                }
            }
        }
    }
    assert!(hits.is_empty(), "文档里出现了教用户关防护的表述：\n{hits:#?}");
}

// spec: → 如实告知未签名
#[test]
fn scenario_build_release_unsigned_notice_is_present() {
    let readme = read("README.md");
    assert!(
        readme.contains("未购买代码签名证书"),
        "README 必须如实告知未签名"
    );
    assert!(
        readme.contains("SHA256") || readme.contains("SHA-256"),
        "README 必须给出校验码核对方法"
    );
    let guide = read("docs/verify-download.md");
    assert!(
        guide.contains("Get-FileHash"),
        "核对指引必须给出可直接执行的命令"
    );
}

// spec: → 许可清单随包且可查 / 清单每次发布重新生成
#[test]
fn scenario_build_release_license_manifest_has_no_copyleft() {
    let raw = read("app/src-tauri/licenses.json");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("许可清单必须是合法 JSON");
    let pkgs = v["packages"].as_array().expect("packages 必须是数组");
    assert!(pkgs.len() > 50, "依赖数看起来不对：{}", pkgs.len());
    assert_eq!(v["self"]["license"], "MIT");

    // 「或 MIT / 或 Apache」的双许可可以选另一边，不算传染
    let dual_ok = ["OR MIT", "OR APACHE", "/MIT", "/APACHE"];
    let blocked = ["GPL", "AGPL", "SSPL", "CDDL", "EPL", "MPL-1"];

    let mut bad = Vec::new();
    for p in pkgs {
        let lic = p["license"].as_str().unwrap_or("").to_uppercase();
        if dual_ok.iter().any(|d| lic.contains(d)) {
            continue;
        }
        if blocked.iter().any(|b| lic.contains(b)) {
            bad.push(format!("{} {} — {lic}", p["name"], p["version"]));
        }
        assert_ne!(lic, "未声明", "许可未声明的依赖：{}", p["name"]);
    }
    assert!(bad.is_empty(), "存在传染性许可的依赖：\n{bad:#?}");
}

// spec: → 零遥测 / 无主动外联
#[test]
fn scenario_build_release_no_http_client_in_dependency_tree() {
    // 零遥测不是靠「我们没写上报代码」，是靠**根本没有联网能力**。
    // 依赖树里一旦出现 HTTP 客户端，这个保证就退化成了自觉。
    let lock = read("Cargo.lock");
    const NETWORK_CRATES: &[&str] = &[
        "reqwest", "hyper", "ureq", "curl", "isahc", "surf", "attohttpc", "tungstenite",
    ];
    let mut found = Vec::new();
    for line in lock.lines() {
        let Some(name) = line.strip_prefix("name = \"") else {
            continue;
        };
        let name = name.trim_end_matches('"');
        if NETWORK_CRATES.contains(&name) {
            found.push(name.to_string());
        }
    }
    assert!(
        found.is_empty(),
        "命令行/引擎的依赖树里出现了 HTTP 客户端：{found:?}。\
         零遥测的保证依赖于「没有联网能力」，不是依赖于自觉"
    );
}

// spec: → 便携版数据隔离
#[test]
fn scenario_build_release_portable_marker_is_documented() {
    // 便携版靠程序旁边的标记文件启用。文件名写死在 core 里，
    // 打包脚本与文档都得跟它一致，不然「解压即用」会变成「解压即写 APPDATA」。
    let script = read("scripts/build-release.py");
    assert!(
        script.contains("steadcopy.portable"),
        "打包脚本必须往便携版里放标记文件"
    );
    let checklist = read("docs/release-checklist.md");
    assert!(checklist.contains("便携版"), "门控清单必须含便携版验证项");
}

// spec: → 格式化未验收则不启用
#[test]
fn scenario_build_release_danger_track_registry_matches_ignored_tests() {
    // 危险轨测试必须登记在册。没登记的 #[ignore] 说明有人偷偷加了危险测试，
    // 那它就不会出现在虚拟机验收清单里，也就永远不会被验收。
    let registry = read("docs/danger-tests.md");
    let src = read("crates/steadcopy-core/tests/format_danger.rs");

    // 只认真正被 #[ignore] 挡住的那些。同一个文件里也有安全轨测试
    // （比如「没设环境变量就跳过」本身），那些不需要登记。
    let mut ignored: Vec<&str> = Vec::new();
    let mut pending = false;
    for line in src.lines() {
        let l = line.trim();
        if l.starts_with("#[ignore") {
            pending = true;
        } else if let Some(rest) = l.strip_prefix("fn ") {
            if pending {
                if let Some(name) = rest.split('(').next() {
                    ignored.push(name);
                }
            }
            pending = false;
        }
    }

    assert!(!ignored.is_empty(), "危险轨测试文件里一个 #[ignore] 都没有？");
    for name in &ignored {
        assert!(
            registry.contains(name),
            "危险轨测试 {name} 没有登记在 docs/danger-tests.md"
        );
    }
}

// spec: → 门面契约（docs/facade-contract.md）
#[test]
fn scenario_app_shell_bridge_matches_registered_commands() {
    // 前端调一个后端没注册的命令，只有跑起来点到那个按钮才会发现。
    // 这里把两侧对起来，编译期就能拦下。
    //
    // 放在 core 的测试里而不是前端：前端没有测试运行器，
    // 为了这一条引入一整套 vitest 不划算，而这条检查本质上是读两个文本文件。
    let rs = read("app/src-tauri/src/lib.rs");
    let ts = read("app/src/bridge.ts");

    // 注册表：generate_handler![...] 里的那一串
    let registered: Vec<String> = rs
        .split_once("tauri::generate_handler![")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(list, _)| {
            list.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .expect("找不到 generate_handler! 列表");
    assert!(registered.len() > 20, "注册的命令太少：{}", registered.len());

    // 前端用到的：bridge.ts 里 invoke<...>("名字")
    let mut used = Vec::new();
    for (i, _) in ts.match_indices("invoke") {
        let rest = &ts[i + "invoke".len()..];
        // 只认真正的调用：invoke<T>("名字") 或 invoke("名字")。
        // 光找 "invoke" 会把顶部那行 import 也算进来
        if !rest.starts_with('<') && !rest.starts_with('(') {
            continue;
        }
        let Some(open) = rest.find("(\"") else { continue };
        let start = open + 2;
        let Some(len) = rest[start..].find('"') else {
            continue;
        };
        used.push(rest[start..start + len].to_string());
    }
    assert!(!used.is_empty(), "bridge.ts 里一个 invoke 都没有？");

    let missing: Vec<&String> = used.iter().filter(|u| !registered.contains(u)).collect();
    assert!(
        missing.is_empty(),
        "前端调用了未注册的命令：{missing:?}\n已注册：{registered:?}"
    );

    // 反向：注册了却没人用的命令是死代码，也说明契约文档该更新了
    let unused: Vec<&String> = registered.iter().filter(|r| !used.contains(r)).collect();
    assert!(
        unused.is_empty(),
        "注册了但前端没用的命令（死代码或文档过期）：{unused:?}"
    );
}

// spec: → 前端零业务逻辑
#[test]
fn scenario_app_shell_only_bridge_talks_to_backend() {
    // 「前端零业务逻辑」这条铁律的可机检部分：与后端通信只有一个出入口。
    // 出入口散开之后，业务逻辑会顺着散开的口子渗进前端——这是经验，不是洁癖。
    let dir = repo_root().join("app/src");
    let mut offenders = Vec::new();
    for e in std::fs::read_dir(&dir).expect("读 app/src") {
        let p = e.expect("目录项").path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if name == "bridge.ts" || p.extension().is_none() {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap_or_else(|_| String::new());
        if text.contains("invoke(") || text.contains("invoke<") {
            offenders.push(name);
        }
    }
    assert!(
        offenders.is_empty(),
        "这些文件绕过 bridge.ts 直接与后端通信：{offenders:?}"
    );
}

// spec: i18n → Scenario: 英文输出无 CJK（界面侧）
#[test]
fn scenario_i18n_ui_has_no_hardcoded_chinese() {
    // 界面文案一律走词典。硬编码一句中文，英文用户就看到一句中文，
    // 而 `tsc` 查不出来——它是合法的字符串。所以在这里机检。
    //
    // 放行两类：`//` 与 `{/* */}` 注释（给维护者看的，不是界面文案），
    // 以及路径模板占位符（`{项目}` 那些是**后端按字面解析的数据**，翻了就坏）。
    const PLACEHOLDERS: &[&str] = &[
        "{项目}", "{日期}", "{设备}", "{卡}", "{时段}", "{年}", "{月}", "{日}",
    ];

    let mut hits = Vec::new();
    for name in ["App.tsx", "components.tsx", "adhoc.tsx"] {
        let text = read(&format!("app/src/{name}"));
        let mut in_block_comment = false;
        for (n, line) in text.lines().enumerate() {
            let l = line.trim();
            if l.starts_with("//") || l.starts_with('*') || l.starts_with("/*") {
                in_block_comment = l.starts_with("/*") && !l.contains("*/");
                continue;
            }
            if in_block_comment {
                if l.contains("*/") {
                    in_block_comment = false;
                }
                continue;
            }
            // 行内 JSX 注释 {/* … */} 与行尾 // 注释先剥掉
            let mut stripped = line.to_string();
            while let (Some(a), Some(b)) = (stripped.find("{/*"), stripped.find("*/}")) {
                if a < b {
                    stripped.replace_range(a..b + 3, "");
                } else {
                    break;
                }
            }
            if l.starts_with("{/*") {
                in_block_comment = !l.contains("*/}");
                continue;
            }
            for p in PLACEHOLDERS {
                stripped = stripped.replace(p, "");
            }
            if steadcopy_core::i18n::has_cjk(&stripped) {
                hits.push(format!("app/src/{name}:{}: {}", n + 1, l));
            }
        }
    }

    assert!(
        hits.is_empty(),
        "界面里出现了硬编码中文，英文用户会直接看到它们——请收进 app/src/i18n/zh.ts：\n{hits:#?}"
    );
}
