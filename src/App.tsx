import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage, type RestoreReport, type Status } from "./api";
import { BreakpointDialog, useBreakpoints } from "./components/BreakpointDialog";
import { Composer } from "./components/Composer";
import { DevicesDialog } from "./components/DevicesDialog";
import { DiffView } from "./components/DiffView";
import { FilterBar } from "./components/FilterBar";
import { FlowTable } from "./components/FlowTable";
import { Inspector } from "./components/Inspector";
import { Onboarding, shouldShowOnboarding } from "./components/Onboarding";
import { RulesDialog } from "./components/RulesDialog";
import { SettingsDialog } from "./components/SettingsDialog";
import { Sidebar } from "./components/Sidebar";
import { Toolbar } from "./components/Toolbar";
import { useResizable } from "./components/ui";
import { clearFlows, files, repeat } from "./lib/actions";
import { useToast } from "./lib/clipboard";
import { fitSidebar, fitTable, INSPECTOR_MIN, SIDEBAR_MIN, TABLE_MIN } from "./lib/layout";
import { MAX_FLOWS, useFlows } from "./state/flows";
import { emptyDraft, useUi } from "./state/ui";

const ScriptsWindow = lazy(() => import("./scripts/ScriptsWindow"));

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<RestoreReport | null>(null);
  const [setupOpen, setSetupOpen] = useState(false);
  const onboardingChecked = useRef(false);
  const filterRef = useRef<HTMLInputElement>(null);
  const toast = useToast((s) => s.message);
  const composer = useUi((s) => s.composer);
  const diff = useUi((s) => s.diff);
  const settingsTab = useUi((s) => s.settingsTab);
  const rulesView = useUi((s) => s.rules);
  const scriptsOpen = useUi((s) => s.scriptsOpen);
  const devicesOpen = useUi((s) => s.devicesOpen);
  const hits = useBreakpoints();
  const [panes, panesWidth] = useWidth();
  const sidebar = useResizable(200, SIDEBAR_MIN, 400, "x", "sidebarWidth", (w) => fitSidebar(panesWidth, w));
  const table = useResizable(760, TABLE_MIN, 2000, "x", "tableWidth", (w) => fitTable(panesWidth, sidebar.size, w));

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
      if (!e.metaKey || e.shiftKey || e.altKey) return;
      // Leave shortcuts to the dialog while one is open (setup, breakpoints and
      // the Scripts editor included), so ⌘K can't clear traffic behind it.
      if (document.querySelector('[role="dialog"][aria-modal="true"]')) return;
      const ui = useUi.getState();
      const selected = useFlows.getState().selectedId;
      const actions: Record<string, () => void> = {
        f: () => {
          filterRef.current?.focus();
          filterRef.current?.select();
        },
        k: () => clearFlows().catch((err) => setError(errorMessage(err))),
        o: () => files.open(),
        s: () => files.saveSession(),
        n: () => ui.openComposer(emptyDraft),
        r: () => selected !== null && repeat(selected),
        ",": () => ui.openSettings("general"),
        l: () => ui.openRules({ tab: "mapLocal" }),
        j: () => ui.openScripts(true),
      };
      const action = actions[e.key];
      if (action) {
        e.preventDefault();
        action();
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
      {status && !status.recording && (
        <Banner tone="warn">
          Recording is paused. Traffic still goes through the proxy and rules still apply, but nothing new is listed.{" "}
          <button
            className="underline"
            onClick={() => api.setRecording(true).then(setStatus).catch((e) => setError(errorMessage(e)))}
          >
            Resume
          </button>
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
      <div ref={panes} className="flex min-h-0 flex-1">
        <div style={{ width: sidebar.size }} className="flex shrink-0 flex-col bg-neutral-50 dark:bg-neutral-900/50">
          <Sidebar />
        </div>
        {sidebar.handle}
        <div style={{ width: table.size }} className="flex min-w-0 shrink-0 flex-col">
          <FilterBar ref={filterRef} />
          <FlowTable />
        </div>
        {table.handle}
        <div style={{ minWidth: INSPECTOR_MIN }} className="flex min-w-0 flex-1 flex-col">
          <Inspector />
        </div>
      </div>
      {setupOpen && status && (
        <Onboarding status={status} setStatus={setStatus} onClose={() => setSetupOpen(false)} />
      )}
      {composer && <Composer initial={composer} />}
      {diff && <DiffView ids={diff} />}
      {settingsTab && <SettingsDialog initialTab={settingsTab} />}
      {rulesView && <RulesDialog view={rulesView} />}
      {devicesOpen && <DevicesDialog />}
      {scriptsOpen && (
        <Suspense fallback={null}>
          <ScriptsWindow />
        </Suspense>
      )}
      {hits.length > 0 && <BreakpointDialog hits={hits} />}
      {toast && (
        <div className="pointer-events-none fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-md bg-neutral-900 px-3 py-1.5 text-xs text-white shadow-lg dark:bg-neutral-100 dark:text-neutral-900">
          {toast}
        </div>
      )}
    </div>
  );
}

/** A ref to attach, and the element's current width (0 until measured). */
function useWidth(): [(el: HTMLDivElement | null) => void, number] {
  const [width, setWidth] = useState(0);
  const ref = useCallback((el: HTMLDivElement | null) => {
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return [ref, width];
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
