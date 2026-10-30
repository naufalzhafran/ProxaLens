import { describe, expect, it } from "vitest";
import type { FlowMeta, Timings } from "../api";
import { describePhases, flowSpan, formatMs, phaseSegments, waterfallRange } from "./timing";

function flow(over: Partial<FlowMeta> = {}): FlowMeta {
  return {
    id: 1,
    method: "GET",
    scheme: "https",
    host: "a.com",
    path: "/",
    url: "https://a.com/",
    httpVersion: "HTTP/2",
    status: 200,
    state: "completed",
    contentType: null,
    startedAt: 1000,
    ttfbMs: 40,
    durationMs: 50,
    requestSize: 0,
    responseSize: 0,
    clientAddr: "",
    error: null,
    wsMessageCount: 0,
    tags: [],
    process: null,
    pid: null,
    timings: null,
    mockId: null,
    ...over,
  };
}

const fresh: Timings = {
  blocked: 1,
  dns: 2,
  connect: 3,
  tls: 4,
  send: 5,
  wait: 25,
  receive: 10,
  reusedConnection: false,
};

describe("phaseSegments", () => {
  it("lists the phases a flow went through, in order", () => {
    const segs = phaseSegments(flow({ timings: fresh }));
    expect(segs.map((s) => [s.phase.key, s.ms])).toEqual([
      ["blocked", 1],
      ["dns", 2],
      ["connect", 3],
      ["tls", 4],
      ["send", 5],
      ["wait", 25],
      ["receive", 10],
    ]);
  });

  it("skips phases that did not happen", () => {
    const reused: Timings = { ...fresh, blocked: null, dns: null, connect: null, tls: null, reusedConnection: true };
    expect(phaseSegments(flow({ timings: reused })).map((s) => s.phase.key)).toEqual(["send", "wait", "receive"]);
  });

  it("falls back to waiting and download without detailed timings", () => {
    expect(phaseSegments(flow()).map((s) => [s.phase.key, s.ms])).toEqual([
      ["wait", 40],
      ["receive", 10],
    ]);
    expect(phaseSegments(flow({ ttfbMs: null, durationMs: null, state: "inProgress" }))).toEqual([]);
  });

  it("shows the download so far while the body is still arriving", () => {
    const partial: Timings = { ...fresh, receive: null };
    const segs = phaseSegments(flow({ timings: partial, durationMs: 60 }));
    expect(segs[segs.length - 1]).toMatchObject({ phase: { key: "receive" }, ms: 20 });
  });
});

describe("waterfall", () => {
  it("spans from the first start to the last end", () => {
    const byId = {
      1: flow({ id: 1, startedAt: 1000, durationMs: 50 }),
      2: flow({ id: 2, startedAt: 1020, durationMs: 100 }),
      3: flow({ id: 3, startedAt: 900, durationMs: null, ttfbMs: null, state: "inProgress" }),
    };
    expect(waterfallRange([1, 2, 3], byId)).toEqual([900, 1120]);
    expect(waterfallRange([], byId)).toBeNull();
    expect(waterfallRange([3], byId)).toBeNull(); // no span at all
    expect(flowSpan(byId[3])).toBe(0);
    expect(flowSpan(flow({ durationMs: null, ttfbMs: 7 }))).toBe(7);
  });

  it("describes the phases for a tooltip", () => {
    const reused: Timings = { ...fresh, blocked: null, dns: null, connect: null, tls: null, reusedConnection: true };
    expect(describePhases(flow({ timings: reused }))).toBe("Send: 5.0 ms\nWaiting: 25 ms\nDownload: 10 ms\nConnection reused");
  });
});

describe("formatMs", () => {
  it("keeps precision where it matters", () => {
    expect(formatMs(0.123)).toBe("0.12 ms");
    expect(formatMs(5.25)).toBe("5.3 ms");
    expect(formatMs(123.6)).toBe("124 ms");
    expect(formatMs(2500)).toBe("2.50 s");
  });
});
