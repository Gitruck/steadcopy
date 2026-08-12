# 危险轨测试登记册

> 本册是**双轨约束制**的账本。制度正本在 openspec 私仓（本仓的 submodule）。
>
> **铁律：本册中的测试 MUST NOT 在主理人本机执行。** 本机只跑安全轨（`cargo test` 默认不含 `#[ignore]`）。
> 登记册与代码不一致 = 视为破坏纪律。新增危险轨测试 MUST 同步登记于此。

## 划轨判据（摘要）

一个测试进危险轨，当且仅当满足以下任一条：

1. 会格式化、抹除、重分区任何卷；
2. 会向非临时目录写入或删除（tempdir 之外的真实路径）；
3. 需要管理员权限才能完成；
4. 会操作物理设备（弹出、卷级 IO）；
5. 失败模式包含「误伤本机数据」的可能。

## 三重闸门

危险轨测试 MUST 同时具备：

| 闸门 | 机制 | 不满足时 |
|---|---|---|
| 1 | `#[ignore]` 标注 | `cargo test` 默认不跑 |
| 2 | 环境变量 `STEADCOPY_DANGER_TESTS=1` | 打印说明并**跳过** |
| 3 | 靶标白名单 `STEADCOPY_DANGER_TARGET=<卷 GUID>` | 未指定 → **panic 中止**；靶标为系统盘或本机固定盘 → **panic 中止** |

闸门 3 为何是中止而非跳过：设了闸门 2 却没设靶标，说明运行者以为自己在跑危险测试但配置有误——这种状态必须响，不能静默。

## 虚拟机验收环境要求

| 项 | 要求 |
|---|---|
| 系统 | Windows 10+ 虚拟机，**与主理人本机完全隔离** |
| 靶标磁盘 | 一块专用虚拟磁盘，**上面 MUST NOT 有任何需要保留的数据** |
| 靶标标识 | 以卷 GUID 形式提供（获取方式见下） |
| 网络 | 不需要 |
| 权限 | 视具体用例，部分需管理员 |

获取靶标卷 GUID（在虚拟机内执行）：

```powershell
Get-Volume | Select-Object DriveLetter, FileSystemLabel, Size, UniqueId
```

运行危险轨（**仅在虚拟机内**）：

```powershell
$env:STEADCOPY_DANGER_TESTS=1; $env:STEADCOPY_DANGER_TARGET="<卷GUID>"; cargo test -- --ignored
```

## 登记表

> 每个条目 MUST 填齐下方模板的全部字段。格式化相关条目由 change
> `add-steadcopy-format-card` 写入（见其 `tasks.md` §7），尚未开工。

### D-001 · 无缓冲读回的缓存绕过探针

| 字段 | 内容 |
|---|---|
| 测试函数 | `probe_unbuffered_read_bypasses_page_cache`（手工探针，非自动测试） |
| 文件 | 待建：`crates/steadcopy-core/tests/unbuffered_danger.rs` |
| 所属 change | `add-steadcopy-core` |
| **为什么危险** | 需要以原始卷句柄（`\\.\X:`）绕过文件系统直接写扇区。写错偏移会**破坏该卷的文件系统**，且需要管理员权限。误把靶标指到本机盘 = 直接毁数据 |
| 环境要求 | Windows 虚拟机 · 管理员权限 · 一块**专用虚拟磁盘**（上面 MUST NOT 有任何需保留的数据） |
| 靶标准备 | ① 虚拟机内新建一块虚拟磁盘并格式化；② `Get-Volume \| Select DriveLetter, UniqueId` 取其卷 GUID；③ 确认该卷不是系统盘、不是任何已配置目的地 |
| 运行命令 | `$env:STEADCOPY_DANGER_TESTS=1; $env:STEADCOPY_DANGER_TARGET="<卷GUID>"; cargo test -- --ignored unbuffered` |
| 预期结果 | 在靶标卷上写文件 → 正常读一遍（使页缓存持有内容）→ 经原始卷句柄篡改该文件所占扇区 → `read_unbuffered` **MUST 读到被篡改后的内容**。**若读到的仍是原内容，说明走了页缓存，无缓冲实现失效，判定不通过。** |
| 最近一次验收 | 2026-08-12 通过（主理人：虚拟机 + 另一台真机各一轮） |

**为什么需要这条**：安全轨的「篡改目的地后校验失败」测试**不能**证明缓存被绕过——经普通文件系统的篡改会同时更新页缓存，带缓冲读也会发现。只有绕过文件系统直接改扇区，才能把「读到的是内存副本还是盘上真实字节」这件事区分开。详见 `add-steadcopy-core/design.md` §2。

### D-002 · 格式化真的抹掉数据

| 字段 | 内容 |
|---|---|
| 测试函数 | `scenario_format_card_wipes_target_volume` |
| 文件 | `crates/steadcopy-core/tests/format_danger.rs` |
| 所属 change | `add-steadcopy-format-card` |
| **为什么危险** | **会真正格式化靶标卷**，卷上一切数据不可恢复。靶标指错 = 直接毁数据 |
| 环境要求 | Windows 虚拟机 · 一块**专用虚拟磁盘**（上面 MUST NOT 有任何需保留的数据） |
| 靶标准备 | 见下方「虚拟机验收 SOP」 |
| 运行命令 | 见 SOP |
| 预期结果 | 格式化前写入的探针文件，格式化后 MUST 已不存在 |
| 最近一次验收 | 2026-08-12 通过（主理人：虚拟机 + 另一台真机各一轮） |

### D-003 · 格式化保留文件系统与卷标

| 字段 | 内容 |
|---|---|
| 测试函数 | `scenario_format_card_preserves_filesystem_and_label` |
| 文件 | `crates/steadcopy-core/tests/format_danger.rs` |
| 所属 change | `add-steadcopy-format-card` |
| **为什么危险** | 同 D-002，会真正格式化靶标卷 |
| 环境要求 | 同 D-002 |
| 预期结果 | 格式化后文件系统类型与卷标 MUST 与格式化前一致（相机对这两项有要求，改掉会导致卡不被相机识别） |
| 最近一次验收 | 2026-08-12 通过（主理人：虚拟机 + 另一台真机各一轮） |

### D-004 · 格式化失败给可读原因

| 字段 | 内容 |
|---|---|
| 测试函数 | `scenario_format_card_failure_is_reported_readably` |
| 文件 | `crates/steadcopy-core/tests/format_danger.rs` |
| 所属 change | `add-steadcopy-format-card` |
| **为什么危险** | 会打开物理卷句柄；虽不写入，但与 D-002/003 同批运行 |
| 预期结果 | 对不存在的卷 MUST 返回中文可读错误，MUST NOT panic、MUST NOT 静默成功 |
| 最近一次验收 | 2026-08-12 通过（主理人：虚拟机 + 另一台真机各一轮） |

---

## 虚拟机验收 SOP（交给主理人执行）

> 这份是**唯一**需要在真实环境跑危险轨的流程。开发机上一律不跑。

### 一、准备虚拟机

1. 建一台 Windows 10/11 虚拟机（Hyper-V / VMware / VirtualBox 均可），**与本机完全隔离**；
2. 给它挂一块**新的空虚拟磁盘**（8 GB 足够），在虚拟机里初始化并格式化为 exFAT，卷标随便起（例如 `DANGER`）；
3. **确认这块盘上没有任何需要保留的东西**——这一步跑完它会被反复格式化。

### 二、装工具链并取源码

```powershell
# 虚拟机内
winget install Rustlang.Rustup
rustup default stable-msvc
# 把仓库拷进虚拟机（U 盘 / 共享文件夹 / git clone 均可）
cd C:\steadcopy
```

### 三、取靶标卷标识

```powershell
Get-Volume | Select-Object DriveLetter, FileSystemLabel, FileSystem, Size, UniqueId
```

找到那块专用盘，记下它的 `DriveLetter`（如 `E:`）或 `UniqueId`（卷 GUID）。

### 四、跑危险轨

```powershell
$env:STEADCOPY_DANGER_TESTS = "1"
$env:STEADCOPY_DANGER_TARGET = "E:"     # 换成上一步记下的靶标
cargo test -p steadcopy-core --test format_danger -- --ignored --test-threads=1
```

`--test-threads=1` 是必要的：三条测试都在动同一块盘，并行会互相打架。

### 五、判定

| 结果 | 含义 |
|---|---|
| 三条全 `ok` | 通过。把结论与日期回填到上面各条目的「最近一次验收」 |
| 任一 `FAILED` | **不通过**。把失败输出贴回来，格式化功能保持关闭不发布 |
| 出现 `panic: 靶标 ... 是系统盘` 或 `不在外接总线上` | **闸门正常工作**——说明靶标指错了，改对再跑 |
| 出现 `已开启 STEADCOPY_DANGER_TESTS 但没有指定 STEADCOPY_DANGER_TARGET` | 同上，闸门正常 |

### 六、顺带验一下闸门本身

在**不设**环境变量的情况下跑一次，三条应显示 `ignored`：

```powershell
Remove-Item Env:STEADCOPY_DANGER_TESTS -ErrorAction SilentlyContinue
cargo test -p steadcopy-core --test format_danger
```

看到 `3 ignored` 才算闸门没被误关。

### 条目模板

| 字段 | 内容 |
|---|---|
| 测试函数 | `scenario_xxx` |
| 文件 | `crates/.../tests/xxx.rs` |
| 所属 change | `add-steadcopy-xxx` |
| **为什么危险** | 具体会造成什么不可逆后果 |
| 环境要求 | 系统 / 权限 / 靶标类型 |
| 靶标准备 | 逐步说明 |
| 运行命令 | 可直接复制 |
| 预期结果 | 判定标准，含「什么情况算不通过」 |
| 最近一次验收 | 日期 / 执行人 / 结论 |
