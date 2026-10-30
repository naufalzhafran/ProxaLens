// The editable request: method, URL, headers and body. Shared by the
// composer dialog and the Mocks page.
import { formatBytes } from "../lib/format";
import type { Draft } from "../state/ui";
import { input, button } from "./ui";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

export function RequestForm({
  draft,
  onChange,
  onSubmit,
  compact = false,
}: {
  draft: Draft;
  onChange: (d: Draft) => void;
  /** ⌘↩ anywhere in the form. */
  onSubmit?: () => void;
  /** Fewer rows, for a pane rather than a dialog. */
  compact?: boolean;
}) {
  const set = (patch: Partial<Draft>) => onChange({ ...draft, ...patch });
  const listId = `request-methods-${compact ? "pane" : "dialog"}`;
  return (
    <div
      className="flex flex-col gap-3"
      onKeyDown={(e) => {
        if (e.key === "Enter" && e.metaKey && onSubmit) {
          e.preventDefault();
          onSubmit();
        }
      }}
    >
      <div className="flex gap-2">
        <input
          aria-label="Method"
          list={listId}
          className={`${input} w-24 uppercase`}
          value={draft.method}
          onChange={(e) => set({ method: e.target.value })}
        />
        <datalist id={listId}>
          {METHODS.map((m) => (
            <option key={m} value={m} />
          ))}
        </datalist>
        <input
          aria-label="URL"
          autoFocus={!compact}
          spellCheck={false}
          className={`${input} flex-1`}
          value={draft.url}
          onChange={(e) => set({ url: e.target.value })}
        />
      </div>
      <label className="flex flex-col gap-1">
        <span className="text-muted">Headers (one “Name: value” per line)</span>
        <textarea
          spellCheck={false}
          rows={compact ? 5 : 8}
          className={`${input} resize-y`}
          value={draft.headers}
          onChange={(e) => set({ headers: e.target.value })}
        />
      </label>
      <label className="flex flex-col gap-1">
        <span className="text-muted">Body</span>
        {draft.binaryBody ? (
          <div className="flex items-center gap-2 rounded-md bg-surface p-2">
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
            rows={compact ? 8 : 10}
            className={`${input} resize-y`}
            value={draft.body}
            onChange={(e) => set({ body: e.target.value })}
          />
        )}
      </label>
    </div>
  );
}
