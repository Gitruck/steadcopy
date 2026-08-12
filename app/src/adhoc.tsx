import { useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  api,
  type AdhocDefaults,
  type Arrival,
  type DeviceKind,
  type SinkScope,
  type SinkSuggestion,
} from "./bridge";
import { t } from "./i18n";

/**
 * 临时拷贝面板：不依赖预设的一次性任务。
 *
 * 设计要点：**目的地是唯一必填**。项目、校验、算法都有能直接用的默认值，
 * 用户可以一路回车过去——「不强制」的意思是有默认值，不是可以为空。
 */
export function AdhocPanel({
  device,
  projects,
  onPlanned,
  onCancel,
  onError,
}: {
  device: { root: string; name: string };
  projects: { id: string; name: string }[];
  onPlanned: (a: Arrival) => void;
  onCancel: () => void;
  onError: (e: string) => void;
}) {
  const [d, setD] = useState<AdhocDefaults | null>(null);
  const [projectId, setProjectId] = useState<string | null>(null);
  const [projectName, setProjectName] = useState("");
  const [dests, setDests] = useState<string[]>([]);
  const [verify, setVerify] = useState(true);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.adhocPrefill().then(
      (p) => {
        setD(p);
        setProjectId(p.project_id);
        setProjectName(p.project_name);
        setDests(p.destinations);
        setVerify(p.verify);
      },
      (e) => onError(String(e))
    );
  }, [onError]);

  if (!d) return <div className="empty">{t("adhoc.preparing")}</div>;

  const creating = projectId === null;

  return (
    <div className="panel arrival">
      <header>
        {t("adhoc.title")}<span className="n">{device.name}</span>
      </header>
      <div className="in col">
        <div className="small muted">
          {t("adhoc.intro")}
        </div>

        <div className="field">
          <label>{t("adhoc.destLabel")}</label>
          {dests.length === 0 && (
            <div className="small dim">{t("adhoc.destEmpty")}</div>
          )}
          {dests.map((x, i) => (
            <div key={x} className="row" style={{ gap: 6, alignItems: "center" }}>
              <span className="path grow">{x}</span>
              <button
                className="btn sm danger"
                onClick={() => setDests(dests.filter((_, j) => j !== i))}
              >
                {t("adhoc.remove")}
              </button>
            </div>
          ))}
          <div>
            <button
              className="btn sm"
              disabled={dests.length >= 4}
              onClick={async () => {
                const p = await openDialog({ directory: true, title: "拷到哪儿" });
                if (typeof p === "string" && !dests.includes(p)) setDests([...dests, p]);
              }}
            >
              {t("adhoc.addDest")}
            </button>
          </div>
        </div>

        <div className="field" style={{ maxWidth: 340 }}>
          <label>{t("adhoc.project")}</label>
          <select
            value={projectId ?? "__new__"}
            onChange={(e) => {
              const v = e.target.value;
              if (v === "__new__") {
                setProjectId(null);
                setProjectName(d.project_name);
              } else {
                setProjectId(v);
                setProjectName(projects.find((p) => p.id === v)?.name ?? "");
              }
            }}
          >
            {projects.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
            <option value="__new__">{t("adhoc.newProject")}</option>
          </select>
          {creating && (
            <>
              <input value={projectName} onChange={(e) => setProjectName(e.target.value)} />
              <span className="small dim">
                {t("adhoc.willCreate")}
              </span>
            </>
          )}
        </div>

        <label className="small row" style={{ gap: 5, alignItems: "center" }}>
          <input type="checkbox" checked={verify} onChange={(e) => setVerify(e.target.checked)} />
          {t("adhoc.verify")}
        </label>
        {!verify && (
          <div className="banner warn">
            {t("adhoc.noVerifyWarn")}
          </div>
        )}

        <div className="row" style={{ gap: 8 }}>
          <button
            className="btn primary"
            disabled={busy || dests.length === 0 || (creating && !projectName.trim())}
            onClick={async () => {
              setBusy(true);
              try {
                onPlanned(
                  await api.planAdhoc({
                    device_root: device.root,
                    project_id: projectId,
                    project_name: projectName.trim(),
                    destinations: dests,
                    verify,
                    algorithm: d.algorithm,
                    eject_after: false,
                  })
                );
              } catch (e) {
                onError(String(e));
              } finally {
                setBusy(false);
              }
            }}
          >
            {t("adhoc.next")}
          </button>
          <button className="btn" onClick={onCancel}>
            {t("app.cancel")}
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * 沉淀提示条：行内、不抢焦点。
 *
 * 时机是刻意的——任务一开跑就挂上，结束后仍留着。用户可能正盯着进度，
 * 也可能拷完就去拔卡了；只在某一瞬间给机会等于没给。
 */
export function SinkBar({
  s,
  onDone,
  onDismiss,
  onError,
}: {
  s: SinkSuggestion;
  onDone: () => void;
  onDismiss: () => void;
  onError: (e: string) => void;
}) {
  const [scope, setScope] = useState<SinkScope>("device");
  const [kind, setKind] = useState<DeviceKind>("camera");
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);

  if (done) {
    return (
      <div className="banner ok">
        {t("sink.done")}
      </div>
    );
  }

  const line =
    s.kind === "diverged"
      ? t("sink.askDiverged", {
          preset: s.preset_name ?? "",
          changed: s.changed.join("、"),
        })
      : t("sink.askNew", { device: s.device_name, project: s.project_name });

  return (
    <div className="sink">
      <div className="grow">
        {/* 措辞是复述他刚做的事，不是「保存为预设」这种功能名 */}
        <div>{line}</div>
        <div className="row small muted" style={{ gap: 8, marginTop: 6, alignItems: "center" }}>
          <span>{t("sink.scope")}</span>
          <select value={scope} onChange={(e) => setScope(e.target.value as SinkScope)}>
            <option value="device">{s.default_scope_label}</option>
            <option value="kind">{t("sink.scopeKind")}</option>
            <option value="any">{t("sink.scopeAny")}</option>
          </select>
          {s.needs_kind && (
            <>
              <span>{t("sink.alsoAs")}</span>
              <select value={kind} onChange={(e) => setKind(e.target.value as DeviceKind)}>
                <option value="camera">摄影卡</option>
                <option value="recorder">录音卡</option>
                <option value="storage">素材盘</option>
              </select>
            </>
          )}
        </div>
      </div>
      <div className="row" style={{ gap: 6 }}>
        <button
          className="btn sm primary"
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            try {
              await api.sinkPreset(scope, s.needs_kind ? kind : undefined);
              setDone(true);
              onDone();
            } catch (e) {
              onError(String(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          {t("sink.remember")}
        </button>
        <button className="btn sm" onClick={onDismiss}>
          {t("sink.no")}
        </button>
      </div>
    </div>
  );
}
