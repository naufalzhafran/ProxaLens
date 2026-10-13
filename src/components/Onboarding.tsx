// First-run setup: trust the root certificate, then route traffic through
// the proxy. Reopened from the toolbar's "Setup…" button.
import { useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api, errorMessage, type Status } from "../api";
import { copy } from "../lib/clipboard";
import { button, primaryButton } from "./ui";

export const ONBOARDED_KEY = "onboardingDone";

export function shouldShowOnboarding(status: Status): boolean {
  try {
    return !status.caTrusted && localStorage.getItem(ONBOARDED_KEY) !== "1";
  } catch {
    return !status.caTrusted;
  }
}

export function Onboarding({
  status,
  setStatus,
  onClose,
}: {
  status: Status;
  setStatus: (s: Status) => void;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run(fn: () => Promise<Status>) {
    setBusy(true);
    setError(null);
    try {
      setStatus(await fn());
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  function finish() {
    try {
      localStorage.setItem(ONBOARDED_KEY, "1");
    } catch {
      /* shown again next launch */
    }
    onClose();
  }

  const addr = status.addr ?? `127.0.0.1:${status.port}`;
  const curl = `curl -x http://${addr} https://httpbin.org/get`;

  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/30 p-6" role="dialog" aria-modal>
      <div className="w-full max-w-lg rounded-xl border border-neutral-200 bg-white p-5 text-xs shadow-2xl dark:border-neutral-700 dark:bg-neutral-900">
        <h2 className="mb-1 text-base font-semibold">Set up Project Proxy</h2>
        <p className="mb-4 text-neutral-500">Two steps let you see HTTP and HTTPS traffic from apps on this Mac.</p>

        <Step
          n={1}
          done={status.caTrusted}
          title="Trust the Project Proxy certificate"
          body={
            <>
              HTTPS traffic is decrypted with a root certificate created for this install only. macOS asks for your
              password to trust it in your login keychain.
              <div className="mt-1 font-mono text-[11px] text-neutral-500">SHA-1 {status.caFingerprint}</div>
            </>
          }
          action={
            status.caTrusted ? (
              <span className="text-emerald-600 dark:text-emerald-400">Trusted</span>
            ) : (
              <div className="flex gap-2">
                <button className={primaryButton} disabled={busy} onClick={() => run(api.installCa)}>
                  Install certificate…
                </button>
                <button
                  className={button}
                  onClick={() => revealItemInDir(status.caCertPath).catch((e) => setError(errorMessage(e)))}
                >
                  Show file
                </button>
              </div>
            )
          }
        />

        <Step
          n={2}
          done={status.systemProxyEnabled}
          title="Switch the proxy on"
          body={
            <>
              Routes this Mac’s HTTP and HTTPS traffic through <span className="font-mono">{addr}</span>. Your previous
              settings are restored when you switch it off or quit, even after a crash.
            </>
          }
          action={
            status.systemProxyEnabled ? (
              <span className="text-emerald-600 dark:text-emerald-400">On</span>
            ) : (
              <button
                className={primaryButton}
                disabled={busy || !status.running}
                onClick={() => run(() => api.setSystemProxy(true))}
              >
                Switch on
              </button>
            )
          }
        />

        <div className="mt-4 rounded-md bg-neutral-100 p-3 dark:bg-neutral-800">
          <p className="mb-1 text-neutral-600 dark:text-neutral-300">
            Or point a single tool at the proxy without changing system settings:
          </p>
          <div className="flex items-center gap-2">
            <code className="selectable flex-1 truncate font-mono">{curl}</code>
            <button className={button} onClick={() => copy(curl, "command")}>
              Copy
            </button>
          </div>
        </div>

        {error && <p className="mt-3 text-red-500">{error}</p>}

        <div className="mt-5 flex justify-end gap-2">
          <button className={button} onClick={finish}>
            {status.caTrusted ? "Done" : "Skip for now"}
          </button>
        </div>
      </div>
    </div>
  );
}

function Step({
  n,
  done,
  title,
  body,
  action,
}: {
  n: number;
  done: boolean;
  title: string;
  body: React.ReactNode;
  action: React.ReactNode;
}) {
  return (
    <div className="mb-3 flex gap-3">
      <div
        className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-[11px] font-semibold ${
          done ? "bg-emerald-500 text-white" : "bg-neutral-200 dark:bg-neutral-700"
        }`}
      >
        {done ? "✓" : n}
      </div>
      <div className="min-w-0 flex-1">
        <h3 className="font-medium">{title}</h3>
        <div className="mb-2 text-neutral-600 dark:text-neutral-400">{body}</div>
        {action}
      </div>
    </div>
  );
}
