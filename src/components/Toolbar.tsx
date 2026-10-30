import { useEffect, useState } from "react";
import {
  ChevronDown,
  FolderOpen,
  Gauge,
  PanelBottom,
  PanelLeft,
  PanelRight,
  Pause,
  Play,
  Send,
  Settings as SettingsIcon,
  ShieldAlert,
  Smartphone,
  Trash2,
  Workflow,
} from "lucide-react";
import { api, errorMessage, type Status } from "../api";
import { clearFlows, files, visibleIds } from "../lib/actions";
import { activeRules } from "../lib/rules";
import { useMocks } from "../state/mocks";
import { emptyDraft, useLayout, useSettings, useTools, useUi, type Page } from "../state/ui";
import { THROTTLE_PRESETS } from "./SettingsDialog";
import { button, ContextMenu, HUE_TEXT, IconButton, Popover, primaryButton, useAnchor, type MenuEntry } from "./ui";
import type { Hue } from "../lib/format";

interface Props {
  status: Status | null;
  setStatus: (s: Status) => void;
  onError: (msg: string) => void;
  onSetup: () => void;
}

const ICON = 14;

export function Toolbar({ status, setStatus, onError, onSetup }: Props) {
  const [busy, setBusy] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  const details = useAnchor();
  const settings = useSettings((s) => s.settings);
  const rules = useTools((s) => s.rules);
  const { openComposer, openSettings, openRules, openDevices } = useUi.getState();
  const { page, setPage, sidebarHidden, inspector, toggleSidebar, toggleInspector } = useLayout();
  const mocksOn = useMocks((s) => s.mocks?.filter((m) => m.enabled).length ?? 0);

  useEffect(() => {
    useSettings.getState().refresh().catch((e) => onError(errorMessage(e)));
    useTools.getState().refresh().catch(() => {});
    useMocks.getState().load().catch(() => {});
  }, [onError]);

  function showMenu(e: React.MouseEvent, items: MenuEntry[]) {
    const r = e.currentTarget.getBoundingClientRect();
    setMenu({ x: r.left, y: r.bottom + 4, items });
  }

  const fileMenu: MenuEntry[] = [
    { label: "Open Session or HAR… (⌘O)", onSelect: files.open },
    { label: "Import HAR…", onSelect: files.importHar },
    "separator",
    { label: "Save Session… (⌘S)", onSelect: files.saveSession },
    { label: "Export All as HAR…", onSelect: () => files.exportHar() },
    { label: "Export Filtered as HAR…", onSelect: () => files.exportHar(visibleIds()) },
  ];

  const throttle = settings?.throttle;
  const throttleName = throttle?.enabled ? (THROTTLE_PRESETS[throttle.preset]?.label ?? "Custom") : null;
  function setThrottle(preset: string | null) {
    if (!settings) return;
    const p = preset ? THROTTLE_PRESETS[preset] : null;
    const next = p
      ? { ...settings.throttle, enabled: true, preset: preset!, bytesPerSec: p.bytesPerSec, latencyMs: p.latencyMs }
      : { ...settings.throttle, enabled: false };
    useSettings
      .getState()
      .save({ ...settings, throttle: next })
      .catch((e) => onError(errorMessage(e)));
  }
  const throttleMenu: MenuEntry[] = [
    { label: "Off", checked: !throttle?.enabled, onSelect: () => setThrottle(null) },
    ...Object.entries(THROTTLE_PRESETS).map(([id, p]) => ({
      label: p.label,
      checked: !!throttle?.enabled && throttle.preset === id,
      onSelect: () => setThrottle(id),
    })),
    "separator",
    { label: "Custom…", onSelect: () => openSettings("throttle") },
  ];

  async function run(fn: () => Promise<Status>) {
    setBusy(true);
    try {
      setStatus(await fn());
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  const running = status?.running ?? false;
  const on = status?.systemProxyEnabled ?? false;
  const recording = status?.recording ?? true;
  const active = rules ? activeRules(rules) : [];
  const ruleCount = active.reduce((n, [, c]) => n + c, 0);

  return (
    <div className="flex items-center gap-1.5 border-b border-line bg-panel px-3 py-2.5">
      <div
        role="group"
        aria-label="Recording"
        className="relative grid shrink-0 grid-cols-2 rounded-full bg-ink/6 p-0.5 text-xs font-medium"
      >
        {/* The thumb slides under whichever segment is on. */}
        <span
          aria-hidden
          className={`absolute inset-y-0.5 left-0.5 w-[calc(50%-2px)] rounded-full shadow-thumb transition-[translate,background-color,box-shadow] duration-200 ease-out motion-reduce:transition-none ${
            recording ? "translate-x-0 bg-accent" : "translate-x-full bg-panel"
          } ${status ? "" : "opacity-50"}`}
        />
        <button
          className={`relative flex items-center justify-center gap-1.5 rounded-full px-2.5 py-0.5 transition-colors duration-200 disabled:opacity-50 ${recording ? "text-on-accent" : "text-muted hover:text-ink"}`}
          disabled={!status}
          aria-pressed={recording}
          title="Record: list new traffic as it arrives"
          onClick={() => !recording && run(() => api.setRecording(true))}
        >
          <Play size={ICON} />
          <span className="hidden lg:inline">Recording</span>
        </button>
        <button
          className={`relative flex items-center justify-center gap-1.5 rounded-full px-2.5 py-0.5 transition-colors duration-200 disabled:opacity-50 ${recording ? "text-muted hover:text-ink" : "text-st-client"}`}
          disabled={!status}
          aria-pressed={!recording}
          title="Pause recording: traffic still flows and rules still apply, but nothing new is listed"
          onClick={() => recording && run(() => api.setRecording(false))}
        >
          <Pause size={ICON} />
          <span className="hidden lg:inline">Paused</span>
        </button>
      </div>

      <button
        role="switch"
        aria-checked={on}
        disabled={busy || (!running && !on)}
        onClick={() => run(() => api.setSystemProxy(!on))}
        title={on ? "Stop routing this Mac’s traffic through the proxy" : "Route this Mac’s traffic through the proxy"}
        className="flex shrink-0 items-center gap-2 rounded-md px-1 py-0.5 text-xs font-medium disabled:opacity-50"
      >
        <span
          className={`relative h-[18px] w-8 rounded-full transition-colors duration-200 motion-reduce:transition-none ${
            on ? "bg-accent" : "bg-ink/12"
          }`}
        >
          <span
            className={`absolute top-0.5 left-0.5 h-3.5 w-3.5 rounded-full bg-panel transition-[translate,box-shadow] duration-200 ease-out motion-reduce:transition-none ${
              on ? "translate-x-3.5" : "translate-x-0 ring-1 ring-ink/30"
            }`}
          />
        </span>
        System Proxy
      </button>

      <StatusPill status={status} onClick={details.open} />

      <Divider />
      <PageTabs page={page} setPage={setPage} mocksOn={mocksOn} />

      <Divider />
      <button className={`${button} flex items-center gap-1`} aria-haspopup="menu" onClick={(e) => showMenu(e, fileMenu)}>
        <FolderOpen size={ICON} className={HUE_TEXT.teal} />
        File
        <ChevronDown size={12} />
      </button>
      <button className={`${button} flex items-center gap-1`} title="Compose a new request (⌘N)" onClick={() => openComposer(emptyDraft)}>
        <Send size={ICON} className={HUE_TEXT.blue} />
        <span className="hidden lg:inline">Compose</span>
      </button>

      <Divider />
      <button
        className={`${button} flex items-center gap-1 ${throttleName ? "bg-st-client/12! text-st-client" : ""}`}
        disabled={!settings}
        aria-haspopup="menu"
        title="Network throttle"
        onClick={(e) => showMenu(e, throttleMenu)}
      >
        <Gauge size={ICON} className={throttleName ? "" : HUE_TEXT.orange} />
        {throttleName ?? <span className="hidden lg:inline">Throttle</span>}
        <ChevronDown size={12} />
      </button>
      <button
        className={`${button} flex items-center gap-1`}
        title={`Map Local, Map Remote, Rewrite, Block (⌘L)${active.length ? `\nActive: ${active.map(([k, n]) => `${n} ${k}`).join(", ")}` : ""}`}
        onClick={() => openRules({ tab: "mapLocal" })}
      >
        <Workflow size={ICON} className={HUE_TEXT.violet} />
        Rules
        <Badge n={ruleCount} hue="violet" />
      </button>

      <div className="ml-auto flex items-center gap-1.5">
        <Cluster>
        <IconButton
          label={sidebarHidden ? "Show sidebar (⌘B)" : "Hide sidebar (⌘B)"}
          className={sidebarHidden ? "" : "text-ink"}
          onClick={toggleSidebar}
        >
          <PanelLeft size={ICON} />
        </IconButton>
        <IconButton
          label={inspector === "bottom" ? "Move inspector to the right" : "Move inspector to the bottom"}
          onClick={toggleInspector}
        >
          {inspector === "bottom" ? <PanelRight size={ICON} /> : <PanelBottom size={ICON} />}
        </IconButton>
        </Cluster>
        <Cluster>
        <IconButton label="Devices: phones, tablets and Simulators" onClick={() => openDevices(true)}>
          <Smartphone size={ICON} />
        </IconButton>
        <IconButton label="Settings (⌘,)" onClick={() => openSettings("general")}>
          <SettingsIcon size={ICON} />
        </IconButton>
        <IconButton label="Clear all flows (⌘K)" onClick={() => clearFlows().catch((e) => onError(errorMessage(e)))}>
          <Trash2 size={ICON} />
        </IconButton>
        </Cluster>
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      {details.at && status && (
        <Popover x={details.at.x} y={details.at.y} label="Proxy status" onClose={details.close}>
          <StatusDetails
            status={status}
            busy={busy}
            run={run}
            onSetup={() => {
              details.close();
              onSetup();
            }}
            onSettings={() => {
              details.close();
              openSettings("general");
            }}
          />
        </Popover>
      )}
    </div>
  );
}

function Divider() {
  return <div className="mx-0.5 h-5 w-px shrink-0 bg-line" />;
}

/** Traffic or Mocks (⌘1, ⌘2). */
function PageTabs({ page, setPage, mocksOn }: { page: Page; setPage: (p: Page) => void; mocksOn: number }) {
  const tabs: { id: Page; label: string; title: string }[] = [
    { id: "traffic", label: "Traffic", title: "Captured requests (⌘1)" },
    { id: "mocks", label: "Mocks", title: "Mocked APIs (⌘2)" },
  ];
  return (
    <div role="tablist" aria-label="Page" className="flex shrink-0 items-center gap-0.5 rounded-full bg-ink/6 p-0.5">
      {tabs.map((t) => (
        <button
          key={t.id}
          role="tab"
          aria-selected={t.id === page}
          title={t.title}
          onClick={() => setPage(t.id)}
          className={`flex items-center gap-1 rounded-full px-3 py-0.5 text-xs font-medium transition-colors ${
            t.id === page ? "bg-panel text-ink shadow-thumb" : "text-muted hover:text-ink"
          }`}
        >
          {t.label}
          {t.id === "mocks" && <Badge n={mocksOn} hue="violet" />}
        </button>
      ))}
    </div>
  );
}

/** Icon buttons sharing one rounded pill, as a group. */
function Cluster({ children }: { children: React.ReactNode }) {
  return <div className="flex items-center gap-0.5 rounded-full bg-ink/5 p-0.5">{children}</div>;
}

const BADGE: Record<Hue, string> = {
  blue: "bg-hue-blue",
  green: "bg-hue-green",
  pink: "bg-hue-pink",
  orange: "bg-hue-orange",
  yellow: "bg-hue-yellow",
  teal: "bg-hue-teal",
  violet: "bg-hue-violet",
  red: "bg-hue-red",
  gray: "bg-hue-gray",
};

function Badge({ n, hue }: { n: number; hue: Hue }) {
  if (!n) return null;
  return <span className={`rounded-full px-1.5 text-[10px] font-semibold leading-4 text-on-hue ${BADGE[hue]}`}>{n}</span>;
}

/** Address, plus a warning when HTTPS can't be decrypted; opens the details. */
function StatusPill({ status, onClick }: { status: Status | null; onClick: (e: React.MouseEvent) => void }) {
  const running = status?.running ?? false;
  const untrusted = status ? !status.caTrusted : false;
  return (
    <button
      className={`${button} flex min-w-0 items-center gap-1.5 font-mono font-normal ${untrusted ? "bg-st-client/12! text-st-client" : ""}`}
      aria-haspopup="dialog"
      title="Listener, certificate and system proxy"
      disabled={!status}
      onClick={onClick}
    >
      <span className={`h-2 w-2 shrink-0 rounded-full ${running ? "bg-st-ok shadow-[0_0_0_3px] shadow-st-ok/20" : "bg-muted"}`} />
      <span className="truncate">{running ? status?.addr : "Stopped"}</span>
      {untrusted && <ShieldAlert size={ICON} className="shrink-0 text-st-client" aria-label="Certificate not trusted" />}
      <ChevronDown size={12} className="shrink-0" />
    </button>
  );
}

function StatusDetails({
  status,
  busy,
  run,
  onSetup,
  onSettings,
}: {
  status: Status;
  busy: boolean;
  run: (fn: () => Promise<Status>) => void;
  onSetup: () => void;
  onSettings: () => void;
}) {
  return (
    <div className="divide-y divide-line">
      <Row
        title="Listener"
        detail={status.running ? `Listening on ${status.addr}` : `Stopped (port ${status.port})`}
        ok={status.running}
        action={
          <button className={button} disabled={busy} onClick={() => run(status.running ? api.stopProxy : () => api.startProxy())}>
            {status.running ? "Stop" : "Start"}
          </button>
        }
      />
      <Row
        title="HTTPS certificate"
        detail={status.caTrusted ? "Trusted; HTTPS traffic is decrypted" : "Not trusted; HTTPS can’t be decrypted"}
        ok={status.caTrusted}
        warn={!status.caTrusted}
        action={
          <button className={status.caTrusted ? button : primaryButton} onClick={onSetup}>
            {status.caTrusted ? "Setup Guide…" : "Set Up…"}
          </button>
        }
      />
      <Row
        title="System proxy"
        detail={status.systemProxyEnabled ? "This Mac’s traffic goes through the proxy" : "Off; only apps pointed at the proxy are captured"}
        ok={status.systemProxyEnabled}
        action={
          <button
            className={button}
            disabled={busy || (!status.running && !status.systemProxyEnabled)}
            onClick={() => run(() => api.setSystemProxy(!status.systemProxyEnabled))}
          >
            {status.systemProxyEnabled ? "Switch Off" : "Switch On"}
          </button>
        }
      />
      <Row
        title="Recording"
        detail={status.recording ? "New traffic is listed" : "Paused; traffic still flows and rules still apply"}
        ok={status.recording}
        warn={!status.recording}
        action={
          <button className={button} onClick={() => run(() => api.setRecording(!status.recording))}>
            {status.recording ? "Pause" : "Resume"}
          </button>
        }
      />
      <div className="flex items-center justify-between px-3 py-2 text-muted">
        <span>Port {status.port}</span>
        <button className="text-accent hover:underline" onClick={onSettings}>
          Change in Settings…
        </button>
      </div>
    </div>
  );
}

function Row({
  title,
  detail,
  ok,
  warn,
  action,
}: {
  title: string;
  detail: string;
  ok: boolean;
  warn?: boolean;
  action: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-2 px-3 py-2">
      <span className={`h-2 w-2 shrink-0 rounded-full ${warn ? "bg-st-client" : ok ? "bg-st-ok" : "bg-muted"}`} />
      <div className="min-w-0 flex-1">
        <div className="font-medium">{title}</div>
        <div className="text-muted">{detail}</div>
      </div>
      {action}
    </div>
  );
}
