import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage, type RestoreReport, type Status } from "./api";
import { Composer } from "./components/Composer";
import { DevicesDialog } from "./components/DevicesDialog";
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
import {
  fitSidebar,
  fitTable,
  fitTableHeight,
  INSPECTOR_MIN,
  INSPECTOR_MIN_HEIGHT,
  SIDEBAR_MIN,
  TABLE_MIN,
  TABLE_MIN_HEIGHT,
} from "./lib/layout";
import { MAX_FLOWS, useFlows } from "./state/flows";
import { emptyDraft, useLayout, useUi } from "./state/ui";
import { useUpdate } from "./state/update";

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<RestoreReport | null>(null);
  const [setupOpen, setSetupOpen] = useState(false);
  const onboardingChecked = useRef(false);
  const filterRef = useRef<HTMLInputElement>(null);
  const toast = useToast((s) => s.message);
  const composer = useUi((s) => s.composer);
  const settingsTab = useUi((s) => s.settingsTab);
  const rulesView = useUi((s) => s.rules);
  const devicesOpen = useUi((s) => s.devicesOpen);
  const update = useUpdate();
  const sidebarHidden = useLayout((s) => s.sidebarHidden);
  const bottom = useLayout((s) => s.inspector === "bottom");
  const [panes, panesSize] = useSize();
  const sidebar = useResizable(200, SIDEBAR_MIN, 400, "x", "sidebarWidth", (w) =>
    fitSidebar(panesSize.width, w, !bottom),
  );
  const table = useResizable(760, TABLE_MIN, 2000, "x", "tableWidth", (w) =>
    fitTable(panesSize.width, sidebarHidden ? 0 : sidebar.size, w),
  );
  const tableHeight = useResizable(320, TABLE_MIN_HEIGHT, 2000, "y", "tableHeight", (h) =>
    fitTableHeight(panesSize.height, h),
  );

  const reload = useCallback(async () => {
    const { flows } = await api.listFlows(0, MAX_FLOWS);
    useFlows.getState().reset(flows);
  }, []);

  // Dev builds aren't installed apps, so there is nothing to update.
  useEffect(() => {
    if (!import.meta.env.DEV) useUpdate.getState().check();
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
      if (!e.metaKey || e.shiftKey || e.altKey || e.repeat) return;
      // Leave shortcuts to the dialog while one is open (setup included), so
      // ⌘K can't clear traffic behind it.
      if (document.querySelector('[role="dialog"][aria-modal="true"]')) return;
      const ui = useUi.getState();
      const selected = useFlows.getState().selectedId;
      const actions: Record<string, () => void> = {
        // Find in the body of the inspector pane last clicked, else filter flows.
        f: () => {
          const scope = document.activeElement?.closest("[data-find-scope]");
          const target = scope?.querySelector<HTMLInputElement>("[data-find]") ?? filterRef.current;
          target?.focus();
          target?.select();
        },
        k: () => clearFlows().catch((err) => setError(errorMessage(err))),
        o: () => files.open(),
        s: () => files.saveSession(),
        n: () => ui.openComposer(emptyDraft),
        r: () => selected !== null && repeat(selected),
        ",": () => ui.openSettings("general"),
        l: () => ui.openRules({ tab: "mapLocal" }),
        b: () => useLayout.getState().toggleSidebar(),
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
      {update.update && !update.dismissed && (update.phase === "available" || update.phase === "installing") && (
        <Banner tone="info" onClose={update.phase === "available" ? update.dismiss : undefined}>
          {update.phase === "installing" ? (
            <>
              Installing ProxaLens v{update.update.version}
              {update.progress !== null && ` (${Math.round(update.progress * 100)}%)`}… The app restarts when it’s done
              {status?.systemProxyEnabled && ", and the system proxy is turned off"}.
            </>
          ) : (
            <>
              ProxaLens v{update.update.version} is available.{" "}
              <button className="underline" onClick={update.install}>
                Install and restart
              </button>
            </>
          )}
        </Banner>
      )}
      {shownError && (
        <Banner tone="error" onClose={() => setError(null)}>
          {shownError}
        </Banner>
      )}
      {status && !status.recording && (
        <Banner tone="warn">
          Recording is paused.{" "}
          {status.systemProxyEnabled
            ? "This Mac’s traffic still goes through the proxy and rules still apply, but nothing new is listed."
            : "Apps pointed at the proxy still go through it and rules still apply, but nothing new is listed."}{" "}
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
          HTTPS traffic can’t be decrypted until the ProxaLens certificate is trusted.{" "}
          <button className="underline" onClick={() => setSetupOpen(true)}>
            Set up…
          </button>
        </Banner>
      )}
      <div ref={panes} className="flex min-h-0 flex-1">
        {!sidebarHidden && (
          <>
            <div style={{ width: sidebar.size }} className="flex shrink-0 flex-col bg-surface">
              <Sidebar />
            </div>
            {sidebar.handle}
          </>
        )}
        {bottom ? (
          <div className="flex min-w-0 flex-1 flex-col">
            <div style={{ height: tableHeight.size }} className="flex min-h-0 shrink-0 flex-col bg-panel">
              <FilterBar ref={filterRef} />
              <FlowTable />
            </div>
            {tableHeight.handle}
            <div style={{ minHeight: INSPECTOR_MIN_HEIGHT }} className="flex min-h-0 flex-1 flex-col">
              <Inspector sideBySide />
            </div>
          </div>
        ) : (
          <>
            <div style={{ width: table.size }} className="flex min-w-0 shrink-0 flex-col bg-panel">
              <FilterBar ref={filterRef} />
              <FlowTable />
            </div>
            {table.handle}
            <div style={{ minWidth: INSPECTOR_MIN }} className="flex min-w-0 flex-1 flex-col">
              <Inspector />
            </div>
          </>
        )}
      </div>
      {setupOpen && status && (
        <Onboarding status={status} setStatus={setStatus} onClose={() => setSetupOpen(false)} />
      )}
      {composer && <Composer initial={composer} />}
      {settingsTab && <SettingsDialog initialTab={settingsTab} />}
      {rulesView && <RulesDialog view={rulesView} />}
      {devicesOpen && <DevicesDialog />}
      {/* Always mounted, so screen readers announce each new message. */}
      <div role="status" aria-live="polite" className="pointer-events-none fixed bottom-4 left-1/2 z-50 -translate-x-1/2">
        {toast && (
          <div className="rounded-lg bg-ink px-3 py-1.5 text-xs text-panel">
            {toast}
          </div>
        )}
      </div>
    </div>
  );
}

/** A ref to attach, and the element's current size (0 until measured). */
function useSize(): [(el: HTMLDivElement | null) => void, { width: number; height: number }] {
  const [size, setSize] = useState({ width: 0, height: 0 });
  const ref = useCallback((el: HTMLDivElement | null) => {
    if (!el) return;
    const observer = new ResizeObserver(([entry]) =>
      setSize({ width: entry.contentRect.width, height: entry.contentRect.height }),
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return [ref, size];
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
    info: "bg-select text-ink",
    warn: "bg-st-client/12 text-ink",
    error: "bg-st-server/12 text-ink",
  }[tone];
  return (
    <div role={tone === "error" ? "alert" : "status"} className={`flex items-center gap-2 px-3 py-1.5 text-xs ${colors}`}>
      <span className="flex-1">{children}</span>
      {onClose && (
        <button className="opacity-60 hover:opacity-100" onClick={onClose} aria-label="Dismiss">
          ✕
        </button>
      )}
    </div>
  );
}
