// Comparing two flows, after httpmon's internal/diff: each message is turned
// into lines (start line, sorted headers, body) and diffed with Myers'
// algorithm.
import type { BodyView, FlowDetail } from "../api";

export type DiffLine = { type: "ctx" | "add" | "del"; text: string };

/** Beyond this many edits, fall back to "all removed, all added". */
const MAX_EDITS = 4000;

export function diffLines(a: string[], b: string[]): DiffLine[] {
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const ctx = (text: string): DiffLine => ({ type: "ctx", text });
  return [
    ...a.slice(0, start).map(ctx),
    ...myers(a.slice(start, endA), b.slice(start, endB)),
    ...a.slice(endA).map(ctx),
  ];
}

function myers(a: string[], b: string[]): DiffLine[] {
  const n = a.length;
  const m = b.length;
  const dels = () => a.map((text): DiffLine => ({ type: "del", text }));
  const adds = () => b.map((text): DiffLine => ({ type: "add", text }));
  if (!n || !m) return [...dels(), ...adds()];

  const max = n + m;
  const off = max + 1;
  const v = new Int32Array(2 * max + 3);
  const trace: Int32Array[] = [];
  let done = false;
  for (let d = 0; d <= Math.min(max, MAX_EDITS) && !done; d++) {
    trace.push(v.slice());
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? v[off + k + 1] : v[off + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && a[x] === b[y]) {
        x++;
        y++;
      }
      v[off + k] = x;
      if (x >= n && y >= m) {
        done = true;
        break;
      }
    }
  }
  if (!done) return [...dels(), ...adds()];

  const out: DiffLine[] = [];
  let x = n;
  let y = m;
  for (let d = trace.length - 1; d >= 0; d--) {
    const vd = trace[d];
    const k = x - y;
    const prevK = k === -d || (k !== d && vd[off + k - 1] < vd[off + k + 1]) ? k + 1 : k - 1;
    const prevX = vd[off + prevK];
    const prevY = prevX - prevK;
    while (x > prevX && y > prevY) {
      out.push({ type: "ctx", text: a[x - 1] });
      x--;
      y--;
    }
    if (d > 0) {
      if (x === prevX) out.push({ type: "add", text: b[y - 1] });
      else out.push({ type: "del", text: a[x - 1] });
    }
    x = prevX;
    y = prevY;
  }
  return out.reverse();
}

function bodyLines(body: BodyView): string[] {
  if (body.size === 0) return [];
  if (body.text === null) return [`<${body.size} bytes of binary data>`];
  let text = body.text;
  try {
    if (/^\s*[[{]/.test(text)) text = JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    /* not JSON; compare as-is */
  }
  return text.split("\n");
}

function headerLines(headers: [string, string][]): string[] {
  return headers.map(([k, v]) => `${k.toLowerCase()}: ${v}`).sort();
}

export function requestLines(d: FlowDetail): string[] {
  return [`${d.meta.method} ${d.meta.url}`, ...headerLines(d.requestHeaders), "", ...bodyLines(d.requestBody)];
}

export function responseLines(d: FlowDetail): string[] {
  const m = d.meta;
  return [
    `${m.httpVersion} ${m.status ?? m.state}`,
    ...headerLines(d.responseHeaders),
    "",
    ...bodyLines(d.responseBody),
  ];
}

export function hasChanges(lines: DiffLine[]): boolean {
  return lines.some((l) => l.type !== "ctx");
}
