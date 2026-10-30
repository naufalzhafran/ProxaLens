// Which modal is open.
import { create } from "zustand";
import { api, type Rules, type Settings } from "../api";
import type { RuleKind } from "../lib/rules";

/** The Rules dialog: which tab, and optionally a new rule to start editing. */
export interface RulesView {
  tab: RuleKind;
  add?: { id: string } & Record<string, unknown>;
}

export type SettingsTab = "general" | "ssl" | "block" | "allow" | "throttle";

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
  /** The settings tab to show, or null when closed. */
  settingsTab: SettingsTab | null;
  rules: RulesView | null;
  devicesOpen: boolean;
  openDevices: (open: boolean) => void;
  openRules: (view: RulesView | null) => void;
  openComposer: (d: Draft) => void;
  openSettings: (tab: SettingsTab | null) => void;
  close: () => void;
}

export const useUi = create<UiState>((set) => ({
  composer: null,
  settingsTab: null,
  rules: null,
  devicesOpen: false,
  openDevices: (devicesOpen) => set({ devicesOpen }),
  openRules: (rules) => set({ rules }),
  openComposer: (composer) => set({ composer }),
  openSettings: (settingsTab) => set({ settingsTab }),
  close: () => set({ composer: null, settingsTab: null, rules: null, devicesOpen: false }),
}));

/** Rules, cached for the toolbar's "active" badge. */
export const useTools = create<{
  rules: Rules | null;
  setRules: (r: Rules) => void;
  refresh: () => Promise<void>;
}>((set) => ({
  rules: null,
  setRules: (rules) => set({ rules }),
  refresh: async () => set({ rules: await api.getRules() }),
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

/** The main window's pages: captured traffic, and the mocked APIs. */
export type Page = "traffic" | "mocks";

/** Main window layout: the page shown, whether the sidebar shows, and where the inspector sits. */
export interface LayoutPrefs {
  page: Page;
  sidebarHidden: boolean;
  inspector: "right" | "bottom";
}

const LAYOUT_KEY = "layout";

function loadLayout(): LayoutPrefs {
  try {
    const v = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "null");
    return {
      page: v?.page === "mocks" ? "mocks" : "traffic",
      sidebarHidden: v?.sidebarHidden === true,
      inspector: v?.inspector === "right" ? "right" : "bottom",
    };
  } catch {
    return { page: "traffic", sidebarHidden: false, inspector: "bottom" };
  }
}

export const useLayout = create<
  LayoutPrefs & { setPage: (p: Page) => void; toggleSidebar: () => void; toggleInspector: () => void }
>((set, get) => {
  const update = (patch: Partial<LayoutPrefs>) => {
    set(patch);
    const { page, sidebarHidden, inspector } = get();
    try {
      localStorage.setItem(LAYOUT_KEY, JSON.stringify({ page, sidebarHidden, inspector }));
    } catch {
      /* kept for this session only */
    }
  };
  return {
    ...loadLayout(),
    setPage: (page) => update({ page }),
    toggleSidebar: () => update({ sidebarHidden: !get().sidebarHidden }),
    toggleInspector: () => update({ inspector: get().inspector === "right" ? "bottom" : "right" }),
  };
});
