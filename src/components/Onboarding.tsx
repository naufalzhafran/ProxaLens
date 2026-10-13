// First-run setup: trust the root certificate, then route traffic through
// the proxy. Reopened from the toolbar's "Setup…" button.
import { useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api, errorMessage, type Status } from "../api";
import { copy } from "../lib/clipboard";
import { button, Modal, primaryButton } from "./ui";

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
    // Escape, the backdrop and ✕ all count as "Skip for now".
    <Modal
      title="Set up ProxaLens"
      narrow
      onClose={finish}
      footer={
        <button className={button} onClick={finish}>
          {status.caTrusted ? "Done" : "Skip for now"}
        </button>
      }
    >
      <p className="mb-4 text-muted">Two steps let you see HTTP and HTTPS traffic from apps on this Mac.</p>

      <Step
        n={1}
        done={status.caTrusted}
        title="Trust the ProxaLens certificate"
        body={
          <>
            HTTPS traffic is decrypted with a root certificate created for this install only. macOS asks for your
            password to trust it in your login keychain.
            <div className="mt-1 font-mono text-[11px] text-muted">SHA-1 {status.caFingerprint}</div>
          </>
        }
        action={
          status.caTrusted ? (
            <span className="text-st-ok">Trusted</span>
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
            <span className="text-st-ok">On</span>
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

      <div className="mt-4 rounded-md bg-surface p-3">
        <p className="mb-1 text-muted">
          Or point a single tool at the proxy without changing system settings:
        </p>
        <div className="flex items-center gap-2">
          <code className="selectable flex-1 truncate font-mono">{curl}</code>
          <button className={button} onClick={() => copy(curl, "command")}>
            Copy
          </button>
        </div>
      </div>

      {error && <p className="mt-3 text-st-server">{error}</p>}
    </Modal>
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
          done ? "bg-accent text-on-accent" : "bg-line"
        }`}
      >
        {done ? "✓" : n}
      </div>
      <div className="min-w-0 flex-1">
        <h3 className="font-medium">{title}</h3>
        <div className="mb-2 text-muted">{body}</div>
        {action}
      </div>
    </div>
  );
}
