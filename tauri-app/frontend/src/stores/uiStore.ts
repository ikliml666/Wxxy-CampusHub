import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { PanelId } from "../shared/types";

// displayName 已迁移到 stores/authStore.ts（批次 B）；此处仅保留面板路由、
// 弹层开关与主题。弹层开关是一次性 UI 状态，经 partialize 排除在持久化之外；
// theme 与 activePanel 一起持久化。persist 键名 campushub-ui 保持不变。
// 密码绝不进 localStorage。
export type Theme = "light" | "dark";

/** PanelId 全集（与 shared/types.ts 联合类型一一对应；persist 迁移校验用）。 */
const PANEL_IDS: readonly PanelId[] = [
  "today",
  "timetable",
  "info",
  "todo",
  "schedule",
  "apps",
  "wallet",
  "power",
  "settings",
] as const;

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
      // M2.5 批次 4 追加 "timetable" 面板：旧持久化值（8 项之一）依然合法、
      // 原样保留；非法值（手改/旧版本残留）兜底回 "today"，防止 PANEL_MAP
      // 查空导致白屏。
      version: 1,
      migrate: (persisted) => {
        const s = persisted as Partial<{ activePanel: PanelId; theme: Theme }>;
        return {
          ...s,
          activePanel:
            s.activePanel && PANEL_IDS.includes(s.activePanel)
              ? s.activePanel
              : "today",
        };
      },
    },
  ),
);
