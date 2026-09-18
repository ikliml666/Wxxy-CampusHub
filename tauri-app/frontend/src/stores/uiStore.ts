import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { PanelId } from "../shared/types";

// displayName 已迁移到 stores/authStore.ts（批次 B）；此处仅保留面板路由、
// 弹层开关与主题。弹层开关是一次性 UI 状态，经 partialize 排除在持久化之外；
// theme 与 activePanel 一起持久化。persist 键名 campushub-ui 保持不变。
// 密码绝不进 localStorage。
export type Theme = "light" | "dark";

export const useUiStore = create<{
  activePanel: PanelId;
  setActivePanel: (p: PanelId) => void;
  theme: Theme;
  toggleTheme: () => void;
  loginDialogOpen: boolean;
  openLoginDialog: () => void;
  closeLoginDialog: () => void;
  avatarDialogOpen: boolean;
  openAvatarDialog: () => void;
  closeAvatarDialog: () => void;
}>()(
  persist(
    (set) => ({
      activePanel: "today",
      setActivePanel: (p) => set({ activePanel: p }),
      theme: "light",
      toggleTheme: () =>
        set((s) => ({ theme: s.theme === "dark" ? "light" : "dark" })),
      loginDialogOpen: false,
      openLoginDialog: () => set({ loginDialogOpen: true }),
      closeLoginDialog: () => set({ loginDialogOpen: false }),
      avatarDialogOpen: false,
      openAvatarDialog: () => set({ avatarDialogOpen: true }),
      closeAvatarDialog: () => set({ avatarDialogOpen: false }),
    }),
    {
      name: "campushub-ui",
      partialize: (s) => ({ activePanel: s.activePanel, theme: s.theme }),
    },
  ),
);
