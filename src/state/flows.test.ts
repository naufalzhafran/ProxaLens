import { describe, expect, it } from "vitest";
import type { FlowMeta } from "../api";
import { formatBytes, formatDuration } from "../lib/format";
import { applyBatch, fromList, matchesFilter, MAX_FLOWS } from "./flows";

function flow(id: number, over: Partial<FlowMeta> = {}): FlowMeta {
  return {
    id,
    method: "GET",
    scheme: "https",
    host: "api.example.com",
    path: `/items/${id}`,
    url: `https://api.example.com/items/${id}`,
    httpVersion: "HTTP/1.1",
    status: null,
    state: "inProgress",
    contentType: null,
    startedAt: 0,
    ttfbMs: null,
    durationMs: null,
    requestSize: 0,
    responseSize: 0,
    clientAddr: "127.0.0.1:5000",
    error: null,
    wsMessageCount: 0,
    ...over,
  };
}

describe("applyBatch", () => {
  it("appends new flows and updates existing ones in place", () => {
    let s: { order: number[]; byId: Record<number, FlowMeta>; selectedId: number | null } = {
      ...fromList([flow(1), flow(2)]),
      selectedId: 2,
    };
    s = { ...s, ...applyBatch(s, { type: "upserts", flows: [flow(2, { status: 200 }), flow(3)] }) };
    expect(s.order).toEqual([1, 2, 3]);
    expect(s.byId[2].status).toBe(200);
    expect(s.selectedId).toBe(2);
  });

  it("clears everything", () => {
    const s = { ...fromList([flow(1)]), selectedId: 1 };
    expect(applyBatch(s, { type: "cleared" })).toEqual({ order: [], byId: {}, selectedId: null });
  });

  it("caps the list and drops the selection when it is evicted", () => {
    const s = { ...fromList([flow(0)]), selectedId: 0 };
    const many = Array.from({ length: MAX_FLOWS }, (_, i) => flow(i + 1));
    const next = applyBatch(s, { type: "upserts", flows: many });
    expect(next.order!.length).toBe(MAX_FLOWS);
    expect(next.byId![0]).toBeUndefined();
    expect(next.selectedId).toBeNull();
  });
});

describe("matchesFilter", () => {
  it("matches every whitespace-separated term", () => {
    const f = flow(7, { method: "POST", status: 404 });
    expect(matchesFilter(f, "")).toBe(true);
    expect(matchesFilter(f, "post example")).toBe(true);
    expect(matchesFilter(f, "404")).toBe(true);
    expect(matchesFilter(f, "post 500")).toBe(false);
  });
});

describe("format", () => {
  it("formats sizes and durations", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatDuration(null)).toBe("");
    expect(formatDuration(1500)).toBe("1.50 s");
  });
});
