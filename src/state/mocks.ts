// The mocked APIs, mirrored from the engine's mocks.json, and which calls
// the user has already seen on the Mocks page.
import { create } from "zustand";
import { api, errorMessage, type Mock } from "../api";
import { useToast } from "../lib/clipboard";
import { findSame } from "../lib/mocks";

const SEEN_KEY = "mocksSeen";
/** How long edits are gathered before they are saved. */
const SAVE_DELAY_MS = 400;

export interface MocksState {
  /** Null until loaded from the engine. */
  mocks: Mock[] | null;
  selectedId: string | null;
  /** The newest flow id seen per mock, for the "new calls" badge. */
  seen: Record<string, number>;
  load: () => Promise<void>;
  /** Adds a mock, or returns the one that already covers the same API. */
  add: (m: Mock) => Promise<Mock>;
  /** Changes a mock; saves shortly after, so typing doesn't save every key. */
  update: (id: string, patch: Partial<Mock>) => void;
  /** Changes a mock and saves at once (a switch, not typing). */
  updateNow: (id: string, patch: Partial<Mock>) => Promise<void>;
  remove: (id: string) => Promise<void>;
  select: (id: string | null) => void;
  markSeen: (id: string, flowId: number) => void;
}

function loadSeen(): Record<string, number> {
  try {
    const v = JSON.parse(localStorage.getItem(SEEN_KEY) ?? "{}");
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}

export const useMocks = create<MocksState>((set, get) => {
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function save(mocks: Mock[]) {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
    try {
      const saved = await api.setMocks({ mocks });
      // Keep edits made while the save was in flight.
      if (get().mocks === mocks) set({ mocks: saved.mocks });
      return saved.mocks;
    } catch (e) {
      useToast.getState().show(`Mocks not saved: ${errorMessage(e)}`);
      throw e;
    }
  }

  function patched(id: string, patch: Partial<Mock>): Mock[] {
    return (get().mocks ?? []).map((m) => (m.id === id ? { ...m, ...patch } : m));
  }

  return {
    mocks: null,
    selectedId: null,
    seen: loadSeen(),
    load: async () => {
      const { mocks } = await api.getMocks();
      set({ mocks });
    },
    add: async (m) => {
      const mocks = get().mocks ?? (await api.getMocks()).mocks;
      const same = findSame(mocks, m);
      if (same) {
        set({ mocks, selectedId: same.id });
        return same;
      }
      const next = [...mocks, m];
      set({ mocks: next, selectedId: m.id });
      await save(next);
      return m;
    },
    update: (id, patch) => {
      const mocks = patched(id, patch);
      set({ mocks });
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        save(get().mocks ?? mocks).catch(() => {});
      }, SAVE_DELAY_MS);
    },
    updateNow: async (id, patch) => {
      const mocks = patched(id, patch);
      set({ mocks });
      await save(mocks).catch(() => {});
    },
    remove: async (id) => {
      const mocks = (get().mocks ?? []).filter((m) => m.id !== id);
      set((s) => ({ mocks, selectedId: s.selectedId === id ? null : s.selectedId }));
      await save(mocks).catch(() => {});
    },
    select: (selectedId) => set({ selectedId }),
    markSeen: (id, flowId) =>
      set((s) => {
        if ((s.seen[id] ?? 0) >= flowId) return {};
        const seen = { ...s.seen, [id]: flowId };
        try {
          localStorage.setItem(SEEN_KEY, JSON.stringify(seen));
        } catch {
          /* badge resets next launch */
        }
        return { seen };
      }),
  };
});
