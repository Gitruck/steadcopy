// 中文词典 —— **正本**。键由它推导，`en.ts` 少一条 `tsc --noEmit` 就红。
//
// 规范：openspec/changes/add-steadcopy-i18n/specs/i18n/spec.md
//
// 只放**界面自有**的文案。core 产的成句（编排结论、错误描述、设备类型）
// 由 core 按 locale 给出，这里不重复一份——同一句话两处定义迟早会漂。

export const zh = {
  // 导航与总体
  "nav.workbench": "工位",
  "nav.presets": "预设",
  "nav.devices": "设备",
  "nav.history": "台账",
  "nav.settings": "设置",
  "app.loadingConfig": "正在载入配置…",
  "app.gotIt": "知道了",
  "app.cancel": "取消",
  "app.save": "保存",
  "app.close": "关闭",
  "app.refresh": "刷新",
  "app.delete": "删除",
  "app.edit": "编辑",
  "app.later": "稍后",
  "app.retry": "重试这一块",
  "app.copyDiagnostics": "复制诊断信息",
  "app.blockFailed": "「{where}」这一块出错了，其他页面不受影响。",
  "app.blockFailedHint": "这多半是程序的问题，不是你操作错了。把下面的诊断信息发给我们即可。",

  // 危险区常驻条
  "danger.stripTitle": "⚠ 危险区有开关处于开启状态",
  "danger.stripSkip": " · 插卡不再询问",
  "danger.stripFormat": " · 拷完自动格式化",
  "danger.stripGo": "（点这里去看）",
  "danger.zoneTitle": "⚠ 危险区",
  "danger.zoneHint": "以下开关会造成不可逆后果，默认全部关闭",

  // 工位
  "workbench.currentProject": "当前项目",
  "workbench.noProject": "还没有项目",
  "workbench.enabledPresets": "启用的预设 {n} 条",
  "workbench.running": "拷贝中",
  "workbench.idle": "待机",
  "workbench.needProject": "还没有项目。先去「设置 → 项目」建一个，插卡才知道往哪拷。",
  "workbench.needPreset": "还没有启用的预设任务。去「预设」配一条，插卡才知道该怎么拷；也可以直接「就拷这一次」。",
  "workbench.devices": "设备",
  "workbench.devicesUsable": "{n} 可用",
  "workbench.reading": "正在读取本机卷…",
  "workbench.waitingCard": "等待插入存储卡",
  "workbench.ignoredHint": "　·　有 {n} 个设备被你标记为忽略，插上不会有反应",
  "workbench.howToStart":
    "插卡时会自动弹确认卡片；卡已经插着、或者想再跑一次，直接点「备份这张卡」——两条路走的是同一套编排，结果完全一样。",
  "workbench.backupThis": "备份这张卡",
  "workbench.copyOnce": "就拷这一次…",
  "workbench.eject": "安全弹出",
  "workbench.format": "格式化…",
  "workbench.canBeSource": "可作为源",
  "workbench.inProgress": "进行中",
  "workbench.pause": "暂停",
  "workbench.resume": "继续",
  "workbench.paused": "已暂停",
  "workbench.speedUnknown": "速度：—",
  "workbench.etaUnknown": "剩余：—",
  "workbench.etaAbout": "剩余约 {d}",
  "workbench.ejected": "{name} 已安全弹出，可以拔了",

  // 插卡确认
  "arrival.newDevice": "发现新设备",
  "arrival.classifyHint":
    "指认之前不会往任何地方写入。这一步绕不过去——对不知道是什么的设备自动动手，风险不可接受。",
  "arrival.detected": "检测到存储卡",
  "arrival.viaPreset": "预设「{name}」",
  "arrival.toCopy": "本次待拷 {n} 个 · {size}",
  "arrival.skipped": "，已跳过 {n} 个",
  "arrival.willCopyTo": "将拷贝到：",
  "arrival.start": "开始拷贝",
  "arrival.editThenCopy": "改一下再拷",

  // 结果
  "result.cancelled": "任务已取消。已完成并校验通过的部分不会重复拷贝。",
  "result.partial": "部分失败：成功 {ok} 个，失败 {bad} 个",
  "result.ok": "拷贝完成：{n} 个文件 · {size} · 全部校验通过",
  "result.okSkipped": "（另跳过 {n} 个，此前已拷并校验通过）",
  "result.failedFiles": "失败的文件",
  "result.reason": "原因",
  "result.viewReport": "查看报告",

  // 临时拷贝
  "adhoc.title": "就拷这一次",
  "adhoc.preparing": "正在准备…",
  "adhoc.intro": "这次不写任何预设。拷完之后可以一键把这次的做法记住，也可以什么都不留。",
  "adhoc.destLabel": "拷到哪儿（必选，最多 4 个）",
  "adhoc.destEmpty": "还没选。这是唯一没法替你决定的东西。",
  "adhoc.addDest": "添加目的地…",
  "adhoc.remove": "移除",
  "adhoc.project": "项目",
  "adhoc.newProject": "新建一个…",
  "adhoc.willCreate": "会自动建这个项目。想细配目的地和路径模板，去「设置 → 项目」。",
  "adhoc.verify": "读回校验",
  "adhoc.noVerifyWarn": "关掉校验后，拷进去的东西坏没坏你不会知道。临时拷贝也不例外。",
  "adhoc.next": "下一步",

  // 预设沉淀
  "sink.remember": "记住这个做法",
  "sink.no": "不用",
  "sink.scope": "范围",
  "sink.scopeKind": "同一类设备",
  "sink.scopeAny": "任何已分类的源设备",
  "sink.alsoAs": "顺便记成",
  "sink.done": "记住了。以后这张卡插上会直接弹确认卡片——想改去「预设」页。",
  "sink.askNew": "以后「{device}」插上，就自动拷进「{project}」？",
  "sink.askDiverged": "这次和预设「{preset}」不一样（改了{changed}）。以后都按这次的来？",

  // 设置
  "settings.title": "设置",
  "settings.language": "语言",
  "settings.languageAuto": "跟随系统",
  "settings.copy": "拷贝",
  "settings.verifyDefault": "读回校验",
  "settings.verifyHint": "从目的地无缓冲读回重算哈希再比对。关掉就发现不了介质写入错误",
  "settings.verifyOffWarn": "校验已关闭——拷进去的东西坏没坏，你不会知道。",
  "settings.about": "关于",
  "settings.guide": "上手教程",
  "settings.openGuide": "打开教程",
  "settings.unsigned":
    "本版本未购买代码签名证书，首次运行 Windows 会提示未知发布者，这是预期行为。请核对公示的 SHA-256 校验码确认来源。",
  "settings.neverDisable": "任何时候都不要为了运行本程序去关闭安全软件。",
  "settings.offline":
    "稳拷不联网：没有账号、没有遥测、没有自动更新，也不做后台更新检查。新版本请自行去项目页面看。",
} as const;

export type Key = keyof typeof zh;
