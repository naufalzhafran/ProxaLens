import { describe, expect, it, vi } from "vitest";
import type { FlowMeta } from "../api";
import { sortRows } from "../components/FlowTable";
import { useColumns } from "./columns";

const flow = (id: number, over: Partial<FlowMeta>): FlowMeta =>
  ({ id, host: "", path: "", status: null, durationMs: null, ...over }) as FlowMeta;

describe("sortRows", () => {
  const byId = {
    1: flow(1, { host: "b.com", status: 500, durationMs: 30 }),
    2: flow(2, { host: "A.com", status: null, durationMs: null }),
    3: flow(3, { host: "a.com", status: 200, durationMs: 10 }),
  };

  it("keeps arrival order when unsorted", () => {
    expect(sortRows([1, 2, 3], byId, null)).toEqual([1, 2, 3]);
  });

  it("sorts text case-insensitively, keeping arrival order for ties", () => {
    expect(sortRows([1, 2, 3], byId, { key: "host", desc: false })).toEqual([2, 3, 1]);
    expect(sortRows([1, 2, 3], byId, { key: "host", desc: true })).toEqual([1, 2, 3]);
  });

  it("sorts numbers with pending values first", () => {
    expect(sortRows([1, 2, 3], byId, { key: "status", desc: false })).toEqual([2, 3, 1]);
    expect(sortRows([1, 2, 3], byId, { key: "time", desc: true })).toEqual([1, 3, 2]);
  });
});

describe("column preferences", () => {
  it("cycles sort, hides columns, sets widths and persists them", () => {
    const saved = new Map<string, string>();
    vi.stubGlobal("localStorage", { getItem: (k: string) => saved.get(k) ?? null, setItem: (k: string, v: string) => saved.set(k, v) });
    const c = () => useColumns.getState();
    c().reset();
    c().cycleSort("size");
    expect(c().sort).toEqual({ key: "size", desc: false });
    c().cycleSort("size");
    expect(c().sort).toEqual({ key: "size", desc: true });
    c().cycleSort("size");
    expect(c().sort).toBeNull();

    c().toggleHidden("app");
    c().setWidth("path", 301.6);
    expect(c().hidden).toEqual(["app"]);
    expect(c().widths).toEqual({ path: 302 });
    expect(JSON.parse(localStorage.getItem("flowColumns")!)).toMatchObject({ hidden: ["app"], widths: { path: 302 } });

    c().setWidth("path", null);
    c().toggleHidden("app");
    expect([c().hidden, c().widths]).toEqual([[], {}]);
    vi.unstubAllGlobals();
  });
});
