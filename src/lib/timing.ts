// Timing phases for the Timing tab and the waterfall column.
import type { FlowMeta, Timings } from "../api";

export type PhaseKey = keyof Omit<Timings, "reusedConnection">;

export interface Phase {
  key: PhaseKey;
  label: string;
  /** Tailwind background class for the bar segment and legend swatch. */
  cls: string;
}

/** Every phase, in the order it happens. */
export const PHASES: Phase[] = [
  { key: "blocked", label: "Blocked", cls: "bg-hue-gray" },
  { key: "dns", label: "DNS", cls: "bg-hue-teal" },
  { key: "connect", label: "Connect", cls: "bg-hue-orange" },
  { key: "tls", label: "TLS", cls: "bg-hue-violet" },
  { key: "send", label: "Send", cls: "bg-hue-blue" },
  { key: "wait", label: "Waiting", cls: "bg-hue-green" },
  { key: "receive", label: "Download", cls: "bg-accent" },
];

export interface Segment {
  phase: Phase;
  ms: number;
}

/**
 * The phases a flow went through, in order, with their durations. Flows
 * without detailed timings (imported, in progress, WebSocket) fall back to
 * waiting and download from the time to first byte and the total.
 */
export function phaseSegments(f: FlowMeta): Segment[] {
  const t = f.timings;
  if (t) {
    const out: Segment[] = [];
    for (const p of PHASES) {
      const ms = t[p.key];
      if (ms !== null) out.push({ phase: p, ms });
    }
    // The body is still arriving: show what we know so far.
    if (t.receive === null && f.durationMs !== null && f.ttfbMs !== null) {
      out.push({ phase: PHASES[6], ms: Math.max(0, f.durationMs - f.ttfbMs) });
    }
    return out;
  }
  const out: Segment[] = [];
  if (f.ttfbMs !== null) out.push({ phase: PHASES[5], ms: f.ttfbMs });
  if (f.durationMs !== null) out.push({ phase: PHASES[6], ms: Math.max(0, f.durationMs - (f.ttfbMs ?? 0)) });
  return out;
}

/** How long the flow took so far, for its bar in the waterfall. */
export function flowSpan(f: FlowMeta): number {
  return f.durationMs ?? f.ttfbMs ?? 0;
}

/** The time range the shown flows cover; null when none has any timing. */
export function waterfallRange(rows: number[], byId: Record<number, FlowMeta>): [number, number] | null {
  let start = Infinity;
  let end = -Infinity;
  for (const id of rows) {
    const f = byId[id];
    if (!f) continue;
    start = Math.min(start, f.startedAt);
    end = Math.max(end, f.startedAt + flowSpan(f));
  }
  if (!isFinite(start) || end <= start) return null;
  return [start, end];
}

/** One line per phase, for a tooltip. */
export function describePhases(f: FlowMeta): string {
  const segs = phaseSegments(f);
  const lines = segs.map((s) => `${s.phase.label}: ${formatMs(s.ms)}`);
  if (f.timings?.reusedConnection) lines.push("Connection reused");
  return lines.join("\n");
}

export function formatMs(ms: number): string {
  if (ms < 1) return `${ms.toFixed(2)} ms`;
  if (ms < 10) return `${ms.toFixed(1)} ms`;
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(2)} s`;
}
