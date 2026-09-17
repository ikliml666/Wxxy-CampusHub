import {
  ArrowRight,
  CreditCard,
  Landmark,
  LayoutGrid,
  Wrench,
  Zap,
  type LucideIcon,
} from "lucide-react";
import type { PanelId } from "@/shared/types";
import { useUiStore } from "@/stores/uiStore";

function greetingByHour(hour: number): string {
  if (hour >= 5 && hour < 11) return "早安";
  if (hour >= 11 && hour < 18) return "午安";
  return "晚上好";
}

// 钱包三卡：数值 M3 接慧新E校实时数据，先以 `--` 占位
const WALLET_CARDS: { label: string; value: string }[] = [
  { label: "一卡通", value: "--" },
  { label: "邮箱", value: "--" },
  { label: "图书借阅", value: "--" },
];

// ponytail: 下一节课依赖课表数据（M2.5 导入后填充），现在恒为 null 以验证「无数据整行隐藏」
const NEXT_CLASS: {
  time: string;
  name: string;
  room: string;
  section: string;
} | null = null;

// 快捷动作：panel 有值则切对应面板，否则为占位（对应页面 M2+ 落地后接上）
const QUICK_ACTIONS: {
  label: string;
  icon: LucideIcon;
  panel?: PanelId;
  arrow?: boolean;
}[] = [
  { label: "查电费", icon: Zap, panel: "power" },
  { label: "卡片充值", icon: CreditCard, panel: "wallet" },
  { label: "网络报修", icon: Wrench },
  { label: "办事大厅", icon: Landmark },
  { label: "全部应用", icon: LayoutGrid, panel: "apps", arrow: true },
];

export function TodayPanel() {
  const displayName = useUiStore((s) => s.displayName);
  const setActivePanel = useUiStore((s) => s.setActivePanel);

  return (
    <section className="mx-auto mt-10 max-w-3xl">
      <h2 className="text-xl font-semibold text-text">
        {greetingByHour(new Date().getHours())}，{displayName ?? "同学"}
      </h2>

      <div className="mt-4 grid grid-cols-3 gap-3">
        {WALLET_CARDS.map((card) => (
          <div
            key={card.label}
            className="relative rounded-[10px] border border-line bg-surface p-4"
          >
            <span
              aria-hidden="true"
              className="absolute left-0 top-0 h-2 w-2 rounded-tl-[10px] bg-wallet"
            />
            <p className="text-sm text-text-2">{card.label}</p>
            <p className="tabular-num mt-2 text-2xl font-semibold text-text">
              {card.value}
            </p>
          </div>
        ))}
      </div>

      {NEXT_CLASS && (
        <div className="relative mt-4 flex items-center gap-2 rounded-[10px] border border-line bg-surface px-4 py-3 text-sm">
          <span
            aria-hidden="true"
            className="absolute left-0 top-0 h-2 w-2 rounded-tl-[10px] bg-sched"
          />
          <span className="shrink-0 font-medium text-text">下一节课</span>
          <span className="truncate text-text-2">
            {NEXT_CLASS.time} · {NEXT_CLASS.name} · {NEXT_CLASS.room}（
            {NEXT_CLASS.section}）
          </span>
        </div>
      )}

      <div className="mt-4 rounded-[10px] border border-line bg-surface p-4">
        <p className="text-sm font-medium text-text">快捷动作</p>
        <div className="mt-3 flex flex-wrap gap-2">
          {QUICK_ACTIONS.map((action) => {
            const panel = action.panel;
            return (
              <button
                key={action.label}
                type="button"
                onClick={panel ? () => setActivePanel(panel) : undefined}
                className="flex items-center gap-2 rounded-[10px] border border-line bg-bg px-4 py-2.5 text-sm text-text transition-colors hover:border-brand hover:text-brand"
              >
                <action.icon size={16} aria-hidden="true" />
                {action.label}
                {action.arrow && <ArrowRight size={14} aria-hidden="true" />}
              </button>
            );
          })}
        </div>
      </div>
    </section>
  );
}
