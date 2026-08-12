//! 精简版与离线版必须是**同一个产品**的两种装法。
//!
//! 规范：能力 `build-release` 的 spec（openspec 私仓）→ Requirement: 两版安装包是同一个产品
//!
//! # 这里防的是一个已经发生过的 bug
//!
//! 离线版原先叫「稳拷-离线版」——文件名一眼可辨，看起来很方便。但 NSIS 的
//! 安装目录、卸载注册表项、开始菜单项**全部由 productName 派生**：
//!
//! ```text
//! INSTDIR   = $PROGRAMFILES64\<productName>
//! UNINSTKEY = Software\Microsoft\Windows\CurrentVersion\Uninstall\<productName>
//! ```
//!
//! 于是两版在 Windows 眼里是两个毫不相干的程序。用离线版的人点一下「安装更新」，
//! 下到的是精简版安装包（更新清单只指一个包，指的是精简版），装完是**第二份**：
//! 「添加删除程序」里两条、两个安装目录，重启之后跑的还是原来那份旧的。
//! 用户看到的现象是「更新装了，但版本号没变」——查起来毫无头绪。
//!
//! 所以两版的差别 MUST 只有 `webviewInstallMode` 一项。这个约束没法靠人记住，
//! 只能钉在测试里：离线版配置一旦碰了身份字段，这里就红。

/// 离线版配置**只准**覆盖这些键。多一个都要在这里过一遍脑子。
///
/// `bundle` 在列里是因为 `webviewInstallMode` 就在它下面——所以 `bundle` 内部
/// 还要再查一层，见下面的 `assert_bundle_only_changes_webview_mode`。
const OFFLINE_MAY_OVERRIDE: [&str; 2] = ["$schema", "bundle"];

/// 决定「这是哪个程序」的字段。离线版配置碰任何一个都是上面那个 bug 的复发。
///
/// `mainBinaryName` 与 `version` 不写在主配置里也没关系——前者默认取 Cargo 包名
/// （`steadcopy-app`），后者取 Cargo.toml 的 version。但**离线版更不许写**：
/// 一旦写了就与主配置分叉，而分叉正是要防的事。
const IDENTITY_KEYS: [&str; 4] = ["productName", "identifier", "mainBinaryName", "version"];

/// 主配置里必须显式写死的身份字段。另外两个由 Cargo 派生，不必写。
const IDENTITY_KEYS_REQUIRED_IN_BASE: [&str; 2] = ["productName", "identifier"];

#[cfg(test)]
mod tests {
    use super::*;

    fn conf(name: &str) -> serde_json::Value {
        let text = std::fs::read_to_string(format!("{}/{name}", env!("CARGO_MANIFEST_DIR")))
            .unwrap_or_else(|e| panic!("读不到 {name}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} 不是合法 JSON: {e}"))
    }

    #[test]
    fn scenario_build_release_two_flavors_are_one_product() {
        let base = conf("tauri.conf.json");
        let offline = conf("tauri.offline.conf.json");

        let top = offline.as_object().expect("离线版配置不是对象");
        for key in top.keys() {
            assert!(
                OFFLINE_MAY_OVERRIDE.contains(&key.as_str()),
                "离线版配置覆盖了 `{key}`。两版的差别只准是 webviewInstallMode——\
                 别的都改了就不是同一个产品的两种装法了。\
                 真要加，先想清楚它会不会让 Windows 把两版当成两个程序。"
            );
        }

        // 身份字段一个都不许出现在离线版配置里（哪怕值写得和主配置一样，
        // 也会在下次改主配置时悄悄分叉）
        for key in IDENTITY_KEYS {
            assert!(
                top.get(key).is_none(),
                "离线版配置里出现了 `{key}`。它决定 Windows 把这两版当成一个程序\
                 还是两个：productName 不同 ⇒ 装到不同目录、注册不同卸载项 ⇒ \
                 离线版用户点更新会装出第二份，而且重启后跑的还是旧的。"
            );
        }
        for key in IDENTITY_KEYS_REQUIRED_IN_BASE {
            assert!(base.get(key).is_some(), "主配置缺 `{key}`");
        }
    }

    /// 版本号写在三个地方，必须一致。
    ///
    /// - 根 `Cargo.toml`（workspace，core 与 cli 继承它）
    /// - `app/src-tauri/Cargo.toml`（`CARGO_PKG_VERSION`，程序自报的版本、
    ///   `gen-latest-json.py` 写进更新清单的版本）
    /// - `app/src-tauri/tauri.conf.json`（NSIS 盖在安装包上的版本）
    ///
    /// 对不上会怎样：更新清单说 0.1.1，程序自报 0.1.0，于是**每次检查更新都提示有新版**；
    /// 装完还是提示，因为装上的那个自报的仍是 0.1.0。用户会以为更新坏了。
    /// 反过来若安装包版本号更低，Windows 的「添加删除程序」里显示的版本会和程序里的对不上。
    #[test]
    fn scenario_build_release_version_is_defined_once() {
        let root = env!("CARGO_MANIFEST_DIR");
        let cargo_version = |path: &str| -> String {
            std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("读不到 {path}: {e}"))
                .lines()
                .find(|l| l.starts_with("version"))
                .and_then(|l| l.split('"').nth(1).map(str::to_string))
                .unwrap_or_else(|| panic!("{path} 里没有 version"))
        };

        let app = cargo_version(&format!("{root}/Cargo.toml"));
        let workspace = cargo_version(&format!("{root}/../../Cargo.toml"));
        let conf = conf("tauri.conf.json")["version"]
            .as_str()
            .expect("tauri.conf.json 没有 version")
            .to_string();

        assert_eq!(
            app, workspace,
            "app/src-tauri/Cargo.toml 是 {app}，根 Cargo.toml 是 {workspace}。\
             发版时三处版本号要一起改（scripts/release.py 会代劳）"
        );
        assert_eq!(
            app, conf,
            "app/src-tauri/Cargo.toml 是 {app}，tauri.conf.json 是 {conf}。\
             前者是程序自报的版本与更新清单里的版本，后者是安装包上盖的版本——\
             对不上会让用户每次检查更新都看到「有新版」，装完还是提示"
        );
    }

    #[test]
    fn scenario_build_release_offline_only_changes_webview_mode() {
        let offline = conf("tauri.offline.conf.json");
        let Some(bundle) = offline.get("bundle") else {
            panic!("离线版配置没有 bundle——那它跟精简版就没区别了，两个包白打");
        };

        // bundle 下只准有 windows，windows 下只准有 webviewInstallMode
        let b = bundle.as_object().expect("bundle 不是对象");
        assert_eq!(
            b.keys().collect::<Vec<_>>(),
            vec!["windows"],
            "离线版的 bundle 里除了 windows 还有别的键"
        );
        let w = b["windows"].as_object().expect("bundle.windows 不是对象");
        assert_eq!(
            w.keys().collect::<Vec<_>>(),
            vec!["webviewInstallMode"],
            "离线版的 bundle.windows 里除了 webviewInstallMode 还有别的键"
        );
        assert_eq!(
            w["webviewInstallMode"]["type"], "offlineInstaller",
            "离线版必须是 offlineInstaller——不是的话它跟精简版一模一样，\
             206 MB 的包白打了，而片场断网的人还是装不上"
        );

        // 反过来钉住精简版：它必须是 downloadBootstrapper，
        // 否则「精简」名不副实，用户以为下 4 MB 结果下了 206 MB
        let base = conf("tauri.conf.json");
        assert_eq!(
            base["bundle"]["windows"]["webviewInstallMode"]["type"], "downloadBootstrapper",
            "精简版必须是 downloadBootstrapper"
        );
    }
}
