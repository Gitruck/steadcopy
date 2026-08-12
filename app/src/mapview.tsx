import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  api,
  events,
  type Device,
  type MapNode,
  type MapRefreshPreview,
  type MapView,
  type UnlistenFn,
} from "./bridge";
import { t } from "./i18n";

// 「导图」画布。规范：openspec/changes/add-steadcopy-copy-map/specs/copy-map/spec.md
//
// 铁律：**前端零业务逻辑。** 这里没有树状态——每一次增删改名换父连线都发给 core，
// core 校验、落盘、返回整棵新树，画布整体重画（设计 D1）。本文件只有两类代码：
// 布局投影（把树算成坐标）与交互转发（把手势翻译成命令）。
//
// 视觉走自家语言（设计 D3）：正交树自动布局 + 直角折线 + 暗底直角面板。
// 不提供自由摆放：树是要落成真实目录的，层级即语义，画面不许与将要发生的事实脱节。

const NODE_W = 150;
const NODE_H = 34;
const COL = NODE_W + 56;
const ROW = NODE_H + 14;
// 连线色板轮转。取既有语义色变量、刻意没有绿——styles.css 的无绿闸门拦着；
// 颜色也不是唯一载体：每根线的车道口都挂着设备名标签
const LINE_COLORS = ["var(--running)", "var(--warn)", "var(--accent)"];
/** 刷新清单折叠时露几条。5 条够判断「这批是不是我要的」，又不至于占半屏。 */
const REFRESH_PREVIEW_COLLAPSED = 5;

type Pos = { x: number; y: number };
type Notice = { kind: "ok" | "warn"; text: string };
type Line = {
  id: string;
  deviceId: string;
  deviceName: string;
  nodeId: string;
  nodeName: string;
  /** 节点在树里的路径（core 算好下发）——与进度事件的 node_path 同口径，进度锚用 */
  nodePath: string;
  color: string;
  lane: number;
};

// 正交树布局：叶子按 DFS 序纵向排，父节点 y 居中于子 span，x = 深度 × 列宽。
// 这是**投影**不是状态——树本体在 core，改一刀整棵重算
function layoutTree(nodes: MapNode[]): { pos: Map<string, Pos>; rows: number; cols: number } {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const pos = new Map<string, Pos>();
  let row = 0;
  let cols = 1;
  const place = (n: MapNode, depth: number): number => {
    cols = Math.max(cols, depth + 1);
    const kids = n.children
      .map((id) => byId.get(id))
      .filter((k): k is MapNode => k !== undefined);
    let y: number;
    if (kids.length === 0) {
      y = row * ROW;
      row++;
    } else {
      const ys = kids.map((k) => place(k, depth + 1));
      y = (ys[0] + ys[ys.length - 1]) / 2;
    }
    pos.set(n.id, { x: depth * COL, y });
    return y;
  };
  nodes.filter((n) => n.parent === null).forEach((r) => place(r, 0));
  return { pos, rows: row, cols };
}

// 名字太长就截断显示。只影响画布上的字，树里的名字原样在 core
function clip(s: string): string {
  return s.length > 18 ? `${s.slice(0, 17)}…` : s;
}

export function MapPanel({ onError }: { onError: (e: string) => void }) {
  const [view, setView] = useState<MapView | null>(null);
  const [devices, setDevices] = useState<Device[]>([]);
  const [sel, setSel] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [nameVal, setNameVal] = useState("");
  const [adding, setAdding] = useState<{ parentId: string | null } | null>(null);
  // 拖设备时悬停在哪个节点上。与拖节点换父的 dropTarget 分开：
  // 两件事同时只会发生一件，但混用一个状态会让「拖到一半切换来源」留下脏高亮
  const [devOver, setDevOver] = useState<string | null>(null);
  // 设备拖拽是 **pointer 事件手写的**，不用 HTML5 DnD。
  // 真机（WebView2）上后者事件送不进页面：先是 dropEffect 的坑，关掉窗口的
  // 原生拖放拦截（dragDropEnabled=false）之后依然收不到 dragover——
  // 而画布节点的 pointer 拖拽一直好用。同一条已验证的路，设备卡照走。
  const [devDrag, setDevDrag] = useState<{ id: string; name: string; x: number; y: number } | null>(null);
  const [addVal, setAddVal] = useState("");
  const [notices, setNotices] = useState<Notice[]>([]);
  const [refreshList, setRefreshList] = useState<MapRefreshPreview | null>(null);
  // 刷新清单默认折叠：真实盘上动辄几十上百个目录，全铺开会把画布挤出屏幕。
  // 折叠展示前几条 + 总数，看全靠「显示全部」，收回靠「收起」。
  const [refreshExpanded, setRefreshExpanded] = useState(false);
  const [tplSel, setTplSel] = useState("");
  const [savingTpl, setSavingTpl] = useState(false);
  const [tplName, setTplName] = useState("");
  const [runningDev, setRunningDev] = useState<string | null>(null);
  // 在跑任务的节点锚（导图任务才有）。同一张卡连两个节点时，
  // 只有 (设备, 节点路径) 都对上的那根线才算「在跑」——光看设备会两根线一起动
  const [runningPath, setRunningPath] = useState<string | null>(null);
  // 派发请求飞行中：按钮禁用。防的是双击/连点在同一份 running 快照上派两遍
  //（主修在后端的占位提前，这里只是保险带）
  const [dispatching, setDispatching] = useState(false);
  const [pct, setPct] = useState(0);
  // 视口：只存原点与宽度，高度按容器长宽比推导，保证屏幕坐标与画布坐标线性对应
  const [vb, setVb] = useState({ x: -80, y: -100, w: 900 });
  const [aspect, setAspect] = useState(16 / 9);
  const [dragging, setDragging] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);

  const svgRef = useRef<SVGSVGElement | null>(null);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const fitted = useRef(false);
  // Enter/Escape 处理完后输入框随即卸载，卸载又触发一次 blur——
  // 不拦的话 Enter 变成提交两次、Escape 变成「取消却提交」。这个旗标就防这一件事
  const skipBlur = useRef(false);
  const drag = useRef<
    | { kind: "pan"; startX: number; startY: number; ox: number; oy: number; moved: boolean }
    | { kind: "node"; id: string; startX: number; startY: number; moved: boolean }
    | null
  >(null);

  const refreshView = useCallback(() => {
    api.mapGet().then(setView).catch((e) => onError(String(e)));
  }, [onError]);
  useEffect(refreshView, [refreshView]);

  const refreshDevices = useCallback(() => {
    api.listDevices().then(setDevices).catch((e) => onError(String(e)));
  }, [onError]);
  useEffect(() => {
    refreshDevices();
    const iv = setInterval(refreshDevices, 5000);
    return () => clearInterval(iv);
  }, [refreshDevices]);

  // 进行中的任务沿连线显示：任务是串行的，同一时刻至多一个设备在跑。
  // 事件驱动，与「谁发起的」无关——导图派发与其他入口共用同一套事件。
  // 挂载时先取一次进度快照垫底：切走 tab 再切回来，错过的事件补不回来，
  // 没有这一步画布会把正在跑的任务显示成静止（快照只垫底，之后仍由事件驱动）
  useEffect(() => {
    api
      .runningSnapshot()
      .then((list) => {
        // 串行闸下至多一个在跑；快照为空就保持空态
        const s = list[0];
        if (s) {
          setRunningDev(s.device_id);
          setRunningPath(s.node_path);
          setPct(s.percent);
        }
      })
      .catch((e) => onError(String(e)));
  }, [onError]);
  useEffect(() => {
    const un: Promise<UnlistenFn>[] = [
      events.onTaskStarted((p) => {
        setRunningDev(p.device_id);
        setRunningPath(p.node_path);
        setPct(0);
      }),
      events.onProgress((p) => setPct(p.percent)),
      events.onTaskFinished(() => {
        setRunningDev(null);
        setRunningPath(null);
        refreshDevices();
      }),
      events.onTaskFailed(() => {
        setRunningDev(null);
        setRunningPath(null);
      }),
      events.onArrival(() => refreshDevices()),
      events.onRemoved(() => refreshDevices()),
    ];
    return () => {
      un.forEach((p) => p.then((f) => f()).catch(() => {}));
    };
  }, [refreshDevices]);

  const ready = view !== null && view.project_id !== null;
  const laid = useMemo(() => layoutTree(view?.nodes ?? []), [view]);
  const lines: Line[] = useMemo(() => {
    if (!view) return [];
    const out: Line[] = [];
    for (const n of view.nodes) {
      for (const a of n.assignments) {
        out.push({
          id: a.id,
          deviceId: a.device_id,
          deviceName: a.device_name,
          nodeId: n.id,
          nodeName: n.name,
          nodePath: n.path,
          color: LINE_COLORS[out.length % LINE_COLORS.length],
          lane: out.length,
        });
      }
    }
    return out;
  }, [view]);
  const nAssign = lines.length;
  const selNode = view?.nodes.find((n) => n.id === sel) ?? null;
  const usable = devices.filter((d) => d.can_be_source);

  const runningName = useMemo(() => {
    if (!runningDev) return null;
    for (const n of view?.nodes ?? []) {
      const a = n.assignments.find((x) => x.device_id === runningDev);
      if (a) return a.device_name;
    }
    return devices.find((d) => d.id === runningDev)?.name ?? null;
  }, [runningDev, view, devices]);

  // ---- 视口：缩放（滚轮）、平移（拖空白）、居中 ----

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) setAspect(r.width / r.height);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [ready]);

  const fit = useCallback(() => {
    const pad = 40;
    const minX = -100 - pad;
    const minY = (lines.length > 0 ? -26 - lines.length * 16 : -20) - pad;
    const maxX = laid.cols * COL - (COL - NODE_W) + pad;
    const maxY = laid.rows * ROW + pad + 50;
    const w = Math.max(maxX - minX, (maxY - minY) * aspect, 480);
    setVb({ x: minX, y: minY, w });
  }, [laid, lines, aspect]);

  // 首次拿到有内容的树就居中一次；之后交给用户
  useEffect(() => {
    if (!fitted.current && view && view.nodes.length > 0) {
      fitted.current = true;
      fit();
    }
  }, [view, fit]);

  const zoomAt = useCallback((cx: number, cy: number, delta: number) => {
    const el = svgRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setVb((v) => {
      const factor = delta > 0 ? 1.12 : 1 / 1.12;
      const w = Math.min(6000, Math.max(240, v.w * factor));
      const fx = (cx - r.left) / r.width;
      const fy = (cy - r.top) / r.height;
      const ratio = r.height / r.width;
      return { x: v.x + fx * (v.w - w), y: v.y + fy * (v.w - w) * ratio, w };
    });
  }, []);

  // React 把 wheel 注册成 passive，preventDefault 会失效，所以自己挂非 passive 的
  useEffect(() => {
    const el = svgRef.current;
    if (!el) return;
    const h = (e: WheelEvent) => {
      e.preventDefault();
      zoomAt(e.clientX, e.clientY, e.deltaY);
    };
    el.addEventListener("wheel", h, { passive: false });
    return () => el.removeEventListener("wheel", h);
  }, [ready, zoomAt]);

  const toSvg = (cx: number, cy: number): Pos => {
    const el = svgRef.current;
    if (!el) return { x: 0, y: 0 };
    const r = el.getBoundingClientRect();
    return {
      x: vb.x + ((cx - r.left) / r.width) * vb.w,
      y: vb.y + ((cy - r.top) / r.height) * (vb.w / aspect),
    };
  };

  const hitNode = (x: number, y: number, exclude: string): string | null => {
    for (const [id, p] of laid.pos) {
      if (id !== exclude && x >= p.x && x <= p.x + NODE_W && y >= p.y && y <= p.y + NODE_H) {
        return id;
      }
    }
    return null;
  };

  // ---- 指针交互：拖空白平移、拖节点换父 ----

  const onSvgPointerDown = (e: React.PointerEvent) => {
    svgRef.current?.setPointerCapture(e.pointerId);
    drag.current = { kind: "pan", startX: e.clientX, startY: e.clientY, ox: vb.x, oy: vb.y, moved: false };
  };

  const onNodePointerDown = (e: React.PointerEvent, id: string) => {
    e.stopPropagation();
    svgRef.current?.setPointerCapture(e.pointerId);
    drag.current = { kind: "node", id, startX: e.clientX, startY: e.clientY, moved: false };
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dist = Math.abs(e.clientX - d.startX) + Math.abs(e.clientY - d.startY);
    if (d.kind === "pan") {
      if (dist > 3) d.moved = true;
      const el = svgRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      const dx = ((e.clientX - d.startX) / r.width) * vb.w;
      const dy = ((e.clientY - d.startY) / r.height) * (vb.w / aspect);
      setVb({ x: d.ox - dx, y: d.oy - dy, w: vb.w });
    } else {
      if (!d.moved && dist > 6) {
        d.moved = true;
        setDragging(d.id);
      }
      if (d.moved) {
        const p = toSvg(e.clientX, e.clientY);
        setDropTarget(hitNode(p.x, p.y, d.id));
      }
    }
  };

  const onPointerUp = () => {
    const d = drag.current;
    drag.current = null;
    if (!d) return;
    if (d.kind === "pan") {
      // 点了空白且没拖动：取消选中
      if (!d.moved) setSel(null);
      return;
    }
    if (d.moved) {
      const target = dropTarget;
      setDragging(null);
      setDropTarget(null);
      // 环检测在 core：拖进自己后代会被拒并给出双语原因，这里只管转发
      if (target) api.mapMoveNode(d.id, target).then(setView, (e) => onError(String(e)));
    } else {
      setSel(d.id);
    }
  };

  // ---- 节点操作：全部转发给 core ----

  const startAdd = (parentId: string | null) => {
    setRenaming(null);
    setAdding({ parentId });
    setAddVal("");
  };

  const commitAdd = async () => {
    if (!adding) return;
    const name = addVal.trim();
    if (!name) {
      setAdding(null);
      return;
    }
    try {
      const prev = new Set((view?.nodes ?? []).map((n) => n.id));
      const v = await api.mapAddNode(adding.parentId, name);
      setView(v);
      setAdding(null);
      const created = v.nodes.find((n) => !prev.has(n.id));
      if (created) setSel(created.id);
    } catch (e) {
      // 名字被 core 拒了：把原因说出来，输入框留着让用户改
      onError(String(e));
    }
  };

  const startRename = (id: string) => {
    const n = view?.nodes.find((x) => x.id === id);
    if (!n) return;
    setAdding(null);
    setRenaming(id);
    setNameVal(n.name);
  };

  const commitRename = async () => {
    if (!renaming) return;
    const n = view?.nodes.find((x) => x.id === renaming);
    const name = nameVal.trim();
    if (!n || !name || name === n.name) {
      setRenaming(null);
      return;
    }
    try {
      setView(await api.mapRenameNode(renaming, name));
      setRenaming(null);
    } catch (e) {
      onError(String(e));
    }
  };

  const removeNode = (id: string) => {
    const n = view?.nodes.find((x) => x.id === id);
    if (!n) return;
    if (!window.confirm(t("map.deleteConfirm", { name: n.name }))) return;
    api.mapDeleteNode(id).then(
      (v) => {
        setView(v);
        setSel(null);
      },
      (e) => onError(String(e))
    );
  };

  const unassign = (l: Line) => {
    if (!window.confirm(t("map.unassignConfirm", { device: l.deviceName, node: l.nodeName }))) {
      return;
    }
    api.mapUnassign(l.id).then(setView, (e) => onError(String(e)));
  };

  /** 光标下是哪个节点。**纯几何判断**，不碰 DOM 命中测试：
   *  第一版用 elementFromPoint，同一段代码在浏览器里全链路通过、
   *  真机 WebView2 上却打不中——不跟环境差异耗，布局坐标本来就在手里
   *  （laid.pos），getScreenCTM 把光标从视口坐标逆变换进 SVG 用户空间，
   *  跟节点矩形做包含判断即可。缩放平移都在矩阵里，preserveAspectRatio 也是。 */
  const nodeUnderPoint = (cx: number, cy: number): string | null => {
    const svg = svgRef.current;
    const m = svg?.getScreenCTM();
    if (!svg || !m) return null;
    const pt = new DOMPoint(cx, cy).matrixTransform(m.inverse());
    // 命中矩形向外扩一圈：拖拽落点不是外科手术，差几个像素不该白拖一趟。
    // 扩量小于行距/列距的一半，不会把相邻节点纳进来
    const PAD = 10;
    for (const [id, q] of laid.pos) {
      if (
        pt.x >= q.x - PAD && pt.x <= q.x + NODE_W + PAD &&
        pt.y >= q.y - PAD && pt.y <= q.y + NODE_H + PAD
      ) {
        return id;
      }
    }
    return null;
  };

  useEffect(() => {
    if (!devDrag) return;
    const move = (e: PointerEvent) => {
      setDevDrag((d) => (d ? { ...d, x: e.clientX, y: e.clientY } : d));
      setDevOver(nodeUnderPoint(e.clientX, e.clientY));
    };
    const up = (e: PointerEvent) => {
      const target = nodeUnderPoint(e.clientX, e.clientY);
      const id = devDrag.id;
      setDevDrag(null);
      setDevOver(null);
      if (target) api.mapAssign(id, target).then(setView, (x) => onError(String(x)));
    };
    const cancel = () => {
      setDevDrag(null);
      setDevOver(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [devDrag !== null]);

  // ---- 派发 / 刷新 / 模板 ----

  const startAll = async () => {
    if (dispatching) return;
    setDispatching(true);
    try {
      const r = await api.mapDispatch();
      const out: Notice[] = [
        r.started > 0
          ? { kind: "ok", text: t("map.dispatchStarted", { n: r.started }) }
          : { kind: "warn", text: t("map.dispatchNothing") },
      ];
      for (const x of r.rejected) {
        out.push({ kind: "warn", text: t("map.rejectedLine", { device: x.device_name, reason: x.reason }) });
      }
      setNotices(out);
    } catch (e) {
      onError(String(e));
    } finally {
      setDispatching(false);
    }
  };

  const previewRefresh = async () => {
    try {
      const r = await api.mapRefreshPreview();
      if (r.additions.length === 0 && r.skipped.length === 0) {
        setNotices([{ kind: "ok", text: t("map.refreshNothing") }]);
        setRefreshExpanded(false);
        setRefreshList(null);
      } else {
        // 就算一条可并入的都没有，也要把「无法并入」的列出来——
        // 否则用户只看到刷新永远没反应，不知道是哪个目录在挡路
        setRefreshList(r);
      }
    } catch (e) {
      onError(String(e));
    }
  };

  const applyRefresh = async () => {
    if (!refreshList) return;
    try {
      // 预览给用户看的清单原样传回：落地只并「重算 diff ∩ 这份确认集」，
      // 预览之后磁盘新冒出来的目录不会被顺手收编
      setView(await api.mapRefreshApply(refreshList.additions));
      setRefreshList(null);
      setNotices([{ kind: "ok", text: t("map.refreshApplied") }]);
    } catch (e) {
      onError(String(e));
    }
  };

  const clearMap = () => {
    if (!view || view.nodes.length === 0) return;
    if (!window.confirm(t("map.clearMapConfirm"))) return;
    api.mapClear().then(setView, (e) => onError(String(e)));
  };

  const applyTpl = () => {
    const tpl = view?.templates.find((x) => x.id === tplSel);
    if (!tpl) return;
    if (!window.confirm(t("map.templateApplyConfirm", { name: tpl.name }))) return;
    api.mapTemplateApply(tpl.id).then(setView, (e) => onError(String(e)));
  };

  const deleteTpl = () => {
    const tpl = view?.templates.find((x) => x.id === tplSel);
    if (!tpl) return;
    if (!window.confirm(t("map.templateDeleteConfirm", { name: tpl.name }))) return;
    api.mapTemplateDelete(tpl.id).then(
      (v) => {
        setView(v);
        setTplSel("");
      },
      (e) => onError(String(e))
    );
  };

  const saveTpl = () => {
    const name = tplName.trim();
    if (!name) return;
    api.mapTemplateSave(name).then(
      (v) => {
        setView(v);
        setSavingTpl(false);
        setTplName("");
      },
      (e) => onError(String(e))
    );
  };

  // ---- 键盘：Tab 加节点（未选中=顶层、选中=子节点）、F2 改名、Delete 删 ----
  //
  // 监听挂在 window 而不是画布 div 上。原先靠 tabIndex 让画布自己收键，
  // 但点节点时指针捕获在 SVG 上、焦点并不落到画布，Tab 就被浏览器当成
  // 焦点切换吃掉了——「按了没反应」。挂 window 后不依赖焦点；
  // 正在输入框里打字（改名、模板名、别的页面元素）一律不劫持。
  const selRef = useRef(sel);
  const startAddRef = useRef(startAdd);
  const startRenameRef = useRef(startRename);
  const removeNodeRef = useRef(removeNode);
  selRef.current = sel;
  startAddRef.current = startAdd;
  startRenameRef.current = startRename;
  removeNodeRef.current = removeNode;

  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (renaming || adding) return;
      const el = e.target as HTMLElement | null;
      if (el && (el.closest("input, textarea, select") || el.isContentEditable)) return;
      if (e.key === "Tab") {
        e.preventDefault();
        startAddRef.current(selRef.current);
      } else if (e.key === "F2") {
        e.preventDefault();
        if (selRef.current) startRenameRef.current(selRef.current);
      } else if (e.key === "Delete") {
        e.preventDefault();
        if (selRef.current) removeNodeRef.current(selRef.current);
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [renaming, adding]);

  // 幽灵输入框的落点：**新节点将来真正落位的地方**，而不是整棵树底下的空白处。
  //
  // 之前放在画布最底部，跟父节点隔着半个屏幕、没有任何视觉连接——用户点了
  // 「加子节点」，输入框却在右下角凭空冒出来，看不出「这框跟我选的节点有什么关系」。
  // 现在：父节点右一列；父节点没有孩子就跟它同一行（第一个孩子正是落在这），
  // 有孩子就排在最后一个孩子下面。顶层节点则排在整棵树的下方首列。
  // 配套画一条到父节点的虚线预览边（见画布渲染处）。
  const ghostPos = useMemo((): Pos | null => {
    if (!adding) return null;
    if (adding.parentId === null) return { x: 0, y: laid.rows * ROW + 10 };
    const pp = laid.pos.get(adding.parentId);
    if (!pp) return { x: 0, y: laid.rows * ROW + 10 };
    const parent = view?.nodes.find((n) => n.id === adding.parentId);
    const kids = (parent?.children ?? [])
      .map((id) => laid.pos.get(id))
      .filter((q): q is Pos => q !== undefined);
    const y = kids.length === 0 ? pp.y : Math.max(...kids.map((q) => q.y)) + ROW;
    return { x: pp.x + COL, y };
  }, [adding, laid, view]);

  const renamePos = renaming ? laid.pos.get(renaming) : undefined;

  return (
    <>
      <div className="topbar">
        <span className="t">{t("nav.map")}</span>
        {view?.project_name && <span className="muted small">{view.project_name}</span>}
        {runningName && (
          <span className="tag t-run">{t("map.running", { device: runningName })}</span>
        )}
        <div className="r">
          <button className="btn sm" disabled={!ready} onClick={() => startAdd(sel)}>
            {sel ? t("map.addChild") : t("map.addNode")}
          </button>
          <button
            className="btn sm"
            disabled={!selNode || selNode.parent === null}
            onClick={() => sel && api.mapMoveNode(sel, null).then(setView, (e) => onError(String(e)))}
          >
            {t("map.toTop")}
          </button>
          <button className="btn sm" disabled={!ready} onClick={fit}>
            {t("map.center")}
          </button>
          <button className="btn sm" disabled={!ready} onClick={previewRefresh}>
            {t("map.refresh")}
          </button>
          <button
            className="btn sm primary"
            disabled={nAssign === 0 || dispatching}
            title={nAssign === 0 ? t("map.startAllNone") : undefined}
            onClick={startAll}
          >
            {t("map.startAll")}
          </button>
        </div>
      </div>

      <div className="body col">
        {view && !ready && <div className="banner warn">{t("map.needProject")}</div>}
        {notices.length > 0 && (
          <div className="col" style={{ gap: 6 }}>
            {notices.map((n, i) => (
              <div key={i} className={n.kind === "ok" ? "banner ok" : "banner warn"}>
                {n.text}
              </div>
            ))}
            <div>
              <button className="btn sm" onClick={() => setNotices([])}>
                {t("app.gotIt")}
              </button>
            </div>
          </div>
        )}

        {ready && (
          <>
            <div className="panel">
              <div className="in row map-tools" style={{ alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                <span className="small muted">{t("map.templates")}</span>
                <select value={tplSel} onChange={(e) => setTplSel(e.target.value)}>
                  <option value="">
                    {view.templates.length === 0 ? t("map.templateNone") : "—"}
                  </option>
                  {view.templates.map((x) => (
                    <option key={x.id} value={x.id}>
                      {x.name}
                    </option>
                  ))}
                </select>
                <button className="btn sm" disabled={!tplSel} onClick={applyTpl}>
                  {t("map.templateApply")}
                </button>
                <button className="btn sm danger" disabled={!tplSel} onClick={deleteTpl}>
                  {t("map.templateDelete")}
                </button>
                {savingTpl ? (
                  <>
                    <input
                      autoFocus
                      placeholder={t("map.templateName")}
                      value={tplName}
                      onChange={(e) => setTplName(e.target.value)}
                      onKeyDown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Enter") saveTpl();
                        else if (e.key === "Escape") setSavingTpl(false);
                      }}
                    />
                    <button className="btn sm primary" disabled={!tplName.trim()} onClick={saveTpl}>
                      {t("map.templateSave")}
                    </button>
                    <button className="btn sm" onClick={() => setSavingTpl(false)}>
                      {t("app.cancel")}
                    </button>
                  </>
                ) : (
                  <button
                    className="btn sm"
                    disabled={view.nodes.length === 0}
                    onClick={() => setSavingTpl(true)}
                  >
                    {t("map.templateSaveAs")}
                  </button>
                )}
                <button
                  className="btn sm"
                  disabled={view.nodes.length === 0}
                  onClick={clearMap}
                >
                  {t("map.clearMap")}
                </button>
                {nAssign === 0 && <span className="small dim">{t("map.startAllNone")}</span>}
                <div className="grow" />
                <span className="small dim">{t("map.hintKeys")}</span>
              </div>
              {/* 「节点 = 目的地根目录下的真实文件夹」必须常驻可见：
                  不亮出根目录，这棵树就悬在半空，刷新清单也像无源之水 */}
              <div className="in row map-landing" style={{ gap: 8, flexWrap: "wrap" }}>
                {view.destinations.length > 0 && (
                  <span className="small muted mono">
                    {t("map.landing", { root: view.destinations[0] })}
                    {view.destinations.length > 1 &&
                      " " + t("map.landingMore", { n: String(view.destinations.length - 1) })}
                  </span>
                )}
                {selNode && view.destinations.length > 0 && (
                  <span className="small mono" style={{ color: "var(--running)" }}>
                    {t("map.selPath", {
                      path: view.destinations[0].replace(/[\\/]+$/, "") + "\\" + selNode.path.replace(/\//g, "\\"),
                    })}
                  </span>
                )}
              </div>
            </div>

            {refreshList && (
              <div className="panel">
                <header>
                  {t("map.refreshTitle")}
                  <span className="n">{refreshList.additions.length}</span>
                </header>
                <div className="in col" style={{ gap: 6 }}>
                  <div className="small muted">
                    {t("map.refreshHint", { root: view.destinations[0] ?? "—" })}
                  </div>
                  {(refreshExpanded
                    ? refreshList.additions
                    : refreshList.additions.slice(0, REFRESH_PREVIEW_COLLAPSED)
                  ).map((p) => (
                    <div key={p} className="path">
                      {p}
                    </div>
                  ))}
                  {refreshList.additions.length > REFRESH_PREVIEW_COLLAPSED &&
                    (refreshExpanded ? (
                      <div>
                        <button className="btn sm" onClick={() => setRefreshExpanded(false)}>
                          {t("map.refreshCollapse")}
                        </button>
                      </div>
                    ) : (
                      <div>
                        <button className="btn sm" onClick={() => setRefreshExpanded(true)}>
                          {t("map.refreshShowAll", { n: String(refreshList.additions.length) })}
                        </button>
                      </div>
                    ))}
                  {refreshList.skipped.length > 0 && (
                    <>
                      <div className="small dim">{t("map.refreshSkipped")}</div>
                      {refreshList.skipped.map((s) => (
                        <div key={s.path} className="path dim">
                          {t("map.refreshSkippedLine", { path: s.path, reason: s.reason })}
                        </div>
                      ))}
                    </>
                  )}
                  <div className="row" style={{ gap: 8 }}>
                    {refreshList.additions.length > 0 && (
                      <button className="btn sm primary" onClick={applyRefresh}>
                        {t("map.refreshApply")}
                      </button>
                    )}
                    <button
                      className="btn sm"
                      onClick={() => {
                        setRefreshList(null);
                        setRefreshExpanded(false);
                      }}
                    >
                      {t("app.cancel")}
                    </button>
                  </div>
                </div>
              </div>
            )}

            <div className="map-wrap">
              <div className="map-src">
                <div className="small muted map-src-h">{t("map.sources")}</div>
                <div className="small dim map-src-hint">{t("map.sourcesHint")}</div>
                {usable.length === 0 && <div className="empty">{t("map.sourcesEmpty")}</div>}
                {usable.map((d) => (
                  <div
                    key={d.id}
                    className="dev usable map-devcard"
                    onPointerDown={(e) => {
                      // 只认主键；起手即进入拖拽态，拖影跟着光标走
                      if (e.button !== 0) return;
                      e.preventDefault();
                      setDevDrag({ id: d.id, name: d.name, x: e.clientX, y: e.clientY });
                    }}
                  >
                    <div className="hd">
                      <span className="nm">{d.name}</span>
                      {d.kind_label && <span className="tag t-neutral">{d.kind_label}</span>}
                    </div>
                    <div className="meta">{d.root}</div>
                  </div>
                ))}
              </div>

              {devDrag &&
                createPortal(
                  <div
                    className="map-dragghost"
                    style={{ left: devDrag.x, top: devDrag.y, transform: "translate(-50%, -50%)" }}
                  >
                    {devDrag.name}
                  </div>,
                  document.body
                )}
              <div className="map-canvas" ref={wrapRef}>
                <svg
                  ref={svgRef}
                  viewBox={`${vb.x} ${vb.y} ${vb.w} ${vb.w / aspect}`}
                  preserveAspectRatio="xMidYMid meet"
                  onPointerDown={onSvgPointerDown}
                  onPointerMove={onPointerMove}
                  onPointerUp={onPointerUp}
                >
                  {view.nodes.map((n) => {
                    if (!n.parent) return null;
                    const pc = laid.pos.get(n.id);
                    const pp = laid.pos.get(n.parent);
                    if (!pc || !pp) return null;
                    const midX = pc.x - (COL - NODE_W) / 2;
                    return (
                      <path
                        key={`e-${n.id}`}
                        className="map-edge"
                        d={`M ${pp.x + NODE_W} ${pp.y + NODE_H / 2} H ${midX} V ${pc.y + NODE_H / 2} H ${pc.x}`}
                      />
                    );
                  })}

                  {lines.map((l) => {
                    const p = laid.pos.get(l.nodeId);
                    if (!p) return null;
                    const laneY = -26 - l.lane * 16;
                    const startX = -96;
                    const descX = p.x - 10 - l.lane * 5;
                    const yC = p.y + NODE_H / 2;
                    // 有节点锚（导图任务）时按 (设备, 节点路径) 匹配——同卡连两个节点
                    // 只亮真正在跑的那根；没有锚的事件（工位任务）退回按设备匹配
                    const running =
                      runningDev !== null &&
                      l.deviceId === runningDev &&
                      (runningPath === null || l.nodePath === runningPath);
                    return (
                      <g key={l.id} className={running ? "map-line-g running" : "map-line-g"}>
                        <path
                          className="map-line"
                          style={{ stroke: l.color }}
                          d={`M ${startX} ${laneY} H ${descX} V ${yC} H ${p.x}`}
                        />
                        <text
                          className="map-line-label"
                          style={{ fill: l.color }}
                          x={startX}
                          y={laneY - 4}
                          onClick={() => unassign(l)}
                        >
                          {running ? `${l.deviceName} · ${Math.round(pct)}%` : l.deviceName}
                        </text>
                      </g>
                    );
                  })}

                  {view.nodes.map((n) => {
                    const p = laid.pos.get(n.id);
                    if (!p) return null;
                    const cls = [
                      "map-node",
                      sel === n.id ? "sel" : "",
                      dropTarget === n.id || devOver === n.id ? "drop" : "",
                      dragging === n.id ? "lift" : "",
                    ]
                      .filter(Boolean)
                      .join(" ");
                    return (
                      <g
                        key={n.id}
                        className={cls}
                        data-node-id={n.id}
                        transform={`translate(${p.x},${p.y})`}
                        onPointerDown={(e) => onNodePointerDown(e, n.id)}
                        onDoubleClick={() => startRename(n.id)}
                      >
                        <rect className="box" width={NODE_W} height={NODE_H} />
                        <rect className="mark" width={3} height={NODE_H} />
                        <text className="nm" x={12} y={NODE_H / 2 + 4}>
                          {clip(n.name)}
                        </text>
                        {n.assignments.length > 0 && (
                          <g className="cnt">
                            <circle cx={NODE_W - 13} cy={NODE_H / 2} r={8} />
                            <text x={NODE_W - 13} y={NODE_H / 2 + 3.5} textAnchor="middle">
                              {n.assignments.length}
                            </text>
                          </g>
                        )}
                      </g>
                    );
                  })}

                  {adding && ghostPos && adding.parentId && laid.pos.get(adding.parentId) && (
                    <path
                      className="map-edge ghost"
                      d={(() => {
                        const pp = laid.pos.get(adding.parentId)!;
                        const midX = ghostPos.x - (COL - NODE_W) / 2;
                        return `M ${pp.x + NODE_W} ${pp.y + NODE_H / 2} H ${midX} V ${ghostPos.y + NODE_H / 2} H ${ghostPos.x}`;
                      })()}
                    />
                  )}
                  {adding && ghostPos && (
                    <foreignObject x={ghostPos.x} y={ghostPos.y} width={NODE_W + 90} height={NODE_H + 6}>
                      <input
                        className="map-inline-input"
                        autoFocus
                        value={addVal}
                        placeholder={t("map.newNamePlaceholder")}
                        onChange={(e) => setAddVal(e.target.value)}
                        onKeyDown={(e) => {
                          e.stopPropagation();
                          if (e.key === "Enter") {
                            skipBlur.current = true;
                            commitAdd();
                          } else if (e.key === "Escape") {
                            skipBlur.current = true;
                            setAdding(null);
                          }
                        }}
                        onBlur={() => {
                          if (skipBlur.current) {
                            skipBlur.current = false;
                            return;
                          }
                          commitAdd();
                        }}
                      />
                    </foreignObject>
                  )}

                  {renaming && renamePos && (
                    <foreignObject x={renamePos.x} y={renamePos.y} width={NODE_W + 90} height={NODE_H + 6}>
                      <input
                        className="map-inline-input"
                        autoFocus
                        value={nameVal}
                        onChange={(e) => setNameVal(e.target.value)}
                        onKeyDown={(e) => {
                          e.stopPropagation();
                          if (e.key === "Enter") {
                            skipBlur.current = true;
                            commitRename();
                          } else if (e.key === "Escape") {
                            skipBlur.current = true;
                            setRenaming(null);
                          }
                        }}
                        onBlur={() => {
                          if (skipBlur.current) {
                            skipBlur.current = false;
                            return;
                          }
                          commitRename();
                        }}
                      />
                    </foreignObject>
                  )}
                </svg>

                {view.nodes.length === 0 && !adding && (
                  <div className="map-empty">
                    <b>{t("map.emptyTitle")}</b>
                    <p>{t("map.emptyWhat")}</p>
                    <p className="dim">{t("map.emptyHow")}</p>
                    <div>
                      <button className="btn primary" onClick={() => startAdd(null)}>
                        {t("map.addFirst")}
                      </button>
                    </div>
                  </div>
                )}
              </div>
            </div>
          </>
        )}
      </div>
    </>
  );
}
