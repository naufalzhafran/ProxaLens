import { describe, expect, it } from "vitest";
import type { BodyView, FlowMeta } from "../api";
import { formatCurl, shellQuote } from "./curl";
import { buildPredicate, compileFilter } from "./filter";
import { hueFor, methodHue } from "./format";
import {
  baseDomain,
  bodyKind,
  hexDump,
  queryParams,
  rawResponse,
  requestCookies,
  responseCookies,
} from "./http";

function flow(method: string, status: number | null, host: string, path: string, ct = ""): FlowMeta {
  return {
    id: 1,
    method,
    scheme: "https",
    host,
    path,
    url: `https://${host}${path}`,
    httpVersion: "HTTP/1.1",
    status,
    state: status === null ? "inProgress" : "completed",
    contentType: ct || null,
    startedAt: 0,
    ttfbMs: null,
    durationMs: null,
    requestSize: 0,
    responseSize: 0,
    clientAddr: "127.0.0.1:1",
    error: null,
    wsMessageCount: 0,
    tags: [], process: null, pid: null,
  };
}

const text = (t: string): BodyView => ({ size: t.length, truncated: false, text: t, base64: null });
const empty: BodyView = { size: 0, truncated: false, text: "", base64: null };

// Ported from httpmon internal/filter/quick_test.go and advanced_test.go.
describe("compileFilter", () => {
  const match = (q: string, f: FlowMeta) => compileFilter(q)!(f);

  it("returns null for empty input", () => {
    expect(compileFilter("")).toBeNull();
    expect(compileFilter("   ")).toBeNull();
  });

  it("matches host+path substrings case-insensitively", () => {
    const f = flow("GET", 200, "api.stripe.com", "/v1/charges");
    expect(match("STRIPE", f)).toBe(true);
    expect(match("charges", f)).toBe(true);
    expect(match("com/v1", f)).toBe(true);
    expect(match("github", f)).toBe(false);
  });

  it("filters by exact status and status range", () => {
    expect(match("s:200", flow("GET", 200, "x.com", "/"))).toBe(true);
    expect(match("s:200", flow("GET", 201, "x.com", "/"))).toBe(false);
    expect(match("s:2xx", flow("GET", 299, "x.com", "/"))).toBe(true);
    expect(match("s:2xx", flow("GET", 404, "x.com", "/"))).toBe(false);
    expect(match("status:4xx", flow("GET", 404, "x.com", "/"))).toBe(true);
    expect(match("s:2xx", flow("GET", null, "x.com", "/"))).toBe(false);
    expect(match("s:abc", flow("GET", 200, "x.com", "/"))).toBe(false);
  });

  it("filters by method, content type and regex", () => {
    expect(match("m:post", flow("POST", 200, "x.com", "/"))).toBe(true);
    expect(match("m:post", flow("GET", 200, "x.com", "/"))).toBe(false);
    expect(match("ct:json", flow("GET", 200, "x.com", "/", "application/json"))).toBe(true);
    expect(match("ct:json", flow("GET", 200, "x.com", "/", "text/html"))).toBe(false);
    expect(match("re:/v[12]/users", flow("GET", 200, "x.com", "/v1/users"))).toBe(true);
    expect(match("re:/v[12]/users", flow("GET", 200, "x.com", "/v3/users"))).toBe(false);
    expect(match("re:(", flow("GET", 200, "x.com", "/"))).toBe(false);
  });

  it("filters by app", () => {
    const f = { ...flow("GET", 200, "x.com", "/"), process: "Google Chrome" };
    expect(match("app:chrome", f)).toBe(true);
    expect(match("p:safari", f)).toBe(false);
    expect(buildPredicate({ text: "", methods: [], statuses: [], scope: { kind: "app", value: "Google Chrome" } })(f)).toBe(true);
  });

  it("negates terms with !", () => {
    expect(match("!m:GET", flow("GET", 200, "x.com", "/"))).toBe(false);
    expect(match("!m:GET", flow("POST", 200, "x.com", "/"))).toBe(true);
    expect(match("!s:4xx", flow("GET", 404, "x.com", "/"))).toBe(false);
    expect(match("!ct:json", flow("GET", 200, "x.com", "/", "text/html"))).toBe(true);
  });

  it("ANDs combined terms", () => {
    const q = "m:POST s:2xx api.example";
    expect(match(q, flow("POST", 201, "api.example.com", "/create"))).toBe(true);
    expect(match(q, flow("GET", 201, "api.example.com", "/create"))).toBe(false);
    expect(match(q, flow("POST", 404, "api.example.com", "/create"))).toBe(false);
    expect(match(q, flow("POST", 201, "other.com", "/create"))).toBe(false);
  });
});

describe("buildPredicate", () => {
  it("combines chips, scope and text", () => {
    const p = buildPredicate({
      text: "users",
      methods: ["POST"],
      statuses: ["2xx"],
      scope: { kind: "domain", value: "example.com" },
    });
    expect(p(flow("POST", 201, "api.example.com", "/users"))).toBe(true);
    expect(p(flow("POST", 201, "example.com", "/users"))).toBe(true);
    expect(p(flow("POST", 201, "notexample.com", "/users"))).toBe(false);
    expect(p(flow("GET", 201, "api.example.com", "/users"))).toBe(false);
    expect(p(flow("POST", 500, "api.example.com", "/users"))).toBe(false);
    expect(p(flow("POST", 201, "api.example.com", "/orders"))).toBe(false);
  });

  it("treats failed flows as the error class", () => {
    const p = buildPredicate({ text: "", methods: [], statuses: ["error"], scope: null });
    expect(p({ ...flow("GET", null, "x.com", "/"), state: "failed" })).toBe(true);
    expect(p(flow("GET", 500, "x.com", "/"))).toBe(false);
  });
});

// Ported from httpmon internal/tui/curl_test.go.
describe("formatCurl", () => {
  it("omits -X for GET and quotes the URL and headers", () => {
    const cmd = formatCurl("GET", "https://api.example.com/v1/users", [["Accept", "application/json"]], null);
    expect(cmd).toBe("curl -H 'Accept: application/json' 'https://api.example.com/v1/users'");
  });

  it("adds method and body", () => {
    const cmd = formatCurl("POST", "https://x.com/u", [["Content-Type", "application/json"], ["content-length", "15"]], text('{"name":"test"}'));
    expect(cmd).toBe(`curl -X POST -H 'Content-Type: application/json' --data-raw '{"name":"test"}' 'https://x.com/u'`);
  });

  it("sorts headers and skips an empty body", () => {
    const cmd = formatCurl("DELETE", "https://x.com/1", [["b", "2"], ["A", "1"]], empty);
    expect(cmd).toBe("curl -X DELETE -H 'A: 1' -H 'b: 2' 'https://x.com/1'");
  });

  it("escapes single quotes", () => {
    expect(shellQuote("it's")).toBe(`'it'\\''s'`);
  });
});

describe("http helpers", () => {
  it("parses query parameters", () => {
    expect(queryParams("https://x.com/a?b=1&c=two%20words&b=3#frag")).toEqual([
      ["b", "1"],
      ["c", "two words"],
      ["b", "3"],
    ]);
    expect(queryParams("https://x.com/a")).toEqual([]);
  });

  it("parses request and response cookies", () => {
    expect(requestCookies([["Cookie", "a=1; b=x=y"]]).map((c) => [c.name, c.value])).toEqual([
      ["a", "1"],
      ["b", "x=y"],
    ]);
    const [c] = responseCookies([["set-cookie", "sid=abc; Path=/; HttpOnly"]]);
    expect(c).toEqual({ name: "sid", value: "abc", attrs: [["Path", "/"], ["HttpOnly", ""]] });
  });

  it("detects body kinds", () => {
    expect(bodyKind(empty, null)).toBe("empty");
    expect(bodyKind(text('{"a":1}'), "text/plain")).toBe("json");
    expect(bodyKind(text("{not json"), "application/json")).toBe("text");
    expect(bodyKind({ size: 3, truncated: false, text: null, base64: "AAEC" }, null)).toBe("binary");
    expect(bodyKind({ size: 3, truncated: false, text: null, base64: "AAEC" }, "image/png")).toBe("image");
  });

  it("renders a hex dump", () => {
    expect(hexDump(new TextEncoder().encode("Hi!\n"))).toBe(
      "00000000  48 69 21 0a                                       Hi!.",
    );
  });

  it("frames a raw response", () => {
    expect(rawResponse(404, "HTTP/1.1", [["A", "1"]], text("nope"))).toBe("HTTP/1.1 404 Not Found\nA: 1\n\nnope");
  });

  it("groups hosts by registrable domain", () => {
    expect(baseDomain("api.example.com")).toBe("example.com");
    expect(baseDomain("example.com")).toBe("example.com");
    expect(baseDomain("a.b.example.co.uk")).toBe("example.co.uk");
    expect(baseDomain("www.tokopedia.co.id")).toBe("tokopedia.co.id");
    expect(baseDomain("127.0.0.1")).toBe("127.0.0.1");
    expect(baseDomain("localhost")).toBe("localhost");
  });
});

describe("colour helpers", () => {
  it("gives common methods their own hue and the rest gray", () => {
    expect(methodHue("GET")).toBe("blue");
    expect(methodHue("post")).toBe("green");
    expect(methodHue("DELETE")).toBe("red");
    expect(methodHue("PROPFIND")).toBe("gray");
  });

  it("keeps a name's hue stable and never gray", () => {
    expect(hueFor("github.com")).toBe(hueFor("github.com"));
    const hues = new Set(["a", "b", "c", "d", "e", "f", "g", "h", "Safari", ""].map(hueFor));
    expect(hues.has("gray")).toBe(false);
    expect(hues.size).toBeGreaterThan(3);
  });
});
