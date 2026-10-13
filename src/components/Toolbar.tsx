import { useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api, errorMessage, type Status } from "../api";
import { useFlows } from "../state/flows";

interface Props {
  status: Status | null;
  setStatus: (s: Status) => void;
  onError: (msg: string) => void;
}

const button =
  "rounded-md border border-neutral-300 px-2.5 py-1 text-xs font-medium hover:bg-neutral-100 disabled:opacity-50 dark:border-neutral-700 dark:hover:bg-neutral-800";

export function Toolbar({ status, setStatus, onError }: Props) {
  const [busy, setBusy] = useState(false);
  const [port, setPort] = useState("");
  const filter = useFlows((s) => s.filter);
  const setFilter = useFlows((s) => s.setFilter);

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
  const portValue = port || String(status?.port ?? 9090);

  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-neutral-200 px-3 py-2 dark:border-neutral-800">
      <span
        className={`h-2.5 w-2.5 rounded-full ${running ? "bg-emerald-500" : "bg-neutral-400"}`}
        title={running ? "Proxy running" : "Proxy stopped"}
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
        <button className={button} disabled={busy} onClick={() => run(api.stopProxy)}>
          Stop
        </button>
      ) : (
        <button className={button} disabled={busy} onClick={() => run(() => api.startProxy(Number(portValue)))}>
          Start
        </button>
      )}
      {running && Number(portValue) !== status?.port && (
        <button className={button} disabled={busy} onClick={() => run(() => api.startProxy(Number(portValue)))}>
          Apply port
        </button>
      )}

      <div className="mx-1 h-5 w-px bg-neutral-200 dark:bg-neutral-800" />

      <label className="flex items-center gap-1.5 text-xs">
        <input
          type="checkbox"
          checked={status?.systemProxyEnabled ?? false}
          disabled={busy || !running}
          onChange={(e) => run(() => api.setSystemProxy(e.target.checked))}
        />
        System proxy
      </label>

      <div className="mx-1 h-5 w-px bg-neutral-200 dark:bg-neutral-800" />

      {status?.caTrusted ? (
        <span className="text-xs text-emerald-600 dark:text-emerald-400">Certificate trusted</span>
      ) : (
        <button className={button} disabled={busy || !status} onClick={() => run(api.installCa)}>
          Install certificate…
        </button>
      )}
      {status && (
        <button
          className={button}
          onClick={() => revealItemInDir(status.caCertPath).catch((e) => onError(errorMessage(e)))}
        >
          Show certificate
        </button>
      )}

      <div className="ml-auto flex items-center gap-2">
        <input
          className="w-56 rounded-md border border-neutral-300 bg-transparent px-2 py-1 text-xs dark:border-neutral-700"
          placeholder="Filter: host, path, method, status"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <button className={button} onClick={() => api.clearFlows().catch((e) => onError(errorMessage(e)))}>
          Clear
        </button>
      </div>
    </div>
  );
}
