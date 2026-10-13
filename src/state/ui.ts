// Which modal is open, plus the flow marked for comparison.
import { create } from "zustand";
import { api, type Settings } from "../api";

export type SettingsTab = "ssl" | "block" | "allow" | "throttle";

export interface Draft {
  method: string;
  url: string;
  /** One "Name: value" per line. */
  headers: string;
  body: string;
  /** A binary body that can't be edited as text; sent unchanged. */
  binaryBody: number[] | null;
}

export const emptyDraft: Draft = { method: "GET", url: "https://", headers: "", body: "", binaryBody: null };

interface UiState {
  composer: Draft | null;
  diff: [number, number] | null;
  /** The settings tab to show, or null when closed. */
  settingsTab: SettingsTab | null;
  compareMark: number | null;
  openComposer: (d: Draft) => void;
  openDiff: (a: number, b: number) => void;
  openSettings: (tab: SettingsTab | null) => void;
  markForCompare: (id: number | null) => void;
  close: () => void;
}

export const useUi = create<UiState>((set) => ({
  composer: null,
  diff: null,
  settingsTab: null,
  compareMark: null,
  openComposer: (composer) => set({ composer }),
  openDiff: (a, b) => set({ diff: [a, b], compareMark: null }),
  openSettings: (settingsTab) => set({ settingsTab }),
  markForCompare: (compareMark) => set({ compareMark }),
  close: () => set({ composer: null, diff: null, settingsTab: null }),
}));

/** The engine's settings, cached for the toolbar. */
export const useSettings = create<{
  settings: Settings | null;
  refresh: () => Promise<void>;
  save: (s: Settings) => Promise<void>;
}>((set) => ({
  settings: null,
  refresh: async () => set({ settings: await api.getSettings() }),
  save: async (s) => set({ settings: await api.setSettings(s) }),
}));
