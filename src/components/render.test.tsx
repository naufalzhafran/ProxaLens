// Smoke tests: render viewers to static HTML (no DOM needed).
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { BodyView } from "../api";
import { BodyViewer } from "./Inspector";
import { childPath, copyText, JsonTree } from "./JsonTree";

const text = (t: string): BodyView => ({ size: t.length, truncated: false, text: t, base64: null });

describe("BodyViewer", () => {
  it("shows JSON as a tree with tree/text/hex modes", () => {
    const html = renderToStaticMarkup(<BodyViewer body={text('{"name":"test","n":[1,2]}')} contentType="application/json" />);
    expect(html).toContain("&quot;name&quot;");
    expect(html).toMatch(/>Tree<.*>Text<.*>Hex</);
  });

  it("previews images and hex-dumps binary", () => {
    const png: BodyView = { size: 3, truncated: true, text: null, base64: "iVBO" };
    expect(renderToStaticMarkup(<BodyViewer body={png} contentType="image/png" />)).toContain(
      'src="data:image/png;base64,iVBO"',
    );
    const html = renderToStaticMarkup(<BodyViewer body={png} contentType={null} />);
    expect(html).toContain("00000000  89 50 4e");
    expect(html).toContain("truncated");
  });

  it("says when there is no body", () => {
    expect(renderToStaticMarkup(<BodyViewer body={text("")} contentType={null} />)).toContain("No body");
  });
});

describe("JsonTree", () => {
  it("collapses beyond depth 2 and pages long arrays", () => {
    const html = renderToStaticMarkup(
      <JsonTree value={{ a: { b: { c: 1 } }, list: Array.from({ length: 250 }, (_, i) => i) }} />,
    );
    expect(html).toContain("1 key"); // { c: 1 } is collapsed
    expect(html).toContain("Show 50 more of 50");
  });

  it("puts a copy button on every row", () => {
    const html = renderToStaticMarkup(<JsonTree value={{ a: 1, b: [true] }} />);
    expect(html).toContain('aria-label="Copy a"');
    expect(html).toContain('aria-label="Copy b"');
    expect(html).toContain('aria-label="Copy b[0]"');
  });

  it("copies strings unquoted and builds paths", () => {
    expect(copyText("hi")).toBe("hi");
    expect(copyText({ a: 1 })).toBe('{\n  "a": 1\n}');
    expect(childPath(childPath(childPath("", "data", false), "0", true), "content-type", false)).toBe(
      'data[0]["content-type"]',
    );
  });
});

describe("Onboarding", () => {
  it("renders in the shared dialog, which can be dismissed", async () => {
    const { Onboarding } = await import("./Onboarding");
    const status = {
      running: true, addr: "127.0.0.1:9090", port: 9090, error: null, caTrusted: false, caCertPath: "/ca.pem",
      caFingerprint: "AB:CD", systemProxyEnabled: false, dataDir: "/d", recording: true,
    };
    const html = renderToStaticMarkup(<Onboarding status={status} setStatus={() => {}} onClose={() => {}} />);
    expect(html).toContain('aria-label="Set up ProxaLens"');
    expect(html).toContain('aria-label="Close"');
    expect(html).toContain("Skip for now");
  });
});

describe("WebSocket filtering", () => {
  it("filters by direction, text and control frames", async () => {
    const { filterWs } = await import("./Inspector");
    const msg = (direction: "clientToServer" | "serverToClient", kind: "text" | "ping", text: string) => ({
      direction,
      at: 0,
      kind,
      size: text.length,
      payload: { size: text.length, truncated: false, text, base64: null },
    });
    const list = [
      msg("clientToServer", "text", "subscribe prices"),
      msg("serverToClient", "text", '{"price":1}'),
      msg("serverToClient", "ping", ""),
    ];
    expect(filterWs(list, "all", "", false)).toEqual([0, 1]);
    expect(filterWs(list, "all", "", true)).toEqual([0, 1, 2]);
    expect(filterWs(list, "serverToClient", "", false)).toEqual([1]);
    expect(filterWs(list, "all", "PRICE", false)).toEqual([0, 1]);
    expect(filterWs(list, "clientToServer", "unsubscribe", false)).toEqual([]);
  });
});

describe("keyboard access", () => {
  it("gives only the selected tab a tab stop", async () => {
    const { Tabs } = await import("./ui");
    const html = renderToStaticMarkup(
      <Tabs value="b" onChange={() => {}} tabs={[{ id: "a", label: "A" }, { id: "b", label: "B" }]} />,
    );
    expect(html).toMatch(/data-tab="a" tabindex="-1"/);
    expect(html).toMatch(/data-tab="b" tabindex="0"/);
  });

  it("renders sidebar rows as focusable buttons, marking the current one", async () => {
    const { Row } = await import("./Sidebar");
    const html = renderToStaticMarkup(
      <>
        <Row label="curl" count={2} selected onClick={() => {}} />
        <Row label="example.com" count={3} selected={false} caret={false} onCaret={() => {}} onClick={() => {}} />
      </>,
    );
    expect(html.match(/<button data-nav-row/g)).toHaveLength(2);
    expect(html).toMatch(/<button data-nav-row="true" aria-current="true" title="curl"/);
    expect(html).toContain('aria-label="Expand example.com"');
    expect(html).toContain('aria-expanded="false"');
  });
});

describe("MocksPage", () => {
  const mock = {
    id: "m1", name: "GET /users", enabled: true, method: "GET", url: "https://api.x.com/users*", regex: false,
    request: { url: "https://api.x.com/users?page=1", headers: [], body: "", bodyBase64: "" },
    response: { status: 200, headers: [["Content-Type", "application/json"]] as [string, string][], body: "[]", delayMs: 0 },
    createdAt: 0,
  };
  const flow = {
    id: 7, method: "GET", scheme: "https", host: "api.x.com", path: "/users", url: "https://api.x.com/users",
    httpVersion: "HTTP/1.1", status: 200, state: "completed" as const, contentType: null, startedAt: 0, ttfbMs: null,
    durationMs: 12, requestSize: 0, responseSize: 2, clientAddr: "127.0.0.1:1", error: null, wsMessageCount: 0,
    tags: ["Mock"], process: "curl", pid: null, timings: null, mockId: "m1",
  };

  it("explains how to add an API when there are none", async () => {
    const { NoMocks } = await import("./MocksPage");
    expect(renderToStaticMarkup(<NoMocks />)).toContain("Mock This API");
  });

  it("lists a mock with its switch and badges its new calls", async () => {
    const { MockRow } = await import("./MocksPage");
    const html = renderToStaticMarkup(<MockRow mock={mock} calls={3} unseen={1} selected={false} />);
    expect(html).toContain('role="switch" aria-checked="true"');
    expect(html).toContain("GET /users");
    expect(html).toContain('title="1 new call"');
    const off = renderToStaticMarkup(<MockRow mock={{ ...mock, enabled: false }} calls={3} unseen={0} selected />);
    expect(off).toContain('role="switch" aria-checked="false"');
    expect(off).toContain('aria-current="true"');
    expect(off).toContain(">3<");
  });

  it("edits the pattern and the mock response", async () => {
    const { Editor } = await import("./MocksPage");
    const html = renderToStaticMarkup(<Editor mock={mock} />);
    expect(html).toContain('aria-label="URL pattern"');
    expect(html).toContain('value="https://api.x.com/users*"');
    expect(html).toContain("Serving mock");
    expect(html).toContain("Content-Type: application/json");
    expect(html).toContain("Format JSON");
  });

  it("lists calls, saying which were mocked", async () => {
    const { Calls } = await import("./MocksPage");
    const html = renderToStaticMarkup(<Calls mock={mock} ids={[7]} byId={{ 7: flow }} />);
    expect(html).toContain("Mocked");
    expect(html).toContain("curl");
    expect(html).toContain('aria-current="true"');
    const real = renderToStaticMarkup(<Calls mock={mock} ids={[7]} byId={{ 7: { ...flow, tags: [] } }} />);
    expect(real).toContain("Real");
    expect(renderToStaticMarkup(<Calls mock={mock} ids={[]} byId={{}} />)).toContain("No calls yet");
  });
});
