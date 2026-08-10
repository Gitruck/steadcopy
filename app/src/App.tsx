import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  api,
  bytes,
  events,
  type AuditResult,
  type Device,
  type HistoryItem,
  type PlanView,
  type Progress,
  type RunView,
  type ScanView,
  type TaskInput,
  type UnlistenFn,
} from "./bridge";

type Tab = "workbench" | "history";

export default function App() {
  const [tab, setTab] = useState<Tab>("workbench");
  const [version, setVersion] = useState("");
  const [dests, setDests] = useState<string[]>([]);

  useEffect(() => {
    api.appVersion().then(setVersion).catch(() => {});
  }, []);

  return (
    <div className="shell">
      <nav className="nav">
        <div className="brand">
          稳拷 <small>steadcopy</small>
        </div>
        <button className={tab === "workbench" ? "on" : ""} onClick={() => setTab("workbench")}>
          工位
        </button>
        <button className={tab === "history" ? "on" : ""} onClick={() => setTab("history")}>
          台账
        </button>
        <div className="spacer" />
        <div className="ver">v{version || "…"}</div>
      </nav>
      <div className="main">
        {tab === "workbench" ? (
          <Workbench dests={dests} setDests={setDests} />
        ) : (
          <History dests={dests} />
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- 工位

function Workbench({
  dests,
  setDests,
}: {
  dests: string[];
  setDests: (d: string[]) => void;
}) {
  const [devices, setDevices] = useState<Device[]>([]);
  const [source, setSource] = useState("");
  const [project, setProject] = useState("未命名项目");
  const [deviceName, setDeviceName] = useState("存储卡");
  const [verify, setVerify] = useState(true);
  const [scanned, setScanned] = useState<ScanView | null>(null);
  const [plan, setPlan] = useState<PlanView | null>(null);
  const [running, setRunning] = useState(false);
  const [copy, setCopy] = useState<Progress | null>(null);
  const [verifyP, setVerifyP] = useState<Progress | null>(null);
  const [stage, setStage] = useState("");
  const [result, setResult] = useState<RunView | null>(null);
  const [error, setError] = useState("");
  const [notices, setNotices] = useState<string[]>([]);
  const [viewing, setViewing] = useState<string | null>(null);

  const input: TaskInput = useMemo(
    () => ({
      source,
      destinations: dests,
      project,
      device_name: deviceName,
      template: "{项目}/{日期}/{设备}",
      verify,
      algorithm: "xxh64",
    }),
    [source, dests, project, deviceName, verify]
  );

  const refresh = useCallback(() => {
    api.listDevices().then(setDevices).catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 4000);
    return () => clearInterval(t);
  }, [refresh]);

  useEffect(() => {
    const un: Promise<UnlistenFn>[] = [
      events.onStage((p) => setStage(p.stage)),
      events.onProgress((p) => (p.stage === "校验" ? setVerifyP(p) : setCopy(p))),
      events.onNotice((m) => setNotices((n) => [...n, m])),
      events.onFileFailed((f) => setNotices((n) => [...n, `${f.path}：${f.reason}`])),
    ];
    return () => {
      un.forEach((p) => p.then((f) => f()).catch(() => {}));
    };
  }, []);

  async function pickSource() {
    const p = await openDialog({ directory: true, title: "选择源（读卡器盘符或任意目录）" });
    if (typeof p === "string") {
      setSource(p);
      setResult(null);
      setPlan(null);
      try {
        setScanned(await api.scan(p));
        setError("");
      } catch (e) {
        setScanned(null);
        setError(String(e));
      }
    }
  }

  async function addDest() {
    const p = await openDialog({ directory: true, title: "选择目的地" });
    if (typeof p === "string" && !dests.includes(p) && dests.length < 4) {
      setDests([...dests, p]);
      setPlan(null);
    }
  }

  async function doPlan() {
    setError("");
    try {
      setPlan(await api.plan(input));
    } catch (e) {
      setPlan(null);
      setError(String(e));
    }
  }

  async function doCopy() {
    setError("");
    setNotices([]);
    setResult(null);
    setCopy(null);
    setVerifyP(null);
    setRunning(true);
    try {
      setResult(await api.startCopy(input));
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
      setStage("");
    }
  }

  const ready = source && dests.length > 0 && !running;
  const insufficient = plan?.destinations.some((d) => d.sufficient === false);

  return (
    <>
      <div className="topbar">
        <span className="t">工位</span>
        <span className="muted small">
          {source ? "已选源" : "未选源"} · 目的地 {dests.length}/4
        </span>
        <div className="r">
          <label className="small" style={{ display: "flex", gap: 5, alignItems: "center" }}>
            <input
              type="checkbox"
              checked={verify}
              disabled={running}
              onChange={(e) => setVerify(e.target.checked)}
            />
            读回校验
          </label>
          {!verify && <span className="tag t-warn">校验已关闭</span>}
        </div>
      </div>

      <div className="body col">
        {error && <div className="banner bad">{error}</div>}

        <div className="row">
          {/* 设备 */}
          <div className="panel" style={{ width: 340, flex: "0 0 340px" }}>
            <header>
              设备<span className="n">{devices.filter((d) => d.can_be_source).length} 可用</span>
            </header>
            <div className="in">
              {devices.length === 0 && <div className="empty">正在读取本机卷…</div>}
              {devices.map((d) => (
                <div key={d.id} className={`dev${d.can_be_source ? " usable" : ""}`}>
                  <div className="hd">
                    <span className="nm">{d.name}</span>
                    {d.can_be_source ? (
                      <span className="tag t-ok">可作为源</span>
                    ) : d.is_system ? (
                      <span className="tag t-neutral">系统盘</span>
                    ) : (
                      <span className="tag t-neutral">{d.bus}</span>
                    )}
                  </div>
                  <div className="meta">
                    {d.file_system || "—"} · {d.bus} · 剩余 {bytes(d.free_bytes)} /{" "}
                    {bytes(d.total_bytes)}
                    {d.fingerprints.length > 0 && ` · ${d.fingerprints.join("、")}`}
                  </div>
                  <div className="bar">
                    <i
                      style={{
                        width: `${
                          d.total_bytes ? ((d.total_bytes - d.free_bytes) / d.total_bytes) * 100 : 0
                        }%`,
                      }}
                    />
                  </div>
                  {d.can_be_source && (
                    <div style={{ marginTop: 8 }}>
                      <button
                        className="btn sm"
                        disabled={running}
                        onClick={async () => {
                          setSource(d.root);
                          setDeviceName(d.name.replace(/\s*\(.*\)$/, ""));
                          setResult(null);
                          setPlan(null);
                          try {
                            setScanned(await api.scan(d.root));
                          } catch (e) {
                            setError(String(e));
                          }
                        }}
                      >
                        用它作为源
                      </button>
                    </div>
                  )}
                </div>
              ))}
              <div style={{ marginTop: 10 }}>
                <button className="btn sm" onClick={refresh}>
                  刷新
                </button>
              </div>
            </div>
          </div>

          {/* 任务 */}
          <div className="panel grow">
            <header>本次任务</header>
            <div className="in col">
              <div className="row">
                <div className="field grow">
                  <label>源</label>
                  <div className="row" style={{ gap: 6 }}>
                    <input className="grow" readOnly value={source} placeholder="未选择" />
                    <button className="btn" disabled={running} onClick={pickSource}>
                      选择…
                    </button>
                  </div>
                </div>
              </div>

              {scanned && (
                <div className="small muted">
                  扫描到 <b className="mono">{scanned.files}</b> 个文件 ·{" "}
                  <b className="mono">{bytes(scanned.total_bytes)}</b>
                  {scanned.junk_excluded > 0 && ` · 已排除系统垃圾 ${scanned.junk_excluded} 个`}
                  {scanned.fingerprints.length > 0 && ` · ${scanned.fingerprints.join("、")}`}
                  <div className="dim" style={{ marginTop: 2 }}>
                    {scanned.categories.map(([k, n, b]) => `${k} ${n} 个 / ${bytes(b)}`).join("　")}
                  </div>
                </div>
              )}

              <div className="row">
                <div className="field grow">
                  <label>项目</label>
                  <input
                    value={project}
                    disabled={running}
                    onChange={(e) => {
                      setProject(e.target.value);
                      setPlan(null);
                    }}
                  />
                </div>
                <div className="field grow">
                  <label>设备名（进落地路径）</label>
                  <input
                    value={deviceName}
                    disabled={running}
                    onChange={(e) => {
                      setDeviceName(e.target.value);
                      setPlan(null);
                    }}
                  />
                </div>
              </div>

              <div className="field">
                <label>目的地（1–4 个，一次读源同时写入）</label>
                {dests.map((d, i) => (
                  <div key={d} className="row" style={{ gap: 6, alignItems: "center" }}>
                    <span className="path grow">
                      {i + 1}. {d}
                    </span>
                    <button
                      className="btn sm danger"
                      disabled={running}
                      onClick={() => {
                        setDests(dests.filter((x) => x !== d));
                        setPlan(null);
                      }}
                    >
                      移除
                    </button>
                  </div>
                ))}
                <div>
                  <button className="btn sm" disabled={running || dests.length >= 4} onClick={addDest}>
                    添加目的地…
                  </button>
                </div>
              </div>

              {plan && (
                <div className="col" style={{ gap: 6 }}>
                  {plan.no_source && <div className="banner warn">源上没有可拷贝的素材</div>}
                  {plan.no_new_source && (
                    <div className="banner ok">没有新素材，本次无需拷贝（此前已拷并校验通过）</div>
                  )}
                  {plan.notices.map((n) => (
                    <div key={n} className="banner warn">
                      历史清单不可读（{n}），本次将执行全量拷贝
                    </div>
                  ))}
                  {!plan.no_source && !plan.no_new_source && (
                    <div className="small">
                      本次待拷 <b className="mono">{plan.to_copy}</b> 个 ·{" "}
                      <b className="mono">{bytes(plan.to_copy_bytes)}</b>
                      {plan.skipped > 0 && `，已跳过 ${plan.skipped} 个`}
                    </div>
                  )}
                  {plan.destinations.map((d) => (
                    <div key={d.landing_dir} className="small">
                      <div className="path">{d.landing_dir}</div>
                      <span className="muted">
                        需要 {bytes(d.required_bytes)} · 可用{" "}
                        {d.available_bytes === null ? "未知" : bytes(d.available_bytes)}{" "}
                      </span>
                      {d.sufficient === true && <span className="tag t-ok">空间充足</span>}
                      {d.sufficient === false && <span className="tag t-bad">空间不足</span>}
                      {d.sufficient === null && <span className="tag t-warn">空间无法确认</span>}
                    </div>
                  ))}
                </div>
              )}

              <div className="row" style={{ gap: 8 }}>
                <button className="btn" disabled={!ready} onClick={doPlan}>
                  预演（不写入）
                </button>
                <button
                  className="btn primary"
                  disabled={!ready || insufficient || plan?.no_source}
                  onClick={doCopy}
                >
                  开始拷贝
                </button>
                {running && (
                  <button className="btn danger" onClick={() => api.cancelCopy()}>
                    取消
                  </button>
                )}
              </div>

              {running && (
                <div className="col" style={{ gap: 8 }}>
                  <div className="small muted">阶段：{stage || "准备中"}</div>
                  <TwoBars copy={copy} verify={verifyP} showVerify={verify} />
                </div>
              )}

              {notices.length > 0 && (
                <div className="banner warn">
                  {notices.map((n, i) => (
                    <div key={i}>{n}</div>
                  ))}
                </div>
              )}

              {result && <Result r={result} onView={setViewing} />}
            </div>
          </div>
        </div>
      </div>

      {viewing && <ReportViewer manifestPath={viewing} onClose={() => setViewing(null)} />}
    </>
  );
}

/** 两条**分别标注**的进度条。Gate 的双进度条没标注，用户只能猜——这是明确要改进的点。 */
function TwoBars({
  copy,
  verify,
  showVerify,
}: {
  copy: Progress | null;
  verify: Progress | null;
  showVerify: boolean;
}) {
  return (
    <>
      <div className="prog copy">
        <div className="lbl">
          <span>拷贝</span>
          <span className="mono">{(copy?.percent ?? 0).toFixed(1)}%</span>
        </div>
        <div className="track">
          <i style={{ width: `${copy?.percent ?? 0}%` }} />
        </div>
        {copy?.current && <div className="path" style={{ marginTop: 3 }}>{copy.current}</div>}
      </div>
      {showVerify && (
        <div className="prog verify">
          <div className="lbl">
            <span>校验</span>
            <span className="mono">{(verify?.percent ?? 0).toFixed(1)}%</span>
          </div>
          <div className="track">
            <i style={{ width: `${verify?.percent ?? 0}%` }} />
          </div>
        </div>
      )}
    </>
  );
}

/** 结果如实呈现：有失败时 MUST NOT 说「完成」。 */
function Result({ r, onView }: { r: RunView; onView: (p: string) => void }) {
  return (
    <div className="col" style={{ gap: 8 }}>
      {r.cancelled ? (
        <div className="banner warn">任务已取消。已完成并校验通过的部分不会重复拷贝。</div>
      ) : r.failed > 0 ? (
        <div className="banner bad">
          部分失败：成功 {r.copied} 个，失败 {r.failed} 个
        </div>
      ) : (
        <div className="banner ok">
          拷贝完成：{r.copied} 个文件 · {bytes(r.bytes_copied)} · 全部校验通过
          {r.skipped > 0 && `（另跳过 ${r.skipped} 个，此前已拷并校验通过）`}
        </div>
      )}
      {r.failures.length > 0 && (
        <table>
          <thead>
            <tr>
              <th>失败的文件</th>
              <th>原因</th>
            </tr>
          </thead>
          <tbody>
            {r.failures.map((f) => (
              <tr key={f.path}>
                <td>{f.path}</td>
                <td className="muted">{f.reason}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {r.manifests.map((m) => (
        <div key={m} className="row" style={{ alignItems: "center", gap: 8 }}>
          <span className="path grow">{m}</span>
          <button className="btn sm primary" onClick={() => onView(m)}>
            查看报告
          </button>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------- 台账

function History({ dests }: { dests: string[] }) {
  const [items, setItems] = useState<HistoryItem[]>([]);
  const [roots, setRoots] = useState<string[]>(dests);
  const [viewing, setViewing] = useState<string | null>(null);
  const [audit, setAudit] = useState<{ path: string; r: AuditResult } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const load = useCallback(async (rs: string[]) => {
    if (rs.length === 0) return setItems([]);
    try {
      setItems(await api.listHistory(rs));
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    setRoots((r) => Array.from(new Set([...r, ...dests])));
  }, [dests]);

  useEffect(() => {
    load(roots);
  }, [roots, load]);

  async function addRoot() {
    const p = await openDialog({ directory: true, title: "选择要扫描历史的目录" });
    if (typeof p === "string" && !roots.includes(p)) setRoots([...roots, p]);
  }

  async function doAudit(path: string) {
    setBusy(true);
    setError("");
    try {
      setAudit({ path, r: await api.runAudit(path) });
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <div className="topbar">
        <span className="t">台账</span>
        <span className="muted small">{items.length} 条记录</span>
        <div className="r">
          <button className="btn sm" onClick={addRoot}>
            添加扫描目录…
          </button>
          <button className="btn sm" onClick={() => load(roots)}>
            刷新
          </button>
        </div>
      </div>
      <div className="body col">
        {error && <div className="banner bad">{error}</div>}
        {roots.length === 0 && (
          <div className="empty">
            还没有可扫描的目录。添加一个目的地目录，这里会列出它下面的全部拷卡记录。
          </div>
        )}
        {roots.length > 0 && items.length === 0 && (
          <div className="empty">这些目录下还没有拷卡记录。</div>
        )}
        {items.length > 0 && (
          <div className="panel">
            <header>拷卡记录<span className="n">{items.length}</span></header>
            <table>
              <thead>
                <tr>
                  <th>时间</th>
                  <th>项目</th>
                  <th>来源</th>
                  <th className="num">文件</th>
                  <th className="num">大小</th>
                  <th>校验</th>
                  <th style={{ width: 190 }} />
                </tr>
              </thead>
              <tbody>
                {items.map((it) => (
                  <tr key={it.manifest_path}>
                    <td className="mono">{it.created_at}</td>
                    <td>{it.project}</td>
                    <td>{it.device}</td>
                    <td className="num">{it.files}</td>
                    <td className="num">{bytes(it.total_bytes)}</td>
                    <td>
                      {it.verified === it.files && it.files > 0 ? (
                        <span className="tag t-ok">全部已校验</span>
                      ) : it.verified === 0 ? (
                        <span className="tag t-warn">未校验</span>
                      ) : (
                        <span className="tag t-warn">
                          {it.verified}/{it.files} 已校验
                        </span>
                      )}
                    </td>
                    <td>
                      <div className="row" style={{ gap: 6 }}>
                        <button className="btn sm primary" onClick={() => setViewing(it.manifest_path)}>
                          报告
                        </button>
                        <button className="btn sm" disabled={busy} onClick={() => doAudit(it.manifest_path)}>
                          复验
                        </button>
                        <button
                          className="btn sm"
                          onClick={() => openPath(it.landing_dir).catch((e) => setError(String(e)))}
                        >
                          打开
                        </button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {busy && <div className="banner warn">正在复验（无缓冲读回，大文件会慢一些）…</div>}
        {audit && <AuditPanel r={audit.r} onClose={() => setAudit(null)} />}
      </div>

      {viewing && <ReportViewer manifestPath={viewing} onClose={() => setViewing(null)} />}
    </>
  );
}

/** 复验四态并列呈现——**不**压缩成一个布尔结论。 */
function AuditPanel({ r, onClose }: { r: AuditResult; onClose: () => void }) {
  const intact = r.intact.length;
  const moved = r.moved.length;
  const missing = r.missing.length;
  const added = r.added.length;
  return (
    <div className="panel">
      <header>
        复验结果<span className="n">算法 {r.algorithm}</span>
        <button className="btn sm" style={{ marginLeft: 8 }} onClick={onClose}>
          关闭
        </button>
      </header>
      <div className="in col">
        {missing === 0 ? (
          <div className="banner ok">数据完好——清单记录的内容全部找得到</div>
        ) : (
          <div className="banner bad">有 {missing} 个文件丢失</div>
        )}
        {!r.complete && <div className="banner warn">复验被中断，结果不完整</div>}
        {r.unverified_at_copy > 0 && (
          <div className="banner warn">
            其中 {r.unverified_at_copy} 个条目在拷贝时未做校验，可信度较低
          </div>
        )}
        <div className="row" style={{ gap: 16 }}>
          <span>
            <span className="tag t-ok">一致</span> <b className="mono">{intact}</b>
          </span>
          <span>
            <span className="tag t-warn">已移动</span> <b className="mono">{moved}</b>
          </span>
          <span>
            <span className="tag t-bad">丢失</span> <b className="mono">{missing}</b>
          </span>
          <span>
            <span className="tag t-warn">新增</span> <b className="mono">{added}</b>
          </span>
        </div>
        {missing > 0 && (
          <table>
            <thead>
              <tr>
                <th>丢失的文件</th>
                <th className="num">大小</th>
                <th>期望校验值</th>
              </tr>
            </thead>
            <tbody>
              {r.missing.map((m) => (
                <tr key={m.relative_path}>
                  <td>{m.relative_path}</td>
                  <td className="num">{bytes(m.size)}</td>
                  <td className="mono">{m.expected_hash}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {moved > 0 && (
          <table>
            <thead>
              <tr>
                <th>已移动</th>
                <th>现在的位置</th>
              </tr>
            </thead>
            <tbody>
              {r.moved.map((m) => (
                <tr key={m.from}>
                  <td>{m.from}</td>
                  <td className="muted">{m.to}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {added > 0 && (
          <div className="small muted">
            另有 {added} 个清单未记录的文件（多出文件本身不是错误，仅作告知）
          </div>
        )}
      </div>
    </div>
  );
}

/** **应用内报告查看器**：报告 HTML 全文塞进沙箱 iframe 渲染，不跳浏览器。 */
function ReportViewer({ manifestPath, onClose }: { manifestPath: string; onClose: () => void }) {
  const [html, setHtml] = useState<string | null>(null);
  const [error, setError] = useState("");
  const ref = useRef<HTMLIFrameElement>(null);

  useEffect(() => {
    api.reportHtml(manifestPath).then(setHtml).catch((e) => setError(String(e)));
  }, [manifestPath]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="viewer" onClick={onClose}>
      <div className="sheet" onClick={(e) => e.stopPropagation()}>
        <header>
          <span className="t">拷卡报告</span>
          <span className="path">{manifestPath.replace(/\.json$/, ".html")}</span>
          <div className="r">
            <button className="btn sm" onClick={() => ref.current?.contentWindow?.print()}>
              打印 / 存为 PDF
            </button>
            <button
              className="btn sm"
              onClick={() => openPath(manifestPath.replace(/\.json$/, ".html")).catch(() => {})}
            >
              在浏览器中打开
            </button>
            <button className="btn sm" onClick={onClose}>
              关闭
            </button>
          </div>
        </header>
        {error ? (
          <div className="banner bad" style={{ margin: 12 }}>
            {error}
          </div>
        ) : html === null ? (
          <div className="empty">正在载入报告…</div>
        ) : (
          <iframe
            ref={ref}
            title="拷卡报告"
            sandbox="allow-same-origin allow-modals"
            srcDoc={html}
          />
        )}
      </div>
    </div>
  );
}
