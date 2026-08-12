#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""发一个版本：改版本号 → 提交 → 打标签 → 推。CI 接手。

    python scripts/release.py 0.1.1          # 走完整流程（推之前会问一次）
    python scripts/release.py 0.1.1 --local  # 只改本地，不提交不推
    python scripts/release.py --check        # 只看现在版本号一致不一致

# 为什么要有这个脚本

版本号写在**三个地方**：根 `Cargo.toml`、`app/src-tauri/Cargo.toml`、
`app/src-tauri/tauri.conf.json`。手改三处，早晚漏一处。漏了的后果不是编译不过，
而是**发出去之后**：更新清单说 0.1.1、程序自报 0.1.0，于是每次检查更新都提示有新版，
装完还提示——用户会以为更新功能坏了。

（`scenario_build_release_version_is_defined_once` 也钉着这条，
所以漏改在测试阶段就会红。这个脚本是让它压根不会发生。）

# 推标签之后会发生什么

`.github/workflows/release.yml` 认 `v*` 标签：打两个安装包 + 便携版 + 校验码 +
两份更新清单，传成**草稿** Release。草稿是刻意的——门控清单里有一半机器测不了
（真机走查、虚拟机验收、误报申报回执），自动发布等于跳过它们。
"""

import argparse
import io
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

# (相对路径, 匹配这一行的正则)。三处都要改，改漏一处就是上面说的那个坑。
SITES = [
    ("Cargo.toml", re.compile(r'^(version\s*=\s*")([^"]+)(")', re.M)),
    (os.path.join("app", "src-tauri", "Cargo.toml"), re.compile(r'^(version\s*=\s*")([^"]+)(")', re.M)),
    (os.path.join("app", "src-tauri", "tauri.conf.json"), re.compile(r'^(\s*"version":\s*")([^"]+)(")', re.M)),
]


def read_versions():
    out = {}
    for rel, pat in SITES:
        text = io.open(os.path.join(ROOT, rel), encoding="utf-8").read()
        m = pat.search(text)
        if not m:
            raise SystemExit(f"{rel} 里找不到 version")
        out[rel] = m.group(2)
    return out


def git(*args, capture=True):
    r = subprocess.run(["git", *args], cwd=ROOT, capture_output=capture,
                       text=True, encoding="utf-8", errors="replace")
    if r.returncode != 0:
        raise SystemExit(f"git {' '.join(args)} 失败：{(r.stderr or '').strip()}")
    return (r.stdout or "").strip()


def check():
    vs = read_versions()
    uniq = set(vs.values())
    for rel, v in vs.items():
        print(f"   {v}  {rel}")
    if len(uniq) != 1:
        raise SystemExit("\n三处版本号不一致——先跑一次本脚本把它们对齐")
    print(f"\n一致：{uniq.pop()}")


def bump(new):
    if not re.fullmatch(r"\d+\.\d+\.\d+", new):
        raise SystemExit(f"版本号得是 x.y.z，收到的是 {new}")
    old = read_versions()
    if len(set(old.values())) != 1:
        print("注意：三处版本号原本就不一致，本次一并对齐")
    for rel, pat in SITES:
        p = os.path.join(ROOT, rel)
        text = io.open(p, encoding="utf-8").read()
        # 只改**第一处**匹配：Cargo.toml 里 [dependencies] 底下也有 version=
        text = pat.sub(lambda m: m.group(1) + new + m.group(3), text, count=1)
        io.open(p, "w", encoding="utf-8", newline="\n").write(text)
        print(f"   {rel}  →  {new}")
    return sorted(set(old.values()))


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("version", nargs="?", help="新版本号，形如 0.1.1")
    ap.add_argument("--local", action="store_true", help="只改本地文件，不提交不打标签不推")
    ap.add_argument("--check", action="store_true", help="只检查三处版本号是否一致")
    args = ap.parse_args()

    if args.check or not args.version:
        check()
        return

    new = args.version
    tag = f"v{new}"

    if not args.local:
        dirty = git("status", "--porcelain")
        if dirty:
            print("工作区不干净，先把手上的改动提交或暂存：\n")
            print(dirty)
            raise SystemExit(1)
        if git("tag", "-l", tag):
            raise SystemExit(f"标签 {tag} 已经存在。发过的版本号不能重发——"
                             "改标签会让已经下载过的人对不上校验码。")

    print(f"改版本号 → {new}")
    old = bump(new)

    # 版本号一致性由测试钉着，这里立刻验一次：错了当场知道，而不是等 CI 跑二十分钟
    print("\n验一遍三处是否一致")
    check()

    if args.local:
        print("\n--local：文件已改，没有提交。")
        return

    print(f"\n即将执行：")
    print(f"   git commit -am '发布 {new}'")
    print(f"   git tag {tag}")
    print(f"   git push origin HEAD {tag}")
    print(f"\n推上去之后 GitHub Actions 会打包并建一个**草稿** Release（约 20 分钟）。")
    print(f"草稿不会自动公开——门控走完由你手动点发布。")
    if input(f"\n继续？(输 {tag} 确认) ").strip() != tag:
        print("已取消。文件已经改了，要还原就 git checkout -- .")
        return

    git("commit", "-am", f"发布 {new}", capture=False)
    git("tag", tag, capture=False)
    git("push", "origin", "HEAD", tag, capture=False)

    print(f"\n已推 {tag}（上一版是 {'/'.join(old)}）。接下来：")
    print("   1. 看 Actions 跑完 → https://github.com/Gitruck/steadcopy/actions")
    print("   2. 从跑完的那次里下载产物 zip")
    print(f"   3. python scripts/publish-mirror.py --zip <下载的zip>")
    print("   4. 按 docs/release-checklist.md 走门控，全过了再点发布 Release")
    # 发布手册是发布机上的本地文件（含内网路径与密钥位置，见 .gitignore），
    # 别人 clone 下来是没有的——所以只在它真的在的时候才提
    if os.path.exists(os.path.join(ROOT, "发布手册.md")):
        print("\n完整步骤见「发布手册.md」。")


if __name__ == "__main__":
    main()
