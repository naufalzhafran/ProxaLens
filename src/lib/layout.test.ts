import { describe, expect, it } from "vitest";
import { fitSidebar, fitTable, INSPECTOR_MIN, SIDEBAR_MIN, TABLE_MIN } from "./layout";

describe("pane fitting", () => {
  it("keeps stored widths when they fit", () => {
    expect(fitSidebar(1280, 200)).toBe(200);
    expect(fitTable(1280, 200, 760)).toBe(760);
  });

  it("shrinks the panes so the inspector keeps its minimum", () => {
    // The 800 px minimum window with the old defaults (200 + 640).
    const sidebar = fitSidebar(800, 200);
    const table = fitTable(800, sidebar, 640);
    expect(table).toBe(TABLE_MIN);
    expect(800 - sidebar - table).toBeGreaterThanOrEqual(INSPECTOR_MIN);
    // A table dragged wide gives way when the window narrows.
    expect(fitTable(1100, 200, 2000)).toBeLessThanOrEqual(1100 - 200 - INSPECTOR_MIN);
  });

  it("never goes below the pane minimums", () => {
    expect(fitSidebar(600, 50)).toBe(SIDEBAR_MIN);
    expect(fitTable(600, 140, 10)).toBe(TABLE_MIN);
  });

  it("leaves widths alone before the row is measured", () => {
    expect(fitSidebar(0, 260)).toBe(260);
    expect(fitTable(0, 260, 900)).toBe(900);
  });
});
