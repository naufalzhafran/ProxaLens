// Native confirmation sheets for destructive actions.
import { ask } from "@tauri-apps/plugin-dialog";

/** Asks a yes/no question; resolves true when the user picks `ok`. */
export async function confirmAction(message: string, ok: string, cancel = "Cancel"): Promise<boolean> {
  try {
    return await ask(message, { kind: "warning", okLabel: ok, cancelLabel: cancel });
  } catch {
    // The dialog plugin is unavailable (e.g. tests); fall back to the webview.
    return window.confirm(message);
  }
}

/** A close handler that asks before throwing away unsaved edits. */
export function guardedClose(dirty: boolean, close: () => void, what = "changes"): () => Promise<void> {
  return async () => {
    if (!dirty || (await confirmAction(`Discard your unsaved ${what}?`, "Discard", "Keep Editing"))) close();
  };
}
