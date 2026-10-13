import { describe, expect, it } from "vitest";
import {
  fitSidebar,
  fitTable,
  fitTableHeight,
  INSPECTOR_MIN,
  INSPECTOR_MIN_HEIGHT,
  SIDEBAR_MIN,
  TABLE_MIN,
  TABLE_MIN_HEIGHT,
} from "./layout";

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

  it("lets the sidebar grow when the inspector is below the table", () => {
    expect(fitSidebar(800, 400, true)).toBeLessThan(400);
    expect(fitSidebar(800, 400, false)).toBe(400);
  });

  it("fits the table height above a bottom inspector", () => {
    expect(fitTableHeight(800, 300)).toBe(300);
    expect(800 - fitTableHeight(800, 2000)).toBeGreaterThanOrEqual(INSPECTOR_MIN_HEIGHT);
    expect(fitTableHeight(800, 10)).toBe(TABLE_MIN_HEIGHT);
    expect(fitTableHeight(0, 900)).toBe(900);
  });
});
