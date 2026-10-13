// Which modal is open, plus the flow marked for comparison.
import { create } from "zustand";
import { api, type Settings } from "../api";
import type { RuleKind } from "../lib/rules";

/** The Rules dialog: which tab, and optionally a new rule to start editing. */
export interface RulesView {
  tab: RuleKind;
  add?: { id: string } & Record<string, unknown>;
}

export type SettingsTab = "ssl" | "block" | "allow" | "throttle" | "protobuf";

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
  rules: RulesView | null;
  scriptsOpen: boolean;
  openScripts: (open: boolean) => void;
  openRules: (view: RulesView | null) => void;
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
  rules: null,
  scriptsOpen: false,
  openScripts: (scriptsOpen) => set({ scriptsOpen }),
  openRules: (rules) => set({ rules }),
  openComposer: (composer) => set({ composer }),
  openDiff: (a, b) => set({ diff: [a, b], compareMark: null }),
  openSettings: (settingsTab) => set({ settingsTab }),
  markForCompare: (compareMark) => set({ compareMark }),
  close: () => set({ composer: null, diff: null, settingsTab: null, rules: null, scriptsOpen: false }),
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
