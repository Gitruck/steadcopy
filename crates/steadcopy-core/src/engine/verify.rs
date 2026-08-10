//! 读回校验：从目的地无缓冲读回、重算哈希、与源哈希比对。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/copy-engine/spec.md`
//! → Requirement: 无缓冲读回校验 / 哈希失败绝不降级
//!
//! # 这里的两条命
//!
//! 1. **读回必须无缓冲。** 否则读到的可能是刚写入时留在页缓存里的副本，
//!    介质写坏完全测不出来。那样的校验比不校验更危险——它给用户假的确定性。
//! 2. **失败必须是 `Err`，不能是「相等」。** 前身项目哈希出错时返回空串，
//!    源与目标双双失败时 `'' === ''` 判定通过。本模块用类型堵死这条路：
//!    比对只接受两个**已经算出来的** `HashValue`，算不出来的情况根本走不到比对。

use std::path::Path;

use crate::engine::hasher::{HashAlgorithm, HashValue, Hasher};
use crate::error::Result;
use crate::platform::VolumeIo;

/// 一次读回校验的结论。
///
/// 刻意不用 `bool`：`Mismatch` 要带上实际算出的哈希，
/// 这样任务结果与报告里能给出「期望 X、实际 Y」，而不是干巴巴一句「校验失败」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyOutcome {
    Match,
    Mismatch { actual: HashValue },
}

impl VerifyOutcome {
    pub const fn is_match(&self) -> bool {
        matches!(self, VerifyOutcome::Match)
    }
}

/// 从目的地**无缓冲**读回文件、算哈希、与 `expected` 比对。
///
/// 返回 `Err` 表示这次校验**没做成**（读不了、算不出），
/// 调用方 MUST 把它当失败处理，**MUST NOT** 当作通过。
pub fn verify_destination(
    io: &dyn VolumeIo,
    path: &Path,
    expected: &HashValue,
) -> Result<VerifyOutcome> {
    let actual = hash_destination(io, path, expected.algorithm())?;
    Ok(if actual.matches(expected) {
        VerifyOutcome::Match
    } else {
        VerifyOutcome::Mismatch { actual }
    })
}

/// 无缓冲读回并算出哈希。任何 IO 失败都以 `Err` 传播。
pub fn hash_destination(
    io: &dyn VolumeIo,
    path: &Path,
    algorithm: HashAlgorithm,
) -> Result<HashValue> {
    let mut hasher = Hasher::new(algorithm);
    io.read_unbuffered(path, &mut |chunk| hasher.update(chunk))?;
    Ok(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::hasher::hash_bytes;
    use crate::platform::volume_io;
    use std::io::Write;
    use std::path::PathBuf;

    fn write_file(dir: &tempfile::TempDir, name: &str, data: &[u8]) -> PathBuf {
        let p = dir.path().join(name);
        let mut f = std::fs::File::create(&p).expect("建文件");
        f.write_all(data).expect("写入");
        f.sync_all().expect("落盘");
        p
    }

    // spec: copy-engine → 无缓冲读回校验 → Scenario: 篡改目的地后校验失败
    #[test]
    fn scenario_copy_engine_verify_detects_tampered_destination() {
        let io = volume_io();
        let dir = tempfile::tempdir().expect("临时目录");
        let original = "steadcopy 原始内容".as_bytes().repeat(500);
        let p = write_file(&dir, "a.bin", &original);
        let expected = hash_bytes(HashAlgorithm::Xxh64, &original);

        // 未被动过：通过
        assert_eq!(
            verify_destination(io.as_ref(), &p, &expected).expect("校验应能执行"),
            VerifyOutcome::Match
        );

        // 篡改目的地上的字节
        let mut tampered = original.clone();
        let mid = tampered.len() / 2;
        tampered[mid] ^= 0xFF;
        write_file(&dir, "a.bin", &tampered);

        // MUST 报不一致，且给出实际哈希
        match verify_destination(io.as_ref(), &p, &expected).expect("校验应能执行") {
            VerifyOutcome::Mismatch { actual } => {
                assert!(!actual.matches(&expected));
                assert!(actual.matches(&hash_bytes(HashAlgorithm::Xxh64, &tampered)));
            }
            VerifyOutcome::Match => panic!("被篡改的文件 MUST NOT 判定为通过"),
        }
    }

    #[test]
    fn scenario_copy_engine_verify_detects_truncation() {
        let io = volume_io();
        let dir = tempfile::tempdir().expect("临时目录");
        let original = b"0123456789".repeat(1000);
        let p = write_file(&dir, "t.bin", &original);
        let expected = hash_bytes(HashAlgorithm::Xxh64, &original);

        // 截断成一半——这是「写到一半就断了」的典型形态
        write_file(&dir, "t.bin", &original[..original.len() / 2]);
        assert!(
            !verify_destination(io.as_ref(), &p, &expected)
                .expect("校验应能执行")
                .is_match(),
            "被截断的文件 MUST NOT 判定为通过"
        );
    }

    // spec: copy-engine → 哈希失败绝不降级 → Scenario: 目的地读回失败
    #[test]
    fn scenario_copy_engine_verify_read_failure_is_error_not_pass() {
        let io = volume_io();
        let dir = tempfile::tempdir().expect("临时目录");
        let missing = dir.path().join("不存在.bin");
        let expected = hash_bytes(HashAlgorithm::Xxh64, b"whatever");

        let err = verify_destination(io.as_ref(), &missing, &expected)
            .expect_err("读不到文件 MUST 报错，MUST NOT 判定为通过");
        // 关键断言：失败是 Err，不是 Ok(Match)
        assert!(err.to_string().contains("读取") || err.context().path.is_some());
    }

    #[test]
    fn scenario_copy_engine_verify_empty_file_has_definite_result() {
        let io = volume_io();
        let dir = tempfile::tempdir().expect("临时目录");
        let p = write_file(&dir, "empty.bin", b"");
        let expected = hash_bytes(HashAlgorithm::Xxh64, b"");
        assert_eq!(
            verify_destination(io.as_ref(), &p, &expected).expect("零字节也要能校验"),
            VerifyOutcome::Match
        );
        // 零字节文件与非零文件的哈希不同——不存在「都算不出所以相等」
        let non_empty = hash_bytes(HashAlgorithm::Xxh64, b"x");
        assert!(!verify_destination(io.as_ref(), &p, &non_empty)
            .expect("应能执行")
            .is_match());
    }

    #[test]
    fn scenario_copy_engine_verify_algorithm_follows_expected() {
        let io = volume_io();
        let dir = tempfile::tempdir().expect("临时目录");
        let data = b"algo follows expected";
        let p = write_file(&dir, "algo.bin", data);

        for algo in [HashAlgorithm::Xxh64, HashAlgorithm::Md5] {
            let expected = hash_bytes(algo, data);
            let out = verify_destination(io.as_ref(), &p, &expected).expect("校验");
            assert_eq!(out, VerifyOutcome::Match, "{algo} 应通过");
        }
    }

    #[test]
    fn scenario_copy_engine_verify_cross_algorithm_never_matches() {
        // 期望值是 MD5，读回按 MD5 算——不会因为算法混用而误判
        let io = volume_io();
        let dir = tempfile::tempdir().expect("临时目录");
        let data = b"cross algo";
        let p = write_file(&dir, "x.bin", data);
        let md5_expected = hash_bytes(HashAlgorithm::Md5, data);
        let xxh_of_same = hash_bytes(HashAlgorithm::Xxh64, data);
        assert!(!md5_expected.matches(&xxh_of_same));
        assert!(verify_destination(io.as_ref(), &p, &md5_expected)
            .expect("校验")
            .is_match());
    }
}
