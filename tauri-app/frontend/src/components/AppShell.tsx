import { useDeferredValue, useEffect } from "react";
import type { ComponentType } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Bell, Search } from "lucide-react";
import type { PanelId } from "@/shared/types";
import { useUiStore } from "@/stores/uiStore";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import AccountMenu from "@/components/AccountMenu";
import DockNav from "@/components/DockNav";
import { TodayPanel } from "@/panels/TodayPanel";
import { TimetablePanel } from "@/panels/TimetablePanel";
import { InfoPanel } from "@/panels/InfoPanel";
import { TodoPanel } from "@/panels/TodoPanel";
import { SchedulePanel } from "@/panels/SchedulePanel";
import { AppsPanel } from "@/panels/AppsPanel";
import { WalletPanel } from "@/panels/WalletPanel";
import { PowerPanel } from "@/panels/PowerPanel";
import { SettingsPanel } from "@/panels/SettingsPanel";

const PANEL_MAP: Record<PanelId, ComponentType> = {
  today: TodayPanel,
  timetable: TimetablePanel,
  info: InfoPanel,
  todo: TodoPanel,
  schedule: SchedulePanel,
  apps: AppsPanel,
  wallet: WalletPanel,
  power: PowerPanel,
  settings: SettingsPanel,
};

export default function AppShell() {
  const activePanel = useUiStore((s) => s.activePanel);
  const theme = useUiStore((s) => s.theme);
  // 快速连切时 useDeferredValue 只渲染最终面板，AnimatePresence 不闪烁
  const deferredPanel = useDeferredValue(activePanel);
  const ActivePanel = PANEL_MAP[deferredPanel];

  // 主题 → DOM 单向同步（开关入口在账号菜单的外观行）
  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
  }, [theme]);

  return (
    <TooltipProvider delayDuration={250}>
      <div className="min-h-screen">
        <header className="sticky top-0 z-40 border-b border-line bg-bg/85">
          <div className="flex h-14 items-center gap-4 px-5">
            {/* 左：品牌块（渐变方标 + 双行标识，收紧行高） */}
            <div className="flex shrink-0 items-center gap-2.5">
              <span
                aria-hidden="true"
                className="flex size-7 select-none items-center justify-center rounded-[9px] text-[15px] font-bold text-white"
                style={{
                  backgroundImage:
                    "linear-gradient(140deg, var(--color-brand), var(--color-info))",
                }}
              >
                锡
              </span>
              <div className="leading-[1.2]">
                <p className="text-body font-semibold text-text">锡院助手</p>
                <p className="text-caption text-text-2">校园工作台</p>
              </div>
            </div>

            {/* 中：搜索胶囊（命令面板 M2 接入，aria-disabled 占位、不做假交互） */}
            <div className="flex min-w-0 flex-1 justify-center">
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    type="button"
                    aria-disabled="true"
                    aria-label="搜索或跳转（命令面板 · M2 接入）"
                    className="flex h-9 w-full max-w-md items-center gap-2.5 rounded-full border border-line bg-surface px-4 text-caption text-text-2 transition-all duration-[var(--dur-fast)] ease-out-soft hover:-translate-y-px hover:border-line-strong hover:shadow-card"
                  >
                    <Search className="size-4 shrink-0" aria-hidden="true" />
                    <span className="flex-1 truncate text-left">
                      搜索或跳转…
                    </span>
                    <kbd className="shrink-0 rounded-[5px] border border-line bg-bg px-1.5 py-0.5 text-[10px] leading-none text-text-2 tabular-num">
                      ⌘K
                    </kbd>
                  </button>
                </TooltipTrigger>
                <TooltipContent>命令面板 · M2 接入</TooltipContent>
              </Tooltip>
            </div>

            {/* 右：铃铛占位（M5 接入）+ 账号区 */}
            <div className="flex shrink-0 items-center gap-1.5">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label="通知"
                    aria-disabled="true"
                  >
                    <Bell className="size-4" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>通知中心 · M5 接入</TooltipContent>
              </Tooltip>
              <AccountMenu />
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
    </TooltipProvider>
  );
}
