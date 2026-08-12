#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""打包脚本的安全轨。

规范：能力 `build-release` 的 spec（openspec 私仓）
      → Requirement: 两版安装包是同一个产品

这两条场景以前挂在「已知缺口」里，理由是「本仓没有 Python 测试基座」。
现在有了——用标准库 `unittest`，不引 pytest：发布流水线上多一个 pip 依赖，
就多一个「装不上就发不了版」的单点。

跑：`python -m unittest discover -s scripts/tests`
"""

import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from loader import Quiet, load, sparse_file  # noqa: E402

br = load("build-release")


class BuildRelease(Quiet, unittest.TestCase):
    def setUp(self):
        super().setUp()
        self.tmp = tempfile.mkdtemp(prefix="steadcopy-br-test-")
        self.bundle = os.path.join(self.tmp, "bundle")
        self.release = os.path.join(self.tmp, "release")
        os.makedirs(self.bundle)
        os.makedirs(self.release)
        # 模块级常量指向真目录，测试里换成临时目录再还原——
        # 不换的话这些测试会去动开发机上真的 release/
        self._saved = (br.BUNDLE, br.RELEASE)
        br.BUNDLE, br.RELEASE = self.bundle, self.release

    def tearDown(self):
        super().tearDown()
        br.BUNDLE, br.RELEASE = self._saved
        shutil.rmtree(self.tmp, ignore_errors=True)

    def put(self, name, mb=1):
        sparse_file(os.path.join(self.bundle, name), int(mb * 1048576))

    # ---------------------------------------------------------------- 场景

    def test_scenario_build_release_flavors_told_apart_by_build_order(self):
        """产物按打包顺序区分而非按文件名。

        两版的 productName 相同（必须相同——不同会让 Windows 当成两个程序，
        离线版用户一点更新就装出第二份），所以 NSIS 产物名一模一样，
        名字上分不出谁是谁。分辨全靠「每趟 build 前清空、打完立刻收走」。

        这里钉的是：**收出来的名字由 flavor 参数决定，与源文件名无关。**
        """
        # 故意用一个「看起来像精简版」的源文件名去收离线版
        self.put("稳拷_0.1.0_x64-setup.exe", mb=150)
        sparse_file(os.path.join(self.bundle, "稳拷_0.1.0_x64-setup.exe.sig"), 400)
        br.collect("offline", "0.1.0")
        self.assertTrue(
            os.path.exists(os.path.join(self.release, "steadcopy_0.1.0_x64-setup-offline.exe")),
            "收出来的名字应当由 flavor 决定，不该跟着源文件名走",
        )
        self.assertTrue(
            os.path.exists(os.path.join(self.release, "steadcopy_0.1.0_x64-setup-offline.exe.sig")),
            ".sig 也要一起收——没有它更新器一律拒装",
        )

        # 反过来：同一个源文件收成精简版，名字里就不该有 offline
        shutil.rmtree(self.release)
        os.makedirs(self.release)
        os.remove(os.path.join(self.bundle, "稳拷_0.1.0_x64-setup.exe"))
        self.put("稳拷_0.1.0_x64-setup.exe", mb=4)
        br.collect("slim", "0.1.0")
        self.assertTrue(
            os.path.exists(os.path.join(self.release, "steadcopy_0.1.0_x64-setup.exe"))
        )

    def test_scenario_build_release_stale_bundle_is_cleared_before_each_build(self):
        """留着上一趟的产物，收的时候就分不清哪个是刚打出来的。

        `clear_bundle()` 必须把 .exe / .sig 清干净；清不干净时 `collect()`
        必须**当场炸**而不是挑一个——挑错的后果是把 4 MB 的精简版当离线版发出去，
        片场断网的人装不上，而这只有他们会遇到。
        """
        self.put("稳拷_0.1.0_x64-setup.exe", mb=4)
        self.put("稳拷-离线版_0.1.0_x64-setup.exe", mb=150)
        with self.assertRaises(SystemExit) as cm:
            br.collect("offline", "0.1.0")
        self.assertIn("说不清该收哪个", str(cm.exception))

        br.clear_bundle()
        self.assertEqual([], os.listdir(self.bundle), "清完应当一个安装包都不剩")

    def test_scenario_build_release_size_mismatch_refuses_to_ship(self):
        """体积量级不符则拒绝出包。

        两版唯一的差别是 webviewInstallMode，体积差 50 倍。所以体积是「两版收反了」
        或「webviewInstallMode 没生效」最便宜的判据——比读配置、比看文件名都硬。
        """
        # 精简版超标：像是把运行时打进去了
        self.put("x_0.1.0_x64-setup.exe", mb=br.SLIM_MAX_MB + 5)
        with self.assertRaises(SystemExit) as cm:
            br.collect("slim", "0.1.0")
        self.assertIn("像是把 WebView2", str(cm.exception))

        # 离线版不足：运行时没打进去，拿到断网片场装不上——而那正是它存在的理由
        br.clear_bundle()
        shutil.rmtree(self.release)
        os.makedirs(self.release)
        self.put("x_0.1.0_x64-setup.exe", mb=br.OFFLINE_MIN_MB - 10)
        with self.assertRaises(SystemExit) as cm:
            br.collect("offline", "0.1.0")
        self.assertIn("运行时没打进去", str(cm.exception))

        # 正常量级要放行，别把闸门修成「什么都拦」
        br.clear_bundle()
        shutil.rmtree(self.release)
        os.makedirs(self.release)
        self.put("x_0.1.0_x64-setup.exe", mb=4)
        br.collect("slim", "0.1.0")
        self.put("y_0.1.0_x64-setup.exe", mb=br.OFFLINE_MIN_MB + 50)
        # 上一趟的还在，先清掉再收
        os.remove(os.path.join(self.bundle, "x_0.1.0_x64-setup.exe"))
        br.collect("offline", "0.1.0")

    def test_scenario_build_release_previous_run_artifacts_are_moved_aside(self):
        """`release/` 开工前必须清空。

        不清的后果是真的：上一趟的产物留着，`checksums()` 按目录枚举会把它一并
        算进 SHA256SUMS.txt，而 `publish-mirror.py` 不带参数时默认就发这个目录——
        于是**改配置之前打的包**被原样推上镜像，校验码还都对得上，
        只是对在错的那批字节上。

        挪走而不是删掉：万一里头有还没归档的东西，人还能捞回来。
        """
        old = os.path.join(self.release, "steadcopy_0.0.9_x64-setup.exe")
        sparse_file(old, 1024)
        br.clear_release()
        self.assertFalse(os.path.exists(old), "上一趟的产物不该留在 release/ 顶层")
        self.assertTrue(
            os.path.exists(os.path.join(self.release, "_stale", "steadcopy_0.0.9_x64-setup.exe")),
            "应当挪进 _stale/ 而不是删掉",
        )


if __name__ == "__main__":
    unittest.main()
