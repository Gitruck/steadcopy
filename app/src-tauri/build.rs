//! 构建期把「这个二进制到底是哪次编译出来的」钉进程序里。
//!
//! 规范：`openspec/changes/add-steadcopy-release/specs/build-release/spec.md`
//! → Requirement: 构建元信息可查
//!
//! 用户报问题时说「我用的是 0.1.0」远远不够——同一个版本号可能对应几十次编译。
//! 提交号 + 工作区是否干净，才定得住来源。

use std::process::Command;

fn main() {
    // 发布脚本会显式传时间戳；本地开发时退化为「本次编译时刻」
    println!("cargo:rerun-if-env-changed=STEADCOPY_BUILD_TIME");
    let build_time = std::env::var("STEADCOPY_BUILD_TIME").unwrap_or_else(|_| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().to_string())
            .unwrap_or_else(|_| String::new())
    });
    println!("cargo:rustc-env=STEADCOPY_BUILD_TIME={build_time}");

    println!("cargo:rustc-env=STEADCOPY_COMMIT={}", git_commit());
    println!("cargo:rustc-env=STEADCOPY_RUSTC={}", rustc_version());

    tauri_build::build()
}

/// 短提交号；工作区有未提交改动时加 `-dirty`，不装作是干净构建。
fn git_commit() -> String {
    let Some(rev) = run("git", &["rev-parse", "--short", "HEAD"]) else {
        return "未知（非 git 工作区）".into();
    };
    match run("git", &["status", "--porcelain"]) {
        Some(s) if !s.is_empty() => format!("{rev}-dirty"),
        _ => rev,
    }
}

fn rustc_version() -> String {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    run(&rustc, &["-V"]).unwrap_or_else(|| "未知".into())
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
