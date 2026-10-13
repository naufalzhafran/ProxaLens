// Client-side mirror of the engine's flow store, kept in sync by batches.
import { create } from "zustand";
import type { FlowBatch, FlowMeta } from "../api";
import type { Scope, StatusClass } from "../lib/filter";

export const MAX_FLOWS = 10_000;
const PINNED_KEY = "pinnedDomains";

export interface FlowsState {
  order: number[];
  byId: Record<number, FlowMeta>;
  selectedId: number | null;
  filter: string;
  methods: string[];
  statuses: StatusClass[];
  scope: Scope;
  pinned: string[];
  applyBatch: (b: FlowBatch) => void;
  reset: (flows: FlowMeta[]) => void;
  select: (id: number | null) => void;
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
  filter: "",
  methods: [],
  statuses: [],
  scope: null,
  pinned: loadPinned(),
  applyBatch: (b) => set((s) => applyBatch(s, b)),
  reset: (flows) => set(() => fromList(flows)),
  select: (id) => set({ selectedId: id }),
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

type Data = Pick<FlowsState, "order" | "byId" | "selectedId">;

export function fromList(flows: FlowMeta[]): Pick<FlowsState, "order" | "byId"> {
  const byId: Record<number, FlowMeta> = {};
  for (const f of flows) byId[f.id] = f;
  return { order: flows.map((f) => f.id), byId };
}

export function applyBatch(s: Data, b: FlowBatch): Partial<Data> {
  if (b.type === "cleared") return { order: [], byId: {}, selectedId: null };
  if (b.type === "resync") return {}; // handled by the caller reloading
  const byId = { ...s.byId };
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
  const selectedId = s.selectedId !== null && s.selectedId in byId ? s.selectedId : null;
  return { order, byId, selectedId };
}
