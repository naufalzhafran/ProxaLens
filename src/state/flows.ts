// Client-side mirror of the engine's flow store, kept in sync by batches.
import { create } from "zustand";
import type { FlowBatch, FlowMeta } from "../api";
import type { Scope, StatusClass } from "../lib/filter";

export const MAX_FLOWS = 10_000;
const PINNED_KEY = "pinnedDomains";

export interface FlowsState {
  order: number[];
  byId: Record<number, FlowMeta>;
  /** The flow the inspector shows: the last one clicked. */
  selectedId: number | null;
  /** Every selected flow, including `selectedId`. */
  selection: number[];
  /** Where a ⇧-click range starts. */
  anchor: number | null;
  filter: string;
  methods: string[];
  statuses: StatusClass[];
  scope: Scope;
  pinned: string[];
  applyBatch: (b: FlowBatch) => void;
  reset: (flows: FlowMeta[]) => void;
  select: (id: number | null) => void;
  /** ⌘-click: adds or removes one flow. */
  toggleSelected: (id: number) => void;
  /** ⇧-click: selects `rows` from the anchor to `id`. */
  selectRange: (id: number, rows: number[]) => void;
  selectAll: (rows: number[]) => void;
  setFilter: (f: string) => void;
  toggleMethod: (m: string) => void;
  toggleStatus: (c: StatusClass) => void;
  setScope: (s: Scope) => void;
  togglePin: (domain: string) => void;
}

export const useFlows = create<FlowsState>((set) => ({
  order: [],
  byId: {},
  selectedId: null,
  selection: [],
  anchor: null,
  filter: "",
  methods: [],
  statuses: [],
  scope: null,
  pinned: loadPinned(),
  applyBatch: (b) => set((s) => applyBatch(s, b)),
  reset: (flows) => set(() => fromList(flows)),
  select: (id) => set({ selectedId: id, selection: id === null ? [] : [id], anchor: id }),
  toggleSelected: (id) =>
    set((s) => {
      if (!s.selection.includes(id)) return { selection: [...s.selection, id], selectedId: id, anchor: id };
      const selection = s.selection.filter((x) => x !== id);
      return { selection, selectedId: selection[selection.length - 1] ?? null, anchor: id };
    }),
  selectRange: (id, rows) =>
    set((s) => {
      const from = rows.indexOf(s.anchor ?? s.selectedId ?? id);
      const to = rows.indexOf(id);
      if (to < 0) return {};
      const [a, b] = from < 0 ? [to, to] : [Math.min(from, to), Math.max(from, to)];
      return { selection: rows.slice(a, b + 1), selectedId: id };
    }),
  selectAll: (rows) => set((s) => ({ selection: rows.slice(), selectedId: s.selectedId ?? rows[rows.length - 1] ?? null })),
  setFilter: (filter) => set({ filter }),
  toggleMethod: (m) => set((s) => ({ methods: toggle(s.methods, m) })),
  toggleStatus: (c) => set((s) => ({ statuses: toggle(s.statuses, c) })),
  setScope: (scope) => set({ scope }),
  togglePin: (domain) =>
    set((s) => {
      const pinned = toggle(s.pinned, domain);
      try {
        localStorage.setItem(PINNED_KEY, JSON.stringify(pinned));
      } catch {
        /* storage unavailable; pins last for this session */
      }
      return { pinned };
    }),
}));

export function toggle<T>(list: T[], item: T): T[] {
  return list.includes(item) ? list.filter((x) => x !== item) : [...list, item];
}

function loadPinned(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(PINNED_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x) => typeof x === "string") : [];
  } catch {
    return [];
  }
}

type Data = Pick<FlowsState, "order" | "byId" | "selectedId" | "selection">;

export function fromList(flows: FlowMeta[]): Pick<FlowsState, "order" | "byId"> {
  const byId: Record<number, FlowMeta> = {};
  for (const f of flows) byId[f.id] = f;
  return { order: flows.map((f) => f.id), byId };
}

export function applyBatch(s: Data, b: FlowBatch): Partial<Data> {
  if (b.type === "cleared") return { order: [], byId: {}, selectedId: null, selection: [] };
  if (b.type === "resync") return {}; // handled by the caller reloading
  const byId = { ...s.byId };
  if (b.type === "removed") {
    for (const id of b.ids) delete byId[id];
    return keepSelection(s, s.order.filter((id) => id in byId), byId);
  }
  let order = s.order;
  const added: number[] = [];
  for (const f of b.flows) {
    if (!(f.id in byId)) added.push(f.id);
    byId[f.id] = f;
  }
  if (added.length) {
    order = order.concat(added);
    if (order.length > MAX_FLOWS) {
      const dropped = order.slice(0, order.length - MAX_FLOWS);
      order = order.slice(-MAX_FLOWS);
      for (const id of dropped) delete byId[id];
    }
  }
  return keepSelection(s, order, byId);
}

/** Drops selected flows that are gone. */
function keepSelection(s: Data, order: number[], byId: Record<number, FlowMeta>): Partial<Data> {
  const selectedId = s.selectedId !== null && s.selectedId in byId ? s.selectedId : null;
  const kept = s.selection.filter((id) => id in byId);
  const selection = kept.length === s.selection.length ? s.selection : kept;
  return { order, byId, selectedId, selection };
}
