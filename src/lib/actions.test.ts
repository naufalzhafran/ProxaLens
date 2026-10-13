import { describe, expect, it } from "vitest";
import type { FlowDetail } from "../api";
import { draftFromDetail, draftToRequest, parseHeaders } from "./actions";
import { formatCurl } from "./curl";

const view = (text: string | null, extra = {}) => ({
  size: text?.length ?? 3,
  truncated: false,
  text,
  base64: text === null ? "AAEC" : null,
  ...extra,
});

function detail(body = view('{"a":1}', { encoding: "gzip", encodedSize: 20 })): FlowDetail {
  return {
    meta: {
      id: 1, method: "POST", scheme: "https", host: "x.com", path: "/a", url: "https://x.com/a",
      httpVersion: "HTTP/1.1", status: 200, state: "completed", contentType: null, startedAt: 0,
      ttfbMs: null, durationMs: null, requestSize: 7, responseSize: 0, clientAddr: "", error: null,
      wsMessageCount: 0, tags: [],
    },
    requestHeaders: [
      ["Host", "x.com"],
      ["Content-Type", "application/json"],
      ["Content-Encoding", "gzip"],
      ["Content-Length", "20"],
    ],
    requestBody: body,
    responseHeaders: [],
    responseBody: view(""),
    wsMessages: [],
  };
}

describe("composer drafts", () => {
  it("drops connection headers and the encoding of a decoded body", () => {
    const d = draftFromDetail(detail());
    expect(d.headers).toBe("Content-Type: application/json");
    expect(d.body).toBe('{"a":1}');
    expect(d.binaryBody).toBeNull();
  });

  it("keeps binary bodies as bytes", () => {
    const d = draftFromDetail(detail(view(null)));
    expect(d.binaryBody).toEqual([0, 1, 2]);
    expect(draftToRequest(d).body).toEqual([0, 1, 2]);
  });

  it("parses headers and encodes text bodies", () => {
    expect(parseHeaders("A: 1\n\n  B:two:parts \nbad")).toEqual([
      ["A", "1"],
      ["B", "two:parts"],
      ["bad", ""],
    ]);
    const r = draftToRequest({ method: " post", url: " https://x.com ", headers: "", body: "hé", binaryBody: null });
    expect(r).toEqual({ method: "POST", url: "https://x.com", headers: [], body: [104, 195, 169] });
  });

  it("drops Content-Encoding from cURL when the body was decoded", () => {
    const d = detail();
    expect(formatCurl("POST", d.meta.url, d.requestHeaders, d.requestBody)).not.toContain("Content-Encoding");
  });
});
