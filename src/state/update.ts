// App updates, published as GitHub releases and checked with the Tauri updater.
import { check, type Update } from "@tauri-apps/plugin-updater";
import { create } from "zustand";
import { api, errorMessage } from "../api";

export type UpdatePhase = "idle" | "checking" | "upToDate" | "available" | "installing" | "error";

interface UpdateState {
  phase: UpdatePhase;
  update: Update | null;
  /** Download progress, 0–1, while installing. */
  progress: number | null;
  error: string | null;
  dismissed: boolean;
  check: () => Promise<void>;
  install: () => Promise<void>;
  dismiss: () => void;
}

export const useUpdate = create<UpdateState>((set, get) => ({
  phase: "idle",
  update: null,
  progress: null,
  error: null,
  dismissed: false,
  check: async () => {
    const { phase } = get();
    if (phase === "checking" || phase === "installing") return;
    set({ phase: "checking", error: null });
    try {
      const update = await check();
      set({ update, phase: update ? "available" : "upToDate", dismissed: false });
    } catch (e) {
      set({ phase: "error", error: errorMessage(e) });
    }
  },
  install: async () => {
    const { update, phase } = get();
    if (!update || phase === "installing") return;
    set({ phase: "installing", progress: null, error: null });
    try {
      let total = 0;
      let done = 0;
      await update.downloadAndInstall((e) => {
        if (e.event === "Started") total = e.data.contentLength ?? 0;
        else if (e.event === "Progress") {
          done += e.data.chunkLength;
          if (total) set({ progress: done / total });
        }
      });
      // Restores the system proxy on the way out, like a normal quit.
      await api.restartApp();
    } catch (e) {
      set({ phase: "error", error: errorMessage(e) });
    }
  },
  dismiss: () => set({ dismissed: true }),
}));
