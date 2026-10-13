// Pane sizes for the main window. Stored sizes are what the user dragged
// to; these functions shrink them so the panes always fit.

export const SIDEBAR_MIN = 140;
export const TABLE_MIN = 320;
export const INSPECTOR_MIN = 300;
/** Heights when the inspector sits below the flow table. */
export const TABLE_MIN_HEIGHT = 120;
export const INSPECTOR_MIN_HEIGHT = 200;
/** Room for the two drag handles between the panes. */
const HANDLES = 6;

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

/**
 * The sidebar width to show in a row `total` pixels wide (0 = not measured
 * yet). `inspectorBeside` is false when the inspector sits below the table.
 */
export function fitSidebar(total: number, width: number, inspectorBeside = true): number {
  if (total <= 0) return width;
  return clamp(width, SIDEBAR_MIN, total - TABLE_MIN - (inspectorBeside ? INSPECTOR_MIN : 0) - HANDLES);
}

/** The flow table width to show next to a sidebar of `sidebar` pixels (0 when hidden). */
export function fitTable(total: number, sidebar: number, width: number): number {
  if (total <= 0) return width;
  return clamp(width, TABLE_MIN, total - sidebar - INSPECTOR_MIN - HANDLES);
}

/** The flow table height to show above the inspector in a column `total` pixels tall. */
export function fitTableHeight(total: number, height: number): number {
  if (total <= 0) return height;
  return clamp(height, TABLE_MIN_HEIGHT, total - INSPECTOR_MIN_HEIGHT - HANDLES / 2);
}
