// Compose a new request, or edit a captured one and send it again.
import { useState } from "react";
import { errorMessage } from "../api";
import { sendDraft } from "../lib/actions";
import { formatBytes } from "../lib/format";
import { useUi, type Draft } from "../state/ui";
import { input, Modal, primaryButton, button } from "./ui";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

export function Composer({ initial }: { initial: Draft }) {
  const close = useUi((s) => s.close);
  const [draft, setDraft] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const set = (patch: Partial<Draft>) => setDraft((d) => ({ ...d, ...patch }));

  async function send() {
    setBusy(true);
    setError(null);
    try {
      await sendDraft(draft);
      close();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="Compose Request"
      onClose={close}
      footer={
        <>
          {error && <span className="mr-auto text-red-500">{error}</span>}
          <span className="mr-2 text-neutral-500">The response appears as a new flow.</span>
          <button className={button} onClick={close}>
            Cancel
          </button>
          <button className={primaryButton} disabled={busy || !draft.url.trim()} onClick={send}>
            {busy ? "Sending…" : "Send (⌘↩)"}
          </button>
        </>
      }
    >
      <div
        className="flex flex-col gap-3"
        onKeyDown={(e) => {
          if (e.key === "Enter" && e.metaKey) {
            e.preventDefault();
            send();
          }
        }}
      >
        <div className="flex gap-2">
          <input
            aria-label="Method"
            list="composer-methods"
            className={`${input} w-24 uppercase`}
            value={draft.method}
            onChange={(e) => set({ method: e.target.value })}
          />
          <datalist id="composer-methods">
            {METHODS.map((m) => (
              <option key={m} value={m} />
            ))}
          </datalist>
          <input
            aria-label="URL"
            autoFocus
            spellCheck={false}
            className={`${input} flex-1`}
            value={draft.url}
            onChange={(e) => set({ url: e.target.value })}
          />
        </div>
        <label className="flex flex-col gap-1">
          <span className="text-neutral-500">Headers (one “Name: value” per line)</span>
          <textarea
            spellCheck={false}
            rows={8}
            className={`${input} resize-y`}
            value={draft.headers}
            onChange={(e) => set({ headers: e.target.value })}
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-neutral-500">Body</span>
          {draft.binaryBody ? (
            <div className="flex items-center gap-2 rounded-md bg-neutral-100 p-2 dark:bg-neutral-800">
              <span className="flex-1">
                Binary body ({formatBytes(draft.binaryBody.length)}) is sent unchanged.
              </span>
              <button className={button} onClick={() => set({ binaryBody: null, body: "" })}>
                Replace with text
              </button>
            </div>
          ) : (
            <textarea
              spellCheck={false}
              rows={10}
              className={`${input} resize-y`}
              value={draft.body}
              onChange={(e) => set({ body: e.target.value })}
            />
          )}
        </label>
      </div>
    </Modal>
  );
}
