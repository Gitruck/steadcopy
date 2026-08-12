# 门面契约

规范：`openspec/changes/add-steadcopy-core/specs/cli-driver/spec.md`、`openspec/changes/add-steadcopy-app/specs/app-shell/spec.md`

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

1. 先改 spec（`openspec/changes/*/specs/`），再改代码
2. 同步改 `app/src/bridge.ts` 的类型——**它是前端唯一的出入口**
3. 回来更新本文件
4. 跨层契约的改动在两侧 proposal 的「Impact」里互链

漏掉第 2 步时 `tsc --noEmit` 会红；漏掉第 1 步没有机器能拦，只能靠人审——这也是 `openspec/README.md` 里禁 propose→apply 一把梭的原因之一。
