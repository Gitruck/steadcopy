//! 时间字段的序列化口径：**一律 RFC 3339 字符串**。
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/config-store/spec.md`
//! → Requirement: 时间字段序列化口径
//!
//! # 为什么需要这个模块
//!
//! `time::OffsetDateTime` 的**默认** serde 实现把时间写成一串数字
//! （`[2026,222,21,25,10,0,0,0]`），不是字符串。这有两个后果：
//!
//! 1. 配置文件不再是人能读的——「出问题时手工救」这条就不成立了；
//! 2. 前端拿到的是数组，而 TS 那边声明的是 `string`。**JSON 边界没有类型检查**，
//!    `tsc` 查不出来，一路裸奔到运行时才炸（`last_seen.replace is not a function`）。
//!
//! manifest 那边加了 `#[serde(with = "time::serde::rfc3339")]`，config 这边漏了，
//! 两处口径不一致本身就是隐患。所以口径收到这一个模块里，**新增时间字段一律用它**，
//! 并且有测试盯着「凡是 `*_at` / `*_seen` 的键都必须是字符串」。
//!
//! # 为什么不直接用 `time::serde::rfc3339`
//!
//! 因为已经有用户的配置文件是用旧的数组形式写下来的。直接换成严格的 rfc3339
//! 会让那份配置解析失败 → 按「损坏不静默重建」的规矩被改名保留 →
//! 用户的项目、预设、设备记忆一次性全丢。
//!
//! 所以这里**读两种、写一种**：旧的数组形式照读不误，写回去一律是字符串。
//! 用户下次保存配置时自动升级，全程无感，也不需要动 `CONFIG_VERSION`——
//! 这个变化在读的方向上是完全向后兼容的。

use serde::{Deserialize, Deserializer, Serializer};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub fn serialize<S: Serializer>(v: &OffsetDateTime, s: S) -> Result<S::Ok, S::Error> {
    time::serde::rfc3339::serialize(v, s)
}

pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<OffsetDateTime, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        /// 现在的写法
        Text(String),
        /// 0.1.0 早期写下的数组形式，只读不写
        Legacy(OffsetDateTime),
    }

    match Either::deserialize(d)? {
        Either::Text(s) => OffsetDateTime::parse(&s, &Rfc3339).map_err(serde::de::Error::custom),
        Either::Legacy(t) => Ok(t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use time::macros::datetime;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct Holder {
        #[serde(with = "crate::serde_time")]
        at: OffsetDateTime,
    }

    // spec: → Scenario: 时间字段序列化为 ISO 字符串
    #[test]
    fn scenario_config_store_time_is_written_as_string() {
        let h = Holder {
            at: datetime!(2026-08-10 21:25:10 UTC),
        };
        let v = serde_json::to_value(&h).expect("序列化");
        assert!(
            v["at"].is_string(),
            "时间必须是字符串，实际是 {:?}——前端声明的是 string，写成数组会在运行时炸",
            v["at"]
        );
        assert_eq!(v["at"], "2026-08-10T21:25:10Z");
    }

    // spec: → Scenario: 旧的数组形式仍然读得出来
    #[test]
    fn scenario_config_store_legacy_time_array_still_reads() {
        // 先拿一份新格式做对照
        let legacy = serde_json::to_string(&Holder {
            at: datetime!(2026-08-10 21:25:10 UTC),
        })
        .expect("先拿一份新格式");
        // 旧格式的真实形状（取自 0.1.0 早期真写出来的配置文件）：
        // [年, 年内第几天, 时, 分, 秒, 纳秒, 时区时, 时区分, 时区秒]
        let old = r#"{"at":[2026,223,2,59,13,422742800,-7,0,0]}"#;

        let from_old: Holder = serde_json::from_str(old).expect("旧格式必须还能读");
        assert_eq!(from_old.at, datetime!(2026-08-11 2:59:13.422742800 -7:00));

        let from_new: Holder = serde_json::from_str(&legacy).expect("新格式当然要能读");
        assert_eq!(from_new.at, datetime!(2026-08-10 21:25:10 UTC));

        // 读进来之后再写出去，一律是新格式——用户下次保存就自动升级了
        let back = serde_json::to_value(&from_old).expect("写回");
        assert!(back["at"].is_string());
    }

    #[test]
    fn scenario_config_store_bad_time_string_is_an_error() {
        // 不是合法时间就该报错，不能悄悄给个默认值
        let e = serde_json::from_str::<Holder>(r#"{"at":"昨天下午"}"#);
        assert!(e.is_err(), "非法时间 MUST NOT 被静默接受");
    }
}
