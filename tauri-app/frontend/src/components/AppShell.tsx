import { useDeferredValue, useEffect, useRef, useState } from "react";
import type { ComponentType } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Bell, LogOut, Moon, Search, Sun, UserRound } from "lucide-react";
import type { PanelId } from "@/shared/types";
import { useUiStore } from "@/stores/uiStore";
import { Button } from "@/components/ui/button";
import DockNav from "@/components/DockNav";
import { TodayPanel } from "@/panels/TodayPanel";
import { InfoPanel } from "@/panels/InfoPanel";
import { TodoPanel } from "@/panels/TodoPanel";
import { SchedulePanel } from "@/panels/SchedulePanel";
import { AppsPanel } from "@/panels/AppsPanel";
import { WalletPanel } from "@/panels/WalletPanel";
import { PowerPanel } from "@/panels/PowerPanel";
import { SettingsPanel } from "@/panels/SettingsPanel";

const PANEL_MAP: Record<PanelId, ComponentType> = {
  today: TodayPanel,
  info: InfoPanel,
  todo: TodoPanel,
  schedule: SchedulePanel,
  apps: AppsPanel,
  wallet: WalletPanel,
  power: PowerPanel,
  settings: SettingsPanel,
};

export default function AppShell({ onLogout }: { onLogout: () => void }) {
  const activePanel = useUiStore((s) => s.activePanel);
  // 快速连切时 useDeferredValue 只渲染最终面板，AnimatePresence 不闪烁
  const deferredPanel = useDeferredValue(activePanel);
  const ActivePanel = PANEL_MAP[deferredPanel];
  const [dark, setDark] = useState(() =>
    document.documentElement.classList.contains("dark"),
  );

  const toggleDark = () => {
    const next = document.documentElement.classList.toggle("dark");
    setDark(next);
  };

  // 头像下拉：极简实现（不引 DropdownMenu 依赖）；打开期间点外部关闭
  const [userMenuOpen, setUserMenuOpen] = useState(false);
  const userMenuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!userMenuOpen) return;
    const onDocMouseDown = (e: MouseEvent) => {
      if (userMenuRef.current && !userMenuRef.current.contains(e.target as Node)) {
        setUserMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", onDocMouseDown);
    return () => document.removeEventListener("mousedown", onDocMouseDown);
  }, [userMenuOpen]);

  return (
    <div className="min-h-screen">
      <header className="flex items-center justify-between px-5 py-3">
        <Button
          variant="ghost"
          size="sm"
          className="gap-2 text-text-2"
          aria-label="全局搜索（占位）"
        >
          <Search className="size-4" />
          <span>搜索</span>
          <kbd className="rounded border border-line px-1 text-[10px] leading-4">
            ⌘K
          </kbd>
        </Button>
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="icon-sm" aria-label="通知（占位）">
            <Bell className="size-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="切换深浅主题"
            onClick={toggleDark}
          >
            {dark ? <Sun className="size-4" /> : <Moon className="size-4" />}
          </Button>
          <div className="relative" ref={userMenuRef}>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="账号菜单"
              aria-haspopup="menu"
              aria-expanded={userMenuOpen}
              onClick={() => setUserMenuOpen((v) => !v)}
            >
              <UserRound className="size-4" />
            </Button>
            {userMenuOpen && (
              <div
                role="menu"
                className="absolute right-0 top-full z-40 mt-2 w-36 rounded-[10px] border border-line bg-surface p-1 shadow-[0_8px_30px_rgb(0_0_0/0.12)]"
              >
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    setUserMenuOpen(false);
                    onLogout();
                  }}
                  className="flex w-full items-center gap-2 rounded-md px-3 py-2 text-sm text-text hover:bg-bg"
                >
                  <LogOut className="size-4" aria-hidden="true" />
                  退出登录
                </button>
              </div>
            )}
          </div>
        </div>
      </header>
      <main className="pb-28">
        <AnimatePresence mode="wait">
          <motion.div
            key={deferredPanel}
            initial={{ opacity: 0, y: 8 }}
            animate={{
              opacity: 1,
              y: 0,
              transition: { type: "spring", stiffness: 400, damping: 40 },
            }}
            exit={{ opacity: 0, transition: { duration: 0.04 } }}
          >
            <ActivePanel />
          </motion.div>
        </AnimatePresence>
      </main>
      <DockNav />
    </div>
  );
}
