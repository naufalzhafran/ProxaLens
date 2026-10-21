import { describe, expect, it } from "vitest";
import type { FlowDetail, MapLocalRule } from "../api";
import { actionsFor, mapLocalFrom, newRule, patternFor, ruleSummary } from "./rules";

describe("rule helpers", () => {
  it("builds a pattern for the same path with any query", () => {
    expect(patternFor("https://a.com/v1/u?id=1")).toBe("https://a.com/v1/u*");
    expect(patternFor("https://a.com/v1/u")).toBe("https://a.com/v1/u*");
  });

  it("creates rules with unique ids and sensible defaults", () => {
    const a = newRule("mapRemote", "x");
    const b = newRule("mapRemote", "x");
    expect(a.id).not.toBe(b.id);
    expect(a).toMatchObject({ enabled: true, url: "x", to: "" });
    expect(ruleSummary("mapRemote", { ...newRule("mapRemote"), to: "http://localhost:3000" })).toBe(
      "→ http://localhost:3000",
    );
  });

  it("turns a captured response into a Map Local rule", () => {
    const view = (t: string) => ({ size: t.length, truncated: false, text: t, base64: null });
    const d = {
      meta: { method: "POST", url: "https://a.com/x?y=1", status: 201 },
      responseHeaders: [
        ["Content-Type", "application/json"],
        ["Content-Encoding", "gzip"],
        ["Content-Length", "9"],
      ],
      responseBody: view('{"ok":1}'),
    } as unknown as FlowDetail;
    const r = mapLocalFrom(d) as MapLocalRule;
    expect(r).toMatchObject({ method: "POST", url: "https://a.com/x*", status: 201, body: '{"ok":1}', file: "" });
    expect(r.headers).toEqual([["Content-Type", "application/json"]]);
  });

  it("offers only the actions a target supports", () => {
    expect(actionsFor("responseStatus")).toEqual(["set"]);
    expect(actionsFor("requestHeader")).toContain("replace");
  });
});
