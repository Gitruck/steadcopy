#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""收走刚打出来的安装包。用法：`collect-bundle.py slim|offline`

CI 与本地共用 `build-release.py` 里的同一段逻辑。CI 自己调 tauri build（要分两趟、
中间还要清目录），但**收哪个、叫什么名、体积对不对**这三件事两边必须一模一样——
各写一份就一定会漂，而漂出来的后果是把 4 MB 的精简版当离线版发出去。
"""

import importlib.util
import os
import sys

for stream in (sys.stdout, sys.stderr):
    try:
        stream.reconfigure(encoding="utf-8")
    except Exception:
        pass

_here = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location(
    "_build_release", os.path.join(_here, "build-release.py")
)
_mod = importlib.util.module_from_spec(_spec)
# build-release.py 顶层没有副作用（main 在 __main__ 守卫里），可以安全导入
_spec.loader.exec_module(_mod)


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in ("slim", "offline", "clear"):
        raise SystemExit("用法：collect-bundle.py slim|offline|clear")
    what = sys.argv[1]
    if what == "clear":
        _mod.clear_bundle()
        print("已清空 NSIS 产物目录")
        return
    os.makedirs(_mod.RELEASE, exist_ok=True)
    _mod.collect(what, _mod.version())


if __name__ == "__main__":
    main()
