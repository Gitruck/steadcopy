import { Component, useEffect, useRef, useState } from "react";
import {
  api,
  bytes,
  type AuditResult,
  type FormatSafety,
  type PlanDest,
} from "./bridge";
import { t } from "./i18n";

/** 危险区容器：视觉隔离 + 排在最末 + 默认全关。 */
export function DangerZone({ children }: { children: React.ReactNode }) {
  return (
    <section className="danger">
      <header>
        {t("danger.zoneTitle")}
        <span className="dim small">{t("danger.zoneHint")}</span>
      </header>
      <div className="in col">{children}</div>
    </section>
  );
}

/**
 * 倒计时确认框。
 *
 * 不可逆操作专用：倒计时归零前确认按钮不可用（冷静期），随时可取消。
 * 秒数由后端配置决定（默认 30，最小 10）——前端只呈现，不自己定规矩。
 */
export function CountdownConfirm({
  title,
  body,
  seconds,
  confirmText,
  requireTyped,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: React.ReactNode;
  seconds: number;
  confirmText: string;
  /** 非空时要求用户手输这个字符串才能确认 */
  requireTyped?: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const [left, setLeft] = useState(seconds);
  const [typed, setTyped] = useState("");

  useEffect(() => {
    if (left <= 0) return;
    const t = setTimeout(() => setLeft((n) => n - 1), 1000);
    return () => clearTimeout(t);
  }, [left]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onCancel();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  const typedOk = !requireTyped || typed.trim() === requireTyped.trim();
  const ready = left <= 0 && typedOk;

  return (
    <div className="viewer" onClick={onCancel}>
      <div className="sheet narrow" onClick={(e) => e.stopPropagation()}>
        <header>
          <span className="t danger-text">{title}</span>
        </header>
        <div className="in col">
          {body}
          {requireTyped && (
            <div className="field">
              <label>{t("confirm.typeToConfirm", { phrase: requireTyped })}</label>
              <input
                value={typed}
                autoFocus
                onChange={(e) => setTyped(e.target.value)}
                placeholder={requireTyped}
              />
              {typed && !typedOk && <span className="small danger-text">{t("confirm.notMatching")}</span>}
            </div>
          )}
          <div className="row" style={{ gap: 8, justifyContent: "flex-end" }}>
            <button className="btn" onClick={onCancel}>
              {t("app.cancel")}
            </button>
            <button className="btn danger" disabled={!ready} onClick={onConfirm}>
              {left > 0 ? t("confirm.withCountdown", { text: confirmText, n: left }) : confirmText}
            </button>
          </div>
          {left > 0 && (
            <div className="small dim" style={{ textAlign: "right" }}>
              {t("confirm.coolDown", { n: left })}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** 目的地清单：显示**渲染后的完整落地路径**与空间结论。 */
export function DestinationList({ dests }: { dests: PlanDest[] }) {
  return (
    <div className="col" style={{ gap: 6 }}>
      {dests.map((d) => (
        <div key={d.landing_dir} className="small">
          <div className="path">{d.landing_dir}</div>
          <span className="muted">
            {t("dest.needAvail", {
              need: bytes(d.required_bytes),
              avail:
                d.available_bytes === null ? t("app.unknown") : bytes(d.available_bytes),
            })}{" "}
          </span>
          {d.sufficient === true && <span className="tag t-ok">{t("dest.enough")}</span>}
          {d.sufficient === false && <span className="tag t-bad">{t("dest.notEnough")}</span>}
          {d.sufficient === null && <span className="tag t-warn">{t("dest.unknownSpace")}</span>}
        </div>
      ))}
    </div>
  );
}

/** 两条**分别标注**的进度条。Gate 的双进度条没标注，用户只能猜——这是明确要改进的点。 */
export function TwoBars({
  copy,
  verify,
  showVerify,
}: {
  copy: number;
  verify: number;
  showVerify: boolean;
}) {
  return (
    <>
      <Bar label={t("settings.copy")} pct={copy} cls="copy" />
      {showVerify && <Bar label={t("progress.verify")} pct={verify} cls="verify" />}
    </>
  );
}

function Bar({ label, pct, cls }: { label: string; pct: number; cls: string }) {
  return (
    <div className={`prog ${cls}`}>
      <div className="lbl">
        <span>{label}</span>
        <span className="mono">{pct.toFixed(1)}%</span>
      </div>
      <div className="track">
        <i style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

/** 复验四态并列呈现——**不**压缩成一个布尔结论。 */
export function AuditPanel({ r, onClose }: { r: AuditResult; onClose: () => void }) {
  return (
    <div className="panel">
      <header>{t("audit.title")}<span className="n">{t("audit.algorithm", { a: r.algorithm })}</span>
        <button className="btn sm" style={{ marginLeft: 8 }} onClick={onClose}>{t("app.close")}</button>
      </header>
      <div className="in col">
        {r.missing.length === 0 ? (
          <div className="banner ok">{t("audit.intactAll")}</div>
        ) : (
          <div className="banner bad">{t("audit.nMissing", { n: r.missing.length })}</div>
        )}
        {!r.complete && <div className="banner warn">{t("audit.incomplete")}</div>}
        {r.unverified_at_copy > 0 && (
          <div className="banner warn">
            {t("audit.nUnverified", { n: r.unverified_at_copy })}
          </div>
        )}
        <div className="row" style={{ gap: 16 }}>
          <span>
            <span className="tag t-ok">{t("audit.intact")}</span> <b className="mono">{r.intact.length}</b>
          </span>
          <span>
            <span className="tag t-warn">{t("audit.moved")}</span> <b className="mono">{r.moved.length}</b>
          </span>
          <span>
            <span className="tag t-bad">{t("audit.missing")}</span> <b className="mono">{r.missing.length}</b>
          </span>
          <span>
            <span className="tag t-warn">{t("audit.added")}</span> <b className="mono">{r.added.length}</b>
          </span>
        </div>
        {r.missing.length > 0 && (
          <table>
            <thead>
              <tr>
                <th>{t("audit.missingFiles")}</th>
                <th className="num">{t("history.size")}</th>
                <th>{t("audit.expectedHash")}</th>
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
        {r.moved.length > 0 && (
          <table>
            <thead>
              <tr>
                <th>{t("audit.moved")}</th>
                <th>{t("audit.nowAt")}</th>
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
        {r.added.length > 0 && (
          <div className="small muted">
            {t("audit.nAdded", { n: r.added.length })}
          </div>
        )}
      </div>
    </div>
  );
}

/** **应用内报告查看器**：报告 HTML 全文塞进沙箱 iframe 渲染，不跳浏览器。 */
export function ReportViewer({
  manifestPath,
  onClose,
}: {
  manifestPath: string;
  onClose: () => void;
}) {
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
          <span className="t">{t("report.title")}</span>
          <span className="path">{manifestPath.replace(/\.json$/, ".html")}</span>
          <div className="r">
            <button className="btn sm" onClick={() => ref.current?.contentWindow?.print()}>{t("report.print")}</button>
            <button
              className="btn sm"
              onClick={() => api.openReportFile(manifestPath).catch((e) => setError(String(e)))}
            >{t("report.openInBrowser")}</button>
            <button className="btn sm" onClick={onClose}>{t("app.close")}</button>
          </div>
        </header>
        {error ? (
          <div className="banner bad" style={{ margin: 12 }}>
            {error}
          </div>
        ) : html === null ? (
          <div className="empty">{t("report.loading")}</div>
        ) : (
          <iframe ref={ref} title={t("report.title")} sandbox="allow-same-origin allow-modals" srcDoc={html} />
        )}
      </div>
    </div>
  );
}

/** 格式化前置检查的呈现。逐条给结论，卡在哪一步一目了然。 */
export function SafetyChecks({ s }: { s: FormatSafety }) {
  return (
    <div className="col" style={{ gap: 4 }}>
      {s.report.checks.map((c) => (
        <div key={c.id} className="small">
          <span className={c.passed ? "tag t-ok" : "tag t-bad"}>{c.passed ? t("app.pass") : t("app.blocked")}</span>{" "}
          <span className="mono">{c.id}</span> <span className="muted">{c.detail}</span>
        </div>
      ))}
    </div>
  );
}

/**
 * 错误边界：一处渲染出错，不该让整个窗口变白。
 *
 * 这条是被真事教的——设备记忆里一个字段的形状不对，`Devices` 抛了个
 * `.replace is not a function`，整片界面就没了，用户看不到任何线索。
 * 现在最坏情况是**那一块**显示一段可复制的诊断信息，别的页照常能用。
 */
export class ErrorBoundary extends Component<
  { children: React.ReactNode; where: string },
  { error: Error | null }
> {
  constructor(props: { children: React.ReactNode; where: string }) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  render() {
    const e = this.state.error;
    if (!e) return this.props.children;
    const detail = [
      t("app.diagnostics", { where: this.props.where, message: e.message }),
      e.stack ?? "",
    ].join("\n");
    return (
      <div className="body col">
        <div className="banner bad">
          {t("app.blockFailed", { where: this.props.where })}
          <div className="small" style={{ marginTop: 6 }}>{t("app.blockFailedHint")}</div>
        </div>
        <pre className="path" style={{ whiteSpace: "pre-wrap", margin: 0 }}>
          {detail}
        </pre>
        <div className="row" style={{ gap: 8 }}>
          <button
            className="btn sm"
            onClick={() => navigator.clipboard.writeText(detail).catch(() => {})}
          >
            {t("app.copyDiagnostics")}
          </button>
          <button className="btn sm primary" onClick={() => this.setState({ error: null })}>
            {t("app.retry")}
          </button>
        </div>
      </div>
    );
  }
}
