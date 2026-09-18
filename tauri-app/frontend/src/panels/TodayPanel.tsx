import { Fragment, useState } from "react";
import {
  ArrowRight,
  CreditCard,
  Landmark,
  LayoutGrid,
  Wrench,
  X,
  Zap,
  type LucideIcon,
} from "lucide-react";
import type { PanelDomain } from "@/components/PanelHeader";
import type { PanelId } from "@/shared/types";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import { Avatar } from "@/components/Avatar";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/shared/cn";

const WEEKDAYS = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"] as const;

function greetingByHour(hour: number): string {
  if (hour >= 5 && hour < 11) return "早安";
  if (hour >= 11 && hour < 18) return "午安";
  return "晚上好";
}

// 钱包三卡：非对称 12 栏（5/4/3），窄窗口自然堆叠；数值 M3 接慧新E校实时数据，先以 `—` 占位
const WALLET_CARDS: {
  label: string;
  note: string;
  span: string;
  accent: PanelDomain;
}[] = [
  { label: "一卡通", note: "实时余额", span: "sm:col-span-5", accent: "wallet" },
  { label: "邮箱", note: "未读邮件", span: "sm:col-span-4", accent: "wallet" },
  { label: "图书借阅", note: "在借图书", span: "sm:col-span-3", accent: "info" },
];

// ponytail: 下一节课依赖课表数据（M2.5 导入后填充），现在恒为 null 以验证「无数据整行隐藏」
const NEXT_CLASS: {
  time: string;
  name: string;
  room: string;
  section: string;
} | null = null;

// 快捷动作：panel 有值则切对应面板；enabled=false 为未落地门户模块（M2+ 接上）
const QUICK_ACTIONS: {
  label: string;
  icon: LucideIcon;
  panel?: PanelId;
  enabled: boolean;
  arrow?: boolean;
}[] = [
  { label: "查电费", icon: Zap, panel: "power", enabled: true },
  { label: "卡片充值", icon: CreditCard, panel: "wallet", enabled: true },
  { label: "网络报修", icon: Wrench, enabled: false },
  { label: "办事大厅", icon: Landmark, enabled: false },
  { label: "全部应用", icon: LayoutGrid, panel: "apps", enabled: true, arrow: true },
];

export function TodayPanel() {
  const status = useAuthStore((s) => s.status);
  const displayName = useAuthStore((s) => s.displayName);
  const avatarBase64 = useAuthStore((s) => s.avatarBase64);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const setActivePanel = useUiStore((s) => s.setActivePanel);
  const [guideDismissed, setGuideDismissed] = useState(false);

  const authed = status === "authed";
  const now = new Date();
  // 本地日期行（不编造教学周：学期数据待 M2.5 接入）
  const dateLabel = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")} ${WEEKDAYS[now.getDay()]}`;

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      {/* 欢迎区：头像 + 时段问候 + 日期行 */}
      <div className="flex items-center gap-4">
        <Avatar
          size="lg"
          src={avatarBase64 ? `data:image/png;base64,${avatarBase64}` : null}
          name={displayName}
        />
        <div className="min-w-0">
          <h2 className="text-display font-semibold text-text">
            {authed
              ? `${greetingByHour(now.getHours())}，${displayName ?? "同学"}`
              : "你好，同学"}
          </h2>
          <p className="tabular-num mt-1 text-caption text-text-2">{dateLabel}</p>
        </div>
      </div>

      {/* 游客登录引导条：本地 state 可关闭，不持久化 */}
      {!authed && !guideDismissed && (
        <Surface
          accent="brand"
          className="mt-4 flex items-center justify-between gap-3 py-3 pl-4 pr-2"
        >
          <p className="text-body text-text">登录后同步课表、一卡通余额与待办</p>
          <div className="flex shrink-0 items-center gap-1">
            <Button size="sm" onClick={openLoginDialog}>
              登录
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="关闭登录引导"
              onClick={() => setGuideDismissed(true)}
            >
              <X aria-hidden="true" />
            </Button>
          </div>
        </Surface>
      )}

      {/* 钱包三卡：非对称栅格，hover 抬升 */}
      <div className="mt-5 grid grid-cols-12 gap-3">
        {WALLET_CARDS.map((card) => (
          <Surface
            key={card.label}
            accent={card.accent}
            hover
            className={cn("col-span-12 px-4 py-4", card.span)}
          >
            <p className="text-caption text-text-2">{card.label}</p>
            <p className="tabular-num mt-2 text-display font-semibold text-text">—</p>
            <p className="mt-3 text-caption text-text-2">{card.note}</p>
          </Surface>
        ))}
      </div>

      {NEXT_CLASS && (
        <Surface accent="sched" className="mt-4 flex items-center gap-2 px-4 py-3">
          <span className="shrink-0 text-body font-medium text-text">下一节课</span>
          <span className="truncate text-body text-text-2">
            {NEXT_CLASS.time} · {NEXT_CLASS.name} · {NEXT_CLASS.room}（{NEXT_CLASS.section}）
          </span>
        </Surface>
      )}

      {/* 快捷动作：游客点击已落地项先登录；未落地项禁用 + tooltip */}
      <Surface className="mt-4 px-4 py-4">
        <p className="text-body font-medium text-text">快捷动作</p>
        <TooltipProvider delayDuration={150}>
          <div className="mt-3 flex flex-wrap gap-2">
            {QUICK_ACTIONS.map((action) => {
              const button = (
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!action.enabled}
                  onClick={() => {
                    if (!authed) {
                      openLoginDialog();
                      return;
                    }
                    if (action.panel) setActivePanel(action.panel);
                  }}
                >
                  <action.icon aria-hidden="true" />
                  {action.label}
                  {action.arrow && <ArrowRight aria-hidden="true" />}
                </Button>
              );
              if (action.enabled) return <Fragment key={action.label}>{button}</Fragment>;
              return (
                <Tooltip key={action.label}>
                  <TooltipTrigger asChild>
                    <span className="inline-flex">{button}</span>
                  </TooltipTrigger>
                  <TooltipContent>门户接入后开放</TooltipContent>
                </Tooltip>
              );
            })}
          </div>
        </TooltipProvider>
      </Surface>

      <p className="mt-6 text-center text-caption text-text-2">今日没有更多安排</p>
    </section>
  );
}
