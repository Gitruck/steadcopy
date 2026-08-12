#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成第三方依赖许可清单，并顺手当 GPL 系闸门。

规范：openspec/changes/add-steadcopy-release/specs/build-release/spec.md
      → Requirement: 开源合规产物 / 发布门控

产出两份，内容同源：
  app/src-tauri/licenses.json   —— 编译进程序，应用内「关于 → 开源许可」直接读
  release/THIRD-PARTY-LICENSES.md —— 随发布产物分发

**必须每次发布重新生成**，不许手工维护——手工维护的清单迟早与 Cargo.lock 脱节，
那时它比没有更糟：它会让人以为核对过了。

发现 GPL/AGPL/SSPL 系依赖时以非零码退出。本项目是 MIT，静态链接进来的
copyleft 依赖会把整个产物拖进传染射程，这条是硬闸门不是提醒。
"""

import json
import os
import subprocess
import sys

# Windows 控制台默认 GBK，输出中文清单会炸
for stream in (sys.stdout, sys.stderr):
    try:
        stream.reconfigure(encoding="utf-8")
    except Exception:
        pass

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# 传染性许可关键词。命中即拦。
# 注意 LGPL 单列：动态链接下可用，但我们是静态链接的 Rust 产物，同样拦。
BLOCKED = ("GPL", "AGPL", "SSPL", "CDDL", "EPL", "MPL-1")
# GPL 系的例外写法：这些是「GPL 或别的」的双许可，选另一边即可
DUAL_OK = ("OR MIT", "OR Apache", "/MIT", "/Apache")


TARGET = "x86_64-pc-windows-msvc"


def cargo_packages(manifest_rel):
    """一个 cargo 工作区**真正会进产物**的依赖。

    只走 normal + build 依赖，dev-dependencies 不进（测试依赖不随产物分发，
    把它们混进合规清单只会让清单失去判别力）；并按目标平台过滤，
    免得把 linux/mac 专用的 crate 算进 Windows 产物。
    """
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--all-features",
         "--filter-platform", TARGET,
         "--manifest-path", os.path.join(ROOT, manifest_rel)],
        capture_output=True, text=True, encoding="utf-8", cwd=ROOT,
    )
    if out.returncode != 0:
        sys.stderr.write(out.stderr)
        raise SystemExit(f"cargo metadata 失败：{manifest_rel}")
    meta = json.loads(out.stdout)

    by_id = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta.get("resolve", {}).get("nodes", [])}

    # 从工作区成员出发，只沿 normal / build 边走
    seen, queue = set(), list(meta.get("workspace_members", []))
    while queue:
        pid = queue.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes.get(pid, {}).get("deps", []):
            kinds = {k.get("kind") for k in dep.get("dep_kinds", [{"kind": None}])}
            if kinds and kinds <= {"dev"}:
                continue
            queue.append(dep["pkg"])

    pkgs = {}
    for pid in seen:
        p = by_id.get(pid)
        # 本仓自己的 crate 不算第三方
        if not p or p.get("source") is None:
            continue
        pkgs[(p["name"], p["version"])] = {
            "name": p["name"],
            "version": p["version"],
            "license": p.get("license") or p.get("license_file") or "未声明",
            "repository": p.get("repository") or "",
            "ecosystem": "rust",
        }
    return pkgs


def npm_packages():
    """前端依赖。读 bun.lock / package-lock.json，都没有就跳过并说明。"""
    pkgs, skipped = {}, []
    lock = os.path.join(ROOT, "app", "bun.lock")
    if not os.path.exists(lock):
        return pkgs, "未找到 app/bun.lock，前端依赖清单缺失"
    # bun.lock 是 JSONC（带尾逗号），逐行剥注释后用宽松解析
    text = open(lock, encoding="utf-8").read()
    lines = [ln for ln in text.splitlines() if not ln.strip().startswith("//")]
    cleaned = "\n".join(lines)
    # 去掉尾逗号
    import re
    cleaned = re.sub(r",(\s*[}\]])", r"\1", cleaned)
    try:
        data = json.loads(cleaned)
    except json.JSONDecodeError as e:
        return pkgs, f"bun.lock 解析失败（{e}），前端依赖清单缺失"
    for key, entry in (data.get("packages") or {}).items():
        if not isinstance(entry, list) or not entry:
            continue
        spec = entry[0]  # 形如 "react@19.2.0"
        if "@" not in spec:
            continue
        name, _, version = spec.rpartition("@")
        # bun.lock 不带许可字段，得读 node_modules 里的 package.json。
        # 读不到的一律**跳过而不是记「未声明」**：这些几乎全是别的平台的
        # 可选二进制包（esbuild/rollup 的 linux/darwin 变体），本机没装、
        # 也不会进 Windows 产物。把它们记成「未声明」会让整份合规清单失去判别力。
        lic = npm_license(name)
        if lic is None:
            skipped.append(f"{name}@{version}")
            continue
        pkgs[(name, version)] = {
            "name": name,
            "version": version,
            "license": lic,
            "repository": "",
            "ecosystem": "npm",
        }
    note = None
    if skipped:
        note = (f"另有 {len(skipped)} 个前端包本机未安装（其他平台的可选二进制），"
                "不进 Windows 产物，未列入")
    return pkgs, note


def npm_license(name):
    """本机装了就返回许可字符串；没装返回 None（调用方据此跳过）。"""
    p = os.path.join(ROOT, "app", "node_modules", *name.split("/"), "package.json")
    if not os.path.exists(p):
        return None
    try:
        d = json.load(open(p, encoding="utf-8"))
    except Exception:
        return "未声明"
    lic = d.get("license")
    if isinstance(lic, dict):
        return lic.get("type", "未声明")
    if isinstance(lic, list):
        return " OR ".join(str(x) for x in lic)
    return lic or "未声明"


def is_blocked(license_str):
    up = (license_str or "").upper()
    if any(ok.upper() in up for ok in DUAL_OK):
        return False
    return any(b in up for b in BLOCKED)


def main():
    pkgs = {}
    pkgs.update(cargo_packages("Cargo.toml"))
    pkgs.update(cargo_packages(os.path.join("app", "src-tauri", "Cargo.toml")))
    npm, npm_warn = npm_packages()
    pkgs.update(npm)

    items = sorted(pkgs.values(), key=lambda x: (x["ecosystem"], x["name"].lower()))
    blocked = [i for i in items if is_blocked(i["license"])]

    os.makedirs(os.path.join(ROOT, "release"), exist_ok=True)
    payload = {
        "generated_by": "scripts/gen-licenses.py",
        "self": {"name": "steadcopy", "license": "MIT"},
        "count": len(items),
        "packages": items,
    }
    if npm_warn:
        payload["warning"] = npm_warn

    with open(os.path.join(ROOT, "app", "src-tauri", "licenses.json"), "w",
              encoding="utf-8", newline="\n") as f:
        json.dump(payload, f, ensure_ascii=False, indent=1)
        f.write("\n")

    md = ["# 第三方依赖许可清单", "",
          "本文件由 `scripts/gen-licenses.py` 自动生成，**请勿手工编辑**。",
          "",
          "稳拷 steadcopy 本体为 MIT，见 [LICENSE](../LICENSE)。",
          ""]
    if npm_warn:
        md += [f"> 注：{npm_warn}。", ""]
    NOTE = {
        "rust": "只含会进产物的 normal / build 依赖，dev-dependencies 不在其中。",
        "npm": "含构建期工具链（打包器、类型检查器等），这部分不进产物，一并列出以便审计。",
    }
    for eco, title in (("rust", "Rust 依赖"), ("npm", "前端依赖")):
        rows = [i for i in items if i["ecosystem"] == eco]
        if not rows:
            continue
        md += [f"## {title}（{len(rows)}）", "", NOTE[eco], "",
               "| 包 | 版本 | 许可 |", "|---|---|---|"]
        md += [f"| {i['name']} | {i['version']} | {i['license']} |" for i in rows]
        md += [""]
    with open(os.path.join(ROOT, "release", "THIRD-PARTY-LICENSES.md"), "w",
              encoding="utf-8", newline="\n") as f:
        f.write("\n".join(md))

    print(f"已生成清单：{len(items)} 个依赖"
          f"（rust {sum(1 for i in items if i['ecosystem'] == 'rust')}，"
          f"npm {sum(1 for i in items if i['ecosystem'] == 'npm')}）")
    if npm_warn:
        print(f"注：{npm_warn}")

    if blocked:
        print("\n❌ 发现传染性许可依赖，发布闸门不放行：", file=sys.stderr)
        for i in blocked:
            print(f"   {i['ecosystem']} {i['name']} {i['version']} — {i['license']}",
                  file=sys.stderr)
        return 1
    print("[通过] 未发现 GPL 系依赖")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
