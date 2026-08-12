#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""把发布产物推到自有镜像，然后**回读一遍确认真的能下**。

规范：能力 `build-release` 的 spec（openspec 私仓）→ Requirement: 更新来源可镜像

    # 门控之前：把包传上去（传了也没人找得到，因为清单还没换）
    python scripts/publish-mirror.py --zip ~/Downloads/steadcopy-v0.1.1.zip

    # 门控全过之后：放出去
    python scripts/publish-mirror.py --zip ~/Downloads/steadcopy-v0.1.1.zip --publish-manifest

    # 放出去之后发现问题
    python scripts/publish-mirror.py --rollback

**为什么分两步。** 写下 `latest.json` 的那一瞬间，所有开着「检查更新」的客户端
就能装到这一版了——镜像是 `endpoints[0]`，它说了算，GitHub 那边的草稿 Release
还没公开**不作数**。所以清单必须等门控 R1–R16 全过之后再写，否则「草稿」这道闸
形同虚设：R10 断网安装没过、R14 杀软误报还没申报，版本却已经发给所有人了。

先传包后放清单还有个好处：206 MB 的上传在门控之前就完成了，最后那一步只写几百字节。

`--zip` 收的是 GitHub Actions 页面上下载下来的产物包，直接指过去就行，
不用自己解压——少一步就少一次「解错目录」。

# 为什么需要这一步

`tauri.conf.json` 的第一个更新端点是 `https://api.ai-mcn.tv:9000/broadcast/steadcopy/latest.json`。
GitHub 在国内常常连不上，而**仓库私有时 GitHub 那个端点对匿名客户端直接 404**——
也就是说，在决定开源之前，自有镜像是唯一真正能用的更新源。

# 为什么必须是 CI 产物、不能本地重编

清单里的 `signature` 是**对具体那批字节**签的。Rust 编译不是逐字节可复现的，
本地重编一次，字节就变了，签名对不上，客户端下完拒装——而这只有在真的
点了「安装更新」之后才会暴露。所以：CI 产什么，镜像挂什么。

# 上传方式

镜像是挂在 NAS 上的静态目录（`T:\\web\\` ↔ `https://api.ai-mcn.tv:9000/`），
发布机上映射成 T 盘，所以「上传」就是复制文件。这台机器在内网/Tailscale 里才能写，
GitHub Actions 的托管跑器够不着——因此**镜像发布是本地的一步**，
在 CI 把产物打好之后手动跑。
"""

import argparse
import hashlib
import io
import json
import os
import shutil
import ssl
import sys
import tempfile
import urllib.request
import zipfile

for stream in (sys.stdout, sys.stderr):
    try:
        stream.reconfigure(encoding="utf-8")
    except Exception:
        pass

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# NAS 上的目标目录与它对应的公开地址。两者必须是同一处的两种叫法——
# 写进 A、从 B 读出来对不上，说明映射关系变了，那时候宁可炸也别默默发布。
NAS_DIR = r"T:\web\broadcast\steadcopy"
PUBLIC_BASE = "https://api.ai-mcn.tv:9000/broadcast/steadcopy"

# 要挂上镜像的东西。两个安装包都挂：镜像存在的理由之一就是国内下不动 GitHub，
# 而离线版 206 MB 恰恰是最下不动的那个。
WANTED_SUFFIXES = (".exe", ".exe.sig")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def check_checksums(src):
    """产物必须与 SHA256SUMS.txt 对得上。

    这一步防的是「拷贝过程中坏了」和「拿错了一批产物」。校验清单是发布页上
    公示的那份，以它为准。
    """
    sums = os.path.join(src, "SHA256SUMS.txt")
    if not os.path.exists(sums):
        raise SystemExit(
            f"{src} 里没有 SHA256SUMS.txt。镜像挂的必须是**发布出去的那批字节**，"
            "没有校验清单就无从确认这一点。"
        )
    expected = {}
    for line in io.open(sums, encoding="utf-8"):
        line = line.strip()
        if not line:
            continue
        digest, name = line.split(" *", 1)
        expected[name] = digest
    bad = []
    for name, digest in expected.items():
        p = os.path.join(src, name)
        if not os.path.exists(p):
            bad.append(f"{name}：清单里有，目录里没有")
        elif sha256(p) != digest:
            bad.append(f"{name}：校验码对不上")
    if bad:
        raise SystemExit("产物与校验清单不符，拒绝发布：\n  " + "\n  ".join(bad))
    print(f"✓ {len(expected)} 个产物与 SHA256SUMS.txt 一致")
    return expected


def mirror_manifest(src):
    """镜像要挂的清单：`latest.mirror.json` 改名成 `latest.json`。

    端点地址是 `.../steadcopy/latest.json`，所以挂上去必须叫这个名字；
    而它的内容与 GitHub 那份的区别只有 `url` 一个字段。
    """
    p = os.path.join(src, "latest.mirror.json")
    if not os.path.exists(p):
        raise SystemExit(
            "缺 latest.mirror.json。先跑 scripts/gen-latest-json.py——"
            "它会同时产 GitHub 与镜像两份清单，签名相同、只有 url 不同。"
        )
    data = json.load(io.open(p, encoding="utf-8"))
    url = data["platforms"]["windows-x86_64"]["url"]
    if not url.startswith(PUBLIC_BASE + "/"):
        raise SystemExit(
            f"清单里的 url 指向 {url}，不在本镜像下（{PUBLIC_BASE}）。\n"
            "挂上去客户端会去别处下载——若那个别处不在编译期白名单里，直接被拒。"
        )
    name = url.rsplit("/", 1)[-1]
    if not os.path.exists(os.path.join(src, name)):
        raise SystemExit(f"清单指向 {name}，但 {src} 里没有这个文件")
    return data, name


def fetch(url, timeout=30):
    ctx = ssl.create_default_context()
    req = urllib.request.Request(url, headers={"User-Agent": "steadcopy-publish-mirror"})
    with urllib.request.urlopen(req, timeout=timeout, context=ctx) as r:
        return r.status, r.read()


def verify_live(src, name, expected_digest, manifest):
    """回读：镜像上现在真的挂着这批字节吗？

    复制成功 ≠ 发布成功。目录可能没被 web 服务收录、可能有缓存、可能路径映射变了。
    只有**从公网地址取回来、逐字节对上**才算发布成功——否则更新链在客户端那头断，
    而客户端那头没人会来告诉你。
    """
    ok = True

    url = f"{PUBLIC_BASE}/latest.json"
    try:
        status, body = fetch(url)
        # 状态先查再解析：非 200 时正文多半不是 JSON，先 loads 会抛进 except，
        # 把「返回了 403」笼统报成「取不到」
        if status != 200:
            raise RuntimeError(f"返回 {status}")
        live = json.loads(body.decode("utf-8"))
        if live != manifest:
            print(f"✗ {url} 取回来的清单与刚发布的不一致（缓存？）")
            ok = False
        else:
            print(f"✓ {url} 内容一致")
    except Exception as e:
        print(f"✗ {url} 取不到：{e}")
        ok = False

    # 安装包只取头几个字节确认可下载——206 MB 全量回读没必要，
    # 但「地址通不通」必须验，因为那正是最容易断的一环
    url = f"{PUBLIC_BASE}/{name}"
    try:
        req = urllib.request.Request(
            url,
            headers={"User-Agent": "steadcopy-publish-mirror", "Range": "bytes=0-1023"},
        )
        with urllib.request.urlopen(req, timeout=30) as r:
            head = r.read(1024)
        local = open(os.path.join(src, name), "rb").read(1024)
        # 长度必须先查。`head[:len(local)] != local[:len(head)]` 在 head 为空时
        # 两边都切成 b"" 判等通过——「地址通了但一个字节都取不到」这个最典型的
        # 镜像故障，恰好是唯一能骗过回读的情形。截断（只回了几百字节）同理。
        want = min(len(local), 1024)
        if len(head) < want:
            print(f"✗ {url} 只回了 {len(head)} 字节（期望至少 {want}），像是空响应或被截断")
            ok = False
        elif head[:want] != local[:want]:
            print(f"✗ {url} 开头字节与本地不同")
            ok = False
        else:
            print(f"✓ {url} 可下载，开头 {want} 字节一致")
    except Exception as e:
        print(f"✗ {url} 取不到：{e}")
        ok = False

    print(f"   （本地 sha256 {expected_digest[:16]}…）")
    return ok


def unpack(zip_path, into):
    """解开 CI 产物包。

    upload-artifact 打的包里，文件可能在根，也可能在一层子目录里——
    所以认「哪一层有 latest.mirror.json」，而不是假设结构。
    """
    with zipfile.ZipFile(zip_path) as z:
        z.extractall(into)
    for dirpath, _dirs, files in os.walk(into):
        if "latest.mirror.json" in files:
            return dirpath
    raise SystemExit(
        f"{os.path.basename(zip_path)} 里找不到 latest.mirror.json。\n"
        "这个包像是不完整，或者不是发布流水线产的那个。"
    )


def rollback(nas):
    """把 latest.json 还原成上一版。

    用在「已经放出去了才发现问题」。安装包本身不删——旧客户端可能正下到一半，
    抽掉文件只会让它们拿到半个包；把清单换回去就够了：新的检查更新不会再看到这一版。
    """
    live = os.path.join(nas, "latest.json")
    prev = os.path.join(nas, "latest.prev.json")
    if not os.path.exists(prev):
        raise SystemExit(
            f"{prev} 不在——没有上一版清单可还原。\n"
            "（第一次发布就没有「上一版」，那种情况下直接删掉 latest.json 即可，"
            "客户端会回落到 GitHub 端点。）"
        )
    shutil.copy2(prev, live)
    data = json.load(io.open(live, encoding="utf-8"))
    print(f"已把 latest.json 还原到 v{data.get('version', '?')}")
    print("安装包没删——正下到一半的客户端不该被抽掉文件。新的检查更新不会再看到撤回的那版。")
    print(f"确认一下：{PUBLIC_BASE}/latest.json")


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    src = ap.add_mutually_exclusive_group()
    src.add_argument("--zip", help="从 Actions 下载下来的产物包，自动解压")
    src.add_argument("--dir", help="产物目录（默认 release/）")
    ap.add_argument("--nas", default=NAS_DIR, help="NAS 上的目标目录")
    ap.add_argument("--publish-manifest", action="store_true",
                    help="写 latest.json —— **这一步一放，更新就对所有客户端上线了**。门控走完再加")
    ap.add_argument("--rollback", action="store_true",
                    help="把 latest.json 还原成上一版（发出去之后发现问题时用）")
    ap.add_argument("--dry-run", action="store_true", help="只说要干什么，不动文件")
    args = ap.parse_args()

    if args.rollback:
        return rollback(args.nas)

    tmp = None
    if args.zip:
        if not os.path.exists(args.zip):
            raise SystemExit(f"找不到 {args.zip}")
        tmp = tempfile.mkdtemp(prefix="steadcopy-mirror-")
        SRC = unpack(os.path.abspath(args.zip), tmp)
        print(f"已解开 {os.path.basename(args.zip)}")
    else:
        SRC = os.path.abspath(args.dir or os.path.join(ROOT, "release"))
    if not os.path.isdir(SRC):
        raise SystemExit(f"产物目录不存在：{SRC}")

    print(f"源：{SRC}")
    print(f"目标：{args.nas}  →  {PUBLIC_BASE}\n")

    sums = check_checksums(SRC)
    manifest, slim_name = mirror_manifest(SRC)
    print(f"✓ 清单指向 {slim_name}（v{manifest['version']}）")

    files = [f for f in sorted(os.listdir(SRC)) if f.endswith(WANTED_SUFFIXES)]
    if not files:
        raise SystemExit("没有可发布的安装包")

    if args.dry_run:
        print("\n[dry-run] 将要复制：")
        for f in files:
            print(f"   {f}  →  {args.nas}\\{f}")
        if args.publish_manifest:
            print(f"   latest.mirror.json  →  {args.nas}\\latest.json  ← **这一步一放，更新就上线了**")
        else:
            print("   （不写 latest.json——加 --publish-manifest 才写）")
        return

    nas_parent = os.path.dirname(args.nas)
    if not os.path.isdir(nas_parent):
        raise SystemExit(
            f"够不着 {nas_parent}。镜像目录挂在 NAS 上，需要在内网 / Tailscale 里"
            "并且盘符已映射——GitHub Actions 的托管跑器到不了这里，这一步只能本地跑。"
        )
    os.makedirs(args.nas, exist_ok=True)

    print("\n复制安装包：")
    for f in files:
        shutil.copy2(os.path.join(SRC, f), os.path.join(args.nas, f))
        print(f"   {f}")

    if not args.publish_manifest:
        print(
            "\n包已经在镜像上了，但**更新还没放出去**——latest.json 没动，"
            "客户端查更新拿到的仍是上一版。\n"
            "按 docs/release-checklist.md 走完门控 R1–R16，再跑一次带 --publish-manifest 的。"
        )
        if tmp:
            shutil.rmtree(tmp, ignore_errors=True)
        return

    # **清单最后写，而且要在门控之后写。**
    #
    # 两层理由，都不是形式主义：
    # 1. 先有包再有清单——反过来的话，两次复制之间检查更新的客户端会拿到一份
    #    指向还不存在的文件的清单，下载 404。
    # 2. 写下这一行的瞬间，**所有开着「检查更新」的客户端就能装到这一版了**。
    #    GitHub 那边的草稿 Release 还没公开不作数——镜像是 endpoints[0]，
    #    它说了算。所以这一步必须排在门控全过之后，否则「草稿」这道闸形同虚设：
    #    R10 断网安装没过、R14 杀软误报没申报，版本却已经发给所有人了。
    live = os.path.join(args.nas, "latest.json")
    if os.path.exists(live):
        # 留一份上一版，出事能放回去（--rollback）
        shutil.copy2(live, os.path.join(args.nas, "latest.prev.json"))
    io.open(live, "w", encoding="utf-8", newline="\n").write(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n"
    )
    print("   latest.json —— 更新已上线")

    print("\n回读确认：")
    if not verify_live(SRC, slim_name, sums.get(slim_name, ""), manifest):
        raise SystemExit(
            "\n镜像回读没过。文件也许复制上去了，但从公网地址取不到正确内容——"
            "在查清楚之前不要对外说更新可用。"
            f"\n想撤回：python scripts/publish-mirror.py --rollback"
        )
    print(f"\n完成。更新端点：{PUBLIC_BASE}/latest.json")


if __name__ == "__main__":
    main()
