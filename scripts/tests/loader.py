#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""按路径加载 scripts/ 下那些带连字符的脚本。

`build-release.py` 这类文件名不是合法的 Python 标识符，`import build_release`
拿不到它们。这里用 importlib 按路径加载——`package-extras.py` 与
`collect-bundle.py` 里已经各写过一份，这是第三处，收成一个。

顶层无副作用（各脚本的 `main()` 都在 `__main__` 守卫里），可以安全导入。
"""

import importlib.util
import os

SCRIPTS = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ROOT = os.path.dirname(SCRIPTS)


def load(name):
    """`load("build-release")` → 模块对象。"""
    path = os.path.join(SCRIPTS, f"{name}.py")
    spec = importlib.util.spec_from_file_location(f"_{name.replace('-', '_')}", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def sparse_file(path, size_bytes):
    """造一个「看起来有那么大」的文件。

    体积闸门的测试要 40 MB / 100 MB 量级的样本，真写那么多字节太慢。
    seek 到末尾写一个字节，NTFS 上是稀疏文件，`os.path.getsize` 报的是完整大小——
    而闸门看的正是 `getsize`。
    """
    with open(path, "wb") as f:
        if size_bytes > 0:
            f.seek(size_bytes - 1)
            f.write(b"\0")


class Quiet:
    """把被测脚本的 print 吞掉。

    这些脚本是给人看的命令行工具，print 很多；测试里全打出来会把
    「哪条红了」淹掉。失败时 unittest 自己会打 traceback 与断言消息，
    那才是要看的东西。
    """

    def setUp(self):
        import contextlib
        import io as _io

        self._quiet = contextlib.redirect_stdout(_io.StringIO())
        self._quiet.__enter__()
        super().setUp()

    def tearDown(self):
        super().tearDown()
        self._quiet.__exit__(None, None, None)
