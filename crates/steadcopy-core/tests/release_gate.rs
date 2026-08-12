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

/// 界面与报告里不许出现绿色。
///
/// 规范：门控 R6（clean-room 自检 · 界面形态不与前身构成近似）
///
/// # 为什么这条是硬约束
///
/// 前身项目的主色是绿的。本项目原作者把「换掉绿色」作为**同意开源的条件**——
/// 也就是说这不是审美偏好，是一条许可条款性质的约束，改回去要付的不是返工成本。
///
/// # 为什么按色相判，不按具体色值判
///
/// 列举「不许用 #3ddc84」只能挡住这一个值，换个近似的绿照样溜进去。
/// 按 HSL 色相拦 80°–160°（黄绿到春绿的整段）才拦得住一类而不是一个。
///
/// 青绿（teal）在 165°–185°，落在闸门外，是刻意的：成功态用青绿——
/// 主色是蓝，纯绿在蓝调界面里像另一个产品的零件；但也不能跟着变蓝，
/// 因为 `--running` 就是蓝的，两者一样的话「还在跑」和「跑完了」一眼分不出。
///
/// 低饱和的颜色不参与判定：接近灰的颜色算出来的色相没有意义
/// （`#262c37` 这种面板线条色的色相会落在任意区间）。
#[test]
fn scenario_app_shell_palette_has_no_green() {
    /// 返回 (色相 0–360, 饱和度 0–1)
    fn hue_sat(hex: &str) -> (f64, f64) {
        let v = |i: usize| i64::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
        let (r, g, b) = (v(0), v(2), v(4));
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        if d == 0.0 {
            return (0.0, 0.0);
        }
        let h = if max == r {
            60.0 * (((g - b) / d) % 6.0)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        let l = (max + min) / 2.0;
        let s = d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-9);
        ((h + 360.0) % 360.0, s.min(1.0))
    }

    // 饱和度低于这个的当灰色处理，不判色相
    const GRAY: f64 = 0.15;
    // 拦下的色相区间：黄绿 → 春绿。青绿（>160）与黄（<80）都在外面
    const GREEN: std::ops::Range<f64> = 80.0..160.0;

    let mut hits = Vec::new();
    for rel in ["app/src/styles.css", "crates/steadcopy-core/src/ledger/report.rs"] {
        let text = read(rel);
        for (n, line) in text.lines().enumerate() {
            let bytes = line.as_bytes();
            for (i, _) in line.match_indices('#') {
                // 只认 #rrggbb；#rgb 与 CSS id 选择器不在此列
                if i + 7 > line.len() {
                    continue;
                }
                let hex = &line[i + 1..i + 7];
                if !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
                    continue;
                }
                // 后面还跟着十六进制位说明这不是六位色（比如八位带 alpha）
                if bytes.get(i + 7).is_some_and(|c| c.is_ascii_hexdigit()) {
                    continue;
                }
                let (h, s) = hue_sat(hex);
                if s >= GRAY && GREEN.contains(&h) {
                    hits.push(format!("{rel}:{}: #{hex}（色相 {h:.0}°）  {}", n + 1, line.trim()));
                }
            }
        }
    }

    assert!(
        hits.is_empty(),
        "调色板里出现了绿色。主色为蓝是原作者同意开源的条件之一，不是审美偏好。\n\
         成功态请用青绿（色相 165°–185°，如 #2dd4bf / #0f766e），\
         它在闸门外且与「正在跑」的蓝区分得开：\n{hits:#?}"
    );

    // 「成功」与「正在跑」必须一眼分得开。
    //
    // 这条与上面那条是一体的：把成功态从纯绿挪走时，最省事的做法是挪成蓝，
    // 而那恰好会撞上 `--running`。所以判据要钉在这儿，不能只钉「别用绿」——
    // 只钉后者的话，一次「顺手都改蓝」就能悄悄毁掉这个工具最高频的一次判断：
    // 拷卡时瞥一眼「还在跑还是跑完了」。
    //
    // 铁律只说了颜色不能是**唯一**信息载体，没说可以让它变成误导。
    let css = read("app/src/styles.css");
    let hue_of = |name: &str| -> f64 {
        let line = css
            .lines()
            .find(|l| l.trim_start().starts_with(&format!("{name}:")))
            .unwrap_or_else(|| panic!("styles.css 里找不到 {name}"));
        let hex = line.split('#').nth(1).expect("值不是十六进制色");
        hue_sat(&hex[..6]).0
    };
    let (ok, running) = (hue_of("--ok"), hue_of("--running"));
    let raw = (ok - running).abs();
    let gap = raw.min(360.0 - raw);
    assert!(
        gap >= 30.0,
        "「校验通过」({ok:.0}°) 与「正在跑」({running:.0}°) 的色相只差 {gap:.0}°，\
         在屏幕上会糊成一片。拷卡时最常瞥的一眼就是这两个状态的区别。"
    );
}
