//! HTML 人话报告：单文件、自包含、可离线打开、可打印为 PDF。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/task-ledger/spec.md`
//! → Requirement: HTML 人话报告
//!
//! 决策 P6：报告形态是 HTML，**不引 PDF 引擎**——浏览器自带的打印就能出 PDF，
//! 为此背一个几十 MB 的渲染器不划算。
//!
//! 措辞面向非技术用户：「校验」「哈希」这类词可以出现（目标人群听得懂拷卡黑话），
//! 但失败原因 MUST 是人话，不是错误码或英文异常名。

use std::fmt::Write as _;
use std::path::Path;

use time::OffsetDateTime;

use crate::i18n::Locale;
use crate::manifest::store::format_time_human;
use crate::manifest::{AuditReport, Manifest};

/// 报告的数据来源。可以由一次任务生成，也可以由一份清单生成。
#[derive(Debug, Clone)]
pub struct ReportInput<'a> {
    pub manifest: &'a Manifest,
    /// 本次任务中最终失败的文件（路径, 原因, 重试次数）
    pub failures: &'a [(String, String, u32)],
    /// 本次跳过的文件数（此前已拷并校验通过）
    pub skipped: usize,
    /// 需要呈现的提示（账本降级等）
    pub notices: &'a [String],
    /// 任务耗时（秒）。`None` 表示报告由清单生成、无耗时信息
    pub elapsed_secs: Option<u64>,
    pub generated_at: OffsetDateTime,
    /// 若同时做了复验，附上四态结果
    pub audit: Option<&'a AuditReport>,
    /// 报告用哪种语言。**报告是要拿给客户看的**，所以它跟界面同一份设置
    pub lang: Locale,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.2} {}", UNITS[i])
    }
}

fn human_duration(secs: u64, lang: Locale) -> String {
    match (secs, lang) {
        (0, Locale::Zh) => "不到 1 秒".to_string(),
        (0, Locale::En) => "under 1s".to_string(),
        (1..=59, Locale::Zh) => format!("{secs} 秒"),
        (1..=59, Locale::En) => format!("{secs}s"),
        (60..=3599, Locale::Zh) => format!("{} 分 {} 秒", secs / 60, secs % 60),
        (60..=3599, Locale::En) => format!("{}m {}s", secs / 60, secs % 60),
        (_, Locale::Zh) => format!("{} 小时 {} 分", secs / 3600, (secs % 3600) / 60),
        (_, Locale::En) => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// 样式内联，**无任何外部资源**——报告拷到别的机器、断网也能原样打开。
const STYLE: &str = r#"
:root{--fg:#1a1d21;--muted:#6b7280;--line:#e5e7eb;--ok:#15803d;--okbg:#f0fdf4;
--warn:#b45309;--warnbg:#fffbeb;--bad:#b91c1c;--badbg:#fef2f2;--accent:#1f2937}
*{box-sizing:border-box}
body{margin:0;padding:32px;font:14px/1.7 -apple-system,"Segoe UI","Microsoft YaHei",sans-serif;
color:var(--fg);background:#fff;max-width:1000px;margin-inline:auto}
h1{font-size:22px;margin:0 0 4px}
h2{font-size:15px;margin:28px 0 10px;padding-bottom:6px;border-bottom:1px solid var(--line)}
.sub{color:var(--muted);font-size:12px;margin-bottom:20px}
.verdict{padding:14px 16px;border-radius:8px;border:1px solid;font-size:15px;font-weight:600;margin:16px 0}
.v-ok{color:var(--ok);background:var(--okbg);border-color:#bbf7d0}
.v-warn{color:var(--warn);background:var(--warnbg);border-color:#fde68a}
.v-bad{color:var(--bad);background:var(--badbg);border-color:#fecaca}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:10px;margin:14px 0}
.cell{border:1px solid var(--line);border-radius:8px;padding:10px 12px}
.cell .k{color:var(--muted);font-size:12px}
.cell .v{font-size:15px;font-weight:600;margin-top:2px;word-break:break-all}
table{width:100%;border-collapse:collapse;font-size:12.5px}
th,td{text-align:left;padding:7px 8px;border-bottom:1px solid var(--line);vertical-align:top}
th{color:var(--muted);font-weight:600;background:#fafafa}
td.mono,.mono{font-family:ui-monospace,"Cascadia Mono",Consolas,monospace;font-size:11.5px;color:var(--muted)}
td.num{text-align:right;white-space:nowrap}
.tag{display:inline-block;padding:1px 7px;border-radius:99px;font-size:11px;font-weight:600}
.t-ok{color:var(--ok);background:var(--okbg)}
.t-warn{color:var(--warn);background:var(--warnbg)}
.t-bad{color:var(--bad);background:var(--badbg)}
.notice{background:var(--warnbg);border-left:3px solid #fbbf24;padding:8px 12px;margin:8px 0;font-size:13px}
footer{margin-top:32px;padding-top:12px;border-top:1px solid var(--line);color:var(--muted);font-size:11.5px}
@media print{body{padding:0;max-width:none}h2{break-after:avoid}tr{break-inside:avoid}
.verdict{break-inside:avoid}thead{display:table-header-group}}
"#;

/// 生成一份自包含的 HTML 报告。
pub fn render_report(input: &ReportInput<'_>) -> String {
    let lang = input.lang;
    // 取文案的小助手。报告里几十处文案，每处都写一个 match 会把渲染逻辑淹掉
    let w = |zh: &'static str, en: &'static str| lang.pick(zh, en);
    let m = input.manifest;
    let total = m.entries.len();
    let verified = m.verified_count();
    let failed = input.failures.len();

    let mut h = String::with_capacity(16 * 1024);
    let _ = write!(
        h,
        "<!DOCTYPE html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n",
        w("zh-CN", "en")
    );
    let _ = write!(
        h,
        "<title>{} · {} · {}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n",
        w("拷卡报告", "Copy report"),
        esc(&m.project),
        esc(&m.source.display_name)
    );

    let _ = write!(
        h,
        "<h1>{}</h1>\n<div class=\"sub\">{}</div>\n",
        w("拷卡报告", "Copy report"),
        esc(&match lang {
            Locale::Zh => format!(
                "项目「{}」 · 来源「{}」 · 生成于 {}",
                m.project,
                m.source.display_name,
                format_time_human(input.generated_at)
            ),
            Locale::En => format!(
                "Project {} · Source {} · Generated {}",
                m.project,
                m.source.display_name,
                format_time_human(input.generated_at)
            ),
        })
    );

    // ---- 结论：报告最该被一眼看到的东西 ----
    let (cls, verdict) = if failed > 0 {
        (
            "v-bad",
            match lang {
                Locale::Zh => format!(
                    "部分失败：{verified} 个文件校验通过，{failed} 个失败（详见下方失败清单）"
                ),
                Locale::En => format!(
                    "Partly failed: {verified} verified, {failed} failed — see the failure list below"
                ),
            },
        )
    } else if verified == total && total > 0 {
        (
            "v-ok",
            match lang {
                Locale::Zh => {
                    format!("全部 {total} 个文件校验通过，共 {}", human_bytes(m.total_bytes()))
                }
                Locale::En => format!(
                    "All {total} file(s) verified · {} total",
                    human_bytes(m.total_bytes())
                ),
            },
        )
    } else if total == 0 {
        (
            "v-warn",
            w("本次没有拷贝任何文件", "Nothing was copied this time").to_string(),
        )
    } else {
        (
            "v-warn",
            match lang {
                Locale::Zh => {
                    format!("{total} 个文件已拷贝，但本次未开启校验——无法确认写入是否完好")
                }
                Locale::En => format!(
                    "{total} file(s) copied, but verification was off — whether they landed intact is unknown"
                ),
            },
        )
    };
    let _ = writeln!(h, "<div class=\"verdict {cls}\">{}</div>", esc(&verdict));

    for n in input.notices {
        let _ = writeln!(h, "<div class=\"notice\">{}</div>", esc(n));
    }

    // ---- 概要 ----
    let _ = writeln!(h, "<h2>{}</h2>\n<div class=\"grid\">", w("概要", "Summary"));
    let mut cell = |k: &str, v: &str| {
        let _ = writeln!(
            h,
            "<div class=\"cell\"><div class=\"k\">{}</div><div class=\"v\">{}</div></div>",
            esc(k),
            esc(v)
        );
    };
    cell(w("项目", "Project"), &m.project);
    cell(w("来源设备", "Source device"), &m.source.display_name);
    cell(
        w("目的地", "Destination"),
        &m.destination_root.display().to_string(),
    );
    cell(w("文件数", "Files"), &format!("{total}"));
    cell(w("总大小", "Total size"), &human_bytes(m.total_bytes()));
    cell(
        w("校验", "Verification"),
        &if verified == total && total > 0 {
            format!("{} · {}", w("已校验", "Verified"), m.algorithm)
        } else if verified == 0 {
            w("未开启", "Off").to_string()
        } else {
            format!(
                "{verified}/{total} {} · {}",
                w("已校验", "verified"),
                m.algorithm
            )
        },
    );
    if input.skipped > 0 {
        cell(
            w("已跳过", "Skipped"),
            &match lang {
                Locale::Zh => format!("{} 个（此前已拷并校验通过）", input.skipped),
                Locale::En => format!("{} (already copied and verified)", input.skipped),
            },
        );
    }
    if let Some(s) = input.elapsed_secs {
        cell(w("耗时", "Elapsed"), &human_duration(s, lang));
        if s > 0 {
            cell(
                w("平均速度", "Average speed"),
                &format!("{}/s", human_bytes(m.total_bytes() / s.max(1))),
            );
        }
    }
    cell(w("完成时间", "Completed"), &format_time_human(m.created_at));
    h.push_str("</div>\n");

    // ---- 失败清单：MUST 显著，不折叠 ----
    if failed > 0 {
        let _ = writeln!(
            h,
            "<h2>{}</h2>\n<table>\n<thead><tr><th>{}</th><th>{}</th><th class=\"num\">{}</th></tr></thead>\n<tbody>",
            w("失败清单", "Failures"),
            w("文件", "File"),
            w("原因", "Reason"),
            w("重试", "Retries")
        );
        for (path, reason, retries) in input.failures {
            let _ = writeln!(
                h,
                "<tr><td>{}</td><td>{}</td><td class=\"num\">{retries}</td></tr>",
                esc(path),
                esc(reason)
            );
        }
        h.push_str("</tbody>\n</table>\n");
    }

    // ---- 复验四态（若有）----
    if let Some(a) = input.audit {
        let c = a.counts();
        let _ = writeln!(
            h,
            "<h2>{}</h2>\n<div class=\"grid\">",
            w("复验结果", "Re-verification")
        );
        for (k, v, t) in [
            (w("一致", "Intact"), c.intact, "t-ok"),
            (w("已移动", "Moved"), c.moved, "t-warn"),
            (w("丢失", "Missing"), c.missing, "t-bad"),
            (w("新增", "Added"), c.added, "t-warn"),
        ] {
            let _ = writeln!(
                h,
                "<div class=\"cell\"><div class=\"k\">{k}</div><div class=\"v\"><span class=\"tag {t}\">{v}</span></div></div>"
            );
        }
        h.push_str("</div>\n");
        if !a.complete {
            let _ = writeln!(
                h,
                "<div class=\"notice\">{}</div>",
                w(
                    "复验被中断，结果不完整。",
                    "Re-verification was interrupted; the result is incomplete."
                )
            );
        }
        if !a.missing.is_empty() {
            let _ = writeln!(
                h,
                "<table>\n<thead><tr><th>{}</th><th class=\"num\">{}</th><th>{}</th></tr></thead>\n<tbody>",
                w("丢失的文件", "Missing files"),
                w("大小", "Size"),
                w("期望校验值", "Expected hash")
            );
            for x in &a.missing {
                let _ = writeln!(
                    h,
                    "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"mono\">{}</td></tr>",
                    esc(&x.relative_path),
                    human_bytes(x.size),
                    esc(&x.expected_hash)
                );
            }
            h.push_str("</tbody>\n</table>\n");
        }
    }

    // ---- 文件清单 ----
    let _ = writeln!(
        h,
        "<h2>{}</h2>\n<table>\n<thead><tr><th>{}</th><th class=\"num\">{}</th><th>{}</th><th>{}</th></tr></thead>\n<tbody>",
        w("文件清单", "File list"),
        w("文件", "File"),
        w("大小", "Size"),
        w("校验值", "Hash"),
        w("状态", "Status")
    );
    for e in &m.entries {
        let (tag, label) = if e.verify.is_verified() {
            ("t-ok", w("已校验", "Verified"))
        } else {
            ("t-warn", w("未校验", "Not verified"))
        };
        let _ = writeln!(
            h,
            "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"mono\">{}</td><td><span class=\"tag {tag}\">{label}</span></td></tr>",
            esc(&e.relative_path),
            human_bytes(e.size),
            esc(&e.source_hash.to_hex())
        );
    }
    h.push_str("</tbody>\n</table>\n");

    let _ = writeln!(
        h,
        "<footer>{}</footer>",
        esc(&match lang {
            Locale::Zh => format!(
                "由 {} {} 生成 · 校验算法 {} · 本报告为单文件，可离线打开，也可用浏览器打印为 PDF",
                m.generator.name, m.generator.version, m.algorithm
            ),
            Locale::En => format!(
                "Generated by {} {} · hash {} · a single self-contained file: opens offline, prints to PDF",
                m.generator.name, m.generator.version, m.algorithm
            ),
        })
    );
    h.push_str("</body>\n</html>\n");
    h
}

/// 生成并写出报告文件。
pub fn write_report(path: &Path, input: &ReportInput<'_>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, render_report(input))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{hash_bytes, HashAlgorithm};
    use crate::manifest::model::{ManifestEntry, SourceRef, VerifyState};
    use time::macros::datetime;

    fn manifest(verified: bool, n: usize) -> Manifest {
        let at = datetime!(2026-08-08 09:30:00 UTC);
        let mut m = Manifest::new(
            SourceRef {
                id: "vol-1".into(),
                display_name: "A7M4主卡".into(),
            },
            "婚礼<张先生>",
            r"D:\素材\婚礼",
            HashAlgorithm::Xxh64,
            at,
        );
        for i in 0..n {
            let name = format!("A{i:04}.MP4");
            let h = hash_bytes(HashAlgorithm::Xxh64, name.as_bytes());
            m.entries.push(ManifestEntry {
                relative_path: name,
                size: 1024 * 1024,
                source_hash: h,
                verify: if verified {
                    VerifyState::Verified {
                        destination_hash: h,
                    }
                } else {
                    VerifyState::NotVerified
                },
                source_modified_at: None,
                completed_at: at,
                retries: 0,
            });
        }
        m
    }

    fn input<'a>(m: &'a Manifest, failures: &'a [(String, String, u32)]) -> ReportInput<'a> {
        ReportInput {
            lang: Locale::Zh,
            manifest: m,
            failures,
            skipped: 0,
            notices: &[],
            elapsed_secs: Some(125),
            generated_at: datetime!(2026-08-08 09:32:05 UTC),
            audit: None,
        }
    }

    // spec: task-ledger → HTML 人话报告 → Scenario: 报告自包含可离线打开
    #[test]
    fn scenario_task_ledger_report_is_self_contained() {
        let m = manifest(true, 3);
        let html = render_report(&input(&m, &[]));
        // 无任何外部资源引用
        for forbidden in ["<link", "<script", "src=\"http", "@import", "url(http"] {
            assert!(
                !html.contains(forbidden),
                "报告 MUST NOT 依赖外部资源，发现 {forbidden}"
            );
        }
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<style>"), "样式必须内联");
        assert!(html.trim_end().ends_with("</html>"));
    }

    // spec: → Scenario: 报告含校验结论
    #[test]
    fn scenario_task_ledger_report_states_verification_verdict() {
        let m = manifest(true, 3);
        let html = render_report(&input(&m, &[]));
        assert!(
            html.contains("全部 3 个文件校验通过"),
            "显著位置应给出中文校验结论"
        );
        assert!(html.contains("xxh64"), "应注明算法");
    }

    #[test]
    fn scenario_task_ledger_report_marks_unverified_clearly() {
        let m = manifest(false, 2);
        let html = render_report(&input(&m, &[]));
        assert!(html.contains("未开启校验"), "未校验时必须明说");
        assert!(html.contains("无法确认写入是否完好"));
        assert!(!html.contains("全部 2 个文件校验通过"));
    }

    // spec: → Scenario: 失败清单不被隐藏
    #[test]
    fn scenario_task_ledger_report_failures_are_prominent() {
        let m = manifest(true, 2);
        let failures = vec![(
            "A0003.MP4".to_string(),
            "校验不一致：期望 abc，实际 def".to_string(),
            2u32,
        )];
        let html = render_report(&input(&m, &failures));
        assert!(html.contains("失败清单"));
        assert!(html.contains("A0003.MP4"));
        assert!(html.contains("校验不一致"));
        // 结论行 MUST NOT 说「全部通过」
        assert!(html.contains("部分失败"));
        assert!(!html.contains("全部 2 个文件校验通过"));
        // 失败清单 MUST NOT 被折叠
        assert!(!html.contains("<details"));
    }

    // spec: → Scenario: 报告可打印
    #[test]
    fn scenario_task_ledger_report_has_print_styles() {
        let html = render_report(&input(&manifest(true, 1), &[]));
        assert!(html.contains("@media print"), "应含打印样式");
        assert!(html.contains("break-inside"), "应避免表格行被打印截断");
        assert!(html.contains("打印为 PDF"), "应告知用户可打印成 PDF");
    }

    #[test]
    fn scenario_task_ledger_report_escapes_html_in_user_content() {
        // 项目名里带尖括号，MUST NOT 破坏结构或注入
        let m = manifest(true, 1);
        let html = render_report(&input(&m, &[]));
        assert!(html.contains("婚礼&lt;张先生&gt;"));
        assert!(!html.contains("婚礼<张先生>"));
    }

    #[test]
    fn scenario_task_ledger_report_includes_audit_when_present() {
        let m = manifest(true, 2);
        let observed = vec![];
        let a = crate::manifest::audit(&m, &observed, true);
        let mut i = input(&m, &[]);
        i.audit = Some(&a);
        let html = render_report(&i);
        assert!(html.contains("复验结果"));
        assert!(html.contains("丢失"));
        assert!(html.contains("A0000.MP4"), "丢失清单应列出文件");
    }

    #[test]
    fn scenario_task_ledger_report_writes_to_disk() {
        let dir = tempfile::tempdir().expect("临时目录");
        let p = dir.path().join("报告").join("r.html");
        let m = manifest(true, 1);
        write_report(&p, &input(&m, &[])).expect("写报告");
        let text = std::fs::read_to_string(&p).expect("读回");
        assert!(text.contains("拷卡报告"));
    }

    #[test]
    fn scenario_task_ledger_report_time_is_human_readable() {
        let m = manifest(true, 1);
        let html = render_report(&input(&m, &[]));
        assert!(html.contains("2026-08-08 09:30:00"), "时间应为人读格式");
        assert!(!html.contains("09:30:00Z"), "报告里不该出现裸 RFC3339");
        assert!(!html.contains("T09:30"), "报告里不该出现 ISO 的 T 分隔符");
    }

    #[test]
    fn scenario_task_ledger_report_sub_second_duration() {
        let m = manifest(true, 1);
        let mut i = input(&m, &[]);
        i.elapsed_secs = Some(0);
        let html = render_report(&i);
        assert!(html.contains("不到 1 秒"), "0 秒应说人话");
    }

    #[test]
    fn scenario_task_ledger_report_empty_manifest() {
        let m = manifest(true, 0);
        let html = render_report(&input(&m, &[]));
        assert!(html.contains("本次没有拷贝任何文件"));
    }
}
