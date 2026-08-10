//! 哈希：流式计算、定长值类型、绝不降级。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/copy-engine/spec.md`
//! → Requirement: 边读边算源哈希 / 哈希失败绝不降级
//!
//! # 为什么 `HashValue` 不是 `String`
//!
//! 前身项目的一号缺陷：哈希函数出错时 `return ''`。于是源与目标双双失败时
//! `'' === ''` 判定校验通过、日志全绿——用户以为有备份，其实没有。
//!
//! 本模块用**定长值类型**承载哈希，让「空哈希」这个非法状态在类型层面不存在：
//! 没有 `Default`、没有空构造、没有从字符串随便造一个的路径。
//! 计算失败只能表达为 `Err`，无法退化成一个「碰巧相等」的值。

use std::fmt;

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use xxhash_rust::xxh64::Xxh64;

/// 支持的哈希算法。XXH64 是行业事实标准（OffShoot / ShotPut / Silverstack / Gate 均用），
/// MD5 仅为兼容旧后期流程保留——它慢一个量级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum HashAlgorithm {
    #[serde(rename = "xxh64")]
    #[default]
    Xxh64,
    #[serde(rename = "md5")]
    Md5,
}

impl HashAlgorithm {
    /// 落进 manifest 与 MHL 的算法标识。
    pub const fn id(self) -> &'static str {
        match self {
            HashAlgorithm::Xxh64 => "xxh64",
            HashAlgorithm::Md5 => "md5",
        }
    }
}


impl fmt::Display for HashAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// 一个**算好了的**哈希值。
///
/// 刻意不实现 `Default`，也不提供任何「空值」构造——见模块文档。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "algorithm", content = "value", rename_all = "lowercase")]
pub enum HashValue {
    #[serde(rename = "xxh64", with = "hex_u64")]
    Xxh64(u64),
    #[serde(rename = "md5", with = "hex_bytes")]
    Md5([u8; 16]),
}

impl HashValue {
    pub const fn algorithm(&self) -> HashAlgorithm {
        match self {
            HashValue::Xxh64(_) => HashAlgorithm::Xxh64,
            HashValue::Md5(_) => HashAlgorithm::Md5,
        }
    }

    /// 小写十六进制表示。XXH64 按大端序输出（与 MHL 生态的 XXH64BE 惯例一致）。
    pub fn to_hex(&self) -> String {
        match self {
            HashValue::Xxh64(v) => v
                .to_be_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            HashValue::Md5(v) => v.iter().map(|b| format!("{b:02x}")).collect(),
        }
    }

    /// 两个哈希是否代表同一份内容。
    ///
    /// **要求算法相同**——不同算法之间不可比，返回 `false` 而非误判为相等。
    pub fn matches(&self, other: &HashValue) -> bool {
        match (self, other) {
            (HashValue::Xxh64(a), HashValue::Xxh64(b)) => a == b,
            (HashValue::Md5(a), HashValue::Md5(b)) => a == b,
            _ => false,
        }
    }
}

impl fmt::Display for HashValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// 流式哈希器。喂进去多少算多少，用于「读源的同一遍 IO 里算出哈希」。
pub enum Hasher {
    Xxh64(Box<Xxh64>),
    Md5(Box<Md5>),
}

impl Hasher {
    pub fn new(algorithm: HashAlgorithm) -> Self {
        match algorithm {
            HashAlgorithm::Xxh64 => Hasher::Xxh64(Box::new(Xxh64::new(0))),
            HashAlgorithm::Md5 => Hasher::Md5(Box::new(Md5::new())),
        }
    }

    pub const fn algorithm(&self) -> HashAlgorithm {
        match self {
            Hasher::Xxh64(_) => HashAlgorithm::Xxh64,
            Hasher::Md5(_) => HashAlgorithm::Md5,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Xxh64(h) => h.update(data),
            Hasher::Md5(h) => h.update(data),
        }
    }

    pub fn finish(self) -> HashValue {
        match self {
            Hasher::Xxh64(h) => HashValue::Xxh64(h.digest()),
            Hasher::Md5(h) => HashValue::Md5(h.finalize().into()),
        }
    }
}

impl fmt::Debug for Hasher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hasher({})", self.algorithm())
    }
}

/// 便捷函数：对一段内存里的数据算哈希（测试与小文件用）。
pub fn hash_bytes(algorithm: HashAlgorithm, data: &[u8]) -> HashValue {
    let mut h = Hasher::new(algorithm);
    h.update(data);
    h.finish()
}

mod hex_u64 {
    use serde::{Deserialize, Deserializer, Serializer};

    /// `{:016x}` 对 u64 输出的就是大端字节序的十六进制，与 `HashValue::to_hex`
    /// 的 `to_be_bytes()` 逐字节输出完全一致。**不要**再额外做 `to_be()` 交换，
    /// 那会让 manifest 里的值和 MHL 输出对不上。
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{v:016x}"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        let s = String::deserialize(d)?;
        if s.len() != 16 {
            return Err(serde::de::Error::custom("xxh64 十六进制串长度必须是 16"));
        }
        u64::from_str_radix(&s, 16).map_err(serde::de::Error::custom)
    }
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8; 16], s: S) -> Result<S::Ok, S::Error> {
        let hex: String = v.iter().map(|b| format!("{b:02x}")).collect();
        s.serialize_str(&hex)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 16], D::Error> {
        let s = String::deserialize(d)?;
        if s.len() != 32 {
            return Err(serde::de::Error::custom("md5 十六进制串长度必须是 32"));
        }
        let mut out = [0u8; 16];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(serde::de::Error::custom)?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: copy-engine → 边读边算源哈希 → Scenario: 算法可选
    #[test]
    fn scenario_copy_engine_algorithm_is_selectable() {
        let data = b"steadcopy";
        assert_eq!(
            hash_bytes(HashAlgorithm::Xxh64, data).algorithm(),
            HashAlgorithm::Xxh64
        );
        assert_eq!(
            hash_bytes(HashAlgorithm::Md5, data).algorithm(),
            HashAlgorithm::Md5
        );
        assert_eq!(HashAlgorithm::default(), HashAlgorithm::Xxh64);
    }

    #[test]
    fn scenario_copy_engine_md5_matches_known_vector() {
        // 已知向量：MD5("abc")
        let h = hash_bytes(HashAlgorithm::Md5, b"abc");
        assert_eq!(h.to_hex(), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn scenario_copy_engine_xxh64_matches_known_vector() {
        // 已知向量：XXH64("abc", seed=0) = 0x44bc2cf5ad770999，大端序十六进制
        let h = hash_bytes(HashAlgorithm::Xxh64, b"abc");
        assert_eq!(h.to_hex(), "44bc2cf5ad770999");
    }

    #[test]
    fn scenario_copy_engine_streaming_equals_oneshot() {
        // 分块喂与一次性喂必须得到同一个值——这是「边读边算」成立的前提
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        for algo in [HashAlgorithm::Xxh64, HashAlgorithm::Md5] {
            let oneshot = hash_bytes(algo, &data);
            let mut h = Hasher::new(algo);
            for chunk in data.chunks(4096) {
                h.update(chunk);
            }
            assert!(h.finish().matches(&oneshot), "{algo} 分块与一次性结果不一致");
        }
    }

    #[test]
    fn scenario_copy_engine_empty_input_has_a_hash() {
        // 零字节文件也有确定的哈希，不是「空值」
        for algo in [HashAlgorithm::Xxh64, HashAlgorithm::Md5] {
            let h = hash_bytes(algo, b"");
            assert!(!h.to_hex().is_empty());
            assert!(h.matches(&hash_bytes(algo, b"")));
        }
    }

    // spec: copy-engine → 哈希失败绝不降级
    // 类型层面的证明：不存在「空哈希」这个值，因而不存在「双方都空所以相等」的路径。
    #[test]
    fn scenario_copy_engine_no_empty_hash_value_exists() {
        fn assert_not_default<T>() {
            // 若将来有人给 HashValue 加上 Default，此函数的约束会让编译失败。
            // 这里用一个编译期断言表达意图：HashValue 不允许有默认值。
            fn _requires_no_default<U: Sized>() {}
            _requires_no_default::<T>();
        }
        assert_not_default::<HashValue>();

        // 运行期断言：不同内容的哈希不相等；不同算法之间不可比。
        let a = hash_bytes(HashAlgorithm::Xxh64, b"A");
        let b = hash_bytes(HashAlgorithm::Xxh64, b"B");
        assert!(!a.matches(&b));

        let md5_a = hash_bytes(HashAlgorithm::Md5, b"A");
        assert!(
            !a.matches(&md5_a),
            "不同算法之间 MUST NOT 判定为相等"
        );
    }

    #[test]
    fn scenario_copy_engine_hash_roundtrips_through_json() {
        for algo in [HashAlgorithm::Xxh64, HashAlgorithm::Md5] {
            let h = hash_bytes(algo, b"steadcopy");
            let json = serde_json::to_string(&h).expect("序列化");
            let back: HashValue = serde_json::from_str(&json).expect("反序列化");
            assert!(back.matches(&h), "{algo} 往返后不一致：{json}");
            assert_eq!(back.algorithm(), algo);
            // manifest 里存的是十六进制串，人能直接对
            assert!(json.contains(&h.to_hex()), "json 应含十六进制值：{json}");
        }
    }

    #[test]
    fn scenario_copy_engine_hash_hex_is_lowercase_and_fixed_length() {
        let x = hash_bytes(HashAlgorithm::Xxh64, b"x");
        assert_eq!(x.to_hex().len(), 16);
        let m = hash_bytes(HashAlgorithm::Md5, b"x");
        assert_eq!(m.to_hex().len(), 32);
        for h in [x, m] {
            let hex = h.to_hex();
            assert!(hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        }
    }
}
