//! 更新包只准从哪些主机下载。
//!
//! 规范：能力 `build-release` 的 spec（openspec 私仓）→ Requirement: 更新来源可镜像
//!
//! # 为什么光有验签还不够
//!
//! 更新包的**真伪**由 ed25519 验签保证：私钥只在发布机与 CI secret 里，
//! 镜像拿不到，所以伪造不出能通过验签的包。这一层已经挡住了「装上恶意程序」。
//!
//! 但清单（`latest.json`）里的 `url` 字段是**下载地址**，验签发生在下载**之后**。
//! 清单来自网络，若某个端点被劫持，它可以把 url 写成任意地址——
//! 客户端会老老实实去请求那个地址，最后因验签失败拒装。装是装不上，
//! 可这一次请求已经发出去了：**谁在什么时候检查更新、从哪个 IP，被第三方看了个干净**。
//!
//! 对一个把「零遥测」写在首页上的工具来说，这条不能留。所以下载地址在**请求发出之前**
//! 先过一遍主机白名单——白名单编译在程序里，跟端点一样不从配置读。
//!
//! # 白名单里为什么是这两个
//!
//! 与 `tauri.conf.json` 的 `plugins.updater.endpoints` 一一对应：清单从哪儿来，
//! 包就只能从哪儿下。多一个主机就多一个能看见你的人。

/// 允许下载更新包的主机。**顺序与 endpoints 一致：自有镜像在前，GitHub 兜底。**
///
/// 改这里必须同时改 `tauri.conf.json` 与 `tauri.offline.conf.json` 的 endpoints，
/// 由 `origin_allowlist_matches_configured_endpoints` 钉住，漏改一处就编译不过测试。
pub const UPDATE_HOSTS: [&str; 2] = ["api.ai-mcn.tv", "github.com"];

/// url 是否 https + 主机恰在白名单内（端口不限——镜像挂在 :9000）。
///
/// 用 `Url` 解析而不是字符串前缀匹配：`https://api.ai-mcn.tv.attacker.com/x.exe`
/// 和 `https://api.ai-mcn.tv@attacker.com/x.exe` 都以白名单主机开头，
/// 前者是子域、后者是 userinfo，真实主机都是 attacker.com。
pub fn is_allowed_update_url(url: &str) -> bool {
    match url::Url::parse(url) {
        Ok(u) => {
            u.scheme() == "https" && u.host_str().is_some_and(|h| UPDATE_HOSTS.contains(&h))
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 白名单必须与配置里的 endpoints 对得上——两处任何一处漏改都该在这里炸。
    ///
    /// 读的是 `tauri.conf.json` 真文件而不是复述一遍常量：复述只能证明我抄对了，
    /// 读文件才能证明**发布出去的那份配置**对。
    #[test]
    fn scenario_build_release_origin_allowlist_matches_configured_endpoints() {
        for conf in ["tauri.conf.json", "tauri.offline.conf.json"] {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/");
            let text = std::fs::read_to_string(format!("{path}{conf}"))
                .unwrap_or_else(|e| panic!("读不到 {conf}: {e}"));
            let v: serde_json::Value = serde_json::from_str(&text).expect("配置不是合法 JSON");

            let Some(eps) = v["plugins"]["updater"]["endpoints"].as_array() else {
                // 离线版配置是增量合并的，没写 plugins 就继承主配置——那就没什么可查的
                continue;
            };
            assert!(!eps.is_empty(), "{conf}: endpoints 是空的，更新永远查不到");

            for (i, ep) in eps.iter().enumerate() {
                let u = ep.as_str().expect("endpoint 不是字符串");
                assert!(
                    is_allowed_update_url(u),
                    "{conf}: endpoints[{i}] = {u} 不在下载白名单里。\
                     清单从哪儿来，包就得能从哪儿下——两处必须一致"
                );
                assert!(
                    !u.contains("{{"),
                    "{conf}: endpoints[{i}] = {u} 带模板占位符。\
                     updater 支持 {{{{current_version}}}} 之类，但那等于把当前版本号\
                     报给服务端——本产品承诺零遥测，地址必须是死的"
                );
            }

            // 顺序即优先级：updater 依次尝试，第一个应答的说了算。
            // 自有镜像在前，国内才连得上；GitHub 在后，自有域名挂了还有兜底。
            let first = eps[0].as_str().unwrap();
            assert!(
                first.contains(UPDATE_HOSTS[0]),
                "{conf}: 第一个端点是 {first}，应当是自有镜像 {}。\
                 顺序反了国内用户每次都要先等 GitHub 超时",
                UPDATE_HOSTS[0]
            );
        }
    }

    #[test]
    fn scenario_build_release_update_download_host_is_allowlisted() {
        // 两个正常来源
        assert!(is_allowed_update_url(
            "https://api.ai-mcn.tv:9000/broadcast/steadcopy/steadcopy_0.1.0_x64-setup.exe"
        ));
        assert!(is_allowed_update_url(
            "https://github.com/Gitruck/steadcopy/releases/download/v0.1.0/steadcopy_0.1.0_x64-setup.exe"
        ));

        // 长得像白名单、其实不是
        assert!(!is_allowed_update_url("https://api.ai-mcn.tv.attacker.com/x.exe"));
        assert!(!is_allowed_update_url("https://api.ai-mcn.tv@attacker.com/x.exe"));
        assert!(!is_allowed_update_url("https://github.com.attacker.com/x.exe"));
        assert!(!is_allowed_update_url("https://notgithub.com/x.exe"));
        assert!(!is_allowed_update_url("https://attacker.example/x.exe"));

        // 明文一律不收：http 能被中间人改成任意字节，虽然最后验签会拒，
        // 但请求本身已经暴露了「这台机器在查更新」
        assert!(!is_allowed_update_url("http://api.ai-mcn.tv/x.exe"));
        assert!(!is_allowed_update_url("http://github.com/x.exe"));

        // 非 http(s) 协议：file:// 能让它去读本地任意文件
        assert!(!is_allowed_update_url("file:///C:/Windows/System32/calc.exe"));

        // 解析不了的
        assert!(!is_allowed_update_url(""));
        assert!(!is_allowed_update_url("https://"));
        assert!(!is_allowed_update_url("api.ai-mcn.tv/x.exe"));
    }
}
