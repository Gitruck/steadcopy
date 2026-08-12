# 发布门控清单

规范：能力 `build-release` 的 spec（openspec 私仓） → Requirement: 发布门控

**任一项未通过就不发版。** 这份清单是「不发」的依据，不是「发了之后补」的备忘。
每次发布把本文件复制一份到 `release/gate-<版本>.md`，逐项填结论与证据，连同产物一起留档。

判定只有三种写法：

- **通过** —— 附证据（命令输出、截图路径、文件路径）
- **不通过** —— 写清卡在哪，并停止发布
- **不适用/已知缺口** —— 只有在本文件已写明的缺口条目里才允许，且缺口 MUST 同时出现在发布说明里

---

## R1 安全轨全绿

```bash
cargo test --workspace
cargo test --manifest-path app/src-tauri/Cargo.toml
```

要求：0 failed。危险轨的 `#[ignore]` 计数与 `docs/danger-tests.md` 登记数一致（多出来的说明有人偷偷加了危险测试）。

`app/src-tauri` 是**独立 workspace**，根目录的 `--workspace` 扫不到它——更新端点白名单那几条测试就住在那里，
少跑第二条命令等于这一门根本没验。

## R2 静态检查全绿

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --manifest-path app/src-tauri/Cargo.toml --all-targets -- -D warnings
cd app && ./node_modules/.bin/tsc --noEmit
```

## R3 Scenario 映射自检

每条 spec 的 `#### Scenario:` 都有同名测试（`scenario_<能力>_<场景>`）。

```bash
python scripts/check-scenarios.py
```

未覆盖的场景要么补测试，要么在本文件写明为已知缺口。

## R4 对抗测试集通过

「本该失败的确实失败了」这一组必须在。至少覆盖：

- 目的地被篡改后校验报错，而不是判等
- 空哈希不等于空哈希（`HashValue` 是定长值类型，压根写不出这种比较）
- 算法不同的哈希不相等
- 路径模板里塞分隔符不会多出一级目录
- 配置损坏不会被静默重建
- 台账库损坏不会被静默重建

## R5 格式化能力的虚拟机验收状态

`docs/danger-tests.md` 的 D-001..D-004 验收结论。

**未验收则：格式化能力在发布版本中保持关闭，且发布说明中不宣称该功能可用。**
判定依据是虚拟机验收记录，不是「本机跑过没事」——本机从来就不跑这些。

| 条目 | 状态 | 验收记录 |
|---|---|---|
| D-001 无缓冲读回确实绕过缓存 | 待验收 | |
| D-002 格式化保留文件系统与卷标 | 待验收 | |
| D-003 安全链拦下系统盘/目的地盘 | 待验收 | |
| D-004 格式化留痕四态齐全 | 待验收 | |

## R6 clean-room 自检留痕

对照 openspec 私仓里记的继承边界：

- 未复制前身任何源码、资源、图标、文案
- 界面形态与前身、与调研过的同类产品均不构成近似（逐屏比对，结论写进 `release/gate-<版本>.md`）
- 名称与商标检索结论仍然成立（复检一次，名字冲突是会随时间变的）

## R7 依赖许可无 GPL 系

```bash
python scripts/gen-licenses.py
```

要求：退出码 0（脚本自带 GPL/AGPL/SSPL/CDDL/EPL 闸门），且 `app/src-tauri/licenses.json`
与 `release/THIRD-PARTY-LICENSES.md` 是**本次**生成的（不是上次留下的）。

## R8 性能基准通过

在同一台机器、同一张卡上跑基准，与上一版比对，回退超过 10% 要有解释。
记录：卡型号、容量、文件数、单目的地与双目的地的吞吐、校验遍耗时。

## R9 真机走查通过

真卡插入 → 确认 → 拷贝 → 校验 → 报告 → 复验，全链路走一遍。至少覆盖：

- 一张相机卡（有 `DCIM` 或厂商目录结构）
- 一个 U 盘或移动硬盘
- 拔卡中断一次，重插后走增量（只拷新增，跳过已完成）

## R10 离线安装验证

用**离线版**安装包在断网机器上安装并启动成功，安装过程中没有任何联网下载（WebView2 运行时随包）。
精简版这一门不适用——它本来就假设机器已有运行时，没有时会去拉。

## R11 便携版验证

- 解压即可运行，不写注册表
- 便携版与安装版的数据目录互不影响（分别建项目，互相看不见）
- 便携版目录整体拷到另一台机器仍可运行

## R12 校验码三处同步

`release/SHA256SUMS.txt`、GitHub Releases 页、官网下载页三处对同一产物给出相同 SHA-256。

```bash
python scripts/gen-checksums.py
```

## R13 未签名说明已就位

- README 有说明
- 官网下载页有说明
- 应用内「设置 → 关于」有说明与核对方法

且**三处都不出现「关闭杀毒软件」「添加信任后运行」之类的表述**。教用户关防护是把安全成本转嫁给用户，本项目不做。

## R14 杀软白名单已申报 + 干净机器全流程走查

- 按 `docs/antivirus-whitelist.md` 完成误报申报，记录申报单号与回执
- 一台从未装过本程序的干净 Windows 机器，从下载 → 校验码核对 → 安装 → 首次运行 → 完成一次拷贝，全程走通

## R15 两版安装包的 productName 相同

两个包是**同一个产品的两种装法**，唯一差别是 `webviewInstallMode`。名字一旦不同，NSIS 会装到不同目录、
注册不同的卸载项——而更新清单指向的是精简版，**离线版用户点一次更新就会装出第二份**，
两份各自监听插卡，谁也不知道自己在用哪一个。

```bash
grep '!define PRODUCTNAME' app/src-tauri/target/release/nsis/x64/installer.nsi
```

这个文件每趟 build 都会覆盖，所以**要在两趟之间各看一次**（`scripts/build-release.py` 打完精简版接着打离线版）。
两次输出必须一字不差。也可以在同一台机器上先装离线版再装精简版，确认安装目录与「应用和功能」里始终只有一项、
只有一个卸载入口。

另外确认 `tauri.offline.conf.json` 里**没有** `productName`——它是增量合并的，多写一行就分家。

## R16 自有镜像发布并回读确认

镜像是 `plugins.updater.endpoints` 的**第一个**端点，GitHub 是兜底。国内常连不上 GitHub，
而**仓库私有期间 GitHub 那个端点对匿名客户端直接 404**——那时候镜像是唯一真正能用的更新源，
它没挂好等于所有人都收不到更新。

镜像目录挂在 NAS 上（`T:\web\broadcast\steadcopy` ↔ `https://api.ai-mcn.tv:9000/broadcast/steadcopy`），
GitHub 托管跑器够不着，**这一步只能在发布机本地跑**：

```bash
# 产物用 CI 打的那批，不要本地重编——签名是对具体那批字节签的。
# --zip 直接收 Actions 页面下载下来的产物包，不用自己解压（少一步就少一次「解错目录」）
python scripts/publish-mirror.py --zip <从 Actions 下载的产物包>
```

三条都要有结论：

- **推上去了**：脚本先拿 `SHA256SUMS.txt` 核对源目录，再把两个安装包、两个 `.sig`
  与 `latest.mirror.json`（改名为 `latest.json`）复制过去
- **回读确认**：脚本从 `https://api.ai-mcn.tv:9000/broadcast/steadcopy/latest.json` 取回清单并与刚发布的逐字比对，
  再对安装包发一次 Range 请求确认真能下。复制成功不等于发布成功——目录可能没被 web 服务收录、可能有缓存，
  而这一环断了只有客户端知道，客户端不会来告诉你
- **与 Releases 同一批字节**：镜像上的安装包 SHA-256 与 `release/SHA256SUMS.txt`、与 Releases 页公示的一致。
  `latest.json` 两份除 `url` 外完全相同，**签名必须相同**——不同就说明有一边是重编的，客户端下完会拒装

---

## 本版本的已知缺口

发版前把这一节改成实际情况，并**原样抄进发布说明**。

- **更新检查默认关闭**：有更新检查，但默认关，且只在用户按下按钮时联网一次，查到也绝不自动安装。代价是默认状态下用户不会被告知新版本——这是拿「不静默更新 + 零遥测」换来的，不打算改。
- **仓库私有期间只有镜像端点可用**：GitHub 那个端点对匿名客户端 404，兜底端点实际上是哑的。开源之前，R16 一旦没做，更新链就是断的。
- **精简版与便携版依赖系统 WebView2**：只有离线版安装包内置运行时。Windows 11 与 Windows 10 22H2 自带；更旧的系统请用离线版安装包。
- **格式化能力**：以 R5 的验收结论为准。
