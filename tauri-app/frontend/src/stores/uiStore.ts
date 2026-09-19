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
  /** 课程自定义色段（批 7 契约 §13.3）：合成色板 = 8 档固定色 + 本段（固定段在前，
   *  旧数据 colorIndex 0..7 行为不变）。十六进制小写 #rrggbb。 */
  customCourseColors: string[];
  addCustomCourseColor: (c: string) => void;
  loginDialogOpen: boolean;
  openLoginDialog: () => void;
  closeLoginDialog: () => void;
  avatarDialogOpen: boolean;
  openAvatarDialog: () => void;
  closeAvatarDialog: () => void;
  /** 命令面板开关（M2 遗留补做）：同样属一次性 UI 状态，不进持久化 */
  commandPaletteOpen: boolean;
  openCommandPalette: () => void;
  closeCommandPalette: () => void;
}>()(
  persist(
    (set) => ({
      activePanel: "today",
      setActivePanel: (p) => set({ activePanel: p }),
      theme: "light",
      toggleTheme: () =>
        set((s) => ({ theme: s.theme === "dark" ? "light" : "dark" })),
      customCourseColors: [],
      // 追加语义（不覆盖已有段）；去重在消费点做（已存在则直接选中旧下标）
      addCustomCourseColor: (c) =>
        set((s) => ({ customCourseColors: [...s.customCourseColors, c] })),
      loginDialogOpen: false,
      openLoginDialog: () => set({ loginDialogOpen: true }),
      closeLoginDialog: () => set({ loginDialogOpen: false }),
      avatarDialogOpen: false,
      openAvatarDialog: () => set({ avatarDialogOpen: true }),
      closeAvatarDialog: () => set({ avatarDialogOpen: false }),
      commandPaletteOpen: false,
      openCommandPalette: () => set({ commandPaletteOpen: true }),
      closeCommandPalette: () => set({ commandPaletteOpen: false }),
    }),
    {
      name: "campushub-ui",
      partialize: (s) => ({
        activePanel: s.activePanel,
        theme: s.theme,
        customCourseColors: s.customCourseColors,
      }),
      // M2.5 批次 4 追加 "timetable" 面板：旧持久化值（8 项之一）依然合法、
      // 原样保留；非法值（手改/旧版本残留）兜底回 "today"，防止 PANEL_MAP
      // 查空导致白屏。
      // 批 7 §13.3：v2 追加 customCourseColors——v0/v1 旧数据缺该字段 → 空数组；
      // 非法形态（手改）同样兜底为空数组。
      version: 2,
      migrate: (persisted) => {
        const s = persisted as Partial<{
          activePanel: PanelId;
          theme: Theme;
          customCourseColors: unknown;
        }>;
        return {
          ...s,
          activePanel:
            s.activePanel && PANEL_IDS.includes(s.activePanel)
              ? s.activePanel
              : "today",
          customCourseColors: Array.isArray(s.customCourseColors)
            ? s.customCourseColors.filter(
                (c): c is string => typeof c === "string" && /^#[0-9a-f]{6}$/.test(c),
              )
            : [],
        };
      },
    },
  ),
);
