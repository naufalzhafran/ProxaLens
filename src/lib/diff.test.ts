import { describe, expect, it } from "vitest";
import type { FlowDetail } from "../api";
import { diffLines, hasChanges, requestLines, responseLines, type DiffLine } from "./diff";

/** Applies a diff to `a` and checks it yields `b`. */
function check(a: string[], b: string[]): DiffLine[] {
  const d = diffLines(a, b);
  expect(d.filter((l) => l.type !== "add").map((l) => l.text)).toEqual(a);
  expect(d.filter((l) => l.type !== "del").map((l) => l.text)).toEqual(b);
  return d;
}

function detail(over: Partial<FlowDetail["meta"]> = {}, body = '{"a":1,"b":2}'): FlowDetail {
  const view = (t: string) => ({ size: t.length, truncated: false, text: t, base64: null });
  return {
    meta: {
      id: 1, method: "GET", scheme: "https", host: "x.com", path: "/a", url: "https://x.com/a",
      httpVersion: "HTTP/1.1", status: 200, state: "completed", contentType: "application/json",
      startedAt: 0, ttfbMs: 1, durationMs: 2, requestSize: 0, responseSize: body.length,
      clientAddr: "", error: null, wsMessageCount: 0, tags: [], ...over,
    },
    requestHeaders: [["Accept", "*/*"]],
    requestBody: view(""),
    responseHeaders: [["Content-Type", "application/json"]],
    responseBody: view(body),
    wsMessages: [],
  };
}

describe("diffLines", () => {
  it("finds minimal edits", () => {
    const d = check(["a", "b", "c", "d"], ["a", "x", "c", "d", "e"]);
    expect(d.filter((l) => l.type !== "ctx")).toEqual([
      { type: "del", text: "b" },
      { type: "add", text: "x" },
      { type: "add", text: "e" },
    ]);
  });

  it("handles empty sides and reorderings", () => {
    check([], ["a"]);
    check(["a"], []);
    check(["a", "b", "c"], ["c", "b", "a"]);
    check(["x", "a", "b", "y", "c"], ["a", "q", "b", "c", "z"]);
  });
});

// After httpmon internal/diff/diff_test.go.
describe("flow comparison", () => {
  it("reports identical flows as unchanged", () => {
    expect(hasChanges(diffLines(responseLines(detail()), responseLines(detail())))).toBe(false);
  });

  it("shows method, status and header changes", () => {
    const req = diffLines(requestLines(detail()), requestLines(detail({ method: "POST" })));
    expect(req.filter((l) => l.type === "add").map((l) => l.text)).toEqual(["POST https://x.com/a"]);
    const res = diffLines(responseLines(detail()), responseLines(detail({ status: 404 })));
    expect(res.filter((l) => l.type === "del").map((l) => l.text)).toEqual(["HTTP/1.1 200"]);
  });

  it("diffs pretty-printed JSON bodies line by line", () => {
    const d = diffLines(responseLines(detail()), responseLines(detail({}, '{"a":1,"b":3}')));
    expect(d.filter((l) => l.type !== "ctx")).toEqual([
      { type: "del", text: '  "b": 2' },
      { type: "add", text: '  "b": 3' },
    ]);
  });
});
