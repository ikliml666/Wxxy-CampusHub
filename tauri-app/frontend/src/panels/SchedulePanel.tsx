import { CalendarDays, ChevronLeft, ChevronRight, Clock } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import { cn } from "@/shared/cn";

const DAY_HEADERS = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"] as const;

export function SchedulePanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const authed = status === "authed";
  // 列序周一为 0；getDay() 周日=0，折算到第 6 列
  const todayCol = (new Date().getDay() + 6) % 7;

  // 周次切换器占位：切换依赖学期数据（querySemesterInfo），M2 接入前禁用
  const weekSwitcher = (
    <TooltipProvider delayDuration={150}>
      <div className="flex items-center gap-1.5">
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="inline-flex">
              <Button variant="outline" size="icon-sm" disabled aria-label="上一周">
                <ChevronLeft aria-hidden="true" />
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent>门户接入后开放</TooltipContent>
        </Tooltip>
        <span className="tabular-num min-w-14 text-center text-body font-medium text-text-2">
          第 1 周
        </span>
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="inline-flex">
              <Button variant="outline" size="icon-sm" disabled aria-label="下一周">
                <ChevronRight aria-hidden="true" />
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent>门户接入后开放</TooltipContent>
        </Tooltip>
      </div>
    </TooltipProvider>
  );

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      <PanelHeader
        title="日程"
        description="一周课程安排总览"
        domain="sched"
        actions={authed ? weekSwitcher : undefined}
      />

      {!authed ? (
        <Surface>
          <EmptyState
            icon={CalendarDays}
            domain="sched"
            title="登录后查看课程表"
            hint="登录后展示本学期课程表与每周课程安排。"
            action={<Button onClick={openLoginDialog}>登录</Button>}
          />
        </Surface>
      ) : (
        <>
          {/* 周视图骨架：7 列 × 3 行，本日列高亮；课程块 M2.5 接 queryAWeekSchedule */}
          <Surface className="overflow-hidden">
            <div className="grid grid-cols-7">
              {DAY_HEADERS.map((d, i) => (
                <div
                  key={d}
                  className={cn(
                    "py-2 text-center text-caption",
                    i === todayCol
                      ? "bg-sched/5 font-medium text-sched"
                      : "text-text-2",
                    i > 0 && "border-l border-line",
                  )}
                >
                  {d}
                </div>
              ))}
              {Array.from({ length: 21 }, (_, i) => {
                const col = i % 7;
                return (
                  <div
                    key={i}
                    className={cn(
                      "min-h-14 border-t border-line",
                      col > 0 && "border-l border-line",
                      col === todayCol && "bg-sched/5",
                    )}
                  />
                );
              })}
            </div>
          </Surface>

          <Surface className="mt-3">
            <EmptyState
              compact
              icon={Clock}
              domain="sched"
              title="数据接入中"
              hint="门户模块开发中 · 该页将在 M2 接入"
            />
          </Surface>
        </>
      )}
    </section>
  );
}
