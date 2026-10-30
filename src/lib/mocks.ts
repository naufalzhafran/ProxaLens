// Helpers for the Mocks page: building a mock from a captured flow, and
// grouping the calls each mock's API received. Matching itself happens in
// the engine (src-tauri/src/engine/mocks.rs).
import type { FlowDetail, FlowMeta, Mock } from "../api";
import type { Draft } from "../state/ui";
import { matcher, patternFor } from "./rules";

/** Headers that describe one connection or transfer, not the API. */
const REQUEST_SKIP = new Set(["content-length", "host", "connection", "proxy-connection", "keep-alive", "transfer-encoding"]);
const RESPONSE_SKIP = new Set(["content-length", "content-encoding", "transfer-encoding", "connection"]);

/** A name for the API: its method and path, without the query. */
export function nameFor(method: string, url: string): string {
  let path = url;
  try {
    path = new URL(url).pathname;
  } catch {
    const q = url.indexOf("?");
    path = q < 0 ? url : url.slice(0, q);
  }
  return `${method} ${path || "/"}`;
}

/** A mock of this flow's API that replays its response. */
export function mockFrom(d: FlowDetail): Mock {
  const m = d.meta;
  const decoded = !!d.requestBody.encoding;
  const headers = d.requestHeaders
    .filter(([k]) => !REQUEST_SKIP.has(k.toLowerCase()))
    // The body is kept decoded, so it is sent decoded too.
    .filter(([k]) => !(decoded && k.toLowerCase() === "content-encoding"));
  const binary = d.requestBody.text === null && d.requestBody.size > 0;
  return {
    ...matcher(patternFor(m.url)),
    name: nameFor(m.method, m.url),
    method: m.method,
    request: {
      url: m.url,
      headers,
      body: binary ? "" : (d.requestBody.text ?? ""),
      bodyBase64: binary ? (d.requestBody.base64 ?? "") : "",
    },
    response: {
      status: m.status ?? 200,
      headers: d.responseHeaders.filter(([k]) => !RESPONSE_SKIP.has(k.toLowerCase())),
      body: d.responseBody.text ?? "",
      delayMs: 0,
    },
    createdAt: Date.now(),
  };
}

/** The mock that already covers the same API, if any. */
export function findSame(mocks: Mock[], m: Mock): Mock | undefined {
  const method = m.method.toUpperCase();
  const url = m.url.trim().toLowerCase();
  return mocks.find((x) => x.method.toUpperCase() === method && x.url.trim().toLowerCase() === url && x.regex === m.regex);
}

/** The composer draft that sends this mock's API again. */
export function draftOf(m: Mock): Draft {
  return {
    method: m.method || "GET",
    url: m.request.url,
    headers: m.request.headers.map(([k, v]) => `${k}: ${v}`).join("\n"),
    body: m.request.body,
    binaryBody: m.request.bodyBase64 ? Array.from(base64Bytes(m.request.bodyBase64)) : null,
  };
}

function base64Bytes(b64: string): Uint8Array {
  try {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  } catch {
    return new Uint8Array();
  }
}

/** Flow ids per mock id, newest first. */
export function callsByMock(order: number[], byId: Record<number, FlowMeta>): Map<string, number[]> {
  const calls = new Map<string, number[]>();
  for (let i = order.length - 1; i >= 0; i--) {
    const f = byId[order[i]];
    if (!f?.mockId) continue;
    let list = calls.get(f.mockId);
    if (!list) calls.set(f.mockId, (list = []));
    list.push(f.id);
  }
  return calls;
}

/** Whether the proxy answered this call with the mock. */
export function wasMocked(f: FlowMeta): boolean {
  return f.tags.includes("Mock");
}

/** Pretty-prints JSON text; returns the text unchanged when it isn't JSON. */
export function prettyJson(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}
