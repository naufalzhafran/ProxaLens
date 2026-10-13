import { useEffect, useState } from "react";
import { api, errorMessage, type BodyView, type FlowDetail } from "../api";
import { formatBytes, formatDuration } from "../lib/format";
import { useFlows } from "../state/flows";

export function FlowDetailPanel() {
  const selectedId = useFlows((s) => s.selectedId);
  // Refetch when the selected flow's summary changes (e.g. response arrives).
  const meta = useFlows((s) => (s.selectedId === null ? null : s.byId[s.selectedId]));
  const [detail, setDetail] = useState<FlowDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (selectedId === null) {
      setDetail(null);
      return;
    }
    let cancelled = false;
    api
      .getFlow(selectedId)
      .then((d) => !cancelled && (setDetail(d), setError(null)))
      .catch((e) => !cancelled && setError(errorMessage(e)));
    return () => {
      cancelled = true;
    };
  }, [selectedId, meta]);

  if (selectedId === null) {
    return <div className="flex flex-1 items-center justify-center text-neutral-500">Select a flow</div>;
  }
  if (error) return <div className="p-4 text-red-500">{error}</div>;
  if (!detail) return null;

  const m = detail.meta;
  return (
    <div className="selectable min-h-0 flex-1 overflow-auto p-3 text-xs">
      <div className="mb-3 break-all font-mono">
        <span className="font-semibold">{m.method}</span> {m.url}
      </div>
      <dl className="mb-4 grid grid-cols-[110px_1fr] gap-y-0.5 text-neutral-600 dark:text-neutral-400">
        <dt>Status</dt>
        <dd>{m.status ?? "—"} {m.error && <span className="text-red-500">{m.error}</span>}</dd>
        <dt>Protocol</dt>
        <dd>{m.httpVersion}</dd>
        <dt>Client</dt>
        <dd className="font-mono">{m.clientAddr}</dd>
        <dt>Time to headers</dt>
        <dd>{formatDuration(m.ttfbMs) || "—"}</dd>
        <dt>Total time</dt>
        <dd>{formatDuration(m.durationMs) || "—"}</dd>
      </dl>

      <Section title="Request headers">
        <Headers headers={detail.requestHeaders} />
      </Section>
      <Section title="Request body">
        <Body body={detail.requestBody} />
      </Section>
      <Section title="Response headers">
        <Headers headers={detail.responseHeaders} />
      </Section>
      <Section title="Response body">
        <Body body={detail.responseBody} />
      </Section>
      {detail.wsMessages.length > 0 && (
        <Section title={`WebSocket messages (${m.wsMessageCount})`}>
          <ul className="font-mono">
            {detail.wsMessages.map((w, i) => (
              <li key={i} className="truncate">
                {w.direction === "clientToServer" ? "↑" : "↓"} [{w.kind}] {w.payload.text ?? `${w.size} bytes`}
              </li>
            ))}
          </ul>
        </Section>
      )}
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="mb-4">
      <h3 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-neutral-500">{title}</h3>
      {children}
    </section>
  );
}

function Headers({ headers }: { headers: [string, string][] }) {
  if (!headers.length) return <p className="text-neutral-500">None</p>;
  return (
    <table className="w-full font-mono">
      <tbody>
        {headers.map(([k, v], i) => (
          <tr key={i} className="align-top">
            <td className="w-1/3 pr-3 text-neutral-500">{k}</td>
            <td className="break-all">{v}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Body({ body }: { body: BodyView }) {
  if (body.size === 0) return <p className="text-neutral-500">Empty</p>;
  return (
    <div>
      <p className="mb-1 text-neutral-500">
        {formatBytes(body.size)}
        {body.truncated && " (truncated)"}
        {body.base64 !== null && " · binary"}
      </p>
      {body.text !== null && (
        <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-all rounded bg-neutral-100 p-2 font-mono dark:bg-neutral-900">
          {body.text}
        </pre>
      )}
    </div>
  );
}
