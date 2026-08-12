#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""一条命令出全部发布产物：安装包 + 便携版 + 命令行 + 许可清单 + 校验码。

规范：openspec/changes/add-steadcopy-release/specs/build-release/spec.md

    python scripts/build-release.py

顺序是刻意的：许可闸门排在编译之前——依赖里混进 GPL 系时，
应该在花二十分钟编译之前就被拦下。

产物落在 release/（文件名一律 ASCII，见 ascii_name 的说明）：
    steadcopy_<版本>_x64-setup.exe          精简版安装包（约 4 MB，装时按需拉 WebView2）
    steadcopy_<版本>_x64-setup-offline.exe  离线版安装包（约 206 MB，运行时打在包里）
    *.exe.sig                               更新器验签用的分离签名
    steadcopy-<版本>-portable.zip           便携版（解压即用，数据落在自己目录里）
    latest.json / latest.mirror.json        更新清单（GitHub 一份、自有镜像一份）
    SHA256SUMS.txt                          校验码，三处公示以它为准
    THIRD-PARTY-LICENSES.md                 第三方依赖许可清单

两个安装包**是同一个产品的两种装法**：productName 相同，差别只在 webviewInstallMode。
不能让它们的 productName 不同——那样 Windows 会当成两个程序，装出两份。
"""

import hashlib
import os
import shutil
import subprocess
import sys
import time
import zipfile

for stream in (sys.stdout, sys.stderr):
    try:
        stream.reconfigure(encoding="utf-8")
    except Exception:
        pass

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
APP = os.path.join(ROOT, "app")
RELEASE = os.path.join(ROOT, "release")
APP_MANIFEST = os.path.join(APP, "src-tauri", "Cargo.toml")
BUNDLE = os.path.join(APP, "src-tauri", "target", "release", "bundle", "nsis")
TARGET_RELEASE = os.path.join(ROOT, "target", "release")

# 便携版标记文件。文件名与 core 的 config::store::PORTABLE_MARKER 必须一致
PORTABLE_MARKER = "steadcopy.portable"

# 两个安装包的大小量级。用来兜住「两版搞混了」——这类错发出去才发现。
#
# 精简版与离线版**唯一的差别是 webviewInstallMode**：一个装的时候去拉运行时，
# 一个把 156 MB 的运行时打进包里。所以体积差是 50 倍，混不混得清一眼就知道。
# 判据放宽到量级而不是精确值，是因为版本迭代体积会变，但不会变一个数量级。
SLIM_MAX_MB = 40
OFFLINE_MIN_MB = 100


def step(n, total, title):
    print(f"\n[{n}/{total}] {title}", flush=True)


def run(cmd, cwd=ROOT, env=None):
    e = dict(os.environ)
    if env:
        e.update(env)
    r = subprocess.run(cmd, cwd=cwd, env=e, shell=isinstance(cmd, str))
    if r.returncode != 0:
        raise SystemExit(f"失败（退出码 {r.returncode}）：{cmd}")


def node_bin(name):
    """node_modules/.bin 下的可执行文件。Windows 上要用 .cmd 那个包装器，
    没后缀的是 sh 脚本，CreateProcess 起不来。"""
    base = os.path.join(APP, "node_modules", ".bin", name)
    for cand in (base + ".cmd", base + ".exe", base):
        if os.path.exists(cand):
            return cand
    raise SystemExit(f"找不到 {name}，先在 app/ 里跑一次 bun install")


def version():
    for line in open(os.path.join(APP, "src-tauri", "Cargo.toml"), encoding="utf-8"):
        if line.startswith("version"):
            return line.split('"')[1]
    raise SystemExit("读不到版本号")


def main():
    v = version()
    stamp = str(int(time.time()))
    # 签名密钥：本地开发时从 .updater-key 读进来（CI 里由 secret 直接给环境变量）。
    # 没有密钥就打不出 .sig，而没有 .sig 的包更新器一律拒装——所以这里明说
    key = os.path.join(ROOT, ".updater-key")
    if "TAURI_SIGNING_PRIVATE_KEY" not in os.environ and os.path.exists(key):
        os.environ["TAURI_SIGNING_PRIVATE_KEY"] = open(key, encoding="utf-8").read().strip()
        os.environ.setdefault("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "")
    if "TAURI_SIGNING_PRIVATE_KEY" not in os.environ:
        print("⚠ 没有签名密钥，本次产物不含 .sig —— 这样的包更新器会拒装")
    total = 7
    print(f"稳拷 steadcopy {v} 发布构建")

    # 1. 许可闸门排最前：GPL 系依赖不该等编译完才发现
    step(1, total, "生成许可清单并过 GPL 闸门")
    run([sys.executable, os.path.join("scripts", "gen-licenses.py")])

    step(2, total, "安全轨测试")
    run(["cargo", "test", "--workspace"])
    # app/src-tauri 是独立 workspace，上面那行扫不到它——不单列一行，
    # 它里头的测试（更新来源白名单等）就永远不会跑
    run(["cargo", "test", "--manifest-path", APP_MANIFEST])

    step(3, total, "静态检查")
    run(["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])
    run(["cargo", "clippy", "--manifest-path", APP_MANIFEST, "--all-targets",
         "--", "-D", "warnings"])
    run([node_bin("tsc"), "--noEmit"], cwd=APP)

    step(4, total, "编译命令行（release）")
    run(["cargo", "build", "--release", "-p", "steadcopy-cli"],
        env={"STEADCOPY_BUILD_TIME": stamp})

    step(5, total, "打包桌面应用与安装包（精简版 + 离线版）")
    os.makedirs(RELEASE, exist_ok=True)
    # 精简版：downloadBootstrapper。本机已有 WebView2 就直接装完，没有才去拉
    clear_bundle()
    run([node_bin("tauri"), "build"], cwd=APP,
        env={"STEADCOPY_BUILD_TIME": stamp})
    collect("slim", v)
    # 离线版：offlineInstaller，运行时整个打进去，断网也能装
    clear_bundle()
    run([node_bin("tauri"), "build", "--config", "src-tauri/tauri.offline.conf.json"],
        cwd=APP, env={"STEADCOPY_BUILD_TIME": stamp})
    collect("offline", v)

    step(6, total, "组装便携版")
    make_portable(v)

    step(7, total, "生成校验码")
    checksums()

    print(f"\n完成。产物在 {RELEASE}\\")
    print("下一步：按 docs/release-checklist.md 逐项走门控，任一项未过就不发版。")


def make_portable(v):
    """便携版：解压即用，数据落在自己目录，不写注册表。

    不含 WebView2 运行时——那是 150 MB 起步的固定版运行时，塞进「便携」不合理。
    Win11 与 Win10 22H2 自带；更旧的系统请用安装包。这条写进包内 README。
    """
    exe = os.path.join(APP, "src-tauri", "target", "release", "steadcopy-app.exe")
    if not os.path.exists(exe):
        # Tauri 产物名随 productName 走，兜底扫一遍
        d = os.path.join(APP, "src-tauri", "target", "release")
        cands = [f for f in os.listdir(d)
                 if f.endswith(".exe") and not f.endswith("-cli.exe")]
        if not cands:
            raise SystemExit(f"找不到应用可执行文件，看看 {d}")
        exe = os.path.join(d, cands[0])

    cli = os.path.join(TARGET_RELEASE, "steadcopy.exe")
    if not os.path.exists(cli):
        cli = os.path.join(TARGET_RELEASE, "steadcopy-cli.exe")

    stage = os.path.join(RELEASE, f"steadcopy-{v}-portable")
    shutil.rmtree(stage, ignore_errors=True)
    os.makedirs(stage, exist_ok=True)

    shutil.copy2(exe, os.path.join(stage, "稳拷 steadcopy.exe"))
    if os.path.exists(cli):
        shutil.copy2(cli, os.path.join(stage, "steadcopy.exe"))
    shutil.copy2(os.path.join(ROOT, "LICENSE"), os.path.join(stage, "LICENSE"))
    tpl = os.path.join(RELEASE, "THIRD-PARTY-LICENSES.md")
    if os.path.exists(tpl):
        shutil.copy2(tpl, os.path.join(stage, "THIRD-PARTY-LICENSES.md"))

    # 标记文件：有它才进便携模式，数据落在同目录的 data\
    with open(os.path.join(stage, PORTABLE_MARKER), "w", encoding="utf-8") as f:
        f.write("这个文件的存在让稳拷以便携模式运行：\n"
                "配置、设备记忆、任务台账全部落在同目录的 data\\ 里，\n"
                "不写 APPDATA，也不写注册表。删掉它就变回普通模式。\n")

    with open(os.path.join(stage, "使用说明.txt"), "w", encoding="utf-8") as f:
        f.write(
            f"稳拷 steadcopy {v} 便携版\n"
            "\n"
            "双击「稳拷 steadcopy.exe」即可运行。数据全部落在同目录的 data\\ 里，\n"
            "整个文件夹拷到别的机器上照样能用，卸载就是删文件夹。\n"
            "\n"
            "便携版与安装版的数据是分开的，同一台机器上并存也互不影响。\n"
            "\n"
            "注意：便携版不含 WebView2 运行时。Windows 11 与 Windows 10 22H2 自带；\n"
            "更旧的系统请改用安装包（安装包内置离线运行时，断网也能装）。\n"
            "\n"
            "本版本未购买代码签名证书，首次运行 Windows 会提示未知发布者，这是预期行为。\n"
            "请核对官网公示的 SHA-256 校验码确认来源。\n"
            "任何时候都不要为了运行本程序去关闭系统防护或安全软件。\n"
            "\n"
            "steadcopy.exe 是命令行版本，用法见 https://github.com/Gitruck/steadcopy\n"
        )

    zip_path = os.path.join(RELEASE, f"steadcopy-{v}-portable.zip")
    if os.path.exists(zip_path):
        os.remove(zip_path)
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as z:
        for dirpath, _dirs, files in os.walk(stage):
            for name in files:
                p = os.path.join(dirpath, name)
                z.write(p, os.path.relpath(p, RELEASE))
    shutil.rmtree(stage, ignore_errors=True)
    print(f"   便携版：{os.path.basename(zip_path)}"
          f"（{os.path.getsize(zip_path) / 1048576:.1f} MB）")


def ascii_name(f, v, flavor):
    """产物文件名一律 ASCII，并按**这一趟打的是哪一版**加后缀。

    程序**装好之后**叫「稳拷」（productName 决定的，那是对的）；但**下载文件名**
    必须是 ASCII——GitHub Releases 的资源 URL 会把非 ASCII 百分号编码，
    更新器按 latest.json 里的原样 url 去取就取不到，而且这种失败只在
    真的发布之后才暴露。

    为什么按打包顺序而不是按文件名判断是哪一版：**两版的 productName 必须相同**。
    曾经离线版叫「稳拷-离线版」，于是文件名里带「离线版」三个字，一眼可辨——
    但那样 Windows 把它当成另一个程序：装到 `Program Files\\稳拷-离线版\\`、
    在「添加删除程序」里单开一条。用离线版的人一点更新，下到的是精简版安装包，
    于是**装出第二份**，重启后跑的还是旧的那份。两版必须是同一个产品的两种装法，
    差别只在 webviewInstallMode，所以名字上区分不了，只能靠顺序——
    每次 build 前清空产物目录，收走的就一定是刚打出来的那个。
    """
    suffix = ".exe.sig" if f.endswith(".sig") else ".exe"
    return f"steadcopy_{v}_x64-setup{'-offline' if flavor == 'offline' else ''}{suffix}"


def clear_bundle():
    """打包前清空 NSIS 产物目录。

    留着上一趟的产物，`collect` 就分不清哪个是刚打出来的——
    而分错的后果是把 4 MB 的精简版当离线版发出去，片场断网的人装不上。
    """
    if os.path.isdir(BUNDLE):
        for f in os.listdir(BUNDLE):
            if f.endswith(".exe") or f.endswith(".sig"):
                os.remove(os.path.join(BUNDLE, f))


def collect(flavor, v):
    """把刚打出来的安装包收进 release/，顺便验体积量级。"""
    got = []
    for f in sorted(os.listdir(BUNDLE)) if os.path.isdir(BUNDLE) else []:
        if f.endswith(".exe") or f.endswith(".sig"):
            dst = os.path.join(RELEASE, ascii_name(f, v, flavor))
            shutil.copy2(os.path.join(BUNDLE, f), dst)
            got.append(dst)

    exes = [p for p in got if p.endswith(".exe")]
    if len(exes) != 1:
        raise SystemExit(
            f"{flavor} 这趟在 {BUNDLE} 里找到 {len(exes)} 个安装包，说不清该收哪个。"
            "打包前应当已经清空过该目录。"
        )
    mb = os.path.getsize(exes[0]) / 1048576
    if flavor == "slim" and mb > SLIM_MAX_MB:
        raise SystemExit(
            f"精简版 {mb:.1f} MB，超过 {SLIM_MAX_MB} MB —— 这个体积像是把 WebView2 "
            "运行时打进去了。多半是两版收反了，或者 webviewInstallMode 没生效。"
        )
    if flavor == "offline" and mb < OFFLINE_MIN_MB:
        raise SystemExit(
            f"离线版只有 {mb:.1f} MB，不足 {OFFLINE_MIN_MB} MB —— 运行时没打进去。"
            "这个包拿到断网的片场装不上，而那正是它存在的唯一理由。"
        )
    if not any(p.endswith(".sig") for p in got):
        print(f"⚠ {flavor} 没有 .sig —— 这样的包更新器会拒装")
    print(f"   {flavor}: {os.path.basename(exes[0])}（{mb:.1f} MB）")


def checksums():
    """把两个安装包与便携版一起算进校验清单。三处公示以这份为准。

    产物由 `collect` 在每趟 build 之后立刻收好并改成 ASCII 名，这里不再兜底改名——
    兜底会掩盖「某一趟没收到」，而那正是要炸出来的事。
    """
    v = version()
    # 校验清单只列用户会下载的东西。`.sig` 是给更新器验签用的，
    # 它自己就是凭证，不需要再为它出一份凭证
    targets = [
        (f, os.path.join(RELEASE, f))
        for f in sorted(os.listdir(RELEASE))
        if f.endswith(".exe") or f.endswith("-portable.zip")
    ]

    if not targets:
        raise SystemExit("没有找到任何发布产物，校验清单不该是空的")

    lines = []
    for name, path in targets:
        h = hashlib.sha256()
        with open(path, "rb") as fh:
            for chunk in iter(lambda: fh.read(1 << 20), b""):
                h.update(chunk)
        lines.append(f"{h.hexdigest()} *{name}")
        print(f"   {name}  {os.path.getsize(path) / 1048576:.1f} MB")

    out = os.path.join(RELEASE, "SHA256SUMS.txt")
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    print(f"   校验清单：{out}")


if __name__ == "__main__":
    main()
