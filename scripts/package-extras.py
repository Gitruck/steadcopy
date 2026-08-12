#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""安装包之外的产物：便携版 + 校验码。

从 build-release.py 里拆出来的，好让 CI 与本地共用同一段逻辑——
CI 自己调 tauri build（要打两个包），但便携版怎么组装、校验码怎么算，
两边必须一模一样，不然本地算出来的校验码跟发布页上的对不上。
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

# 复用 build-release 里已经写好的两段，不重写一遍
import importlib.util

_spec = importlib.util.spec_from_file_location(
    "_build_release", os.path.join(os.path.dirname(os.path.abspath(__file__)), "build-release.py")
)
_mod = importlib.util.module_from_spec(_spec)
# build-release.py 顶层没有副作用（main 在 __main__ 守卫里），可以安全导入
_spec.loader.exec_module(_mod)

for stream in (sys.stdout, sys.stderr):
    try:
        stream.reconfigure(encoding="utf-8")
    except Exception:
        pass


def main():
    v = _mod.version()
    print(f"组装便携版与校验码（{v}）")
    _mod.make_portable(v)
    _mod.checksums()


if __name__ == "__main__":
    main()
