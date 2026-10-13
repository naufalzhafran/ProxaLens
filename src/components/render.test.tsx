// Smoke tests: render viewers to static HTML (no DOM needed).
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { BodyView } from "../api";
import { BodyViewer } from "./Inspector";
import { JsonTree } from "./JsonTree";

const text = (t: string): BodyView => ({ size: t.length, truncated: false, text: t, base64: null });

describe("BodyViewer", () => {
  it("shows JSON as a tree with tree/text/hex modes", () => {
    const html = renderToStaticMarkup(<BodyViewer body={text('{"name":"test","n":[1,2]}')} contentType="application/json" />);
    expect(html).toContain("&quot;name&quot;");
    expect(html).toMatch(/>tree<.*>text<.*>hex</);
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
});

describe("BreakpointDialog", () => {
  it("shows an editable request with its countdown", async () => {
    const { BreakpointDialog } = await import("./BreakpointDialog");
    const hit = {
      flowId: 7,
      phase: "request" as const,
      method: "POST",
      url: "https://a.com/x",
      status: null,
      headers: [["Content-Type", "text/plain"]] as [string, string][],
      body: { size: 2, truncated: false, text: "hi", base64: null },
      deadline: Date.now() + 90_000,
    };
    const html = renderToStaticMarkup(<BreakpointDialog hits={[hit, { ...hit, flowId: 8 }]} />);
    expect(html).toContain("Breakpoint · Request #7");
    expect(html).toContain('value="https://a.com/x"');
    expect(html).toContain("Content-Type: text/plain");
    expect(html).toContain("1 more paused");
    expect(html).toMatch(/Continues unchanged in 1:(29|30)/);
    // Only Execute or Abort may release a paused flow.
    expect(html).not.toContain('aria-label="Close"');
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
    expect(html).toContain('aria-label="Set up Project Proxy"');
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
