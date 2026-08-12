# 门面契约

规范：能力 `cli-driver` 的 spec（openspec 私仓）、能力 `app-shell` 的 spec（openspec 私仓）

**铁律：前端零业务逻辑。** 这份文档是那条铁律的具体形态——路径渲染、增量判定、空间计算、哈希、安全检查、倒计时下限、卷标比对，**全部在 core 里算**。界面只负责发命令、订阅事件、把结论摆出来。

命令定义在 [`app/src-tauri/src/lib.rs`](../app/src-tauri/src/lib.rs)，前端唯一出入口在 [`app/src/bridge.ts`](../app/src/bridge.ts)。**这两处以外不许出现 `invoke(`。**

---

## 命令

### 配置

| 命令 | 入参 | 出参 | 说明 |
|---|---|---|---|
| `get_config` | — | `Config` | 全量配置 |
| `save_settings` | `settings` | — | **倒计时下限在这里硬拒**，界面上的 input 拦不住任何人 |
| `upsert_project` | `input` | `Config` | 新建或更新项目 |
| `delete_project` | `id` | `Config` | 连带清理指向它的预设 |
| `set_current_project` | `id` | `Config` | |
| `upsert_preset` | `preset` | `Config` | |
| `delete_preset` | `id` | `Config` | |
| `move_preset` | `id`, `up` | `Config` | 顺序即优先级 |
| `set_device_kind` | `id`, `kind` | `Config` | 指认设备类型 |
| `rename_device` | `id`, `name` | `Config` | |
| `forget_device` | `id` | `Config` | |
| `preview_path` | `root`, `template`, `project`, `device` | `String` | **前端 MUST 调它**，不许自己实现一份渲染——预览与实际必然漂移 |
| `config_path` | — | `String` | |
| `validate_countdown_secs` | `secs` | `u32` | |

`Settings.locale` 取值 `auto` / `zh` / `en`，默认 `auto`（跟系统，判不出来落中文）。**core 与界面共用这一份设置**——core 产成句、界面产自有文案，两边同一个语言。

配置类命令一律返回**整份新配置**而不是差量：界面不必自己合并状态，也就不会出现「界面显示的和磁盘上的不一致」。

### 设备与到达

| 命令 | 入参 | 出参 | 说明 |
|---|---|---|---|
| `list_devices` | — | `DeviceView[]` | 含 `can_be_source`（准入判据在 core） |
| `start_watching` | — | `bool` | 幂等；返回 false 表示已在监听 |
| `arrive_now` | `deviceRoot` | `ArrivalView` | 手动对某个卷跑一次编排 |
| `confirm_and_run` | `deviceId` | `RunView` | 消费一次已规划好的到达 |
| `dismiss_arrival` | `deviceId` | — | 丢弃这次到达的计划 |
| `cancel_copy` | — | — | |
| `set_paused` | `paused` | — | 暂停 / 继续，块边界响应 |
| `eject_device` | `deviceRoot` | — | 任务进行中一律拒绝 |
| `scan` | `source` | `ScanView` | 只扫不拷 |

### 临时拷贝与沉淀

| 命令 | 入参 | 出参 | 说明 |
|---|---|---|---|
| `adhoc_prefill` | — | `AdhocDefaults` | 临时拷贝面板的预填值。**每个字段都有能直接用的默认值** |
| `plan_adhoc` | `input` | `ArrivalView` | 规划一次临时拷贝。零副作用：不建目录、不写文件、**不落新项目** |
| `sink_preset` | `scope`, `kind?`, `name?` | `Config` | 把刚跑完那次的做法记成预设。一次点击完成，不跳编辑器 |

临时拷贝复用 `confirm_and_run` 执行——**下游分不出任务来源是预设还是临时**。这是刻意的：分得出来，迟早有人为「临时」写一条跳过校验的捷径。

`scope` 取值 `device` / `kind` / `any`，认不出来一律退回最窄的 `device`——放宽必须是显式动作。

`ArrivalView.outcome` 是**九选一的字符串枚举**，每一种「没跑起来」的原因都能被如实呈现：

```
needs_classification  从未见过，等指认。此状态下零写入，且无人值守档也绕不过去
ignored               已标记忽略，静默
already_running       该设备上已有任务
no_preset             没有预设匹配
no_project            预设指向的项目不可用
no_source             卡上没有素材
no_new_source         没有新素材（此前已拷并校验通过）
insufficient_space    目的地空间不足
planned               可以跑了
```

`planned` 时 `requires_confirmation` 决定要不要等用户点。**这个值由 core 判定**，界面与后端的自动路径都只是执行它的结论。

`next_step` 是**每个结论的出口**——只告知不给下一步，等于把死路装修了一下：

```
classify_or_copy_once       指认类型，或就拷这一次
copy_once                   直接开一次临时拷贝
choose_another_destination  换个目的地
view_last_report            看上次的报告
confirm_and_run             确认后开跑
nothing                     不用做什么（已忽略 / 已有任务在跑）
```

### 导图

「导图」把项目的目录结构画成节点图：加节点、把设备连到节点、「全部开始」。**树逻辑全在 core，门面零业务**——节点名校验（非法字符 / 保留名 / 兄弟重名 / 深度与长度上限）、环检测、模板双向转换、刷新 diff 全部在 core 里判，门面只转发命令、回传视图，界面拿到视图整棵重画。

`MapView` 是导图的唯一视图出参：`nodes` 是整棵树（每个节点带自己的落位 `assignments`），`templates` 是模板清单（`[{id, name}]`）。导图命令一律返回**整份 `MapView`** 而不是差量——理由同配置类命令：界面不必自己合并状态，也就不会出现「画布上看着合法、落盘时才报错」。

| 命令 | 入参 | 出参 | 说明 |
|---|---|---|---|
| `map_get` | — | `MapView` | 当前项目的树与模板清单 |
| `map_add_node` | `parentId`, `name` | `MapView` | `parentId` 为 `null` = 挂根下。校验失败当场拒绝，树保持原状 |
| `map_rename_node` | `nodeId`, `name` | `MapView` | 校验同上。节点名允许 `{占位符}`——与字符串模板同一套词表，派发时才渲染 |
| `map_delete_node` | `nodeId` | `MapView` | 连带子树与其上落位。**只动导图，绝不删磁盘目录** |
| `map_move_node` | `nodeId`, `newParentId` | `MapView` | 拖拽换父；`newParentId` 为 `null` = 提到顶层。不许挂到自己或自己的后代下（环检测在 core） |
| `map_assign` | `deviceId`, `nodeId` | `MapView` | 拖一根连线。一张卡可连多个节点、一个节点可收多张卡；**完全相同的设备-节点对拒绝重复**——那只会派出两份一样的任务 |
| `map_unassign` | `assignmentId` | `MapView` | 摘一根连线 |
| `map_dispatch` | — | `{started, rejected[]}` | 起了几个任务 + 没派出去的逐条带原因（`{device_name, reason}`）。**派发走 adhoc 同路，下游不可区分**，详见下文。派发被接受的那一刻设备即被记入占用——排队中的卡再点「全部开始」或发临时拷贝都会被拒，不会重复起任务 |
| `map_refresh_preview` | — | `{additions[], skipped[]}` | `additions` 是可并入的相对路径候选；`skipped` 是名字进不了树的目录（`{path, reason}`，原因 core 成句），只呈现、永不合并——一条坏名不堵死整批刷新。**只读**，一个字节都不写 |
| `map_refresh_apply` | `confirmed` | `MapView` | 确认后并入。`confirmed` 就是预览返回、用户点头的那份 `additions` **原样传回**；落地只并「重算 diff ∩ 确认集」——预览之后磁盘新冒出的目录不会被顺手收编。只增不删；合并对确认集仍是原子的 |
| `map_template_save` | `name` | `MapView` | 树存成模板。落位被剥掉——落位是工位现场的事，换个项目、换一天，卡都不是同一批 |
| `map_template_apply` | `templateId` | `MapView` | 套用到当前项目 |
| `map_clear` | — | `MapView` | 清空当前项目的整棵树与连线。磁盘与模板都不动——导图从不删用户文件 |
| `map_template_delete` | `templateId` | `MapView` | |

错误一律 `Result<_, String>`，串来自 core 的 `describe(lang)`（`lang` 取配置的语言设置）——门面不造句，界面与命令行不会漂出两套说法。

`map_dispatch` 走 `build_adhoc_spec` 同一条构造路径：目标路径 = 项目目的地根 + 节点在树里的路径（占位符此刻才渲染，复用 organize-rules 的 `PathTemplate`）。**下游——队列、引擎、清单、台账、报告——分不出任务来自导图还是预设还是临时拷贝**，这是刻意的：分得出来，迟早有人为「导图任务」写捷径，而捷径总是从跳过校验开始。「校验不可跳过」（连「跟随全局默认」都不给）、「正在跑的设备拒绝重复起任务」「整卡镜像不偷偷过滤」这些不变量因此自动继承，一条不用重写。派发不做 all-or-nothing：三张卡里一张在跑，另两张没理由陪绑——能走的照走，被拒的逐条带原因呈现。

导图**不新增事件**：派发出去的就是普通任务，进度与结束走既有的 `task-*` 事件族，画布上沿连线显示的进度消费的也是同一份负载。`task-started` 与进度负载带可选的 `node_path`（**导图来源才有**，其余入口为 `null`）——同一张卡连两个节点时，画布按 `(device_id, node_path)` 锚定是哪根线在跑；没有 `node_path` 的事件退回按设备匹配。`MapView` 里每个节点带 `path`（core 算好的树内路径，与 `node_path` 同一口径），前端不自己爬树拼路径。

另有一条只读命令 `running_snapshot`（— → `[{device_id, percent, stage_code, node_path?}]`）：进行中任务的进度快照。事件发完即逝，切走 tab 再切回来的面板重挂载补不回错过的事件，挂载时先取这份快照垫底、再接事件流。它不驱动任何判定，排队中（尚未开跑）的任务不在其中。

### 台账

| 命令 | 入参 | 出参 |
|---|---|---|
| `list_history` | `onlyFailed`, `limit?` | `TaskRecord[]` |
| `task_files` | `taskId`, `status?` | `FileRecord[]` |
| `clear_history` | — | — |
| `format_attempts` | — | `FormatAttempt[]` |
| `report_html` | `manifestPath` | `String`（报告全文，塞沙箱 iframe） |
| `run_audit` | `manifestPath` | `AuditResult`（四态） |

### 格式化（危险区）

| 命令 | 入参 | 出参 | 说明 |
|---|---|---|---|
| `check_format` | `deviceRoot` | `FormatSafetyView` | 先跑便宜的 G1–G3，不过就不扫整卷 |
| `do_format` | `deviceRoot`, `typedLabel` | — | **执行前把安全链再跑一遍**，前端点过什么不作数 |

### 更新

| 命令 | 入参 | 出参 | 说明 |
|---|---|---|---|
| `check_update` | — | `UpdateInfo`（`available` / `current` / `version?` / `notes?` / `date?`） | 只在用户按下时查一次。设置里关着就直接报错返回，**连请求都不发出去** |
| `install_update` | — | — | 下载**之前**把地址过一遍主机白名单，不在白名单直接拒；有任务在跑也拒 |

`Settings.update_check` 默认 `false`：没有后台轮询、没有启动时自动检查，关掉之后界面上连按钮都没有。
检查与安装是两个命令，中间隔着用户看到版本号之后的又一次决定——**没有任何路径能让它自己装上**。

更新端点与下载主机白名单都**编译在程序里**，不从配置读：地址一旦可配，谁能改配置文件谁就能让所有客户端装任意程序。

白名单卡在下载**之前**这一点是刻意的。包的真伪由验签保证（私钥只在发布机与 CI secret 里），但验签发生在下载**之后**，
而清单里的 `download_url` 来自网络——端点被劫持时它可以是任意地址。不先卡主机，包虽然装不上（验签会拒），
可请求已经发出去了：**谁在什么时候查更新、从哪个 IP，都泄给了第三方**。对一个把零遥测写在首页上的工具，这条不能留。

### 关于

| 命令 | 入参 | 出参 |
|---|---|---|
| `app_version` | — | `String` |
| `build_info` | — | `BuildInfo`（版本 / 提交 / 构建时间 / 工具链 / 是否便携 / 数据目录） |
| `third_party_licenses` | — | 许可清单 JSON |
| `open_config_file` | — | — |
| `open_report_file` | `manifestPath` | — |
| `reveal_landing_dir` | `manifestPath` | — |
| `open_guide` | — | — |

后四个刻意**不接受任意路径或网址**：配置路径由后端自己算；报告路径必须是稳拷凭证目录里既存的 `.html`；定位落地目录只收清单路径，目录由后端从清单位置往上推两级；教程地址写死在后端常量里，**前端连传一个任意 URL 的机会都没有**。前端因此拿不到「让 shell 打开任意路径」这个能力，`capabilities/default.json` 里也就不需要 `opener:allow-open-path`。

---

## 事件

| 事件 | 负载 | 何时发 |
|---|---|---|
| `device-arrived` | `ArrivalView` | 设备到达且需要用户注意 |
| `device-removed` | — | 设备移除 |
| `task-started` | `deviceId` | 任务开跑（两条路径共用） |
| `task-stage` | `{stage, percent, current}` | 阶段切换 |
| `task-progress` | `{stage, percent, current, bytes_per_sec, eta_secs}` | 进度（**限流在消费方**，引擎发全量、后端按 100ms 收）。速度与 ETA 算不出来时是 `null`，**不拿 0 冒充** |
| `task-file-failed` | `{path, reason}` | 单个文件失败 |
| `task-notice` | `String` | 需要告知的提示（账本降级、自动弹出结果、没触发自动格式化的原因） |
| `task-finished` | `RunView` | 任务结束 |
| `task-failed` | `String` | 任务本身出错 |
| `format-proposed` | `FormatSafetyView` | 拷完全绿且危险区开着「拷完自动格式化」 |
| `sink-suggested` | `SinkView` | 本次做法与已记住的不一致时。**任务一开跑就发**，界面挂成行内提示并保留到结果里 |
| `watch-error` | `String` | 监听启动失败 |

**运行状态一律由事件驱动**，前端没有「我发起的任务」这个概念——无人值守档的任务是后端自己起的，如果前端只认自己发起的，那种任务的进度就没人显示。

---

## 命令行退出码

命令行是一等公民，也是端到端测试的驱动面。

| 码 | 含义 |
|---|---|
| `0` | 成功。**「无新素材」也是 0**——没有新东西要拷是正常结果，不是错误 |
| `1` | 终态族错误（空间不足、配置非法、源不可读、**复验发现数据丢失**——重跑同一份清单答案只会一样） |
| `2` | 可重试族错误（设备中途移除、IO 抖动） |
| `3` | 用户取消 |
| `4` | 用法错误 |

`--json` 下 stdout 只出 JSON，日志一律走 stderr。

---

## 改动纪律

新增或修改命令/事件时：

1. 先改 spec（openspec 私仓），再改代码
2. 同步改 `app/src/bridge.ts` 的类型——**它是前端唯一的出入口**
3. 回来更新本文件
4. 跨层契约的改动在两侧 proposal 的「Impact」里互链

漏掉第 2 步时 `tsc --noEmit` 会红；漏掉第 1 步没有机器能拦，只能靠人审——这也是三制度里禁 propose→apply 一把梭的原因之一。
