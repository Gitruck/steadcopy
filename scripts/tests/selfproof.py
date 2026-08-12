#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""自证：把每道闸门逐个弄坏，确认对应的测试真的会红。

    python scripts/tests/selfproof.py

# 为什么要有这个

一条永远绿的测试和没有测试是一回事，而且更糟——它让人以为这块有人看着。
「探针先自证能响」这条纪律在这里同样适用：护栏写完必须证明它**响过**。

做法：对被测脚本的源码做一处最小改动（注释掉那道断言 / 把阈值调到不可能触发），
在临时副本上跑对应的测试，确认它红；然后还原。**不动仓库里的文件**——
全部在 tempdir 里的副本上做，跑完即弃。
"""

import io
import os
import re
import shutil
import subprocess
import sys
import tempfile

for stream in (sys.stdout, sys.stderr):
    try:
        stream.reconfigure(encoding="utf-8")
    except Exception:
        pass

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPTS = os.path.dirname(HERE)
ROOT = os.path.dirname(SCRIPTS)

# (说明, 被弄坏的脚本, 正则, 替换成, 应当变红的测试)
#
# 每一处替换都要**恰好拆掉那一道闸**，不能顺手拆别的——
# 拆多了会分不清「是这道闸响的还是别的响的」。
BREAKAGES = [
    (
        "体积量级断言（精简版超标不再拦）",
        "build-release.py",
        r"if flavor == \"slim\" and mb > SLIM_MAX_MB:",
        'if flavor == "slim" and False:',
        "test_scenario_build_release_size_mismatch_refuses_to_ship",
    ),
    (
        "体积量级断言（离线版不足不再拦）",
        "build-release.py",
        r"if flavor == \"offline\" and mb < OFFLINE_MIN_MB:",
        'if flavor == "offline" and False:',
        "test_scenario_build_release_size_mismatch_refuses_to_ship",
    ),
    (
        "「产物不止一个就炸」（改成随便挑一个）",
        "build-release.py",
        r"if len\(exes\) != 1:",
        "if False:",
        "test_scenario_build_release_stale_bundle_is_cleared_before_each_build",
    ),
    (
        "flavor 后缀（改成永远不加 -offline，退化成按文件名区分）",
        "build-release.py",
        r"\{'-offline' if flavor == 'offline' else ''\}",
        "",
        "test_scenario_build_release_flavors_told_apart_by_build_order",
    ),
    (
        "release/ 开工清空（改成什么都不挪）",
        "build-release.py",
        r"leftovers = \[f for f in os\.listdir\(RELEASE\) if not f\.startswith\(\"_\"\)\]",
        "leftovers = []",
        "test_scenario_build_release_previous_run_artifacts_are_moved_aside",
    ),
    (
        "校验码比对（改成不比）",
        "publish-mirror.py",
        r"elif sha256\(p\) != digest:",
        "elif False:",
        "test_scenario_build_release_mirror_refuses_when_bytes_differ",
    ),
    (
        "清单 url 必须在本镜像下（改成不查）",
        "publish-mirror.py",
        r"if not url\.startswith\(PUBLIC_BASE \+ \"/\"\):",
        "if False:",
        "test_scenario_build_release_manifest_points_into_this_mirror",
    ),
    (
        # 复刻**真正的旧 bug**：`want` 取自 head 而不是 local。
        # 空正文时 want=0，两边都切成 b"" 判等通过——「地址通了但一个字节都取不到」
        # 就这样被放行。第一版自证脚本这里写错了（只在条件后面加了 `and False`，
        # 而后面的 elif 仍然拦得住），自证当场把这个错抓了出来。
        "回读长度断言（复刻旧 bug：want 取自 head）",
        "publish-mirror.py",
        r"want = min\(len\(local\), 1024\)",
        "want = len(head)",
        "test_scenario_build_release_readback_failure_blocks_announcement",
    ),
    (
        "「没加 --publish-manifest 就不写清单」（改成总是写）",
        "publish-mirror.py",
        r"if not args\.publish_manifest:",
        "if False:",
        "test_scenario_build_release_manifest_stays_put_until_gate_passes",
    ),
    (
        "先包后清单（改成先写清单再复制）",
        "publish-mirror.py",
        r'    print\("\\n复制安装包："\)',
        '    io.open(os.path.join(args.nas, "latest.json"), "w", encoding="utf-8",'
        ' newline="\\n").write(json.dumps(manifest, ensure_ascii=False, indent=2) + "\\n")\n'
        '    print("\\n复制安装包：")',
        "test_scenario_build_release_manifest_is_written_after_packages",
    ),
    (
        "撤回时不还原清单（改成什么都不做）",
        "publish-mirror.py",
        r"    shutil\.copy2\(prev, live\)",
        "    pass",
        "test_scenario_build_release_rollback_restores_previous_manifest",
    ),
]


def run_one(desc, script, pattern, repl, test):
    """在临时副本里弄坏 `script`，跑 `test`，返回它是否变红。"""
    work = tempfile.mkdtemp(prefix="steadcopy-selfproof-")
    try:
        shutil.copytree(SCRIPTS, os.path.join(work, "scripts"))
        target = os.path.join(work, "scripts", script)
        src = io.open(target, encoding="utf-8").read()
        broken, n = re.subn(pattern, repl, src, count=1)
        if n != 1:
            return None, f"改不动：{pattern} 在 {script} 里匹配到 {n} 处（源码变了？）"
        io.open(target, "w", encoding="utf-8", newline="\n").write(broken)

        out = subprocess.run(
            [sys.executable, "-m", "unittest", "-v",
             f"test_{'build_release' if script.startswith('build') else 'publish_mirror'}"
             f".{'BuildRelease' if script.startswith('build') else 'PublishMirror'}.{test}"],
            cwd=os.path.join(work, "scripts", "tests"),
            capture_output=True, text=True, encoding="utf-8", errors="replace",
        )
        went_red = out.returncode != 0
        return went_red, (out.stderr or "").strip().splitlines()[-1] if not went_red else ""
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main():
    print(f"自证 {len(BREAKAGES)} 道闸门（每道单独弄坏一次，确认对应测试变红）\n")
    bad = []
    for desc, script, pattern, repl, test in BREAKAGES:
        red, why = run_one(desc, script, pattern, repl, test)
        if red is True:
            print(f"  ✓ {desc}", flush=True)
        elif red is None:
            print(f"  ! {desc} —— {why}", flush=True)
            bad.append((desc, why))
        else:
            print(f"  ✗ {desc} —— 弄坏了测试却还是绿的{('：' + why) if why else ''}", flush=True)
            bad.append((desc, "闸门没响"))

    if bad:
        print(f"\n[拦下] {len(bad)} 道闸门没有自证成功：", file=sys.stderr)
        for desc, why in bad:
            print(f"   {desc}：{why}", file=sys.stderr)
        print("\n一条永远绿的测试比没有测试更糟——它让人以为这块有人看着。", file=sys.stderr)
        return 1

    print(f"\n{len(BREAKAGES)} 道闸门全部自证能响。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
