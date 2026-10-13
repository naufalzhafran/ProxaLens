// Client-side mirror of the engine's flow store, kept in sync by batches.
import { create } from "zustand";
import type { FlowBatch, FlowMeta } from "../api";

export const MAX_FLOWS = 10_000;

export interface FlowsState {
  order: number[];
  byId: Record<number, FlowMeta>;
  selectedId: number | null;
  filter: string;
  applyBatch: (b: FlowBatch) => void;
  reset: (flows: FlowMeta[]) => void;
  select: (id: number | null) => void;
  setFilter: (f: string) => void;
}

export const useFlows = create<FlowsState>((set) => ({
  order: [],
  byId: {},
  selectedId: null,
  filter: "",
  applyBatch: (b) => set((s) => applyBatch(s, b)),
  reset: (flows) => set(() => fromList(flows)),
  select: (id) => set({ selectedId: id }),
  setFilter: (filter) => set({ filter }),
}));

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

/** Case-insensitive match against method, host, path and status. */
export function matchesFilter(f: FlowMeta, filter: string): boolean {
  const q = filter.trim().toLowerCase();
  if (!q) return true;
  return q
    .split(/\s+/)
    .every((term) =>
      [f.method, f.host, f.path, f.status?.toString() ?? ""].some((v) => v.toLowerCase().includes(term)),
    );
}
