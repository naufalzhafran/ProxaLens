import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage, type RestoreReport, type Status } from "./api";
import { FilterBar } from "./components/FilterBar";
import { FlowTable } from "./components/FlowTable";
import { Inspector } from "./components/Inspector";
import { Onboarding, shouldShowOnboarding } from "./components/Onboarding";
import { Sidebar } from "./components/Sidebar";
import { Toolbar } from "./components/Toolbar";
import { useResizable } from "./components/ui";
import { useToast } from "./lib/clipboard";
import { MAX_FLOWS, useFlows } from "./state/flows";

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<RestoreReport | null>(null);
  const [setupOpen, setSetupOpen] = useState(false);
  const onboardingChecked = useRef(false);
  const filterRef = useRef<HTMLInputElement>(null);
  const toast = useToast((s) => s.message);
  const sidebar = useResizable(200, 140, 400, "x", "sidebarWidth");
  const table = useResizable(640, 320, 2000, "x", "tableWidth");

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
  // refresh periodically to catch certificate trust changes, and refresh at
  // once when the tray menu changes something.
  useEffect(() => {
    let alive = true;
    const tick = () =>
      api
        .getStatus()
        .then((s) => {
          if (!alive) return;
          setStatus(s);
          if (!onboardingChecked.current) {
            onboardingChecked.current = true;
            if (shouldShowOnboarding(s)) setSetupOpen(true);
          }
        })
        .catch(() => {});
    tick();
    const t = setInterval(tick, 3000);
    const unlisten = api.onStatusChanged(tick);
    return () => {
      alive = false;
      clearInterval(t);
      unlisten.then((f) => f()).catch(() => {});
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey) return;
      if (e.key === "f") {
        e.preventDefault();
        filterRef.current?.focus();
        filterRef.current?.select();
      } else if (e.key === "k") {
        e.preventDefault();
        api.clearFlows().catch((err) => setError(errorMessage(err)));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const shownError = error ?? status?.error ?? null;

  return (
    <div className="flex h-full flex-col">
      <Toolbar status={status} setStatus={setStatus} onError={setError} onSetup={() => setSetupOpen(true)} />
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
      {status && !status.caTrusted && !setupOpen && (
        <Banner tone="warn">
          HTTPS traffic can’t be decrypted until the Project Proxy certificate is trusted.{" "}
          <button className="underline" onClick={() => setSetupOpen(true)}>
            Set up…
          </button>
        </Banner>
      )}
      <div className="flex min-h-0 flex-1">
        <div style={{ width: sidebar.size }} className="flex shrink-0 flex-col bg-neutral-50 dark:bg-neutral-900/50">
          <Sidebar />
        </div>
        {sidebar.handle}
        <div style={{ width: table.size }} className="flex min-w-0 shrink-0 flex-col">
          <FilterBar ref={filterRef} />
          <FlowTable />
        </div>
        {table.handle}
        <div className="flex min-w-[280px] flex-1 flex-col">
          <Inspector />
        </div>
      </div>
      {setupOpen && status && (
        <Onboarding status={status} setStatus={setStatus} onClose={() => setSetupOpen(false)} />
      )}
      {toast && (
        <div className="pointer-events-none fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-md bg-neutral-900 px-3 py-1.5 text-xs text-white shadow-lg dark:bg-neutral-100 dark:text-neutral-900">
          {toast}
        </div>
      )}
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
