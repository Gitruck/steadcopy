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
//! # 白名单管到哪一跳
//!
//! 管**每一跳**。只卡第一跳是纸面防线：一个被劫持的端点回一份 url 写着自有域名的
//! 清单，过了白名单，然后由那台服务器 302 到任意地址——客户端老老实实连过去，
//! 该泄的一样泄。所以还要给 reqwest 装一个逐跳复查的重定向策略。
//!
//! 白名单本身与 `tauri.conf.json` 的 `plugins.updater.endpoints` 一一对应：
//! 清单从哪儿来，包就只能从哪儿下。多一个主机就多一个能看见你的人——
//! 唯一的例外是 GitHub 自己的对象存储，见 `UPDATE_HOST_SUFFIXES`。

/// 允许下载更新包的主机。**顺序与 endpoints 一致：自有镜像在前，GitHub 兜底。**
///
/// 改这里必须同时改 `tauri.conf.json` 与 `tauri.offline.conf.json` 的 endpoints，
/// 由 `origin_allowlist_matches_configured_endpoints` 钉住，漏改一处就编译不过测试。
pub const UPDATE_HOSTS: [&str; 2] = ["api.ai-mcn.tv", "github.com"];

/// 允许的主机后缀。**只为 GitHub 的资源分发而存在。**
///
/// `github.com/.../releases/download/...` 从来不直接返回文件，它**必然** 302 到
/// GitHub 自己的对象存储（历史上是 `objects.githubusercontent.com`，后来还出现过
/// `release-assets.githubusercontent.com`）。也就是说：GitHub 这条兜底路要能走通，
/// 跳到 `*.githubusercontent.com` 是必经之路，把它挡在外面等于兜底端点根本不工作。
///
/// 放宽到后缀而不是列举具体主机，是因为那个子域名 GitHub 说改就改，
/// 写死哪一个都会在某天静默失效——而失效的表现是「更新下不下来」，
/// 只有用户会遇到，你不会。
///
/// 前导的点是关键：没有它，`evilgithubusercontent.com` 也会匹配上。
/// 带上点之后要伪造就得注册 `githubusercontent.com` 的子域，那是 GitHub 自己的事。
const UPDATE_HOST_SUFFIXES: [&str; 1] = [".githubusercontent.com"];

/// 最多跟几跳重定向。GitHub 正常是一跳；给到 5 是留余量，
/// 但不给 reqwest 默认的 10——每一跳都是一次「这台机器在下更新」的广播。
pub const MAX_REDIRECTS: usize = 5;

/// url 是否 https + 主机在白名单内（端口不限——镜像挂在 :9000）。
///
/// 用 `Url` 解析而不是字符串前缀匹配：`https://api.ai-mcn.tv.attacker.com/x.exe`
/// 和 `https://api.ai-mcn.tv@attacker.com/x.exe` 都以白名单主机开头，
/// 前者是子域、后者是 userinfo，真实主机都是 attacker.com。
///
/// **这个判定要用在每一跳上，不能只用在第一跳。** 只卡第一跳的话，
/// 一个被劫持的端点回一份 url 写着自有域名的清单就能过关，然后由那台服务器
/// 302 到任意地址——客户端老老实实连过去，IP、时间、UA 全泄出去，
/// 最后才因为验签失败拒装。装是没装上，可要防的那件事已经发生了。
pub fn is_allowed_update_url(url: &str) -> bool {
    match url::Url::parse(url) {
        Ok(u) => {
            u.scheme() == "https"
                && u.host_str().is_some_and(|h| {
                    UPDATE_HOSTS.contains(&h)
                        || UPDATE_HOST_SUFFIXES.iter().any(|s| h.ends_with(s))
                })
        }
        Err(_) => false,
    }
}

/// 对某一跳的判定。单独拎出来是为了能测——`reqwest::redirect::Attempt`
/// 在库外造不出来，判定逻辑埋在闭包里就等于没测过。
#[derive(Debug, PartialEq, Eq)]
pub enum Hop {
    /// 跟过去
    Follow,
    /// 停下并报错（附上原因）
    Reject(&'static str),
}

/// 这一跳跟不跟。
pub fn redirect_decision(url: &str, hops_so_far: usize) -> Hop {
    if hops_so_far >= MAX_REDIRECTS {
        return Hop::Reject("重定向跳数过多");
    }
    if is_allowed_update_url(url) {
        Hop::Follow
    } else {
        Hop::Reject("重定向到了不允许的来源")
    }
}

/// 重定向策略：每一跳都过一遍白名单，跳出去就报错停下。
///
/// 用 `error` 而不是 `stop`：`stop` 会把那个 302 响应原样交给调用方，
/// 于是「下载」拿到的是一段重定向页面的字节，最后表现为验签失败——
/// 一个和「包被篡改」长得一模一样、但成因完全不同的错误。宁可当场说清楚。
pub fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        match redirect_decision(attempt.url().as_str(), attempt.previous().len()) {
            Hop::Follow => attempt.follow(),
            // 先取出主机名再 error：Attempt 会被 error 消费掉
            Hop::Reject(why) => {
                let host = attempt.url().host_str().unwrap_or("?").to_string();
                attempt.error(format!("{why}：{host}"))
            }
        }
    })
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

        // GitHub 的资源实际落在它自己的对象存储上，必须放行——
        // 拦掉的话 GitHub 这条兜底路整条不通
        assert!(is_allowed_update_url(
            "https://objects.githubusercontent.com/github-production-release-asset/x"
        ));
        assert!(is_allowed_update_url(
            "https://release-assets.githubusercontent.com/releases/x"
        ));
        // 但后缀匹配不许退化成「包含」：少了前导点这一条就会放行
        assert!(!is_allowed_update_url("https://evilgithubusercontent.com/x.exe"));
        assert!(!is_allowed_update_url("https://githubusercontent.com.attacker.com/x.exe"));

        // 解析不了的
        assert!(!is_allowed_update_url(""));
        assert!(!is_allowed_update_url("https://"));
        assert!(!is_allowed_update_url("api.ai-mcn.tv/x.exe"));
    }

    /// 白名单要在**每一跳**上成立。
    ///
    /// 这条防的是：被劫持的端点回一份 url 完全合规的清单（过第一道），
    /// 再由那台服务器 302 到站外。只查第一跳的话这条链畅通无阻，
    /// 而它正是 `is_allowed_update_url` 的注释里点名要防的那件事。
    #[test]
    fn scenario_build_release_every_redirect_hop_is_rechecked() {
        // GitHub 的正常路径：一跳到它自己的对象存储
        assert_eq!(
            redirect_decision("https://objects.githubusercontent.com/x", 1),
            Hop::Follow
        );
        // 镜像正常不重定向，但真跳到自己身上也放行
        assert_eq!(
            redirect_decision("https://api.ai-mcn.tv:9000/broadcast/steadcopy/x.exe", 1),
            Hop::Follow
        );

        // 跳出白名单：拒
        assert!(matches!(
            redirect_decision("https://attacker.example/x.exe", 1),
            Hop::Reject(_)
        ));
        // 降级到明文：拒。302 到 http 是经典的降级手法
        assert!(matches!(
            redirect_decision("http://api.ai-mcn.tv/x.exe", 1),
            Hop::Reject(_)
        ));

        // 跳数上限。到了上限即使地址合规也停——无限重定向本身就是一种打法
        assert_eq!(redirect_decision("https://github.com/x", MAX_REDIRECTS - 1), Hop::Follow);
        assert!(matches!(
            redirect_decision("https://github.com/x", MAX_REDIRECTS),
            Hop::Reject(_)
        ));
    }
}
