//! 装完之后核对一次：装上的真是清单答应的那个版本吗？
//!
//! 规范：能力 `build-release` 的 spec（openspec 私仓）→ Requirement: 更新结果自证
//!
//! # 验签管不到版本号
//!
//! minisign 签的是**安装包的字节**。清单里的 `version` 是明文，跟签名没有任何绑定。
//! 于是一个被攻陷的更新源可以这么干：
//!
//! 1. 从公开的 Releases 页抓一个**历史版本**的安装包和它**真实的签名**（都是公开物）；
//! 2. 挂一份清单：`{"version": "9.9.9", url: 指向那个旧包, signature: 它真实的旧签名}`；
//! 3. 用户看到「有新版本 9.9.9」，点安装；
//! 4. 主机白名单过（主机就是更新源自己）、**验签通过**（字节确实是真私钥签的）；
//! 5. 装上的是那个有已知问题的旧版。而且此后每次检查更新，
//!    `当前版本 < 9.9.9` 恒成立——**反复重装，永远停在旧版**。
//!
//! 整条链不需要私钥、不需要前端漏洞，只需要拿下更新源。现有的两道防线
//! （验签、主机白名单）一条都拦不住。
//!
//! # 这里做什么、不做什么
//!
//! **不做**：拦住第一次。要在下载前就识破，得把版本号钉进受签名保护的载荷里，
//! 那是发布格式的改动，不是客户端一侧能单独解决的。
//!
//! **做**：装完重启之后核对一次。装之前记下「清单答应的版本」，重启后与
//! `CARGO_PKG_VERSION` 比对——对不上就说明更新源给的信息不实，
//! **停掉更新检查并把话说清楚**。循环被打断，用户知道发生了什么。
//!
//! 顺带它还兜住一类完全无关的事故：安装包因为任何原因没真正覆盖旧版
//! （历史上就有过——两版 productName 不同，装到了另一个目录）。
//! 那种情况的现象同样是「更新装了但版本号没变」，同样在这里被抓住。

use std::path::PathBuf;

/// 记着「上一次点安装时，更新源答应给的是哪个版本」。
///
/// 放在配置目录而不是安装目录：安装目录会被安装包覆写，而这个标记必须活过那一次覆写。
fn sentinel() -> PathBuf {
    steadcopy_core::config::config_dir().join("pending-update")
}

/// 装完了对不上的两种情形。
#[derive(Debug, PartialEq, Eq)]
pub enum Anomaly {
    /// 装上的版本比答应的低。**这是被攻陷的更新源最可能的样子。**
    Downgraded { promised: String, actual: String },
    /// 版本压根没变。安装包没真正覆盖旧版，或者装到了别的地方。
    Unchanged { promised: String },
}

impl Anomaly {
    pub fn describe(&self, lang: steadcopy_core::i18n::Locale) -> String {
        use steadcopy_core::i18n::Locale;
        match self {
            Anomaly::Downgraded { promised, actual } => match lang {
                Locale::Zh => format!(
                    "更新异常：更新源说这是 {promised}，装上之后却是 {actual}。\
                     这不是正常的更新结果——更新源给的信息与实际不符，可能已被篡改。\
                     更新检查已自动关闭；请到官网核对校验码后手动下载安装。"
                ),
                Locale::En => format!(
                    "Update anomaly: the update source promised {promised}, but {actual} was installed. \
                     That is not a normal outcome — the source's information does not match reality and may have been tampered with. \
                     Update checking has been turned off; please download manually and verify the checksum."
                ),
            },
            Anomaly::Unchanged { promised } => match lang {
                Locale::Zh => format!(
                    "更新异常：装完之后版本号仍是 {promised} 之前的那个，说明安装包没有真正覆盖旧版。\
                     更新检查已自动关闭，以免反复重装；请到官网手动下载安装。"
                ),
                Locale::En => format!(
                    "Update anomaly: the version did not change after installing (source promised {promised}), \
                     so the installer did not actually replace the old build. \
                     Update checking has been turned off to avoid a reinstall loop; please download and install manually."
                ),
            },
        }
    }
}

/// 纯判定，好测。`actual` 是程序自报的版本，`promised` 是清单答应的。
pub fn judge(promised: &str, actual: &str) -> Option<Anomaly> {
    if promised == actual {
        return None;
    }
    // 版本号按数字段比，不按字符串比："0.10.0" 字符串上小于 "0.9.0"
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['.', '-', '+'])
            .map_while(|p| p.parse::<u64>().ok())
            .collect()
    };
    let (p, a) = (parse(promised), parse(actual));
    if !p.is_empty() && !a.is_empty() && a > p {
        // 装上的比答应的还新。清单落后于现实（比如刚手动装过新版），不是异常
        return None;
    }
    Some(Anomaly::Downgraded {
        promised: promised.to_string(),
        actual: actual.to_string(),
    })
}

/// 点安装之前记一笔：**答应的版本**和**装之前的版本**。
///
/// 两个都要记，才分得清「被骗了」和「压根没装上」——它们的现象一样
/// （版本号不对），成因和处置完全不同。
///
/// 写失败不算致命：它只是让后面那次核对失去依据，不该因此挡住用户更新。
pub fn record_promised(promised: &str, before: &str) {
    let path = sentinel();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, format!("{promised}\n{before}"));
}

/// 启动时核对一次并**清掉标记**（无论结论如何）。
///
/// 一定要清：留着的话每次启动都报同一件事，而用户已经知道了——
/// 反复报警的下场是被无视，那比不报还糟。
pub fn take_anomaly(current: &str) -> Option<Anomaly> {
    take_anomaly_at(&sentinel(), current)
}

/// 同上，但标记文件的位置由调用方给——好在测试里用临时目录，
/// 不必碰真实配置目录（碰了就会在跑测试的人机器上留脏东西）。
pub fn take_anomaly_at(path: &std::path::Path, current: &str) -> Option<Anomaly> {
    let raw = std::fs::read_to_string(path).ok()?;
    let _ = std::fs::remove_file(path);
    let mut lines = raw.lines();
    let promised = lines.next()?.trim();
    let before = lines.next().unwrap_or("").trim();
    if promised.is_empty() {
        return None;
    }
    if !before.is_empty() && current == before && current != promised {
        // 一个字节没变：安装包没真正覆盖旧版
        return Some(Anomaly::Unchanged {
            promised: promised.to_string(),
        });
    }
    judge(promised, current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_build_release_update_result_is_self_verified() {
        // 正常：装上的就是答应的
        assert_eq!(judge("0.1.1", "0.1.1"), None);

        // 被攻陷的更新源最可能的样子：答应 9.9.9，装上的是旧版
        assert_eq!(
            judge("9.9.9", "0.1.0"),
            Some(Anomaly::Downgraded {
                promised: "9.9.9".into(),
                actual: "0.1.0".into()
            })
        );

        // 版本号没动：安装包没真正覆盖旧版（历史上 productName 分叉就是这个现象）
        assert!(judge("0.1.1", "0.1.0").is_some());

        // 装上的比答应的新，不算异常：清单落后于现实，比如用户自己先手动装了新版
        assert_eq!(judge("0.1.0", "0.2.0"), None);

        // **按数字段比，不按字符串比。** 字符串上 "0.10.0" < "0.9.0"，
        // 照字符串判会把一次正常升级报成降级攻击——误报比漏报更快让人关掉告警
        assert_eq!(judge("0.9.0", "0.10.0"), None);
        assert!(judge("0.10.0", "0.9.0").is_some());

        // 前缀 v 与预发布后缀不该让判定翻车
        assert_eq!(judge("v0.1.0", "0.1.1"), None);
    }

    /// 「被骗」与「压根没装上」要分得开：现象一样，处置不一样。
    ///
    /// 顺带钉住「标记一定被清掉」——留着的话每次启动都报同一件事，
    /// 反复报警的下场是被无视，那比不报还糟。
    #[test]
    fn scenario_build_release_update_result_tells_downgrade_from_no_op() {
        let dir = std::env::temp_dir().join("steadcopy-update-verify-test");
        let _ = std::fs::create_dir_all(&dir);
        let mark = |name: &str, promised: &str, before: &str| {
            let p = dir.join(name);
            std::fs::write(&p, format!("{promised}\n{before}")).expect("写标记");
            p
        };

        // 装之前 0.1.0、答应 9.9.9、装完还是 0.1.0 —— 安装包没生效。
        // 不能报成「被降级到 0.1.0」：它本来就是 0.1.0，没人降它
        let p = mark("noop", "9.9.9", "0.1.0");
        assert_eq!(
            take_anomaly_at(&p, "0.1.0"),
            Some(Anomaly::Unchanged { promised: "9.9.9".into() })
        );
        assert!(!p.exists(), "核对完标记必须清掉，否则每次启动都报同一件事");

        // 装之前 0.2.0、答应 9.9.9、装完变成 0.1.0 —— 真被塞了个旧包
        let p = mark("downgrade", "9.9.9", "0.2.0");
        assert_eq!(
            take_anomaly_at(&p, "0.1.0"),
            Some(Anomaly::Downgraded {
                promised: "9.9.9".into(),
                actual: "0.1.0".into()
            })
        );

        // 正常升级：答应 0.1.1、装完就是 0.1.1
        let p = mark("ok", "0.1.1", "0.1.0");
        assert_eq!(take_anomaly_at(&p, "0.1.1"), None);
        assert!(!p.exists());

        // 没有标记（从没点过安装）：什么都不报
        assert_eq!(take_anomaly_at(&dir.join("nope"), "0.1.0"), None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
