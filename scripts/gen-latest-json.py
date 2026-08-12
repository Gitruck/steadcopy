#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成更新器要读的 latest.json。**每个端点一份，签名相同。**

规范：能力 `build-release` 的 spec（openspec 私仓）
      → Requirement: 更新检查非强制 / Requirement: 更新来源可镜像

# 这份文件里有什么

版本号、发布说明、以及**安装包的下载地址与它的签名**。客户端读它，
拿编译进程序的公钥验签，签名对不上就拒绝安装。

# 为什么要生成两份

`tauri.conf.json` 的 endpoints 有两个：自有镜像在前、GitHub 在后。
两个地址上各挂一份清单，除了 `url` 字段指向自己那份拷贝之外**完全相同**——
尤其是 `signature`，因为两边挂的是**同一批字节**。

若两边字节不同（比如各自本地编译一次），签名就对不上，更新必然失败；
所以镜像上的包一定是从 CI 产物复制过去的，不是重新编的。

# 为什么签名比来源更重要

镜像是为了国内连得上，可它终究是「网上某台机器」。但**私钥只在发布机与
CI secret 里**，镜像没有它就伪造不出有效签名——所以「从哪儿下载」是可用性问题，
「装不装得成」是密码学问题，两者分开。

反过来，签名也管不了「镜像干脆不给你更新」：挂一份旧清单，客户端就以为
已经是最新。这不会把人骗去装坏东西（版本号只增不减，updater 只在
远端版本更高时才提示），但会让人停在旧版上——所以镜像归自己管，不用第三方代理。

# 指向哪个包

**精简版。** 离线版 206 MB，让所有人为了升级下 206 MB 不合理；
而能收到更新提示的机器，本机必然已经装着 WebView2 了。
"""

import argparse
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

# 自有镜像的公开地址。与 tauri.conf.json 的第一个 endpoint 同源——
# 改一处必须改另一处，由 update_origin.rs 的
# scenario_build_release_origin_allowlist_matches_configured_endpoints 钉住。
MIRROR_BASE = "https://api.ai-mcn.tv:9000/broadcast/steadcopy"


def version():
    for line in io.open(
        os.path.join(ROOT, "app", "src-tauri", "Cargo.toml"), encoding="utf-8"
    ):
        if line.startswith("version"):
            return line.split('"')[1]
    raise SystemExit("读不到版本号")


def slim_installer():
    """更新指向精简版：能收到更新提示的机器本来就已经有 WebView2 了。"""
    slim = [
        p
        for p in glob.glob(os.path.join(RELEASE, "*.exe"))
        if OFFLINE_MARK not in os.path.basename(p)
    ]
    if not slim:
        raise SystemExit("找不到精简版安装包——latest.json 不能指向一个不存在的文件")
    if len(slim) > 1:
        raise SystemExit(f"精简版安装包不止一个，说不清该指哪个：{slim}")
    return slim[0]


def signature(installer):
    sig_path = installer + ".sig"
    if not os.path.exists(sig_path):
        raise SystemExit(
            f"缺签名文件 {os.path.basename(sig_path)}。\n"
            "没签名的更新包客户端一律拒装——检查 CI 里的 TAURI_SIGNING_PRIVATE_KEY 是否配好。"
        )
    sig = io.open(sig_path, encoding="utf-8").read().strip()
    if not sig:
        raise SystemExit("签名文件是空的，拒绝生成 latest.json")
    return sig


def manifest(v, sig, url):
    data = {
        "version": v,
        "notes": f"稳拷 {v}。完整说明见发布页。",
        "platforms": {"windows-x86_64": {"signature": sig, "url": url}},
    }
    pub = os.environ.get("STEADCOPY_PUB_DATE", "")
    if pub:
        data["pub_date"] = pub
    return data


def write(path, data):
    io.open(path, "w", encoding="utf-8", newline="\n").write(
        json.dumps(data, ensure_ascii=False, indent=2) + "\n"
    )


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--mirror-base",
        default=MIRROR_BASE,
        help="自有镜像上这一版所在目录的公开地址（默认 %(default)s）",
    )
    args = ap.parse_args()

    v = version()
    tag = os.environ.get("GITHUB_REF_NAME") or f"v{v}"
    repo = os.environ.get("GITHUB_REPOSITORY", "Gitruck/steadcopy")

    installer = slim_installer()
    name = os.path.basename(installer)
    sig = signature(installer)

    # GitHub Releases 的资源地址
    gh_url = f"https://github.com/{repo}/releases/download/{tag}/{name}"
    # 自有镜像上的同一批字节
    mirror_url = f"{args.mirror_base.rstrip('/')}/{name}"

    write(os.path.join(RELEASE, "latest.json"), manifest(v, sig, gh_url))
    write(os.path.join(RELEASE, "latest.mirror.json"), manifest(v, sig, mirror_url))

    print(f"两份清单 → {name}（同一批字节，同一个签名）")
    print(f"   latest.json         {gh_url}")
    print(f"   latest.mirror.json  {mirror_url}")
    print(f"   签名: {sig[:32]}…")
    print(
        "\n注意：latest.mirror.json 上传到镜像时要**改名为 latest.json**——\n"
        "     端点地址是 .../steadcopy/latest.json。publish-mirror.py 会代劳。"
    )


if __name__ == "__main__":
    main()
