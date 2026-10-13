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
