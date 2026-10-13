// Pane widths for the main window. Stored widths are what the user dragged
// to; these functions shrink them so the three panes always fit.

export const SIDEBAR_MIN = 140;
export const TABLE_MIN = 320;
export const INSPECTOR_MIN = 300;
/** Room for the two drag handles between the panes. */
const HANDLES = 6;

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

/** The sidebar width to show in a row `total` pixels wide (0 = not measured yet). */
export function fitSidebar(total: number, width: number): number {
  if (total <= 0) return width;
  return clamp(width, SIDEBAR_MIN, total - TABLE_MIN - INSPECTOR_MIN - HANDLES);
}

/** The flow table width to show next to a sidebar of `sidebar` pixels. */
export function fitTable(total: number, sidebar: number, width: number): number {
  if (total <= 0) return width;
  return clamp(width, TABLE_MIN, total - sidebar - INSPECTOR_MIN - HANDLES);
}
