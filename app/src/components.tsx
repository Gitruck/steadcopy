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
              <label>请输入「{requireTyped}」以确认</label>
              <input
                value={typed}
                autoFocus
                onChange={(e) => setTyped(e.target.value)}
                placeholder={requireTyped}
              />
              {typed && !typedOk && <span className="small danger-text">还对不上</span>}
            </div>
          )}
          <div className="row" style={{ gap: 8, justifyContent: "flex-end" }}>
            <button className="btn" onClick={onCancel}>
              取消
            </button>
            <button className="btn danger" disabled={!ready} onClick={onConfirm}>
              {left > 0 ? `${confirmText}（${left}）` : confirmText}
            </button>
          </div>
          {left > 0 && (
            <div className="small dim" style={{ textAlign: "right" }}>
              冷静一下——{left} 秒后才能点
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
            需要 {bytes(d.required_bytes)} · 可用{" "}
            {d.available_bytes === null ? "未知" : bytes(d.available_bytes)}{" "}
          </span>
          {d.sufficient === true && <span className="tag t-ok">空间充足</span>}
          {d.sufficient === false && <span className="tag t-bad">空间不足</span>}
          {d.sufficient === null && <span className="tag t-warn">空间无法确认</span>}
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
      <Bar label="拷贝" pct={copy} cls="copy" />
      {showVerify && <Bar label="校验" pct={verify} cls="verify" />}
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
      <header>
        复验结果<span className="n">算法 {r.algorithm}</span>
        <button className="btn sm" style={{ marginLeft: 8 }} onClick={onClose}>
          关闭
        </button>
      </header>
      <div className="in col">
        {r.missing.length === 0 ? (
          <div className="banner ok">数据完好——清单记录的内容全部找得到</div>
        ) : (
          <div className="banner bad">有 {r.missing.length} 个文件丢失</div>
        )}
        {!r.complete && <div className="banner warn">复验被中断，结果不完整</div>}
        {r.unverified_at_copy > 0 && (
          <div className="banner warn">
            其中 {r.unverified_at_copy} 个条目在拷贝时未做校验，可信度较低
          </div>
        )}
        <div className="row" style={{ gap: 16 }}>
          <span>
            <span className="tag t-ok">一致</span> <b className="mono">{r.intact.length}</b>
          </span>
          <span>
            <span className="tag t-warn">已移动</span> <b className="mono">{r.moved.length}</b>
          </span>
          <span>
            <span className="tag t-bad">丢失</span> <b className="mono">{r.missing.length}</b>
          </span>
          <span>
            <span className="tag t-warn">新增</span> <b className="mono">{r.added.length}</b>
          </span>
        </div>
        {r.missing.length > 0 && (
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
        {r.moved.length > 0 && (
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
        {r.added.length > 0 && (
          <div className="small muted">
            另有 {r.added.length} 个清单未记录的文件（多出文件本身不是错误，仅作告知）
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
          <span className="t">拷卡报告</span>
          <span className="path">{manifestPath.replace(/\.json$/, ".html")}</span>
          <div className="r">
            <button className="btn sm" onClick={() => ref.current?.contentWindow?.print()}>
              打印 / 存为 PDF
            </button>
            <button
              className="btn sm"
              onClick={() => api.openReportFile(manifestPath).catch((e) => setError(String(e)))}
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
          <iframe ref={ref} title="拷卡报告" sandbox="allow-same-origin allow-modals" srcDoc={html} />
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
          <span className={c.passed ? "tag t-ok" : "tag t-bad"}>{c.passed ? "通过" : "拦下"}</span>{" "}
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
    const detail = `${this.props.where}：${e.message}\n${e.stack ?? ""}`;
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
