// Compose a new request, or edit a captured one and send it again.
import { useState } from "react";
import { errorMessage } from "../api";
import { sendDraft } from "../lib/actions";
import { useUi, type Draft } from "../state/ui";
import { RequestForm } from "./RequestForm";
import { Modal, primaryButton, button } from "./ui";

export function Composer({ initial }: { initial: Draft }) {
  const close = useUi((s) => s.close);
  const [draft, setDraft] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
          {error && <span className="mr-auto text-st-server">{error}</span>}
          <span className="mr-2 text-muted">The response appears as a new flow.</span>
          <button className={button} onClick={close}>
            Cancel
          </button>
          <button className={primaryButton} disabled={busy || !draft.url.trim()} onClick={send}>
            {busy ? "Sending…" : "Send (⌘↩)"}
          </button>
        </>
      }
    >
      <RequestForm draft={draft} onChange={setDraft} onSubmit={send} />
    </Modal>
  );
}
