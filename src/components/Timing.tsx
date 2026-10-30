// The Timing tab: where a flow's time went, phase by phase.
import type { FlowDetail } from "../api";
import { formatBytes, formatTime } from "../lib/format";
import { formatMs, phaseSegments } from "../lib/timing";

export function Timing({ detail }: { detail: FlowDetail }) {
  const m = detail.meta;
  const segments = phaseSegments(m);
  const total = segments.reduce((n, s) => n + s.ms, 0);
  const facts: [string, React.ReactNode][] = [
    ["Started", `${new Date(m.startedAt).toLocaleDateString()} ${formatTime(m.startedAt)}`],
    ["Time to headers", m.ttfbMs === null ? "—" : formatMs(m.ttfbMs)],
    ["Total time", m.durationMs === null ? "—" : formatMs(m.durationMs)],
    ["Request size", formatBytes(m.requestSize)],
    ["Response size", formatBytes(m.responseSize)],
    ["Protocol", m.httpVersion],
    ["Client", m.clientAddr],
  ];

  return (
    <div className="flex flex-col gap-3">
      {segments.length > 0 && (
        <div>
          <div
            className="flex h-2.5 w-full overflow-hidden rounded bg-line"
            role="img"
            aria-label={segments.map((s) => `${s.phase.label} ${formatMs(s.ms)}`).join(", ")}
          >
            {segments.map((s) => (
              <div
                key={s.phase.key}
                className={s.phase.cls}
                style={{ width: `${total > 0 ? (s.ms / total) * 100 : 0}%` }}
                title={`${s.phase.label}: ${formatMs(s.ms)}`}
              />
            ))}
          </div>
          <dl className="mt-2 grid grid-cols-[auto_1fr_auto] gap-x-3 gap-y-0.5">
            {segments.map((s) => (
              <div key={s.phase.key} className="contents">
                <dt className="flex items-center">
                  <span aria-hidden className={`inline-block h-2 w-2 rounded-sm ${s.phase.cls}`} />
                </dt>
                <dd className="text-muted">{s.phase.label}</dd>
                <dd className="text-right font-mono tabular-nums">{formatMs(s.ms)}</dd>
              </div>
            ))}
          </dl>
          <p className="mt-1 text-muted">{explain(m)}</p>
        </div>
      )}
      <dl className="grid grid-cols-[120px_1fr] gap-y-0.5">
        {facts.map(([k, v]) => (
          <div key={k} className="contents">
            <dt className="text-muted">{k}</dt>
            <dd className="font-mono">{v}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

function explain(m: FlowDetail["meta"]): string {
  const t = m.timings;
  if (!t) {
    if (m.state === "inProgress") return "Phases appear once the response headers arrive.";
    return "Only waiting and download are known for this flow.";
  }
  if (t.reusedConnection) return "Sent over a connection an earlier request opened, so no DNS, connect or TLS time.";
  if (t.connect === null) return "Answered without contacting a server.";
  const parts = ["A new connection was opened for this request"];
  if (t.dns === null) parts.push("to an IP address, so no DNS lookup");
  if (t.tls === null) parts.push("without TLS");
  return `${parts.join(", ")}.`;
}
