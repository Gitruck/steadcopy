# 源设备接入方式与卡内结构（调研结论）

> 2026-08-10 调研。本文是 `device` 与 `organize` 两层实现的**事实依据**，写代码前先读。
> 标注「待真机确认」的条目 MUST NOT 被当作既定事实写进判定逻辑。

## 一、总表：Windows 上到底怎么被读取

判据的技术根源：Windows 把设备栈一分为二——**MSC**（USB Mass Storage）走存储栈，挂成卷、有盘符；
**MTP/PTP** 走 **WPD**（Windows Portable Devices），只能经 COM 访问，**没有卷、没有盘符**。
微软官方确认便携设备**无法被分配盘符，也没有可用的路径语法**。

| 源设备 | 挂载形态 | 有盘符 |
|---|---|---|
| 相机卡 + 读卡器（SD/CFe/CF/XQD） | USB MSC | ✅ |
| Sony 机身直连（MassStorage 模式） | 可移动磁盘 | ✅ |
| Sony 机身直连（Auto/MTP，Win 上默认落 MTP） | 便携设备 | ❌ |
| Panasonic 机身直连 PC(Storage) | 可移动磁盘，**只读** | ✅ |
| Canon EOS 直连 | 便携设备（PTP） | ❌ |
| Nikon Z 直连 | 便携设备（MTP/PTP） | ❌ |
| Fujifilm「USB CARD READER」 | 便携设备（**名字骗人，实为 PTP**） | ❌ |
| 安卓手机 | 便携设备（MTP） | ❌ |
| iPhone / iPad | 便携设备（PTP，只暴露 DCIM） | ❌ |
| 大疆运动相机 / Pocket / 无人机 | USB MSC（内存与卡是**两个卷**） | ✅ |
| DJI 遥控器（RC/RC Pro/RC2） | 多为 MTP（Android 底子）· **待真机确认** | 多数 ❌ |
| Insta360（USB Drive Mode） | USB MSC | ✅ |
| **DJI Mic / Mic 2 / Mic 3 发射器 TX** | **USB MSC（普通 U 盘）** | ✅ |
| DJI Mic 接收器 RX | USB **音频**设备，无录音存储 | ❌ |
| U 盘 / 移动硬盘 / SSD | USB MSC 或 UASP/NVMe 桥接 | ✅ |
| Zoom / Tascam / Sound Devices | USB MSC（需在机器上选 Storage 模式） | ✅ |
| Atomos / BMD Video Assist | 普通卷（exFAT）；HFS+ 则 Windows 读不了 | ✅ / 条件 |

**主路径 = 读卡器 + 盘符。** 机身直连与手机直连一律当降级路径，并在界面上主动引导用户改用读卡器。

## 二、三条推翻既有规格的发现

### F1 · CFexpress 常被判成「固定磁盘」——不可用 `DRIVE_REMOVABLE` 过滤源

CFexpress 本质是 NVMe over PCIe，读卡器桥接芯片（常见 Realtek RTL9210B）往往透传 NVMe 身份、**不置 removable 位**。后果：托盘没有安全弹出；**任何按 `DRIVE_REMOVABLE` 过滤导入源的软件直接看不到卡**——Lightroom Classic 与 Adobe Bridge 都有实证。这是读卡器固件属性，Windows 侧改不了。

另有多起 Win10/11 报告 CFexpress **枚举了但无盘符**，需在磁盘管理手工指定。

**判定规则（正向证明，替代 `DRIVE_REMOVABLE`）**：

一个卷可作为源卡，当且仅当**全部**成立：
1. **不是系统盘**；
2. **不是任何已配置的目的地所在卷**；
3. **总线类型 ∈ {USB, Thunderbolt/1394, SD, MMC}**（经 `IOCTL_STORAGE_QUERY_PROPERTY` 的 `StorageAdapterProperty` 取 `BusType`）——这一条能同时容纳「removable 位为假的 CFexpress」与「排除内置 NVMe/SATA 盘」；
4. 加分信号（用于自动分类，非准入条件）：卷根存在**设备指纹目录**——`DCIM` / `PRIVATE` / `XDROOT` / `CONTENTS` / `CANONXF` / `DJI_AUDIO` / `MISC` / `Movie` / `MP_ROOT` / `AVF_INFO`。

**「有卷无盘符」MUST 支持**：经 `\\?\Volume{GUID}\` 访问。

### F2 · 行业共识是整卡镜像，不做文件白名单

sidecar 缺一个就可能让整条素材在 NLE 里废掉（Canon 的 `INDEX.MIF` 是 Canon 应用挂载卡的硬性前提；Insta360 官方明说双 `.insv` 缺一个就认不出）。

**规则修正**：**默认整卡镜像**（拷贝卷上全部内容，排除系统垃圾）。类型过滤改为**显式 opt-in**，且开启时 MUST 警示「可能遗漏配套文件导致素材在剪辑软件里不可用」。过滤优先作为**视图层**能力（列表里隐藏辅助文件）而非拷贝层能力。

可安全排除的系统垃圾：`System Volume Information`、`$RECYCLE.BIN`、`Thumbs.db`、`.Trashes`、`.Spotlight-V100`、`.fseventsd`、`.TemporaryItems`、`.DS_Store`、`._*`（AppleDouble）。

### F3 · iOS 默认把 HEIC/HEVC 转码后才交给 PC

转码**发生在 iPhone 端**，设置位于「设置 → 应用 → 照片 → 传输到 Mac 或 PC」：

- **自动（默认）**：高效格式的照片/视频在传给 PC 时转成 **JPEG / H.264**；
- **保留原始格式**：不转换。

因此**默认设置下拷到的不是原文件**，且任何拷贝路径都逃不掉。更麻烦的是行为不稳定：iOS 17 起「自动」会先探测宿主机能力——PC 装了 HEIF/HEVC 扩展就不转了，于是同一台手机在 Win10 给 JPG、Win11 给 HEIC。

**产品红线**：检测到 iOS 源时 MUST 强提示改为「保留原始格式」；若拷到的是 JPEG 而设备上是 HEIC，本次备份 MUST 被标记为「非原始文件」。**不许静默。**

## 三、MTP/WPD 通道的能力边界

Rust 侧路径：`winmtp` crate 做主干（唯一现成可用），缺的能力用它 re-export 的 windows-rs `Win32::Devices::PortableDevices` 裸接口补。COM 用 `COINIT_APARTMENTTHREADED`。传输照微软官方 sample：`IPortableDeviceResources::GetStream` 拿 `pdwOptimalBufferSize` 当块大小循环读，并**把累计字节数与 `WPD_OBJECT_SIZE` 对账**。
libusb/rusb **不可行**——Windows 上 MTP 接口被 WPD 内核驱动独占，换 WinUSB 驱动后 Explorer 就看不到设备了。

**能力边界（决定产品定位）**：

| 项 | 实情 |
|---|---|
| 文件系统语义 | 无。对象/事务模型，只有 ObjectID 树 |
| 随机访问 | 无。`IStream` 当 forward-only 用 |
| 无缓冲读 | **完全不适用**——没有 `HANDLE`、没有卷、没有扇区 |
| 文件大小 | **可能被截断**：`ObjectCompressedSize` 是 UINT32，>4 GB 时规范要求填哨兵值，但很多实现直接 `size & 0xFFFFFFFF` |
| 大文件 | ≥4 GB 公认高危，中途静默丢文件的报告大量存在 |
| 时间戳 | 多数设备只到秒，部分退化到 2 秒粒度，有报成 1970 的实例。**不可作去重判据** |
| 原地校验 | 无。设备端不提供 hash，只能读两遍比对 |
| 断点续传 | 协议无 resume、无 checksum 概念 |

**结论：MTP 通道做不到「可信备份」。** 产品策略是**支持但降级标注**——MTP 来源的拷贝，报告里明确打上「未经完整校验（MTP 协议限制）」，并在界面引导「手机请用 SD 卡 / 相机请用读卡器 / iPhone 请录到外置 SSD」。

## 四、卡内目录结构

### Sony
```
DCIM/100MSDCF/              照片
PRIVATE/M4ROOT/CLIP/        XAVC S 正片 .MP4 + 同名 .XML
PRIVATE/M4ROOT/SUB/         代理片段
PRIVATE/M4ROOT/THMBNL/      缩略图
PRIVATE/XDROOT/Clip/        专业机 .MXF
PRIVATE/AVCHD/BDMV/STREAM/  AVCHD .MTS
AVF_INFO/                   AVCHD 播放列表/管理数据库
MP_ROOT/                    纯 MP4 模式
```
指纹：`PRIVATE/M4ROOT` ⇒ Sony 消费/准专业；`PRIVATE/XDROOT` ⇒ Sony 专业机。

### Canon
```
DCIM/100CANON/              照片 + 视频
DCIM/CANONMSC/              管理信息与缩略图元数据（官方警告勿删）
MISC/                       DPOF
CONTENTS/CLIPS001/          XF-AVC .MXF
CANONXF/CLIPS001/           老 XF 系
CANONXF/CLIPS001/INDEX.MIF  卡上所有 clip 的索引，Canon 应用挂载卡的硬性前提
```
Cinema RAW Light（.crm）目录结构**待确认**。

### Nikon
```
DCIM/100NC*/                DSC_####.NEF/.JPG/.MOV/.NEV/.MP4
NC_FLLST.DAT                卡根索引
```
**N-RAW 一拍产多个文件**：`.NEV` + 同名 `.MP4` 代理（+ 部分 `.DAT`），必须成组。
双卡 Backup 模式会在两张卡产生**完全相同的文件**——是重复不是两条素材。

### Panasonic
```
DCIM/100_PANA/                      JPG/.RW2/.MP4/.MOV/.HSP
PRIVATE/AVCHD/BDMV/STREAM/*.MTS
PRIVATE/AVCHD/BDMV/{INDEX.BDM, MOVIEOBJ.BDM}, CLIPINF/*.CPI, PLAYLIST/*.MPL
PRIVATE/AVCHD/AVCHDTN/
```
每 999 张换一个 `1xx_PANA`。**已有真实事故：只导 DCIM、忘了 PRIVATE，然后把卡格了。**

### Fujifilm
`DCIM/1xx_FUJI/`，`DSCF####.JPG/.RAF/.MOV`。官方无完整目录结构文档，**待真机确认**。

### 大疆
```
DCIM/DJI_001/ 或 DCIM/1xxMEDIA
  DJI_0001.MP4   正片
  DJI_0001.LRF   720p 低码率代理
  DJI_0001.SRT   飞行遥测字幕（须与 MP4 同目录才被识别）
  DJI_0001.JPG   缩略图/抓拍
MISC/FC8282.db   机型命名的 SQLite 索引
```
新机型改时间戳命名 `DJI_20250731171839_0066_D.MP4`。内置存储与 SD 卡是**两个独立卷**。
新机型（Air 3S、Mini 5 Pro）趋势是遥测内嵌进 MP4、不再产 SRT。
4 GB 分段后的具体命名规律**待真机确认**。

### Insta360
```
DCIM/Camera01/
  VID_<时间戳>_00_657.insv   镜头一
  VID_<时间戳>_10_657.insv   镜头二（≥5.7K 360 才有）
  LRV_<时间戳>_01_657.lrv    低分预览（索引用 01/11，与 VID 的 00/10 不同）
  IMG_..._00_xxx.insp        360 照片
  PRO_...                    单镜头 SteadyCam 模式
```
**官方明说：同一条 clip 生成的所有文件必须一起拷，不能删、不能改名。**

### GoPro（用户未列，但存量大且是差异点）
```
DCIM/100GOPRO/
  GX010527.MP4   HEVC，章节 01，片号 0527
  GX020527.MP4   同一条的第二段
  GH010527.MP4   AVC 编码
```
**命名陷阱：章节号在前、片号在后。** 按字典序排会把不同视频的章节交错。
MUST 按 **(片号, 章节号)** 二级排序，**MUST NOT** 按文件名或修改时间（拔电池会丢时钟）。
分段阈值长期 4 GB，HERO11/12/13 起提高到 2–12 GB。`.LRV`/`.THM` 在卡上的确切位置**待确认**。

### 录音机
- **Zoom**：`/FOLDER01`–`/FOLDER10`，元数据内嵌 WAV 的 BEXT + iXML，无 sidecar。
- **Tascam**：`/MUSIC`。
- **Sound Devices MixPre**：按日期建文件夹，文件名 `MixPre-001` 递增（跨卡继续递增）。另有 `Settings/`、`UNDO/`、**`TRASH/`（可能有用户误删但还想要的素材，别默认跳过）**。每目录一个 `*_REPORT.CSV` 官方 sound report（**是 .csv，不是 .sdreport**）。
- **SD 838/888**：888 有内置 256 GB SSD + 双 SD 同时录，一次传输可能出现多个卷。

### DJI 无线麦（重点确认项）
**录音存在发射器 TX 上，不在接收器 RX 上。** TX 用 USB-C 单独插电脑，识别为可移动磁盘，录音在根目录 **`DJI_AUDIO`**，WAV 格式。RX 插电脑是当麦克风/声卡用，不是存储。

型号差异**别一刀切**：
- DJI Mic 一代 / Mic 2：TX 8 GB；
- **DJI Mic Mini（原版）：TX 没有内录功能**；
- Mic Mini 2S：TX 14.5 GB；
- **Mic 3：TX 32 GB，支持双文件内录**（原始轨 + 增强轨，一次拍摄产两个 WAV，MUST 识别为一组）+ 内嵌时码。

分段：内录每 30 分钟切一个 WAV；32-bit float 时每 23 分钟。存储写满**覆盖最早的录音**。

UMS 判定的证据链是官方导出流程 + `usbstor.sys` 驱动栈（MTP 不经过它），**建议真机插一次确认盘符**——这是最容易验证的一条。

## 五、需识别为「一组」的分段/续拍文件

| 设备 | 成组规则 |
|---|---|
| GoPro | `GX{章节}{片号}.MP4`，按片号聚合、章节排序 |
| DJI | 4 GB 切段，文件号连续递增（**待真机确认**） |
| Insta360 360 模式 | 同一时间戳的 `_00`/`_10` 双 insv + lrv |
| Nikon N-RAW | 同名 `.NEV` + `.MP4` + `.DAT` |
| DJI Mic | 每 30 min（32-bit float 每 23 min）切一个 WAV |
| DJI Mic 3 双文件模式 | 一次录音产两个 WAV |
| Sony AVCHD | 一条长片跨多个 `.MTS`，靠 `PLAYLIST/*.MPL` 串起来 |
| 相机双卡 Backup | 两张卡同名同内容——是重复不是两条素材 |

## 六、Windows 侧的坑

- **Windows 会写卡**：更新访问时间戳、创建回收站元数据、写 `System Volume Information`、触发 autorun，任何一项都会改变介质哈希。已有真实事故：写入 `System Volume Information` 后卡回到设备报损坏。
  → 应把写保护状态**读出来告诉用户**（**不自作主张改注册表**，那是修改系统设置）；枚举前 `SetErrorMode(SEM_FAILCRITICALERRORS)` 抑制「请插入磁盘」弹窗。
  → **磁盘级写保护对 MTP/PTP 无效**（走的不是磁盘 IOCTL 通道），机身直连连这条防线都没有。
- **SD 卡物理锁**是读卡器遵守的**建议性**标志，不是硬件互锁；拨片磨损松动是「莫名其妙写保护」的高频原因。
- **多卡槽读卡器**每个槽占一个盘符，**空槽也占**。`GetDriveType` 对空槽照样返回 `DRIVE_REMOVABLE`，但打开卷会失败并弹窗。枚举前先抑制错误弹窗，再用 `IOCTL_STORAGE_CHECK_VERIFY` 或 `GetDiskFreeSpaceEx` 探测有无介质。
- **目标盘若是 FAT32**，>4 GB 文件直接拷不进去，MUST 在开拷前检测目标文件系统。
- **HFS+ 的卡/盘在 Windows 上挂不上**，应识别为「有分区但无法挂载」并给人话提示，而不是当成空卡。
- **磁盘管理弹「初始化磁盘为 GPT」时绝不能点**，会毁掉相机分区布局。
- **XQD 时代需装厂商驱动**（Sony XQD Memory Card Driver / ProGrade），SD/CFexpress 读卡器则纯即插即用。
- **无缓冲读的实际收益**：在 USB 可移动介质上吞吐优势基本消失，真正价值是**避免缓存污染**——拷 100 GB 级素材时不把系统内存吃光。这才是开它的理由（校验正确性是另一回事）。

## 七、设备身份识别（跨机器）

- **卷 GUID**：同机最稳，但**换台机器就变**。
- **VSN**（`GetVolumeInformation`）：**相机每次格式化都会变**，且可被工具改写。
- **物理序列号**（`IOCTL_STORAGE_QUERY_PROPERTY`）：**读卡器后面的 SD 卡通常报的是读卡器序列号，不是卡的**——裸卡场景不可用。

→ **复合身份**：`(VSN, 卷标, 文件系统, 总容量, 卡内设备指纹目录)`，卷 GUID 仅作同机内的快速键。

## 八、设备插拔检测

`WM_DEVICECHANGE`(0x0219) + `DBT_DEVICEARRIVAL`(0x8000) / `DBT_DEVICEREMOVECOMPLETE`(0x8004)，
`lParam` 先当 `DEV_BROADCAST_HDR` 看 `dbch_devicetype`，卷类型（`DBT_DEVTYP_VOLUME`=2）转
`DEV_BROADCAST_VOLUME`，从 `dbcv_unitmask` 位图解盘符。

- **卷到达是自动广播给所有顶层窗口的，不需要 `RegisterDeviceNotification`**（设备接口类才需要）。
- `DBT_DEVNODES_CHANGED`(0x0007) 是噪声，枚举期间连发多次，**别当信号**。
- Tauri 里在 `setup()` 拿到 HWND 后用 `SetWindowSubclass` 挂子类化窗口过程。

## 九、查漏补缺（按优先级）

**P0**：多卡槽读卡器 / 多卡并发（中文创作者标配双槽，ShotPut Pro 支持一次并发卸载 5 张卡，是基线不是加分项；调度应按物理设备而非按卡分组）· CFexpress Type A/B 的两个坑 · xxHash64 + MHL 报告 · **「至少两个已验证目标才允许格卡」的硬约束应内建而非只写文档**。

**P1**：SSD 外录设备（Atomos 平铺根目录 + `UNITNAME_S001_S001_T001.mov`；**iPhone 15 Pro 起直录 ProRes 到外置 SSD，盘上就是 `DCIM/100APPLE`、exFAT，插 PC 就是普通卷**——对手机创作者极有价值）· NAS 作为源 · **手机 USB 传输模式的引导层**（做好了比支持 MTP 传输本身更有价值）· iOS 第三方相机 App 素材（Blackmagic Camera / FiLMiC Pro 存在 App 沙盒里，DCIM 完全看不到）。

**P2**：无人机遥控器内存（存的是低质缓存+录屏，不是原片，建议识别到但引导走 SD 卡）· 行车记录仪（VIOFO 结构明确：`DCIM/Movie`、`Movie/RO`、`Movie/Parking`、`DCIM/Photo`；70迈**待查**）· GoPro 完整支持（分章排序陷阱是别人普遍做错的地方，做对了是差异点）。

**P3 明确不做但要有话术**：MTP 通道的「完整备份」承诺 · Fujifilm/Canon/Nikon 机身直连（实测 120 GB 约 4 小时、时间戳可能丢失、大文件不稳，明确不支持 + 引导用读卡器，比做个半残实现更好）· 云端相册作为源。

## 十、待真机确认清单

1. **DJI Mic TX 插上确认盘符**（最容易验证，且判定链路依赖它）
2. DJI 遥控器（RC/RC Pro/RC2）挂载形态：MSC 还是 MTP
3. DJI 4 GB 分段后的命名规律
4. GoPro `.LRV`/`.THM` 在卡上的确切位置
5. Fujifilm 卡内完整目录结构
6. Canon Cinema RAW Light（.crm）目录结构
7. Panasonic GH6/GH7 的 ProRes / CFexpress / USB-SSD 目录结构
8. Sony a7 IV / a7R V / FX3 的 `USB Connection Mode` 出厂默认值（仅 a7S III 有实锤）
