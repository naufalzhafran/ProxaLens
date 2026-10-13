import { useEffect, useState } from "react";
import { api, errorMessage, type Status } from "../api";
import { files, visibleIds } from "../lib/actions";
import { emptyDraft, useSettings, useUi } from "../state/ui";
import { THROTTLE_PRESETS } from "./SettingsDialog";
import { button, ContextMenu, type MenuEntry } from "./ui";

interface Props {
  status: Status | null;
  setStatus: (s: Status) => void;
  onError: (msg: string) => void;
  onSetup: () => void;
}

export function Toolbar({ status, setStatus, onError, onSetup }: Props) {
  const [busy, setBusy] = useState(false);
  const [port, setPort] = useState("");
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  const settings = useSettings((s) => s.settings);
  const { openComposer, openSettings, openRules, openScripts } = useUi.getState();

  useEffect(() => {
    useSettings.getState().refresh().catch((e) => onError(errorMessage(e)));
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
  const throttleLabel = !throttle?.enabled
    ? "Throttle: Off"
    : `Throttle: ${THROTTLE_PRESETS[throttle.preset]?.label ?? "Custom"}`;
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
    { label: `${throttle?.enabled ? "" : "✓ "}Off`, onSelect: () => setThrottle(null) },
    ...Object.entries(THROTTLE_PRESETS).map(([id, p]) => ({
      label: `${throttle?.enabled && throttle.preset === id ? "✓ " : ""}${p.label}`,
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
  const portValue = port || String(status?.port ?? 9090);

  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-neutral-200 px-3 py-2 dark:border-neutral-800">
      <button
        role="switch"
        aria-checked={on}
        disabled={busy || (!running && !on)}
        onClick={() => run(() => api.setSystemProxy(!on))}
        title={on ? "Stop routing this Mac’s traffic through the proxy" : "Route this Mac’s traffic through the proxy"}
        className="flex items-center gap-2 rounded-md px-1 py-0.5 text-xs font-medium disabled:opacity-50"
      >
        <span
          className={`relative h-[18px] w-8 rounded-full transition-colors ${on ? "bg-emerald-500" : "bg-neutral-300 dark:bg-neutral-700"}`}
        >
          <span
            className={`absolute top-0.5 h-3.5 w-3.5 rounded-full bg-white shadow transition-all ${on ? "left-4" : "left-0.5"}`}
          />
        </span>
        Proxy {on ? "On" : "Off"}
      </button>

      <div className="mx-1 h-5 w-px bg-neutral-200 dark:bg-neutral-800" />

      <span
        className={`h-2 w-2 rounded-full ${running ? "bg-emerald-500" : "bg-neutral-400"}`}
        title={running ? "Listening" : "Stopped"}
      />
      <span className="font-mono text-xs text-neutral-600 dark:text-neutral-400">
        {running ? status?.addr : "Stopped"}
      </span>
      <input
        aria-label="Port"
        className="w-16 rounded-md border border-neutral-300 bg-transparent px-1.5 py-0.5 font-mono text-xs dark:border-neutral-700"
        value={portValue}
        onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
      />
      {running ? (
        Number(portValue) !== status?.port ? (
          <button className={button} disabled={busy} onClick={() => run(() => api.startProxy(Number(portValue)))}>
            Apply port
          </button>
        ) : (
          <button className={button} disabled={busy} onClick={() => run(api.stopProxy)}>
            Stop
          </button>
        )
      ) : (
        <button className={button} disabled={busy} onClick={() => run(() => api.startProxy(Number(portValue)))}>
          Start
        </button>
      )}

      <div className="mx-1 h-5 w-px bg-neutral-200 dark:bg-neutral-800" />
      <button className={button} onClick={(e) => showMenu(e, fileMenu)}>
        File ▾
      </button>
      <button className={button} title="Compose a new request (⌘N)" onClick={() => openComposer(emptyDraft)}>
        Compose
      </button>
      <button
        className={`${button} ${throttle?.enabled ? "border-amber-400 text-amber-700 dark:text-amber-300" : ""}`}
        disabled={!settings}
        onClick={(e) => showMenu(e, throttleMenu)}
      >
        {throttleLabel} ▾
      </button>

      <button className={button} title="Map Local, Map Remote, Rewrite, Block, Breakpoints (⌘L)" onClick={() => openRules({ tab: "mapLocal" })}>
        Rules…
      </button>
      <button className={button} title="JavaScript onRequest / onResponse hooks (⌘J)" onClick={() => openScripts(true)}>
        Scripts…
      </button>

      <div className="ml-auto flex items-center gap-2">
        <button className={button} title="SSL Proxying, Block and Allow lists, Throttle (⌘,)" onClick={() => openSettings("ssl")}>
          Settings…
        </button>
        {status && (
          <button
            className={`${button} ${status.caTrusted ? "" : "border-amber-400 text-amber-700 dark:text-amber-300"}`}
            onClick={onSetup}
          >
            {status.caTrusted ? "Setup…" : "Certificate not trusted…"}
          </button>
        )}
        <button
          className={button}
          title="Clear all flows (⌘K)"
          onClick={() => api.clearFlows().catch((e) => onError(errorMessage(e)))}
        >
          Clear
        </button>
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
}
