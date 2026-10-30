// Flow table column preferences: hidden columns, widths and sort order,
// remembered across launches.
import { create } from "zustand";

export type ColumnKey =
  | "id"
  | "app"
  | "method"
  | "status"
  | "host"
  | "path"
  | "type"
  | "size"
  | "time"
  | "waterfall"
  | "started";
export interface Sort {
  key: ColumnKey;
  desc: boolean;
}

interface Prefs {
  hidden: ColumnKey[];
  /** Widths the user dragged to, in px. */
  widths: Partial<Record<ColumnKey, number>>;
  sort: Sort | null;
}

interface ColumnsState extends Prefs {
  toggleHidden: (key: ColumnKey) => void;
  setWidth: (key: ColumnKey, width: number | null) => void;
  /** Header click: ascending, then descending, then unsorted. */
  cycleSort: (key: ColumnKey) => void;
  reset: () => void;
}

const KEY = "flowColumns";
const defaults: Prefs = { hidden: [], widths: {}, sort: null };

function load(): Prefs {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (!v || typeof v !== "object") return defaults;
    return {
      hidden: Array.isArray(v.hidden) ? v.hidden : [],
      widths: v.widths && typeof v.widths === "object" ? v.widths : {},
      sort: v.sort && typeof v.sort.key === "string" ? { key: v.sort.key, desc: !!v.sort.desc } : null,
    };
  } catch {
    return defaults;
  }
}

function persist({ hidden, widths, sort }: Prefs) {
  try {
    localStorage.setItem(KEY, JSON.stringify({ hidden, widths, sort }));
  } catch {
    /* kept for this session only */
  }
}

export const useColumns = create<ColumnsState>((set, get) => {
  const update = (patch: Partial<Prefs>) => {
    set(patch);
    persist(get());
  };
  return {
    ...load(),
    toggleHidden: (key) => {
      const { hidden } = get();
      update({ hidden: hidden.includes(key) ? hidden.filter((k) => k !== key) : [...hidden, key] });
    },
    setWidth: (key, width) => {
      const widths = { ...get().widths };
      if (width === null) delete widths[key];
      else widths[key] = Math.round(width);
      update({ widths });
    },
    cycleSort: (key) => {
      const { sort } = get();
      update({ sort: sort?.key !== key ? { key, desc: false } : !sort.desc ? { key, desc: true } : null });
    },
    reset: () => update(defaults),
  };
});
