#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Scenario → 测试 的覆盖自检。

规范：openspec/README.md → TDD 铁律 T1（每条 spec 场景对应一个同名测试）
      openspec/changes/add-steadcopy-release/specs/build-release/spec.md → 发布门控 R3

**这个脚本能证明什么、不能证明什么，先说清楚：**

- 能证明：某个能力的 `#### Scenario:` 条数 > 以 `scenario_<能力>_` 开头的测试条数。
  这时一定有场景没落测试，是硬缺口。
- 不能证明：数量对得上就都覆盖了。场景标题是中文、测试名是英文 snake_case，
  两者没有机械映射；数量相等只说明「没有明显缺口」。
  逐条对应仍然要人看——这是提醒，不是替代。

UI 类能力（ui-*、app-shell）没有自动化测试，靠真机走查，默认豁免并单独列出。
"""

import os
import re
import subprocess
import sys

for stream in (sys.stdout, sys.stderr):
    try:
        stream.reconfigure(encoding="utf-8")
    except Exception:
        pass

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# 只能靠真机走查的能力：不产出自动化测试，不参与数量比对
MANUAL_ONLY = {"app-shell", "ui-copy-confirm", "ui-history", "ui-onboarding",
               "ui-projects", "ui-settings", "ui-workbench"}

# 逐条豁免：某个场景不由同前缀的测试覆盖时，必须在这里写明理由。
# 键是**场景标题原文**——标题一改，豁免就自动失效、闸门重新报警，
# 不会出现「当年豁免过，后来场景改了没人发现」。
EXEMPT = {
    "build-release": {
        "断网机器安装成功": "真机走查（门控 R10）",
        "非管理员账户安装": "真机走查（门控 R10）",
        "三处校验码一致": "发布流程（门控 R12），不是代码行为",
        "用户可核对": "发布流程（门控 R12），不是代码行为",
        "不静默更新": "install_update 与 check_update 是两个命令，中间隔着用户的一次点击；"
                      "「不会自己装」这件事测不出来，只能看代码与真机走查（门控 R9）",
        "关闭后不联网": "check_update 在 update_check 为假时直接返回错误、不建 updater；"
                        "「没发出请求」要抓包才证得了，归真机走查",
        "任务进行中不打扰": "界面行为（有任务时不渲染更新入口），真机走查",
        "关于页显示构建信息": "界面呈现，真机走查",
        "任一门未过则不发版": "流程约束，不是代码行为",

        # 白名单的五条子场景由同一个测试一并覆盖。不拆成五个测试是因为它们验的是
        # 同一个函数的同一组判据，拆开只是把 assert 分行；失败信息里带着被拒的 URL，
        # 定位不靠测试名。
        "子域被拒": "由 scenario_build_release_update_download_host_is_allowlisted 覆盖",
        "userinfo 被拒": "同上",
        "http 被拒": "同上",
        "file 协议被拒": "同上",

        # 端点顺序与回落是**联网行为**：要证明「镜像可达时没发出对 GitHub 的请求」，
        # 得抓包或起假服务，两者都不是单测能做的。归门控 R16 与真机走查。
        "镜像可达时不走 GitHub": "联网行为，抓包才证得了；门控 R16 + 真机走查",
        "镜像不可达时回落 GitHub": "同上",
        "仓库私有时 GitHub 端点 404 而镜像仍可用": "同上（已在复核中实测过两个端点的 404 行为）",

        # （这里原先挂着五条【缺口】：Python 发布脚本的行为没有测试。
        #   现在 scripts/tests/ 用标准库 unittest 覆盖了，并有 selfproof.py
        #   逐道弄坏闸门确认测试真的会红——豁免已删。）
    },
    "device-registry": {
        "插入设备触发到达事件": "由 scenario_preset_autorun_mock_watcher_delivers_events 覆盖",
        "拔出设备触发移除事件": "由 scenario_preset_autorun_mock_watcher_delivers_events 覆盖",
        "启动时枚举既有设备": "应用层启动扫描，真机走查（门控 R9）",
        "盘符延迟分配": "应用层退避重试，真机走查（门控 R9）",
        "重试耗尽的降级": "应用层退避重试，真机走查（门控 R9）",
        "不向源卡写入标记": "由 scenario_copy_engine_source_card_is_untouched 覆盖",
    },
    "i18n": {
        "同一句话不存在第二份定义": "架构约束，机检不了；靠人审（core 产成句、界面只放自有文案）",
        "切换即时生效": "界面行为（切完 reload 重设语言），真机走查",
        "漏一条译文编译不过": "架构保证：Rust 侧穷尽 match、TS 侧 Record<Key,string>，机器测不了「编译失败」",
    },
    "preset-sinking": {
        "一次点击完成": "界面行为（SinkBar 点完即写、不跳编辑器），真机走查",
        "传输中可见且不抢焦点": "界面行为（行内提示条挂在进度面板里），真机走查",
        "任务结束后仍可沉淀": "界面行为（结果面板里保留同一个提示条），真机走查",
    },
    "format-card": {
        "倒计时归零前不可执行": "界面行为（CountdownConfirm），虚拟机验收 D-002",
        "无批量入口": "架构约束，人审；没有批量入口这件事测不出来，只能看代码",
        "缺少危险参数即拒绝": "命令行需交互终端，虚拟机验收",
    },
}


def scenarios_by_capability():
    caps = {}
    for dirpath, _dirs, files in os.walk(os.path.join(ROOT, "openspec")):
        if "spec.md" not in files:
            continue
        cap = os.path.basename(dirpath)
        text = open(os.path.join(dirpath, "spec.md"), encoding="utf-8").read()
        titles = re.findall(r"^#### Scenario:\s*(.+?)\s*$", text, re.M)
        caps.setdefault(cap, []).extend(titles)
    return caps


# 要列举的两个 workspace。`app/src-tauri` **不在**根 workspace 里
# （根 Cargo.toml 的 exclude 把它排除了），只跑 `--workspace` 会看不见它——
# 更新来源白名单、两版打包守卫那几条测试就住在那儿，漏掉的表现是
# 本脚本把它们对应的场景全判成缺口，然后有人为了让闸门变绿去加豁免。
WORKSPACES = [
    ["cargo", "test", "--workspace", "--", "--list"],
    ["cargo", "test", "--manifest-path", os.path.join("app", "src-tauri", "Cargo.toml"),
     "--", "--list"],
]


def python_test_names():
    """Python 测试的名字。

    发布脚本的场景由 `scripts/tests/` 下的 unittest 覆盖。方法名统一带 `test_` 前缀
    （unittest 靠它发现），这里剥掉，好与 Rust 侧的 `scenario_<能力>_` 归一。

    用 `TestLoader.discover` 而不是正则扫源码：拿的是**真实注册**的测试，
    与 Rust 侧用 `cargo test --list` 是同一个口径——写在文件里但没被收进去的不算。
    """
    import unittest

    names = set()
    tests_dir = os.path.join(ROOT, "scripts", "tests")
    if not os.path.isdir(tests_dir):
        return names
    sys.path.insert(0, tests_dir)
    try:
        suite = unittest.TestLoader().discover(tests_dir, top_level_dir=tests_dir)
    finally:
        sys.path.remove(tests_dir)

    def walk(s):
        for item in s:
            if isinstance(item, unittest.TestSuite):
                walk(item)
            else:
                n = item._testMethodName
                names.add(n[len("test_"):] if n.startswith("test_") else n)

    walk(suite)
    return names


def test_names():
    """列出全部测试名。用 `cargo test -- --list`，拿的是真实注册的测试。"""
    names = python_test_names()
    for cmd in WORKSPACES:
        out = subprocess.run(
            cmd, capture_output=True, text=True,
            encoding="utf-8", errors="replace", cwd=ROOT,
        )
        if out.returncode != 0:
            sys.stderr.write(out.stderr)
            raise SystemExit(f"失败：{' '.join(cmd)}")
        for line in out.stdout.splitlines():
            if line.endswith(": test"):
                names.add(line[: -len(": test")].strip().rsplit("::", 1)[-1])
    return names


def main():
    caps = scenarios_by_capability()
    names = test_names()

    rows, gaps, stale = [], [], []
    for cap in sorted(caps):
        titles = caps[cap]
        n_spec = len(titles)
        if cap in MANUAL_ONLY:
            rows.append((cap, n_spec, "—", 0, "真机走查"))
            continue

        exempt = EXEMPT.get(cap, {})
        # 豁免的标题必须在 spec 里真的存在，否则说明标题改过、豁免已过期
        for title in exempt:
            if title not in titles:
                stale.append((cap, title))
        n_exempt = sum(1 for t in titles if t in exempt)

        prefix = "scenario_" + cap.replace("-", "_") + "_"
        n_test = sum(1 for n in names if n.startswith(prefix))
        ok = n_test >= n_spec - n_exempt
        rows.append((cap, n_spec, n_test, n_exempt, "通过" if ok else "缺口"))
        if not ok:
            gaps.append((cap, n_spec - n_exempt, n_test))

    w = max(len(r[0]) for r in rows)
    print(f"{'能力'.ljust(w)}  场景  豁免  测试  结论")
    for cap, a, b, ex, note in rows:
        print(f"{cap.ljust(w)}  {str(a).rjust(4)}  {str(ex).rjust(4)}  "
              f"{str(b).rjust(4)}  {note}")

    auto = [r for r in rows if r[0] not in MANUAL_ONLY]
    print(f"\n自动化能力 {len(auto)} 个，场景 {sum(r[1] for r in auto)} 条"
          f"（豁免 {sum(r[3] for r in auto)} 条），对应测试 {sum(r[2] for r in auto)} 条")
    print("提醒：数量相等只说明没有明显缺口，逐条对应仍需人看。")

    if any(r[3] for r in auto):
        print("\n豁免明细（每条都要有理由，理由过期就该删）：")
        for cap in sorted(EXEMPT):
            for title, reason in EXEMPT[cap].items():
                print(f"   {cap} / {title} —— {reason}")

    if stale:
        print("\n[拦下] 以下豁免在 spec 里已找不到对应场景（标题改了？场景删了？）：",
              file=sys.stderr)
        for cap, title in stale:
            print(f"   {cap}：{title}", file=sys.stderr)
    if gaps:
        print("\n[拦下] 以下能力的测试数少于（场景数 − 豁免数）：", file=sys.stderr)
        for cap, need, have in gaps:
            print(f"   {cap}：需覆盖 {need}，测试 {have}", file=sys.stderr)
    return 1 if (gaps or stale) else 0


if __name__ == "__main__":
    raise SystemExit(main())
