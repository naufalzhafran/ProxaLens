import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { create } from "zustand";

/** A short-lived message shown at the bottom of the window. */
export const useToast = create<{ message: string | null; show: (m: string) => void }>((set) => {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return {
    message: null,
    show: (message) => {
      clearTimeout(timer);
      set({ message });
      timer = setTimeout(() => set({ message: null }), 1800);
    },
  };
});

export async function copy(text: string, what: string) {
  try {
    await writeText(text);
    useToast.getState().show(`Copied ${what}`);
  } catch (e) {
    useToast.getState().show(`Could not copy: ${e}`);
  }
}
