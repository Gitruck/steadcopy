# Agent 指引 · steadcopy（稳拷）

## 动手前必读

1. [`openspec/project.md`](openspec/project.md) —— 立项上下文：定位与免费逻辑、**与前身项目的 clean-room 继承边界**、品类基线与市场空位、架构、Windows 深坑、依赖许可地图、V1 范围与非目标、术语表。
2. [`openspec/README.md`](openspec/README.md) —— **三制度正本**：SDD（OpenSpec）+ TDD（规格锚定的 Detroit 式）+ 双轨约束制；含决策台账 P1–P8。
3. [`openspec/config.yaml`](openspec/config.yaml) —— 机器规则（propose / design / tasks 三阶段的硬要求）。

## 项目焦点

Windows 拷卡（DIT offload）工具。开源 MIT，长期免费含商用。**不承担营收**，是产品矩阵的信用资产——所以零账号、零付费墙、零服务端依赖、离线可用。

一条交付链路，改一处往往要顺链协调：

- `crates/steadcopy-core/` —— 平台无关业务核（引擎 / 账本 / 设备 / 组织 / 台账）
- `crates/steadcopy-cli/` —— 薄驱动面，**E2E 测试的唯一入口**
- `app/` —— Tauri 2 壳，**零业务逻辑**
- `openspec/specs/<capability>/spec.md` —— 长期规范正本

## 护栏（Guardrails）

**架构铁律**

- 前端零业务逻辑。路径渲染 / 增量判定 / 空间计算 / 哈希一律调 core。**前端算一份 = 两份必然漂移。**
- **MUST NOT** 用 PowerShell / robocopy / 外部 sidecar 二进制干系统活。
- **MUST NOT** 解析任何本地化文本输出来判断状态（前身解析 robocopy 中文输出，换英文系统即盲）。
- **MUST NOT** 在 Tauri capabilities 里放行任意命令执行或宽泛 fs 权限（前身放行 `powershell` 任意参数 = 前端 XSS 即 RCE）。

**数据安全铁律**

- **源卡只读**：MUST NOT 向源设备写入任何文件，含设备身份标记。
- **读回校验必须无缓冲**（`FILE_FLAG_NO_BUFFERING`），否则读到页缓存 = 校验表演。
- **哈希失败绝不降级**：MUST NOT 存在「双方取到空值因而相等，判定通过」的路径（前身一号缺陷）。哈希用定长值类型，不用字符串。
- **可移动性判定用正向证明 + 失败即拒绝**：MUST NOT 用「不在固定盘列表中即视为可移动」的反向排除（前身因 WMI 键名拼错导致该检查恒真，可对任意盘格式化）。
- 显式报错，绝不静默降级。良性降级打可读 INFO，未知才升级别。

**流程铁律**

- **禁止 propose → apply 一把梭。** apply 前人审 `tasks.md`，archive 前人审 `archived.md`。
- change 名一旦定下不可改；不回改已归档 change。
- 重要决策 MUST 写进工件——只留在对话里就等于丢。
- **危险轨测试 MUST NOT 在本机执行**，登记见 [`docs/danger-tests.md`](docs/danger-tests.md)。
- 探索性想法放 `.plans/`，不硬塞 OpenSpec change。

**法律铁律（clean-room）**

本项目重写自一个交接来的项目，交接人要求重写 + 重命名 + 重定调、UI 不能像。
（前身的具体身份与逐条继承边界见 openspec/project.md —— 那份归 Gitea 私仓，不进公开仓。）

- **MUST NOT** 复制前身的代码 / 文案 / 词典 / UI 布局 / 配色 / 窗口形态。
- **MUST NOT** 复刻竞品有识别度的表达：影视飓风 Gate 的脑图画布 + 拖拽指派、Kocard 的三色流水线卡片布局。
- **MUST NOT** 链接或抄写 GPL 系代码（xcp / Rapid Photo Downloader / OffloadBuddy）；xxHash 库 BSD-2 可链，其 `xxhsum` CLI 是 GPL 不可抄。
- 品类公有域概念（多目的地、校验、路径模板、队列）可自由实现——四家商业产品全部具备，不构成任一家的专有表达。

## 测试纪律速览

- **Scenario 即测试**：每条 spec 的 `#### Scenario:` 对应一个 `scenario_<capability>_<slug>` 测试函数。没有 Scenario 不写实现，没有测试不标 done。
- **红 → 绿 → 重构**，顺序不可倒。
- **不 mock 文件系统**——用 `tempfile::TempDir` 真实 IO。仅允许两类替身：`DeviceEventSource`（无法真插拔）与 `Clock`（倒计时/退避）。
- **对抗测试是一等公民**：故意篡改目的地、故意让哈希失败、故意中途拔卡——数据完整性代码的正确性只能这样证明。
- 覆盖率不设百分比 KPI（会诱导写无断言的假测试），设结构性要求。
