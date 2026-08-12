#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成更新器要读的 latest.json。

规范：能力 `build-release` 的 spec（openspec 私仓）→ Requirement: 更新检查非强制

# 这份文件里有什么

版本号、发布说明、以及**每个平台的安装包地址与它的签名**。客户端读它，
拿编译进程序的公钥验签，签名对不上就拒绝安装。

# 为什么签名比来源更重要

更新包可能从镜像下载（国内直连 GitHub 常常不通）。镜像是第三方，不可信。
但**私钥只在发布机与 CI secret 里**，镜像没有它就伪造不出有效签名——
所以「从哪儿下载」是可用性问题，「装不装得成」是密码学问题，两者分开。

指向哪个包：**精简版**。离线版 205 MB，让所有人为了升级下 205 MB 不合理；
而能收到更新提示的机器，本机必然已经装着 WebView2 了。
"""

import glob
import io
import json
import os
import sys

for stream in (sys.stdout, sys.stderr):
    try:
        stream.reconfigure(encoding="utf-8")
    except Exception:
        pass

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RELEASE = os.path.join(ROOT, "release")

# 离线版的产物名里有这个标记，用来把两个包分开
OFFLINE_MARK = "offline"


def version():
    for line in io.open(
        os.path.join(ROOT, "app", "src-tauri", "Cargo.toml"), encoding="utf-8"
    ):
        if line.startswith("version"):
            return line.split('"')[1]
    raise SystemExit("读不到版本号")


def main():
    v = version()
    tag = os.environ.get("GITHUB_REF_NAME") or f"v{v}"
    repo = os.environ.get("GITHUB_REPOSITORY", "Gitruck/steadcopy")

    # 更新指向精简版：能收到更新提示的机器本来就已经有 WebView2 了
    slim = [
        p
        for p in glob.glob(os.path.join(RELEASE, "*.exe"))
        if OFFLINE_MARK not in os.path.basename(p)
    ]
    if not slim:
        raise SystemExit("找不到精简版安装包——latest.json 不能指向一个不存在的文件")
    if len(slim) > 1:
        raise SystemExit(f"精简版安装包不止一个，说不清该指哪个：{slim}")
    installer = slim[0]
    name = os.path.basename(installer)

    sig_path = installer + ".sig"
    if not os.path.exists(sig_path):
        raise SystemExit(
            f"缺签名文件 {os.path.basename(sig_path)}。\n"
            "没签名的更新包客户端一律拒装——检查 CI 里的 TAURI_SIGNING_PRIVATE_KEY 是否配好。"
        )
    signature = io.open(sig_path, encoding="utf-8").read().strip()
    if not signature:
        raise SystemExit("签名文件是空的，拒绝生成 latest.json")

    # 下载地址走 GitHub Releases 的 assets。客户端那边的 endpoints 可以配镜像，
    # 但**这份 json 里的 url 是权威地址**——镜像若要代理，代理的是这条 url
    url = f"https://github.com/{repo}/releases/download/{tag}/{name}"

    data = {
        "version": v,
        "notes": f"稳拷 {v}。完整说明见 Releases 页。",
        "pub_date": os.environ.get("STEADCOPY_PUB_DATE", ""),
        "platforms": {"windows-x86_64": {"signature": signature, "url": url}},
    }
    if not data["pub_date"]:
        del data["pub_date"]

    out = os.path.join(RELEASE, "latest.json")
    io.open(out, "w", encoding="utf-8", newline="\n").write(
        json.dumps(data, ensure_ascii=False, indent=2) + "\n"
    )
    print(f"latest.json → {name}")
    print(f"   url: {url}")
    print(f"   签名: {signature[:32]}…")


if __name__ == "__main__":
    main()
