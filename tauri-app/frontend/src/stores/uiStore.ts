import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { PanelId } from "../shared/types";

export const useUiStore = create<{
  activePanel: PanelId;
  setActivePanel: (p: PanelId) => void;
}>()(
  persist(
    (set) => ({ activePanel: "today", setActivePanel: (p) => set({ activePanel: p }) }),
    { name: "campushub-ui" },
  ),
);
