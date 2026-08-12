import { useCallback, useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  api,
  bytes,
  duration,
  events,
  KIND_LABEL,
  STATUS_LABEL,
  type Arrival,
  type BuildInfo,
  type AuditResult,
  type Config,
  type Device,
  type DeviceKind,
  type FileRecord,
  type FormatAttempt,
  type FormatSafety,
  type LicenseList,
  type Preset,
  type Project,
  type RunView,
  type Settings,
  type SinkSuggestion,
  type TaskRecord,
  type UnlistenFn,
} from "./bridge";
import { AdhocPanel, SinkBar } from "./adhoc";
import { resolveLang, setLang, t } from "./i18n";
import {
  AuditPanel,
  CountdownConfirm,
  DangerZone,
  ErrorBoundary,
  DestinationList,
  ReportViewer,
  SafetyChecks,
  TwoBars,
} from "./components";

type Tab = "workbench" | "presets" | "devices" | "history" | "settings";

const TABS: Tab[] = ["workbench", "presets", "devices", "history", "settings"];

const TAB_KEY = {
  workbench: "nav.workbench",
  presets: "nav.presets",
  devices: "nav.devices",
  history: "nav.history",
  settings: "nav.settings",
} as const;

export default function App() {
  const [tab, setTab] = useState<Tab>("workbench");
  const [cfg, setCfg] = useState<Config | null>(null);
  const [version, setVersion] = useState("");
  const [error, setError] = useState("");
  const [viewing, setViewing] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const c = await api.getConfig();
      // 语言在这儿定：配置是唯一来源，界面与 core 用同一份设置
      setLang(resolveLang(c.settings.locale));
      setCfg(c);
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    reload();
    api.appVersion().then(setVersion).catch(() => {});
    api.startWatching().catch((e) => setError(String(e)));
    const un = events.onWatchError((m) => setError(m));
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, [reload]);

  const danger =
    cfg?.settings.skip_confirmation || cfg?.settings.format_after_copy || false;

  return (
    <div className="shell">
      <nav className="nav">
        <div className="brand">
          稳拷 <small>steadcopy</small>
        </div>
        {TABS.map((x) => (
          <button key={x} className={tab === x ? "on" : ""} onClick={() => setTab(x)}>
            {t(TAB_KEY[x])}
          </button>
        ))}
        <div className="spacer" />
        <div className="ver">v{version || "…"}</div>
      </nav>
      <div className="main">
        {danger && (
          <div className="danger-strip" onClick={() => setTab("settings")}>
            {t("danger.stripTitle")}
            {cfg?.settings.skip_confirmation && t("danger.stripSkip")}
            {cfg?.settings.format_after_copy && t("danger.stripFormat")}
            <span className="dim">{t("danger.stripGo")}</span>
          </div>
        )}
        {error && (
          <div className="banner bad" style={{ margin: 12 }}>
            {error}
            <button className="btn sm" style={{ marginLeft: 8 }} onClick={() => setError("")}>
              {t("app.gotIt")}
            </button>
          </div>
        )}
        {/* 每页各自兜底：一页炸了不该把整个窗口带走 */}
        <ErrorBoundary where={t(TAB_KEY[tab])} key={tab}>
          {!cfg ? (
            <div className="empty">{t("app.loadingConfig")}</div>
          ) : tab === "workbench" ? (
            <Workbench cfg={cfg} reload={reload} onView={setViewing} onError={setError} />
          ) : tab === "presets" ? (
            <Presets cfg={cfg} reload={reload} onError={setError} />
          ) : tab === "devices" ? (
            <Devices cfg={cfg} reload={reload} onError={setError} />
          ) : tab === "history" ? (
            <History onView={setViewing} onError={setError} />
          ) : (
            <SettingsPage cfg={cfg} reload={reload} onError={setError} />
          )}
        </ErrorBoundary>
      </div>
      {viewing && <ReportViewer manifestPath={viewing} onClose={() => setViewing(null)} />}
    </div>
  );
}

// ---------------------------------------------------------------- 工位

function Workbench({
  cfg,
  reload,
  onView,
  onError,
}: {
  cfg: Config;
  reload: () => void;
  onView: (p: string) => void;
  onError: (e: string) => void;
}) {
  const [devices, setDevices] = useState<Device[]>([]);
  const [arrival, setArrival] = useState<Arrival | null>(null);
  const [running, setRunning] = useState(false);
  const [stage, setStage] = useState("");
  const [copyPct, setCopyPct] = useState(0);
  const [verifyPct, setVerifyPct] = useState(0);
  const [current, setCurrent] = useState<string | null>(null);
  const [speed, setSpeed] = useState<number | null>(null);
  const [eta, setEta] = useState<number | null>(null);
  const [result, setResult] = useState<RunView | null>(null);
  const [notices, setNotices] = useState<string[]>([]);
  const [paused, setPaused] = useState(false);
  const [adhocFor, setAdhocFor] = useState<{ root: string; name: string } | null>(null);
  const [sink, setSink] = useState<SinkSuggestion | null>(null);
  const [formatTarget, setFormatTarget] = useState<{ root: string; safety: FormatSafety } | null>(
    null
  );

  const refresh = useCallback(() => {
    api.listDevices().then(setDevices).catch((e) => onError(String(e)));
  }, [onError]);

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 5000);
    return () => clearInterval(t);
  }, [refresh]);

  // 运行状态一律由后端事件驱动。无人值守档的任务是后端自己起的，
  // 前端没有「我发起的」这个概念，只有「现在在跑」。
  useEffect(() => {
    const un: Promise<UnlistenFn>[] = [
      events.onArrival((a) => {
        setArrival(a);
        setAdhocFor(null);
        setSink(null);
        setResult(null);
        refresh();
        reload();
      }),
      events.onRemoved(() => refresh()),
      events.onTaskStarted(() => {
        setArrival(null);
        setAdhocFor(null);
        setRunning(true);
        setPaused(false);
        setNotices([]);
        setResult(null);
        setCopyPct(0);
        setVerifyPct(0);
      }),
      events.onTaskFinished((r) => {
        setResult(r);
        setRunning(false);
        setStage("");
        setCurrent(null);
        refresh();
      }),
      events.onTaskFailed((m) => {
        onError(m);
        setRunning(false);
        setStage("");
        setCurrent(null);
        refresh();
      }),
      events.onStage((p) => setStage(p.stage)),
      events.onProgress((p) => {
        if (p.stage === "校验") setVerifyPct(p.percent);
        else setCopyPct(p.percent);
        setCurrent(p.current);
        setSpeed(p.bytes_per_sec);
        setEta(p.eta_secs);
      }),
      events.onNotice((m) => setNotices((n) => [...n, m])),
      events.onFileFailed((f) => setNotices((n) => [...n, `${f.path}：${f.reason}`])),
      // 拷完全绿 + 危险区开关开着时后端会提议格式化。走的还是那套倒计时确认，
      // 一步都不少——「自动」省掉的只是找按钮，不是省掉确认。
      events.onFormatProposed((s) => setFormatTarget({ root: s.root, safety: s })),
      // 沉淀建议：任务一开跑就来，界面把它挂在进度旁边，结束后仍留着
      events.onSinkSuggested((s) => setSink(s)),
    ];
    return () => {
      un.forEach((p) => p.then((f) => f()).catch(() => {}));
    };
  }, [refresh, reload, onError]);

  async function start(deviceId: string) {
    // 状态交给 task-started / task-finished 事件；这里只负责把错误捞出来
    try {
      await api.confirmAndRun(deviceId);
    } catch (e) {
      onError(String(e));
      setRunning(false);
    }
  }

  async function classify(deviceId: string, kind: DeviceKind) {
    try {
      await api.setDeviceKind(deviceId, kind);
      reload();
      setArrival(null);
      // 指认完立刻再走一遍编排，用户不必重新插卡
      const dev = devices.find((d) => d.id === deviceId);
      if (dev && kind !== "ignored") {
        setArrival(await api.arriveNow(dev.root));
      }
      refresh();
    } catch (e) {
      onError(String(e));
    }
  }

  const usable = devices.filter((d) => d.can_be_source);

  return (
    <>
      <div className="topbar">
        <span className="t">工位</span>
        {/* 换项目是拍摄期最高频的操作之一，不该埋在设置页三层深处 */}
        {cfg.projects.length > 0 ? (
          <label className="row small muted" style={{ gap: 6, alignItems: "center" }}>
            {t("workbench.currentProject")}
            <select
              value={cfg.current_project ?? cfg.projects[0].id}
              disabled={running}
              onChange={(e) =>
                api.setCurrentProject(e.target.value).then(reload, (x) => onError(String(x)))
              }
            >
              {cfg.projects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
        ) : (
          <span className="muted small">{t("workbench.noProject")}</span>
        )}
        <span className="muted small">
          {t("workbench.enabledPresets", { n: cfg.presets.filter((p) => p.enabled).length })}
        </span>
        <div className="r">
          {running ? (
            <span className="tag t-run">{t("workbench.running")}</span>
          ) : (
            <span className="tag t-neutral">{t("workbench.idle")}</span>
          )}
        </div>
      </div>

      <div className="body col">
        {cfg.projects.length === 0 && (
          <div className="banner warn">
            {t("workbench.needProject")}
          </div>
        )}
        {cfg.projects.length > 0 && cfg.presets.filter((p) => p.enabled).length === 0 && (
          <div className="banner warn">
            {t("workbench.needPreset")}
          </div>
        )}

        {adhocFor && !arrival && (
          <AdhocPanel
            device={adhocFor}
            projects={cfg.projects.map((p) => ({ id: p.id, name: p.name }))}
            onPlanned={(a) => {
              setAdhocFor(null);
              setArrival(a);
            }}
            onCancel={() => setAdhocFor(null)}
            onError={onError}
          />
        )}

        {arrival && (
          <ArrivalCard
            a={arrival}
            running={running}
            onCopyOnce={() => {
              const dev = devices.find((x) => x.id === arrival.device_id);
              api.dismissArrival(arrival.device_id).catch(() => {});
              setArrival(null);
              if (dev) setAdhocFor({ root: dev.root, name: dev.name });
              else onError("这个设备现在不在线，插上再试");
            }}
            onViewLastReport={() => {
              api.listHistory(false, 20).then(
                (h) => {
                  const t = h.find(
                    (x) => x.source_id === arrival.device_id && x.manifests.length > 0
                  );
                  if (t) onView(t.manifests[0]);
                  else onError("台账里还没有这台设备的记录");
                },
                (e) => onError(String(e))
              );
            }}
            onStart={() => start(arrival.device_id)}
            onClassify={(k) => classify(arrival.device_id, k)}
            onLater={() => {
              api.dismissArrival(arrival.device_id).catch(() => {});
              setArrival(null);
            }}
          />
        )}

        {running && (
          <div className="panel">
            <header>
              {t("workbench.inProgress")}<span className="n">{stage}</span>
            </header>
            <div className="in col">
              <TwoBars copy={copyPct} verify={verifyPct} showVerify={cfg.settings.verify_default} />
              <div className="small muted">
                {/* 算不出来就说算不出来，不拿 0 或「计算中」糊弄 */}
                {speed === null ? t("workbench.speedUnknown") : `${bytes(speed)}/s`}
                {"　"}
                {eta === null ? t("workbench.etaUnknown") : t("workbench.etaAbout", { d: duration(eta) })}
              </div>
              {current && <div className="path">{current}</div>}
              <div className="row" style={{ gap: 6 }}>
                <button
                  className="btn sm"
                  onClick={() => {
                    const next = !paused;
                    api.setPaused(next).then(() => setPaused(next), (e) => onError(String(e)));
                  }}
                >
                  {paused ? t("workbench.resume") : t("workbench.pause")}
                </button>
                <button className="btn danger sm" onClick={() => api.cancelCopy()}>
                  {t("app.cancel")}
                </button>
                {paused && <span className="tag t-warn">{t("workbench.paused")}</span>}
              </div>
              {/* 沉淀提示挂在这儿：行内、不抢焦点，用户想盯进度就无视它 */}
              {sink && (
                <SinkBar
                  s={sink}
                  onDone={reload}
                  onDismiss={() => setSink(null)}
                  onError={onError}
                />
              )}
            </div>
          </div>
        )}

        {notices.length > 0 && (
          <div className="banner warn">
            {notices.map((n, i) => (
              <div key={i}>{n}</div>
            ))}
          </div>
        )}

        {result && (
          <>
            <Result r={result} onView={onView} />
            {/* 任务结束后仍能点：只在传输那一瞬间给机会等于没给 */}
            {sink && (
              <SinkBar s={sink} onDone={reload} onDismiss={() => setSink(null)} onError={onError} />
            )}
          </>
        )}

        <div className="panel">
          <header>
            {t("workbench.devices")}
            <span className="n">{t("workbench.devicesUsable", { n: usable.length })}</span>
            <button className="btn sm" style={{ marginLeft: 8 }} onClick={refresh}>
              刷新
            </button>
          </header>
          <div className="in">
            {usable.length > 0 && !running && !arrival && (
              <div className="small muted" style={{ marginBottom: 8 }}>
                {t("workbench.howToStart")}
              </div>
            )}
            {devices.length === 0 && <div className="empty">{t("workbench.reading")}</div>}
            {usable.length === 0 && devices.length > 0 && (
              <div className="empty">
                {t("workbench.waitingCard")}
                {cfg.devices.some((d) => d.kind === "ignored") &&
                  t("workbench.ignoredHint", {
                    n: cfg.devices.filter((d) => d.kind === "ignored").length,
                  })}
              </div>
            )}
            {devices
              .filter((d) => d.can_be_source)
              .map((d) => (
                <div key={d.id} className="dev usable">
                  <div className="hd">
                    <span className="nm">{d.name}</span>
                    {d.kind_label && <span className="tag t-neutral">{d.kind_label}</span>}
                    <span className="tag t-ok">{t("workbench.canBeSource")}</span>
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
                  <div className="row" style={{ marginTop: 8, gap: 6, alignItems: "center" }}>
                    {/* 这是这张卡上最主要的动作，样式与措辞都得压过旁边两个。
                        「用它作为源」是内部黑话，用户想的是「把这张卡备份了」 */}
                    <button
                      className="btn primary"
                      disabled={running}
                      onClick={async () => {
                        try {
                          setArrival(await api.arriveNow(d.root));
                        } catch (e) {
                          onError(String(e));
                        }
                      }}
                    >
                      {t("workbench.backupThis")}
                    </button>
                    <button
                      className="btn sm"
                      disabled={running}
                      onClick={() => setAdhocFor({ root: d.root, name: d.name })}
                    >
                      {t("workbench.copyOnce")}
                    </button>
                    <button
                      className="btn sm"
                      disabled={running}
                      onClick={() =>
                        api.ejectDevice(d.root).then(
                          () => {
                            setNotices((n) => [...n, t("workbench.ejected", { name: d.name })]);
                            refresh();
                          },
                          (e) => onError(String(e))
                        )
                      }
                    >
                      {t("workbench.eject")}
                    </button>
                    <button
                      className="btn sm danger"
                      style={{ marginLeft: "auto" }}
                      disabled={running}
                      onClick={async () => {
                        try {
                          setFormatTarget({ root: d.root, safety: await api.checkFormat(d.root) });
                        } catch (e) {
                          onError(String(e));
                        }
                      }}
                    >
                      {t("workbench.format")}
                    </button>
                  </div>
                </div>
              ))}
          </div>
        </div>
      </div>

      {formatTarget && (
        <FormatDialog
          root={formatTarget.root}
          safety={formatTarget.safety}
          onClose={() => setFormatTarget(null)}
          onDone={() => {
            setFormatTarget(null);
            refresh();
          }}
          onError={onError}
        />
      )}
    </>
  );
}

/** 插卡确认卡片。**默认必须点一次**——参数全部预填好，但不替用户按下按钮。 */
function ArrivalCard({
  a,
  running,
  onStart,
  onClassify,
  onLater,
  onCopyOnce,
  onViewLastReport,
}: {
  a: Arrival;
  running: boolean;
  onStart: () => void;
  onClassify: (k: DeviceKind) => void;
  onLater: () => void;
  onCopyOnce: () => void;
  onViewLastReport: () => void;
}) {
  /** 每个「不能做」的结论都要带一个「那就这样做」——出口按什么走由 core 判定。 */
  const exit = () => {
    switch (a.next_step) {
      case "copy_once":
      case "classify_or_copy_once":
      case "choose_another_destination":
        return (
          <button className="btn primary" onClick={onCopyOnce}>
            {a.next_step === "choose_another_destination" ? "换个目的地拷" : "就拷这一次"}
          </button>
        );
      case "view_last_report":
        return (
          <button className="btn" onClick={onViewLastReport}>
            看上次的报告
          </button>
        );
      default:
        return null;
    }
  };

  if (a.outcome === "needs_classification") {
    return (
      <div className="panel arrival">
        <header>{t("arrival.newDevice")}</header>
        <div className="in col">
          <div className="banner warn">{a.summary}</div>
          <div className="small muted">
            {t("arrival.classifyHint")}
          </div>
          <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
            {(["camera", "recorder", "storage", "ignored"] as DeviceKind[]).map((k) => (
              <button key={k} className="btn" onClick={() => onClassify(k)}>
                {KIND_LABEL[k]}
              </button>
            ))}
            {exit()}
            <button className="btn" onClick={onLater}>
              {t("app.later")}
            </button>
          </div>
        </div>
      </div>
    );
  }

  if (a.outcome !== "planned") {
    return (
      <div className="panel arrival">
        <header>插卡</header>
        <div className="in col">
          <div className={a.outcome === "insufficient_space" ? "banner bad" : "banner warn"}>
            {a.summary}
          </div>
          {a.destinations.length > 0 && <DestinationList dests={a.destinations} />}
          <div className="row" style={{ gap: 8 }}>
            {exit()}
            <button className="btn" onClick={onLater}>
              {t("app.gotIt")}
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="panel arrival">
      <header>
        {t("arrival.detected")}
        <span className="n">{t("arrival.viaPreset", { name: a.preset_name ?? "" })}</span>
      </header>
      <div className="in col">
        <div className="row" style={{ alignItems: "baseline", gap: 10 }}>
          <span style={{ fontSize: 15, fontWeight: 600 }}>{a.device_name}</span>
          <span className="muted small">
            本次待拷 <b className="mono">{a.to_copy}</b> 个 ·{" "}
            <b className="mono">{bytes(a.to_copy_bytes)}</b>
            {a.skipped > 0 && `，已跳过 ${a.skipped} 个`}
          </span>
        </div>
        {a.categories.length > 0 && (
          <div className="small dim">
            {a.categories.map(([k, n, b]) => `${k} ${n} 个 / ${bytes(b)}`).join("　")}
          </div>
        )}
        <div className="small muted">{t("arrival.willCopyTo")}</div>
        <DestinationList dests={a.destinations} />
        <div className="row" style={{ gap: 8 }}>
          <button className="btn primary" disabled={running} onClick={onStart}>
            {t("arrival.start")}
          </button>
          {/* 「改一下」复用临时拷贝那条路：同一套参数面板、同一套规划，
              不再写第二份编辑器——两份迟早会漂 */}
          <button className="btn" disabled={running} onClick={onCopyOnce}>
            {t("arrival.editThenCopy")}
          </button>
          <button className="btn" disabled={running} onClick={onLater}>
            {t("app.later")}
          </button>
        </div>
      </div>
    </div>
  );
}

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

/** 格式化确认：看备份凭据 + 手输卷标 + 倒计时，三重。 */
function FormatDialog({
  root,
  safety,
  onClose,
  onDone,
  onError,
}: {
  root: string;
  safety: FormatSafety;
  onClose: () => void;
  onDone: () => void;
  onError: (e: string) => void;
}) {
  if (!safety.passed) {
    const failed = safety.report.checks.find((c) => !c.passed);
    return (
      <div className="viewer" onClick={onClose}>
        <div className="sheet narrow" onClick={(e) => e.stopPropagation()}>
          <header>
            <span className="t danger-text">不能格式化</span>
          </header>
          <div className="in col">
            <div className="banner bad">
              {failed ? `${failed.id}：${failed.detail}` : "前置检查未通过"}
            </div>
            <SafetyChecks s={safety} />
            <div className="row" style={{ justifyContent: "flex-end" }}>
              <button className="btn" onClick={onClose}>
                知道了
              </button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <CountdownConfirm
      title="⚠ 格式化存储卡"
      seconds={safety.countdown_secs}
      confirmText="格式化"
      requireTyped={safety.confirm_phrase}
      onCancel={onClose}
      onConfirm={async () => {
        try {
          await api.doFormat(root, safety.confirm_phrase);
          onDone();
        } catch (e) {
          onError(String(e));
          onClose();
        }
      }}
      body={
        <div className="col" style={{ gap: 10 }}>
          <div>
            即将格式化 <b>{safety.device_name}</b>（{safety.file_system}，
            {safety.label.trim() ? `卷标「${safety.label}」` : "无卷标"}）
          </div>
          <SafetyChecks s={safety} />
          <div className="banner bad">此操作不可撤销，卡上全部数据将被清除。</div>
          <div className="small muted">格式化后会保留原文件系统与卷标，相机才认得出这张卡。</div>
        </div>
      }
    />
  );
}

// ---------------------------------------------------------------- 预设

function Presets({
  cfg,
  reload,
  onError,
}: {
  cfg: Config;
  reload: () => void;
  onError: (e: string) => void;
}) {
  const [editing, setEditing] = useState<Preset | null>(null);

  async function save(p: Preset) {
    try {
      await api.upsertPreset(p);
      setEditing(null);
      reload();
    } catch (e) {
      onError(String(e));
    }
  }

  return (
    <>
      <div className="topbar">
        <span className="t">预设任务</span>
        <span className="muted small">决定「什么设备插上就怎么拷」，由窄到宽第一条命中</span>
        <div className="r">
          <button
            className="btn sm primary"
            onClick={() =>
              setEditing({
                id: `pst-new-${Date.now().toString(16)}`,
                name: "新预设",
                enabled: true,
                match: { kind: "any_classified_source" },
                project_id: cfg.current_project,
                verify: true,
                algorithm: "xxh64",
                eject_after: false,
              })
            }
          >
            新建预设
          </button>
        </div>
      </div>
      <div className="body col">
        {cfg.presets.length === 0 && (
          <div className="empty">
            还没有预设。插卡之后稳拷需要知道「这类卡该拷进哪个项目」——建一条吧。
          </div>
        )}
        {cfg.presets.map((p, i) => (
          <div key={p.id} className="panel">
            <div className="in row" style={{ alignItems: "center", gap: 10 }}>
              <span className="dim mono">{i + 1}</span>
              <div className="grow">
                <div className="row" style={{ gap: 8, alignItems: "center" }}>
                  <b>{p.name}</b>
                  {p.enabled ? (
                    <span className="tag t-ok">已启用</span>
                  ) : (
                    <span className="tag t-neutral">已停用</span>
                  )}
                </div>
                <div className="small muted">
                  匹配 {matchLabel(p.match, cfg)} → 项目「
                  {cfg.projects.find((x) => x.id === p.project_id)?.name ?? "当前项目"}」 · 校验{" "}
                  {p.verify ? "开" : "关"} · {p.algorithm}
                </div>
              </div>
              <button className="btn sm" disabled={i === 0} onClick={() => api.movePreset(p.id, true).then(reload)}>
                ↑
              </button>
              <button
                className="btn sm"
                disabled={i === cfg.presets.length - 1}
                onClick={() => api.movePreset(p.id, false).then(reload)}
              >
                ↓
              </button>
              <button className="btn sm" onClick={() => save({ ...p, enabled: !p.enabled })}>
                {p.enabled ? "停用" : "启用"}
              </button>
              <button className="btn sm" onClick={() => setEditing(p)}>
                编辑
              </button>
              <button
                className="btn sm danger"
                onClick={() => api.deletePreset(p.id).then(reload).catch((e) => onError(String(e)))}
              >
                删除
              </button>
            </div>
          </div>
        ))}
      </div>
      {editing && (
        <PresetEditor
          cfg={cfg}
          preset={editing}
          onCancel={() => setEditing(null)}
          onSave={save}
        />
      )}
    </>
  );
}

function matchLabel(m: Preset["match"], cfg: Config): string {
  if (m.kind === "any_classified_source") return "任何已分类的源设备";
  if (m.kind === "kind") return `全部${KIND_LABEL[m.device_kind]}`;
  const d = cfg.devices.find((x) => x.id === m.device_id);
  return `指定设备「${d?.custom_name ?? m.device_id.slice(0, 20)}」`;
}

function PresetEditor({
  cfg,
  preset,
  onCancel,
  onSave,
}: {
  cfg: Config;
  preset: Preset;
  onCancel: () => void;
  onSave: (p: Preset) => void;
}) {
  const [p, setP] = useState<Preset>(preset);
  const matchKind = p.match.kind;

  return (
    <div className="viewer" onClick={onCancel}>
      <div className="sheet narrow" onClick={(e) => e.stopPropagation()}>
        <header>
          <span className="t">预设任务</span>
        </header>
        <div className="in col">
          <div className="field">
            <label>名称</label>
            <input value={p.name} onChange={(e) => setP({ ...p, name: e.target.value })} />
          </div>

          <div className="field">
            <label>匹配什么设备（越窄越优先）</label>
            <select
              value={matchKind}
              onChange={(e) => {
                const k = e.target.value;
                if (k === "any_classified_source") setP({ ...p, match: { kind: "any_classified_source" } });
                else if (k === "kind") setP({ ...p, match: { kind: "kind", device_kind: "camera" } });
                else
                  setP({
                    ...p,
                    match: { kind: "device", device_id: cfg.devices[0]?.id ?? "" },
                  });
              }}
            >
              <option value="device">指定设备（最窄）</option>
              <option value="kind">某一类设备</option>
              <option value="any_classified_source">任何已分类的源设备（最宽）</option>
            </select>
          </div>

          {p.match.kind === "kind" && (
            <div className="field">
              <label>哪一类</label>
              <select
                value={p.match.device_kind}
                onChange={(e) =>
                  setP({ ...p, match: { kind: "kind", device_kind: e.target.value as DeviceKind } })
                }
              >
                <option value="camera">摄影卡</option>
                <option value="recorder">录音卡</option>
                <option value="storage">素材盘</option>
              </select>
            </div>
          )}

          {p.match.kind === "device" && (
            <div className="field">
              <label>哪一台（来自设备记忆库）</label>
              <select
                value={p.match.device_id}
                onChange={(e) => setP({ ...p, match: { kind: "device", device_id: e.target.value } })}
              >
                {cfg.devices.length === 0 && <option value="">还没有记住任何设备</option>}
                {cfg.devices.map((d) => (
                  <option key={d.id} value={d.id}>
                    {d.custom_name}（{KIND_LABEL[d.kind]}）
                  </option>
                ))}
              </select>
            </div>
          )}

          <div className="field">
            <label>拷进哪个项目</label>
            <select
              value={p.project_id ?? ""}
              onChange={(e) => setP({ ...p, project_id: e.target.value || null })}
            >
              <option value="">用当前项目</option>
              {cfg.projects.map((x) => (
                <option key={x.id} value={x.id}>
                  {x.name}
                </option>
              ))}
            </select>
          </div>

          <div className="row" style={{ gap: 16 }}>
            <label className="small row" style={{ gap: 5, alignItems: "center" }}>
              <input
                type="checkbox"
                checked={p.verify}
                onChange={(e) => setP({ ...p, verify: e.target.checked })}
              />
              读回校验
            </label>
            <label className="small row" style={{ gap: 5, alignItems: "center" }}>
              <input
                type="checkbox"
                checked={p.eject_after}
                onChange={(e) => setP({ ...p, eject_after: e.target.checked })}
              />
              拷完自动弹出
            </label>
            <div className="field">
              <label>算法</label>
              <select
                value={p.algorithm}
                onChange={(e) => setP({ ...p, algorithm: e.target.value as "xxh64" | "md5" })}
              >
                <option value="xxh64">XXH64（快，推荐）</option>
                <option value="md5">MD5（慢，兼容旧流程）</option>
              </select>
            </div>
          </div>

          {!p.verify && (
            <div className="banner warn">
              关掉读回校验后，将无法发现介质写入错误——拷进去的东西坏没坏你不会知道。
            </div>
          )}

          <div className="row" style={{ gap: 8, justifyContent: "flex-end" }}>
            <button className="btn" onClick={onCancel}>
              取消
            </button>
            <button className="btn primary" onClick={() => onSave(p)}>
              保存
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- 设备

function Devices({
  cfg,
  reload,
  onError,
}: {
  cfg: Config;
  reload: () => void;
  onError: (e: string) => void;
}) {
  const active = cfg.devices.filter((d) => d.kind !== "ignored");
  const ignored = cfg.devices.filter((d) => d.kind === "ignored");

  const row = (d: (typeof cfg.devices)[number]) => (
    <tr key={d.id}>
      <td>
        <input
          className="inline"
          value={d.custom_name}
          onChange={(e) =>
            api.renameDevice(d.id, e.target.value).then(reload).catch((x) => onError(String(x)))
          }
        />
      </td>
      <td>
        <select
          value={d.kind}
          onChange={(e) =>
            api
              .setDeviceKind(d.id, e.target.value as DeviceKind)
              .then(reload)
              .catch((x) => onError(String(x)))
        }
        >
          {(["unclassified", "camera", "recorder", "storage", "ignored"] as DeviceKind[]).map((k) => (
            <option key={k} value={k}>
              {KIND_LABEL[k]}
            </option>
          ))}
        </select>
      </td>
      <td className="muted">{d.last_label}</td>
      <td className="num">{bytes(d.total_bytes)}</td>
      <td className="mono dim">{d.last_seen.replace("T", " ").slice(0, 19)}</td>
      <td>
        <button
          className="btn sm danger"
          onClick={() => api.forgetDevice(d.id).then(reload).catch((x) => onError(String(x)))}
        >
          删除记忆
        </button>
      </td>
    </tr>
  );

  return (
    <>
      <div className="topbar">
        <span className="t">设备</span>
        <span className="muted small">已记住 {cfg.devices.length} 个</span>
      </div>
      <div className="body col">
        {cfg.devices.length === 0 && (
          <div className="empty">记忆库还是空的。插一张卡，稳拷会记住它。</div>
        )}
        {active.length > 0 && (
          <div className="panel">
            <header>
              已记住的设备<span className="n">{active.length}</span>
            </header>
            <table>
              <thead>
                <tr>
                  <th>名字</th>
                  <th style={{ width: 110 }}>类型</th>
                  <th>卷标</th>
                  <th className="num">容量</th>
                  <th>最近见到</th>
                  <th style={{ width: 100 }} />
                </tr>
              </thead>
              <tbody>{active.map(row)}</tbody>
            </table>
          </div>
        )}
        {ignored.length > 0 && (
          <div className="panel">
            <header>
              已忽略<span className="n">{ignored.length}</span>
            </header>
            <div className="in small muted">
              这些设备插上不会有任何提示。「插卡没反应」多半就是因为它在这里。
            </div>
            <table>
              <thead>
                <tr>
                  <th>名字</th>
                  <th style={{ width: 110 }}>类型</th>
                  <th>卷标</th>
                  <th className="num">容量</th>
                  <th>最近见到</th>
                  <th style={{ width: 100 }} />
                </tr>
              </thead>
              <tbody>{ignored.map(row)}</tbody>
            </table>
          </div>
        )}
      </div>
    </>
  );
}

// ---------------------------------------------------------------- 台账

function History({
  onView,
  onError,
}: {
  onView: (p: string) => void;
  onError: (e: string) => void;
}) {
  const [items, setItems] = useState<TaskRecord[]>([]);
  const [onlyFailed, setOnlyFailed] = useState(false);
  const [detail, setDetail] = useState<{ task: TaskRecord; files: FileRecord[] } | null>(null);
  const [audit, setAudit] = useState<AuditResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [attempts, setAttempts] = useState<FormatAttempt[]>([]);
  const [showAttempts, setShowAttempts] = useState(false);

  const load = useCallback(() => {
    api.listHistory(onlyFailed).then(setItems).catch((e) => onError(String(e)));
  }, [onlyFailed, onError]);

  useEffect(load, [load]);

  return (
    <>
      <div className="topbar">
        <span className="t">台账</span>
        <span className="muted small">{items.length} 条记录</span>
        <div className="r">
          <label className="small row" style={{ gap: 5, alignItems: "center" }}>
            <input
              type="checkbox"
              checked={onlyFailed}
              onChange={(e) => setOnlyFailed(e.target.checked)}
            />
            只看有失败的
          </label>
          <button
            className="btn sm"
            onClick={() => {
              api.formatAttempts().then((a) => {
                setAttempts(a);
                setShowAttempts(true);
              });
            }}
          >
            格式化留痕
          </button>
          <button
            className="btn sm"
            disabled={busy}
            onClick={async () => {
              // 独立复验：不依赖台账记录，直接挑一份清单来核
              const f = await openDialog({
                title: "选择要复验的清单",
                filters: [{ name: "稳拷清单", extensions: ["json"] }],
              });
              if (typeof f !== "string") return;
              setBusy(true);
              try {
                setAudit(await api.runAudit(f));
              } catch (e) {
                onError(String(e));
              } finally {
                setBusy(false);
              }
            }}
          >
            复验某份清单…
          </button>
          <button className="btn sm" onClick={load}>
            刷新
          </button>
        </div>
      </div>
      <div className="body col">
        {items.length === 0 && <div className="empty">还没有拷卡记录。</div>}
        {items.length > 0 && (
          <div className="panel">
            <table>
              <thead>
                <tr>
                  <th>时间</th>
                  <th>项目</th>
                  <th>来源</th>
                  <th className="num">文件</th>
                  <th className="num">大小</th>
                  <th className="num">耗时</th>
                  <th>结果</th>
                  <th style={{ width: 230 }} />
                </tr>
              </thead>
              <tbody>
                {items.map((t) => (
                  <tr key={t.id}>
                    <td className="mono">{t.finished_at.replace("T", " ").slice(0, 19)}</td>
                    <td>{t.project}</td>
                    <td>{t.source_name}</td>
                    <td className="num">
                      {t.copied}
                      {t.skipped > 0 && <span className="dim">+{t.skipped}跳</span>}
                    </td>
                    <td className="num">{bytes(t.total_bytes)}</td>
                    <td className="num">{duration(t.elapsed_secs)}</td>
                    <td>
                      <span
                        className={
                          t.status === "ok"
                            ? "tag t-ok"
                            : t.status === "cancelled"
                              ? "tag t-warn"
                              : "tag t-bad"
                        }
                      >
                        {STATUS_LABEL[t.status]}
                      </span>
                    </td>
                    <td>
                      <div className="row" style={{ gap: 6 }}>
                        {t.manifests.length === 0 ? (
                          <button className="btn sm primary" disabled>
                            报告
                          </button>
                        ) : (
                          // 多目的地会落多份凭证，每份一个报告——不能只给第一份，
                          // 那等于把另外几个目的地的结论藏起来
                          t.manifests.map((m, i) => (
                            <button key={m} className="btn sm primary" onClick={() => onView(m)}>
                              报告{t.manifests.length > 1 ? ` ${i + 1}` : ""}
                            </button>
                          ))
                        )}
                        <button
                          className="btn sm"
                          disabled={busy || t.manifests.length === 0}
                          onClick={async () => {
                            setBusy(true);
                            try {
                              setAudit(await api.runAudit(t.manifests[0]));
                            } catch (e) {
                              onError(String(e));
                            } finally {
                              setBusy(false);
                            }
                          }}
                        >
                          复验
                        </button>
                        <button
                          className="btn sm"
                          disabled={t.manifests.length === 0}
                          onClick={() =>
                            api.revealLandingDir(t.manifests[0]).catch((e) => onError(String(e)))
                          }
                        >
                          打开目录
                        </button>
                        <button
                          className="btn sm"
                          onClick={async () => {
                            try {
                              setDetail({ task: t, files: await api.taskFiles(t.id) });
                            } catch (e) {
                              onError(String(e));
                            }
                          }}
                        >
                          明细
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
        {audit && <AuditPanel r={audit} onClose={() => setAudit(null)} />}

        {detail && (
          <div className="panel">
            <header>
              文件明细 · {detail.task.project} / {detail.task.source_name}
              <button className="btn sm" style={{ marginLeft: 8 }} onClick={() => setDetail(null)}>
                关闭
              </button>
            </header>
            {detail.files.some((f) => f.status === "failed") && (
              <div className="in">
                <div className="banner bad">
                  有 {detail.files.filter((f) => f.status === "failed").length} 个文件失败
                </div>
              </div>
            )}
            <table>
              <thead>
                <tr>
                  <th>文件</th>
                  <th className="num">大小</th>
                  <th>状态</th>
                  <th>原因</th>
                </tr>
              </thead>
              <tbody>
                {[...detail.files]
                  .sort((a, b) => (a.status === "failed" ? -1 : b.status === "failed" ? 1 : 0))
                  .map((f) => (
                    <tr key={f.relative_path}>
                      <td>{f.relative_path}</td>
                      <td className="num">{bytes(f.size)}</td>
                      <td>
                        <span
                          className={
                            f.status === "failed"
                              ? "tag t-bad"
                              : f.status === "skipped"
                                ? "tag t-neutral"
                                : "tag t-ok"
                          }
                        >
                          {f.status === "failed" ? "失败" : f.status === "skipped" ? "跳过" : "已拷"}
                        </span>
                      </td>
                      <td className="muted">{f.reason ?? ""}</td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
        )}

        {showAttempts && (
          <div className="panel">
            <header>
              格式化留痕<span className="n">{attempts.length}</span>
              <button
                className="btn sm"
                style={{ marginLeft: 8 }}
                onClick={() => setShowAttempts(false)}
              >
                关闭
              </button>
            </header>
            <div className="in small muted">
              格式化是唯一销毁数据的操作，无论成功、失败、被拒还是被取消都留痕。
            </div>
            {attempts.length === 0 ? (
              <div className="empty">还没有过格式化尝试。</div>
            ) : (
              <table>
                <thead>
                  <tr>
                    <th>时间</th>
                    <th>设备</th>
                    <th>触发</th>
                    <th>检查</th>
                    <th>结果</th>
                    <th>原因</th>
                  </tr>
                </thead>
                <tbody>
                  {attempts.map((a) => (
                    <tr key={a.id}>
                      <td className="mono">{a.at.replace("T", " ").slice(0, 19)}</td>
                      <td>{a.device_name}</td>
                      <td className="dim">{a.trigger}</td>
                      <td className="mono dim">{a.checks}</td>
                      <td>
                        <span className={a.result === "ok" ? "tag t-ok" : "tag t-bad"}>
                          {a.result === "ok"
                            ? "已格式化"
                            : a.result === "rejected"
                              ? "被拒绝"
                              : a.result === "cancelled"
                                ? "已取消"
                                : "失败"}
                        </span>
                      </td>
                      <td className="muted">{a.reason ?? ""}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        )}
      </div>
    </>
  );
}

// ---------------------------------------------------------------- 设置

function SettingsPage({
  cfg,
  reload,
  onError,
}: {
  cfg: Config;
  reload: () => void;
  onError: (e: string) => void;
}) {
  const [s, setS] = useState<Settings>(cfg.settings);
  const [path, setPath] = useState("");
  const [editing, setEditing] = useState<Project | null>(null);
  const [confirmDanger, setConfirmDanger] = useState<null | "skip" | "format" | "clear">(null);

  useEffect(() => setS(cfg.settings), [cfg.settings]);
  useEffect(() => {
    api.configPath().then(setPath).catch(() => {});
  }, []);

  const save = async (next: Settings) => {
    try {
      await api.saveSettings(next);
      setS(next);
      // reload 会重新读配置并 setLang，所以切语言即时生效，不用重启
      reload();
    } catch (e) {
      onError(String(e));
      setS(cfg.settings);
    }
  };

  return (
    <>
      <div className="topbar">
        <span className="t">设置</span>
        <span className="muted small path">{path}</span>
      </div>
      <div className="body col">
        <div className="panel">
          <header>项目与目的地</header>
          <div className="in col">
            {cfg.projects.length === 0 && (
              <div className="empty">还没有项目。建一个，插卡才知道往哪拷。</div>
            )}
            {cfg.projects.map((p) => (
              <div key={p.id} className="row" style={{ alignItems: "center", gap: 8 }}>
                <div className="grow">
                  <div className="row" style={{ gap: 8, alignItems: "center" }}>
                    <b>{p.name}</b>
                    {cfg.current_project === p.id && <span className="tag t-ok">当前项目</span>}
                  </div>
                  {p.destinations.map((d) => (
                    <div key={d.id} className="path">
                      {d.enabled ? "☑" : "☐"} {d.root} · {d.template}
                    </div>
                  ))}
                </div>
                {cfg.current_project !== p.id && (
                  <button
                    className="btn sm"
                    onClick={() => api.setCurrentProject(p.id).then(reload)}
                  >
                    设为当前
                  </button>
                )}
                <button className="btn sm" onClick={() => setEditing(p)}>
                  编辑
                </button>
                <button
                  className="btn sm danger"
                  onClick={() =>
                    api.deleteProject(p.id).then(reload).catch((e) => onError(String(e)))
                  }
                >
                  删除
                </button>
              </div>
            ))}
            <div>
              <button
                className="btn sm primary"
                onClick={() =>
                  setEditing({
                    id: "",
                    name: "新项目",
                    created_at: new Date().toISOString(),
                    destinations: [],
                  })
                }
              >
                新建项目
              </button>
            </div>
          </div>
        </div>

        <div className="panel">
          <header>{t("settings.language")}</header>
          <div className="in">
            <select
              value={s.locale}
              onChange={(e) => save({ ...s, locale: e.target.value })}
              style={{ maxWidth: 220 }}
            >
              <option value="auto">{t("settings.languageAuto")}</option>
              <option value="zh">中文</option>
              <option value="en">English</option>
            </select>
          </div>
        </div>

        <div className="panel">
          <header>{t("settings.copy")}</header>
          <div className="in col">
            <Toggle
              label="读回校验"
              hint="从目的地无缓冲读回重算哈希再比对。关掉就发现不了介质写入错误"
              checked={s.verify_default}
              onChange={(v) => save({ ...s, verify_default: v })}
            />
            {!s.verify_default && (
              <div className="banner warn">
                校验已关闭——拷进去的东西坏没坏，你不会知道。
              </div>
            )}
            <Toggle
              label="拷完自动安全弹出"
              hint="全部校验通过后自动弹出源卡"
              checked={s.eject_after}
              onChange={(v) => save({ ...s, eject_after: v })}
            />
            <div className="field" style={{ maxWidth: 220 }}>
              <label>校验失败后的重拷次数</label>
              <input
                type="number"
                min={0}
                max={5}
                value={s.retries}
                onChange={(e) => save({ ...s, retries: Number(e.target.value) })}
              />
            </div>
          </div>
        </div>

        <div className="panel">
          <header>插卡</header>
          <div className="in col">
            <Toggle
              label="自动预填项目与目的地"
              hint="关掉后插卡只提示，项目与目的地需要你现选"
              checked={s.auto_prefill}
              onChange={(v) => save({ ...s, auto_prefill: v })}
            />
            <div className="small muted">
              当前档位：
              <b>
                {s.auto_prefill && s.skip_confirmation
                  ? "无人值守档（插卡直接开跑）"
                  : s.auto_prefill
                    ? "确认档（预填好，等你点一次）"
                    : "手动档（每次现选）"}
              </b>
            </div>
          </div>
        </div>

        <About path={path} onError={onError} />

        <DangerZone>
          <Toggle
            label="跳过插卡确认"
            hint="插入已分类的设备后直接开始拷贝，不再询问。未分类的新设备仍会先要求指认——这条绕不过去"
            checked={s.skip_confirmation}
            danger
            onChange={(v) => (v ? setConfirmDanger("skip") : save({ ...s, skip_confirmation: false }))}
          />
          <Toggle
            label="拷贝完成后格式化源卡"
            hint="仅当全部目的地完成且全部文件校验通过时才会触发；触发时仍会弹倒计时，可取消"
            checked={s.format_after_copy}
            danger
            onChange={(v) =>
              v ? setConfirmDanger("format") : save({ ...s, format_after_copy: false })
            }
          />
          <div className="field" style={{ maxWidth: 260 }}>
            <label>不可逆操作的确认倒计时（秒，最小 10）</label>
            <input
              type="number"
              min={10}
              value={s.countdown_secs}
              onChange={(e) => setS({ ...s, countdown_secs: Number(e.target.value) })}
              onBlur={() => save(s)}
            />
          </div>
          <div className="row">
            <button className="btn danger sm" onClick={() => setConfirmDanger("clear")}>
              清空全部台账数据
            </button>
          </div>
        </DangerZone>
      </div>

      {editing && (
        <ProjectEditor
          project={editing}
          onCancel={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            reload();
          }}
          onError={onError}
        />
      )}

      {confirmDanger === "skip" && (
        <CountdownConfirm
          title="⚠ 开启「跳过插卡确认」"
          seconds={s.countdown_secs}
          confirmText="我明白，开启"
          onCancel={() => setConfirmDanger(null)}
          onConfirm={() => {
            save({ ...s, skip_confirmation: true });
            setConfirmDanger(null);
          }}
          body={
            <div className="col" style={{ gap: 8 }}>
              <div>开启后，插入**已分类**的设备会直接开始拷贝，不再弹确认。</div>
              <div className="small muted">
                未分类的新设备仍然会先要求你指认类型——那条绕不过去，对不知道是什么的设备自动写入风险不可接受。
              </div>
              <div className="banner warn">工位顶栏会常驻提示，免得你忘了自己开过。</div>
            </div>
          }
        />
      )}

      {confirmDanger === "format" && (
        <CountdownConfirm
          title="⚠ 开启「拷完自动格式化源卡」"
          seconds={s.countdown_secs}
          confirmText="我明白，开启"
          onCancel={() => setConfirmDanger(null)}
          onConfirm={() => {
            save({ ...s, format_after_copy: true });
            setConfirmDanger(null);
          }}
          body={
            <div className="col" style={{ gap: 8 }}>
              <div>开启后，当一次任务的**全部目的地都完成且全部文件校验通过**时，会提议格式化源卡。</div>
              <div className="small muted">
                任一目的地失败、关闭了校验、存在失败文件、任务被取消——任何一种情况都不会触发。
                触发时仍会弹出倒计时，期间可以取消。
              </div>
              <div className="banner bad">格式化不可撤销。</div>
            </div>
          }
        />
      )}

      {confirmDanger === "clear" && (
        <CountdownConfirm
          title="⚠ 清空全部台账数据"
          seconds={s.countdown_secs}
          confirmText="清空"
          onCancel={() => setConfirmDanger(null)}
          onConfirm={async () => {
            try {
              await api.clearHistory();
            } catch (e) {
              onError(String(e));
            }
            setConfirmDanger(null);
          }}
          body={
            <div className="col" style={{ gap: 8 }}>
              <div>只清空**本机**的任务历史与格式化留痕。</div>
              <div className="banner ok">
                目的地上的素材与凭证不受影响；依据凭证的复验仍然可用。
              </div>
            </div>
          }
        />
      )}
    </>
  );
}

/** 关于页：构建来源、数据位置、未签名说明、开源许可。 */
function About({ path, onError }: { path: string; onError: (e: string) => void }) {
  const [info, setInfo] = useState<BuildInfo | null>(null);
  const [lic, setLic] = useState<LicenseList | null>(null);
  const [showLic, setShowLic] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    api.buildInfo().then(setInfo).catch((e) => onError(String(e)));
  }, [onError]);

  return (
    <div className="panel">
      <header>
        关于
        <span className="n">{info?.portable ? "便携版" : "安装版"}</span>
      </header>
      <div className="in col small">
        <table className="kv">
          <tbody>
            <tr>
              <th>版本</th>
              <td className="mono">{info?.version ?? "…"}</td>
            </tr>
            <tr>
              <th>构建标识</th>
              <td className="mono">{info?.commit ?? "…"}</td>
            </tr>
            <tr>
              <th>构建时间</th>
              <td className="mono">{info?.build_time ?? "…"}</td>
            </tr>
            <tr>
              <th>工具链</th>
              <td className="mono">
                {info?.rustc ?? "…"} · Tauri {info?.tauri ?? "…"}
              </td>
            </tr>
            <tr>
              <th>数据目录</th>
              <td className="path">{info?.data_dir ?? "…"}</td>
            </tr>
            <tr>
              <th>配置文件</th>
              <td className="path">{path}</td>
            </tr>
          </tbody>
        </table>

        <div className="row" style={{ gap: 6, flexWrap: "wrap" }}>
          <button
            className="btn sm"
            disabled={!info}
            onClick={() => {
              if (!info) return;
              navigator.clipboard.writeText(info.signature).then(
                () => {
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1500);
                },
                (e) => onError(String(e))
              );
            }}
          >
            {copied ? "已复制" : "复制构建标识"}
          </button>
          <button className="btn sm" onClick={() => api.openConfigFile().catch((e) => onError(String(e)))}>
            打开配置文件
          </button>
          <button
            className="btn sm primary"
            onClick={() => api.openGuide().catch((e) => onError(String(e)))}
          >
            {t("settings.openGuide")}
          </button>
          <button
            className="btn sm"
            onClick={async () => {
              try {
                if (!lic) setLic(await api.thirdPartyLicenses());
                setShowLic(true);
              } catch (e) {
                onError(String(e));
              }
            }}
          >
            开源许可
          </button>
        </div>

        <div className="banner warn">
          {t("settings.unsigned")}
          <b> {t("settings.neverDisable")}</b>
        </div>
        <div className="muted">{t("settings.offline")}</div>

        {showLic && lic && (
          <div className="viewer" onClick={() => setShowLic(false)}>
            <div className="sheet" onClick={(e) => e.stopPropagation()}>
              <header>
                <span className="t">开源许可</span>
                <span className="muted small">
                  稳拷本体 {lic.self.license} · 第三方依赖 {lic.count} 个
                </span>
                <div className="r">
                  <button className="btn sm" onClick={() => setShowLic(false)}>
                    关闭
                  </button>
                </div>
              </header>
              <div className="in" style={{ overflow: "auto" }}>
                {lic.warning && <div className="banner warn">{lic.warning}</div>}
                <table>
                  <thead>
                    <tr>
                      <th>包</th>
                      <th style={{ width: 90 }}>版本</th>
                      <th style={{ width: 70 }}>生态</th>
                      <th>许可</th>
                    </tr>
                  </thead>
                  <tbody>
                    {lic.packages.map((p) => (
                      <tr key={`${p.ecosystem}/${p.name}@${p.version}`}>
                        <td className="mono">{p.name}</td>
                        <td className="mono dim">{p.version}</td>
                        <td className="dim">{p.ecosystem}</td>
                        <td className="muted">{p.license}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function Toggle({
  label,
  hint,
  checked,
  onChange,
  danger,
}: {
  label: string;
  hint: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  danger?: boolean;
}) {
  return (
    <label className={`toggle${danger ? " danger-item" : ""}`}>
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span>
        <b>{label}</b>
        <span className="small muted"> {hint}</span>
      </span>
    </label>
  );
}

function ProjectEditor({
  project,
  onCancel,
  onSaved,
  onError,
}: {
  project: Project;
  onCancel: () => void;
  onSaved: () => void;
  onError: (e: string) => void;
}) {
  const [name, setName] = useState(project.name);
  const [dests, setDests] = useState(
    project.destinations.map((d) => ({
      id: d.id,
      root: d.root,
      template: d.template,
      enabled: d.enabled,
    }))
  );
  const [previews, setPreviews] = useState<Record<number, string>>({});

  // 预览一律调后端——前端自己实现一份渲染，预览与实际必然漂移
  useEffect(() => {
    dests.forEach((d, i) => {
      api
        .previewPath(d.root, d.template, name || "项目名", "设备名")
        .then((p) => setPreviews((prev) => ({ ...prev, [i]: p })))
        .catch((e) => setPreviews((prev) => ({ ...prev, [i]: `模板不合法：${e}` })));
    });
  }, [dests, name]);

  return (
    <div className="viewer" onClick={onCancel}>
      <div className="sheet" onClick={(e) => e.stopPropagation()} style={{ height: "auto" }}>
        <header>
          <span className="t">项目</span>
        </header>
        <div className="in col" style={{ overflow: "auto" }}>
          <div className="field">
            <label>项目名</label>
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </div>

          <div className="field">
            <label>目的地（1–4 个，一次读源同时写入）</label>
            {dests.map((d, i) => (
              <div key={i} className="panel" style={{ marginBottom: 8 }}>
                <div className="in col" style={{ gap: 6 }}>
                  <div className="row" style={{ gap: 6, alignItems: "center" }}>
                    <input
                      type="checkbox"
                      checked={d.enabled}
                      onChange={(e) =>
                        setDests(dests.map((x, j) => (j === i ? { ...x, enabled: e.target.checked } : x)))
                      }
                    />
                    <input className="grow" readOnly value={d.root} />
                    <button
                      className="btn sm"
                      onClick={async () => {
                        const p = await openDialog({ directory: true, title: "选择目的地" });
                        if (typeof p === "string")
                          setDests(dests.map((x, j) => (j === i ? { ...x, root: p } : x)));
                      }}
                    >
                      选择…
                    </button>
                    <button
                      className="btn sm danger"
                      onClick={() => setDests(dests.filter((_, j) => j !== i))}
                    >
                      移除
                    </button>
                  </div>
                  <div className="row" style={{ gap: 6, alignItems: "center" }}>
                    <span className="small muted">路径模板</span>
                    <input
                      className="grow"
                      value={d.template}
                      onChange={(e) =>
                        setDests(dests.map((x, j) => (j === i ? { ...x, template: e.target.value } : x)))
                      }
                    />
                  </div>
                  <div className="row" style={{ gap: 4, flexWrap: "wrap" }}>
                    {["{项目}", "{日期}", "{设备}", "{卡}", "{时段}", "{年}", "{月}", "{日}"].map(
                      (ph) => (
                        <button
                          key={ph}
                          className="btn sm"
                          onClick={() =>
                            setDests(
                              dests.map((x, j) =>
                                j === i ? { ...x, template: x.template + ph } : x
                              )
                            )
                          }
                        >
                          {ph}
                        </button>
                      )
                    )}
                  </div>
                  <div className="path">预览：{previews[i] ?? "…"}</div>
                </div>
              </div>
            ))}
            <button
              className="btn sm"
              disabled={dests.length >= 4}
              onClick={async () => {
                const p = await openDialog({ directory: true, title: "添加目的地" });
                if (typeof p === "string")
                  setDests([
                    ...dests,
                    { id: null as unknown as string, root: p, template: "{项目}/{日期}/{设备}", enabled: true },
                  ]);
              }}
            >
              添加目的地…
            </button>
          </div>

          <div className="row" style={{ gap: 8, justifyContent: "flex-end" }}>
            <button className="btn" onClick={onCancel}>
              取消
            </button>
            <button
              className="btn primary"
              onClick={async () => {
                try {
                  await api.upsertProject({
                    id: project.id || null,
                    name,
                    destinations: dests.map((d) => ({
                      id: d.id || null,
                      root: d.root,
                      template: d.template,
                      enabled: d.enabled,
                    })),
                  });
                  onSaved();
                } catch (e) {
                  onError(String(e));
                }
              }}
            >
              保存
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
