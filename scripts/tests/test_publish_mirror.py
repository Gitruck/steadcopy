#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""镜像发布脚本的安全轨。

规范：能力 `build-release` 的 spec（openspec 私仓）
      → Requirement: 镜像与发布页挂同一批字节

三条场景以前挂在「已知缺口」里。它们值得测，因为**每一条的失败都只有用户会遇到**：
镜像上挂了错的字节、挂了指向不存在文件的清单、或者压根没挂上去——
客户端那头验签失败、下载 404，而客户端那头没人会来告诉你。

回读那条要一个真的 HTTP 服务，用标准库 `http.server` 起在 127.0.0.1 上。
不 mock `urlopen`：这条防线防的正是「网络那头到底给了什么」，
mock 掉就等于把要测的东西假设成对的。
"""

import http.server
import io
import json
import os
import shutil
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from loader import Quiet, load  # noqa: E402

pm = load("publish-mirror")

SLIM = "steadcopy_0.1.0_x64-setup.exe"
BODY = b"MZ" + bytes(range(256)) * 12  # 3074 字节，够 Range 取满 1024


def sha256_of(data):
    import hashlib

    return hashlib.sha256(data).hexdigest()


class Serve(http.server.BaseHTTPRequestHandler):
    """一台可以被指使着「给出错误答案」的镜像。

    `mode` 由测试设定：正常 / 空正文 / 截断 / 内容不符 / 404。
    """

    mode = "ok"
    manifest = {}

    def log_message(self, *_a):  # 别把测试输出淹了
        pass

    def _send(self, code, body, ctype="application/octet-stream"):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)

    def do_GET(self):
        if self.mode == "404":
            return self._send(404, b"not found", "text/html")
        if self.path.endswith("latest.json"):
            data = json.dumps(Serve.manifest, ensure_ascii=False, indent=2) + "\n"
            if self.mode == "stale-manifest":
                bad = json.loads(json.dumps(Serve.manifest))
                bad["version"] = "0.0.1"
                data = json.dumps(bad, ensure_ascii=False, indent=2) + "\n"
            return self._send(200, data.encode("utf-8"), "application/json")
        if self.path.endswith(".exe"):
            if self.mode == "empty":
                return self._send(200, b"")
            if self.mode == "truncated":
                return self._send(206, BODY[:100])
            if self.mode == "wrong":
                return self._send(206, b"X" * 1024)
            return self._send(206, BODY[:1024])
        return self._send(404, b"")


class PublishMirror(Quiet, unittest.TestCase):
    def setUp(self):
        super().setUp()
        self.tmp = tempfile.mkdtemp(prefix="steadcopy-pm-test-")
        self.src = os.path.join(self.tmp, "src")
        self.nas = os.path.join(self.tmp, "nas", "steadcopy")
        os.makedirs(self.src)
        os.makedirs(os.path.dirname(self.nas))

        with open(os.path.join(self.src, SLIM), "wb") as f:
            f.write(BODY)
        with open(os.path.join(self.src, SLIM + ".sig"), "w", encoding="utf-8") as f:
            f.write("untrusted comment: signature\nAAAA\n")
        self.write_sums({SLIM: sha256_of(BODY)})

        self._saved_base = pm.PUBLIC_BASE
        self.manifest = {
            "version": "0.1.0",
            "notes": "test",
            "platforms": {
                "windows-x86_64": {
                    "signature": "AAAA",
                    "url": f"{pm.PUBLIC_BASE}/{SLIM}",
                }
            },
        }
        self.write_manifest(self.manifest)

    def tearDown(self):
        super().tearDown()
        pm.PUBLIC_BASE = self._saved_base
        shutil.rmtree(self.tmp, ignore_errors=True)

    def write_sums(self, mapping):
        lines = [f"{d} *{n}" for n, d in mapping.items()]
        io.open(os.path.join(self.src, "SHA256SUMS.txt"), "w",
                encoding="utf-8", newline="\n").write("\n".join(lines) + "\n")

    def write_manifest(self, data):
        io.open(os.path.join(self.src, "latest.mirror.json"), "w",
                encoding="utf-8", newline="\n").write(
            json.dumps(data, ensure_ascii=False, indent=2) + "\n"
        )

    def serve(self, mode):
        """起一台本地镜像，把 PUBLIC_BASE 指过去。返回关服务的函数。"""
        Serve.mode = mode
        Serve.manifest = self.manifest
        srv = http.server.HTTPServer(("127.0.0.1", 0), Serve)
        t = threading.Thread(target=srv.serve_forever, daemon=True)
        t.start()
        pm.PUBLIC_BASE = f"http://127.0.0.1:{srv.server_port}"
        return lambda: (srv.shutdown(), srv.server_close())

    # ---------------------------------------------------------------- 场景

    def test_scenario_build_release_mirror_refuses_when_bytes_differ(self):
        """字节不一致则拒绝发布。

        清单里的签名是对**具体那批字节**签的。挂上去的若不是那批（拷贝坏了、
        拿错了一批产物、本地重编过一次），客户端下完验签失败、拒装——
        而这只有在用户真的点了「安装更新」之后才会暴露。
        """
        # 校验码对不上
        self.write_sums({SLIM: "0" * 64})
        with self.assertRaises(SystemExit) as cm:
            pm.check_checksums(self.src)
        self.assertIn("校验码对不上", str(cm.exception))

        # 清单里有、目录里没有
        self.write_sums({SLIM: sha256_of(BODY), "不存在.exe": "1" * 64})
        with self.assertRaises(SystemExit) as cm:
            pm.check_checksums(self.src)
        self.assertIn("目录里没有", str(cm.exception))

        # 压根没有校验清单：没有它就无从确认「挂的是发出去的那批字节」
        os.remove(os.path.join(self.src, "SHA256SUMS.txt"))
        with self.assertRaises(SystemExit) as cm:
            pm.check_checksums(self.src)
        self.assertIn("SHA256SUMS.txt", str(cm.exception))

    def test_scenario_build_release_manifest_points_into_this_mirror(self):
        """清单的 url 必须落在本镜像下，且指向的包必须真的在。

        指到别处去，客户端要么下到别人的东西（若那个别处碰巧在编译期白名单里），
        要么直接被白名单拒——两种都是发布事故。
        """
        bad = json.loads(json.dumps(self.manifest))
        bad["platforms"]["windows-x86_64"]["url"] = "https://attacker.example/x.exe"
        self.write_manifest(bad)
        with self.assertRaises(SystemExit) as cm:
            pm.mirror_manifest(self.src)
        self.assertIn("不在本镜像下", str(cm.exception))

        # url 合规但文件不在
        gone = json.loads(json.dumps(self.manifest))
        gone["platforms"]["windows-x86_64"]["url"] = f"{pm.PUBLIC_BASE}/不存在.exe"
        self.write_manifest(gone)
        with self.assertRaises(SystemExit) as cm:
            pm.mirror_manifest(self.src)
        self.assertIn("没有这个文件", str(cm.exception))

    def test_scenario_build_release_readback_failure_blocks_announcement(self):
        """回读不通过则不宣布更新可用。

        复制成功 ≠ 发布成功：目录可能没被 web 服务收录、可能有缓存、
        可能路径映射变了。四种坏答案都必须判失败——尤其是**空正文**：
        它曾经能骗过回读（两边都切成空串判等），而「地址通了但一个字节都取不到」
        恰恰是最典型的镜像故障。
        """
        for mode, why in [
            ("empty", "空正文"),
            ("truncated", "截断"),
            ("wrong", "内容不符"),
            ("404", "取不到"),
        ]:
            stop = self.serve(mode)
            try:
                ok = pm.verify_live(self.src, SLIM, sha256_of(BODY), self.manifest)
            finally:
                stop()
            self.assertFalse(ok, f"{why}的镜像必须判为回读失败")

        # 清单内容与刚发布的不一致（缓存没刷新）也要判失败
        stop = self.serve("stale-manifest")
        try:
            ok = pm.verify_live(self.src, SLIM, sha256_of(BODY), self.manifest)
        finally:
            stop()
        self.assertFalse(ok, "镜像上挂的清单与刚发布的不一致时必须判失败")

        # 一切正常时要放行——别把闸门修成「什么都拦」，那和没有闸门一样没用
        stop = self.serve("ok")
        try:
            ok = pm.verify_live(self.src, SLIM, sha256_of(BODY), self.manifest)
        finally:
            stop()
        self.assertTrue(ok, "正常镜像必须判为通过")

    def test_scenario_build_release_manifest_is_written_after_packages(self):
        """清单先于安装包写入则视为失败。

        顺序反过来的话，在两次复制之间检查更新的客户端会拿到一份指向还不存在的
        文件的清单，下载 404。

        测法：在写 `latest.json` 的那一刻给 NAS 目录拍张快照，
        断言那时候安装包已经在了。不去断言「调用了哪些函数」——那测的是实现，
        换个写法就红，而要保的是**结果的先后**。
        """
        snapshot = {}
        real_open = pm.io.open

        def spy(path, *a, **kw):
            if str(path).endswith("latest.json") and "w" in (a[0] if a else kw.get("mode", "")):
                snapshot["at_manifest_write"] = sorted(os.listdir(self.nas))
            return real_open(path, *a, **kw)

        stop = self.serve("ok")
        pm.io.open = spy
        argv = sys.argv
        try:
            self.manifest["platforms"]["windows-x86_64"]["url"] = f"{pm.PUBLIC_BASE}/{SLIM}"
            self.write_manifest(self.manifest)
            Serve.manifest = self.manifest
            sys.argv = ["publish-mirror.py", "--dir", self.src, "--nas", self.nas,
                        "--publish-manifest"]
            pm.main()
        finally:
            pm.io.open = real_open
            sys.argv = argv
            stop()

        self.assertIn("at_manifest_write", snapshot, "根本没写清单？")
        self.assertIn(SLIM, snapshot["at_manifest_write"],
                      "写清单的时候安装包必须已经在镜像上了——先有包再有清单")
        self.assertIn(SLIM + ".sig", snapshot["at_manifest_write"],
                      "签名也要先于清单到位")

    def test_scenario_build_release_manifest_stays_put_until_gate_passes(self):
        """不带 `--publish-manifest` 就不许写清单。

        写下 `latest.json` 的瞬间，所有开着「检查更新」的客户端就能装到这一版了。
        GitHub 那边还是草稿**不作数**——镜像是第一个端点，它说了算。
        所以清单必须等门控全过之后再写，否则「草稿 Release」那道闸形同虚设。
        """
        argv = sys.argv
        try:
            sys.argv = ["publish-mirror.py", "--dir", self.src, "--nas", self.nas]
            pm.main()
        finally:
            sys.argv = argv

        self.assertTrue(os.path.exists(os.path.join(self.nas, SLIM)),
                        "包应当已经传上去了——传了也没人找得到，因为清单还没换")
        self.assertFalse(os.path.exists(os.path.join(self.nas, "latest.json")),
                         "没加 --publish-manifest 就写了清单，等于门控前就把版本发出去了")

    def test_scenario_build_release_rollback_restores_previous_manifest(self):
        """撤回：把清单换回上一版，安装包不删。

        安装包不删是刻意的——旧客户端可能正下到一半，抽掉文件只会让它们拿到半个包。
        换回清单就够了：新的检查更新不会再看到撤回的那版。
        """
        os.makedirs(self.nas, exist_ok=True)
        prev = {"version": "0.0.9", "platforms": {}}
        io.open(os.path.join(self.nas, "latest.prev.json"), "w",
                encoding="utf-8", newline="\n").write(json.dumps(prev) + "\n")
        io.open(os.path.join(self.nas, "latest.json"), "w",
                encoding="utf-8", newline="\n").write(json.dumps(self.manifest) + "\n")
        keep = os.path.join(self.nas, SLIM)
        io.open(keep, "w", encoding="utf-8").write("x")

        pm.rollback(self.nas)

        live = json.load(io.open(os.path.join(self.nas, "latest.json"), encoding="utf-8"))
        self.assertEqual("0.0.9", live["version"], "清单应当换回上一版")
        self.assertTrue(os.path.exists(keep), "安装包不该被删——有人可能正下到一半")

        # 没有上一版可还原时要说清楚，而不是默默什么都不做
        os.remove(os.path.join(self.nas, "latest.prev.json"))
        with self.assertRaises(SystemExit):
            pm.rollback(self.nas)


if __name__ == "__main__":
    unittest.main()
