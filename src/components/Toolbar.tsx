import { useEffect, useState } from "react";
import {
  ChevronDown,
  Code,
  FolderOpen,
  Gauge,
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
import { emptyDraft, useSettings, useTools, useUi } from "../state/ui";
import { THROTTLE_PRESETS } from "./SettingsDialog";
import { button, ContextMenu, IconButton, Popover, primaryButton, useAnchor, type MenuEntry } from "./ui";

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
  const scripts = useTools((s) => s.scripts);
  const { openComposer, openSettings, openRules, openScripts, openDevices } = useUi.getState();

  useEffect(() => {
    useSettings.getState().refresh().catch((e) => onError(errorMessage(e)));
    // Rules can also change through the MCP server while the app is in the
    // background, so look again whenever the window comes back.
    const refresh = () => useTools.getState().refresh().catch(() => {});
    refresh();
    const unlisten = api.onScriptsChanged(refresh);
    window.addEventListener("focus", refresh);
    return () => {
      window.removeEventListener("focus", refresh);
      unlisten.then((f) => f()).catch(() => {});
    };
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
  const scriptCount = scripts.filter((s) => s.enabled).length;

  return (
    <div className="flex items-center gap-1.5 border-b border-line bg-panel px-3 py-2">
      <div
        role="group"
        aria-label="Recording"
        className="relative grid shrink-0 grid-cols-2 rounded-lg bg-ink/6 p-0.5 text-xs font-medium"
      >
        {/* The thumb slides under whichever segment is on. */}
        <span
          aria-hidden
          className={`absolute inset-y-0.5 left-0.5 w-[calc(50%-2px)] rounded-md transition-[translate,background-color,box-shadow] duration-200 ease-out motion-reduce:transition-none ${
            recording ? "translate-x-0 bg-accent" : "translate-x-full bg-panel ring-1 ring-line"
          } ${status ? "" : "opacity-50"}`}
        />
        <button
          className={`relative flex items-center justify-center gap-1.5 rounded-md px-2.5 py-0.5 transition-colors duration-200 disabled:opacity-50 ${recording ? "text-on-accent" : "text-muted hover:text-ink"}`}
          disabled={!status}
          aria-pressed={recording}
          title="Record: list new traffic as it arrives"
          onClick={() => !recording && run(() => api.setRecording(true))}
        >
          <Play size={ICON} />
          <span className="hidden lg:inline">Recording</span>
        </button>
        <button
          className={`relative flex items-center justify-center gap-1.5 rounded-md px-2.5 py-0.5 transition-colors duration-200 disabled:opacity-50 ${recording ? "text-muted hover:text-ink" : "text-st-client"}`}
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
      <button className={`${button} flex items-center gap-1`} aria-haspopup="menu" onClick={(e) => showMenu(e, fileMenu)}>
        <FolderOpen size={ICON} />
        File
        <ChevronDown size={12} />
      </button>
      <button className={`${button} flex items-center gap-1`} title="Compose a new request (⌘N)" onClick={() => openComposer(emptyDraft)}>
        <Send size={ICON} />
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
        <Gauge size={ICON} />
        {throttleName ?? <span className="hidden lg:inline">Throttle</span>}
        <ChevronDown size={12} />
      </button>
      <button
        className={`${button} flex items-center gap-1`}
        title={`Map Local, Map Remote, Rewrite, Block, Breakpoints (⌘L)${active.length ? `\nActive: ${active.map(([k, n]) => `${n} ${k}`).join(", ")}` : ""}`}
        onClick={() => openRules({ tab: "mapLocal" })}
      >
        <Workflow size={ICON} />
        Rules
        <Badge n={ruleCount} />
      </button>
      <button
        className={`${button} flex items-center gap-1`}
        title={`JavaScript onRequest / onResponse hooks (⌘J)${scriptCount ? `\n${scriptCount} enabled` : ""}`}
        onClick={() => openScripts(true)}
      >
        <Code size={ICON} />
        Scripts
        <Badge n={scriptCount} />
      </button>

      <div className="ml-auto flex items-center gap-1.5">
        <IconButton label="Devices: phones, tablets and Simulators" onClick={() => openDevices(true)}>
          <Smartphone size={ICON} />
        </IconButton>
        <IconButton label="Settings (⌘,)" onClick={() => openSettings("general")}>
          <SettingsIcon size={ICON} />
        </IconButton>
        <IconButton label="Clear all flows (⌘K)" onClick={() => clearFlows().catch((e) => onError(errorMessage(e)))}>
          <Trash2 size={ICON} />
        </IconButton>
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

function Badge({ n }: { n: number }) {
  if (!n) return null;
  return <span className="rounded-full bg-accent px-1.5 text-[10px] font-semibold leading-4 text-on-accent">{n}</span>;
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
      <span className={`h-2 w-2 shrink-0 rounded-full ${running ? "bg-accent" : "bg-muted"}`} />
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
