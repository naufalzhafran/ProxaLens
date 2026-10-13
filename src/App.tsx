import { useCallback, useEffect, useState } from "react";
import { api, errorMessage, type RestoreReport, type Status } from "./api";
import { FlowDetailPanel } from "./components/FlowDetailPanel";
import { FlowTable } from "./components/FlowTable";
import { Toolbar } from "./components/Toolbar";
import { MAX_FLOWS, useFlows } from "./state/flows";

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<RestoreReport | null>(null);

  const reload = useCallback(async () => {
    const { flows } = await api.listFlows(0, MAX_FLOWS);
    useFlows.getState().reset(flows);
  }, []);

  useEffect(() => {
    api.takeRestoreNotice().then(setNotice).catch(() => {});
    reload()
      .then(() =>
        api.subscribeFlows((b) => {
          if (b.type === "resync") reload();
          else useFlows.getState().applyBatch(b);
        }),
      )
      .catch((e) => setError(errorMessage(e)));
  }, [reload]);

  // The proxy auto-starts in the background; poll status until it settles,
  // and refresh periodically to catch certificate trust changes.
  useEffect(() => {
    let alive = true;
    const tick = () => api.getStatus().then((s) => alive && setStatus(s)).catch(() => {});
    tick();
    const t = setInterval(tick, 3000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, []);

  const shownError = error ?? status?.error ?? null;

  return (
    <div className="flex h-full flex-col">
      <Toolbar status={status} setStatus={setStatus} onError={setError} />
      {notice && (
        <Banner tone="info" onClose={() => setNotice(null)}>
          Restored your network proxy settings from a previous session ({notice.restored.join(", ")}).
        </Banner>
      )}
      {shownError && (
        <Banner tone="error" onClose={() => setError(null)}>
          {shownError}
        </Banner>
      )}
      {status && !status.caTrusted && (
        <Banner tone="warn">
          HTTPS traffic can’t be decrypted until the Project Proxy certificate is trusted. Click “Install
          certificate…”.
        </Banner>
      )}
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-[3] flex-col border-r border-neutral-200 dark:border-neutral-800">
          <FlowTable />
        </div>
        <div className="flex min-w-0 flex-[2] flex-col">
          <FlowDetailPanel />
        </div>
      </div>
    </div>
  );
}

function Banner({
  tone,
  children,
  onClose,
}: {
  tone: "info" | "warn" | "error";
  children: React.ReactNode;
  onClose?: () => void;
}) {
  const colors = {
    info: "bg-sky-50 text-sky-900 dark:bg-sky-950 dark:text-sky-200",
    warn: "bg-amber-50 text-amber-900 dark:bg-amber-950 dark:text-amber-200",
    error: "bg-red-50 text-red-900 dark:bg-red-950 dark:text-red-200",
  }[tone];
  return (
    <div className={`flex items-center gap-2 px-3 py-1.5 text-xs ${colors}`}>
      <span className="flex-1">{children}</span>
      {onClose && (
        <button className="opacity-60 hover:opacity-100" onClick={onClose} aria-label="Dismiss">
          ✕
        </button>
      )}
    </div>
  );
}
