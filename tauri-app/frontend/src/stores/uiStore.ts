import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { PanelId } from "../shared/types";

// displayName（显示名，非凭据）持久化：重启后 check_session 保持登录但命令面不返回
// 显示名，问候语从 persist 恢复；密码绝不进 localStorage（仅存在于表单 state）。
export const useUiStore = create<{
  activePanel: PanelId;
  setActivePanel: (p: PanelId) => void;
  displayName: string | null;
  setDisplayName: (n: string | null) => void;
}>()(
  persist(
    (set) => ({
      activePanel: "today",
      setActivePanel: (p) => set({ activePanel: p }),
      displayName: null,
      setDisplayName: (n) => set({ displayName: n }),
    }),
    { name: "campushub-ui" },
  ),
);
