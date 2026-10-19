// Native confirmation sheets for destructive actions.
import { ask } from "@tauri-apps/plugin-dialog";

// Native sheets queue up instead of replacing each other, so a second request
// (a repeated ⌘K, a double click) would leave a stale copy behind the first.
let open = false;

/**
 * Asks a yes/no question; resolves true when the user picks `ok`. While one
 * question is showing, further ones resolve false without opening another.
 */
export async function confirmAction(message: string, ok: string, cancel = "Cancel"): Promise<boolean> {
  if (open) return false;
  open = true;
  try {
    return await ask(message, { kind: "warning", okLabel: ok, cancelLabel: cancel });
  } catch {
    // The dialog plugin is unavailable (e.g. tests); fall back to the webview.
    return window.confirm(message);
  } finally {
    open = false;
  }
}

/** A close handler that asks before throwing away unsaved edits. */
export function guardedClose(dirty: boolean, close: () => void, what = "changes"): () => Promise<void> {
  return async () => {
    if (!dirty || (await confirmAction(`Discard your unsaved ${what}?`, "Discard", "Keep Editing"))) close();
  };
}
