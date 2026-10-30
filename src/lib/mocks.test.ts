import { describe, expect, it } from "vitest";
import type { FlowDetail, FlowMeta, Mock } from "../api";
import { callsByMock, draftOf, findSame, mockFrom, nameFor, prettyJson, wasMocked } from "./mocks";

const view = (text: string | null, extra = {}) => ({
  size: text?.length ?? 3,
  truncated: false,
  text,
  base64: text === null ? "AAEC" : null,
  ...extra,
});

function meta(over: Partial<FlowMeta> = {}): FlowMeta {
  return {
    id: 1, method: "POST", scheme: "https", host: "api.x.com", path: "/users?page=2", url: "https://api.x.com/users?page=2",
    httpVersion: "HTTP/1.1", status: 201, state: "completed", contentType: "application/json", startedAt: 0,
    ttfbMs: null, durationMs: null, requestSize: 7, responseSize: 2, clientAddr: "", error: null,
    wsMessageCount: 0, tags: [], process: null, pid: null, timings: null, mockId: null, ...over,
  };
}

function detail(over: Partial<FlowDetail> = {}): FlowDetail {
  return {
    meta: meta(),
    requestHeaders: [
      ["Host", "api.x.com"],
      ["Content-Type", "application/json"],
      ["Content-Encoding", "gzip"],
      ["Content-Length", "20"],
      ["Authorization", "Bearer t"],
    ],
    requestBody: view('{"a":1}', { encoding: "gzip", encodedSize: 20 }),
    responseHeaders: [
      ["Content-Type", "application/json"],
      ["Content-Length", "2"],
      ["Content-Encoding", "br"],
      ["X-Request-Id", "abc"],
    ],
    responseBody: view("[]", { encoding: "br" }),
    wsMessages: [],
    ...over,
  };
}

describe("mockFrom", () => {
  it("names the API, matches its path with any query, and seeds request and response", () => {
    const m = mockFrom(detail());
    expect(m.name).toBe("POST /users");
    expect(m.method).toBe("POST");
    expect(m.url).toBe("https://api.x.com/users*");
    expect(m.enabled).toBe(true);
    expect(m.id).not.toBe("");
    expect(m.request.url).toBe("https://api.x.com/users?page=2");
    expect(m.request.headers).toEqual([
      ["Content-Type", "application/json"],
      ["Authorization", "Bearer t"],
    ]);
    expect(m.request.body).toBe('{"a":1}');
    expect(m.request.bodyBase64).toBe("");
    expect(m.response).toEqual({
      status: 201,
      headers: [
        ["Content-Type", "application/json"],
        ["X-Request-Id", "abc"],
      ],
      body: "[]",
      delayMs: 0,
    });
  });

  it("keeps a binary request body as base64", () => {
    const m = mockFrom(detail({ requestBody: view(null) }));
    expect(m.request.body).toBe("");
    expect(m.request.bodyBase64).toBe("AAEC");
    expect(draftOf(m).binaryBody).toEqual([0, 1, 2]);
  });

  it("falls back to 200 and an empty body when the response never came", () => {
    const m = mockFrom(detail({ meta: meta({ status: null, state: "failed" }), responseBody: view(null) }));
    expect(m.response.status).toBe(200);
    expect(m.response.body).toBe("");
  });
});

describe("draftOf", () => {
  it("turns the kept request back into a composer draft", () => {
    const d = draftOf(mockFrom(detail()));
    expect(d).toEqual({
      method: "POST",
      url: "https://api.x.com/users?page=2",
      headers: "Content-Type: application/json\nAuthorization: Bearer t",
      body: '{"a":1}',
      binaryBody: null,
    });
  });
});

describe("findSame", () => {
  it("matches the same method and pattern, ignoring case and spacing", () => {
    const a = mockFrom(detail());
    const b: Mock = { ...a, id: "other", method: "post", url: " HTTPS://api.x.com/users* " };
    expect(findSame([a], b)).toBe(a);
    expect(findSame([a], { ...b, method: "GET" })).toBeUndefined();
    expect(findSame([a], { ...b, regex: true })).toBeUndefined();
    expect(findSame([], b)).toBeUndefined();
  });
});

describe("callsByMock", () => {
  it("groups flows by mock, newest first, skipping unmatched ones", () => {
    const flows = [
      meta({ id: 1, mockId: "a" }),
      meta({ id: 2, mockId: null }),
      meta({ id: 3, mockId: "b" }),
      meta({ id: 4, mockId: "a", tags: ["Mock"] }),
    ];
    const byId = Object.fromEntries(flows.map((f) => [f.id, f]));
    const calls = callsByMock([1, 2, 3, 4], byId);
    expect([...calls.keys()]).toEqual(["a", "b"]);
    expect(calls.get("a")).toEqual([4, 1]);
    expect(calls.get("b")).toEqual([3]);
    expect(wasMocked(byId[4])).toBe(true);
    expect(wasMocked(byId[1])).toBe(false);
  });
});

describe("helpers", () => {
  it("names by path only, even for unparsable URLs", () => {
    expect(nameFor("GET", "https://x.com/a/b?c=1")).toBe("GET /a/b");
    expect(nameFor("GET", "https://x.com")).toBe("GET /");
    expect(nameFor("GET", "nope?x")).toBe("GET nope");
  });

  it("pretty-prints JSON and leaves other text alone", () => {
    expect(prettyJson('{"a":[1]}')).toBe('{\n  "a": [\n    1\n  ]\n}');
    expect(prettyJson("<html>")).toBe("<html>");
  });
});
