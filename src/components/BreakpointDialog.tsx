// Shown while flows are paused at a breakpoint: edit the request or
// response, then execute or abort it.
import { useEffect, useState } from "react";
import { api, errorMessage, type BreakpointHit } from "../api";
import { parseHeaders } from "../lib/actions";
import { formatBytes } from "../lib/format";
import { bodyBytes } from "../lib/http";
import { button, input, Modal, primaryButton } from "./ui";

/** Keeps the list of paused flows up to date. */
export function useBreakpoints(): BreakpointHit[] {
  const [hits, setHits] = useState<BreakpointHit[]>([]);
  useEffect(() => {
    const load = () => api.listBreakpoints().then(setHits).catch(() => {});
    load();
    const unlisten = api.onBreakpointsChanged(load);
    return () => {
      unlisten.then((f) => f()).catch(() => {});
    };
  }, []);
  return hits;
}

export function BreakpointDialog({ hits }: { hits: BreakpointHit[] }) {
  const hit = hits[0];
  // Remount the editor for each paused flow so its fields start fresh.
  return <Editor key={`${hit.flowId}-${hit.phase}`} hit={hit} waiting={hits.length - 1} />;
}

function Editor({ hit, waiting }: { hit: BreakpointHit; waiting: number }) {
  const binary = hit.body.text === null && hit.body.size > 0;
  const [method, setMethod] = useState(hit.method);
  const [url, setUrl] = useState(hit.url);
  const [status, setStatus] = useState(String(hit.status ?? 200));
  const [headers, setHeaders] = useState(hit.headers.map(([k, v]) => `${k}: ${v}`).join("\n"));
  const [body, setBody] = useState(hit.body.text ?? "");
  const [error, setError] = useState<string | null>(null);
  const left = useCountdown(hit.deadline);
  const isRequest = hit.phase === "request";

  async function resume(abort: boolean) {
    try {
      await api.resumeBreakpoint(
        hit.flowId,
        hit.phase,
        abort,
        abort
          ? undefined
          : {
              method,
              url,
              status: Number(status) || 0,
              headers: parseHeaders(headers),
              body: binary ? Array.from(bodyBytes(hit.body)) : Array.from(new TextEncoder().encode(body)),
            },
      );
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <Modal
      title={`Breakpoint · ${isRequest ? "Request" : "Response"} #${hit.flowId}`}
      wide
      onClose={() => resume(false)}
      footer={
        <>
          {error && <span className="mr-auto text-red-500">{error}</span>}
          <span className="mr-auto text-neutral-500">
            Continues unchanged in {formatCountdown(left)}
            {waiting > 0 && ` · ${waiting} more paused`}
          </span>
          {waiting > 0 && (
            <button className={button} onClick={() => api.resumeAllBreakpoints().catch((e) => setError(errorMessage(e)))}>
              Continue All
            </button>
          )}
          <button className={button} onClick={() => resume(true)}>
            Abort
          </button>
          <button className={primaryButton} onClick={() => resume(false)}>
            Execute (⌘↩)
          </button>
        </>
      }
    >
      <div
        className="flex flex-col gap-3"
        onKeyDown={(e) => {
          if (e.key === "Enter" && e.metaKey) {
            e.preventDefault();
            resume(false);
          }
        }}
      >
        {isRequest ? (
          <div className="flex gap-2">
            <input aria-label="Method" className={`${input} w-24 uppercase`} value={method} onChange={(e) => setMethod(e.target.value)} />
            <input aria-label="URL" spellCheck={false} className={`${input} flex-1`} value={url} onChange={(e) => setUrl(e.target.value)} />
          </div>
        ) : (
          <div className="flex items-center gap-2">
            <span className="min-w-0 flex-1 truncate font-mono text-neutral-500">
              {hit.method} {hit.url}
            </span>
            <label className="flex items-center gap-2">
              Status
              <input aria-label="Status" type="number" className={`${input} w-20`} value={status} onChange={(e) => setStatus(e.target.value)} />
            </label>
          </div>
        )}
        <label className="flex flex-col gap-1">
          <span className="text-neutral-500">Headers</span>
          <textarea rows={8} spellCheck={false} className={`${input} resize-y`} value={headers} onChange={(e) => setHeaders(e.target.value)} />
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-neutral-500">Body</span>
          {binary ? (
            <p className="rounded-md bg-neutral-100 p-2 dark:bg-neutral-800">
              Binary body ({formatBytes(hit.body.size)}) is passed on unchanged.
            </p>
          ) : (
            <textarea rows={12} spellCheck={false} className={`${input} resize-y`} value={body} onChange={(e) => setBody(e.target.value)} />
          )}
        </label>
      </div>
    </Modal>
  );
}

function useCountdown(deadline: number): number {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  return Math.max(0, deadline - now);
}

function formatCountdown(ms: number): string {
  const s = Math.ceil(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}
