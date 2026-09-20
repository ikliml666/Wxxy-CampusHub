import type { LucideIcon } from "lucide-react";
import {
  ArrowLeftRight,
  ArrowRight,
  BarChart3,
  CreditCard,
  Landmark,
  Receipt,
  ScanFace,
  Settings2,
  Zap,
} from "lucide-react";
import { Surface } from "@/components/Surface";
import { domainVar } from "@/components/PanelHeader";
import { Button } from "@/components/ui/button";
import { cn } from "@/shared/cn";
import type { EcardCardsOverview, EcardClientConfig, EcardView } from "@/shared/types";

type Tile = {
  id: Exclude<EcardView, "home">;
  label: string;
  hint: string;
  icon: LucideIcon;
  /** 大屏列跨度（窄屏一律单列）；宫格故意不做均匀 3×3——电费自带两个口径，占两格更如实 */
  span?: string;
  /** 主行动（充值）：域色实底，视觉权重最高 */
  primary?: boolean;
  /**
   * 可选门控：学校配置满足条件才显示（如银行卡按 `enabledApps` 门控）。
   * 不传 = 恒显示；config 缺省（概览未就绪）时也不门控。
   */
  visible?: (config: EcardClientConfig) => boolean;
};

const TILES: Tile[] = [
  {
    id: "recharge",
    label: "充值",
    hint: "给校园卡充值",
    icon: CreditCard,
    primary: true,
  },
  {
    id: "power",
    label: "宿舍电费",
    hint: "余额查询 · 充值 · 缴费记录",
    icon: Zap,
    span: "lg:col-span-2",
  },
  { id: "bill", label: "流水账单", hint: "收支明细与筛选", icon: Receipt },
  { id: "stats", label: "收支统计", hint: "本月 / 本年趋势", icon: BarChart3 },
  {
    // 始终显示：除挂失外还含改密/限额/圈存，不受 showLost 门控（挂失分区在子页内门控）
    id: "cardops",
    label: "卡设置",
    hint: "挂失 · 密码 · 限额 · 圈存",
    icon: Settings2,
  },
  {
    id: "transfer",
    label: "账户转账",
    hint: "卡账户 ↔ 电子账户",
    icon: ArrowLeftRight,
  },
  {
    // 本校 getAllApps 清单含 yinhangka / bind-bank-card ⇒ 显示；两者都缺则隐藏。
    // 清单不含 bind-campus-card ⇒ 不做多卡绑定入口（后端命令存在但本页不渲染）。
    id: "bank",
    label: "银行卡",
    hint: "绑定与解绑",
    icon: Landmark,
    visible: (config) =>
      config.enabledApps.includes("yinhangka") ||
      config.enabledApps.includes("bind-bank-card"),
  },
  {
    id: "face",
    label: "人脸采集",
    hint: "上传 / 更换刷脸照片",
    icon: ScanFace,
  },
];

/** 主余额带的取值口径：`config.balanceShowsElectronic` 为真用电子账户，否则卡账户。 */
function primaryBalance(data: EcardCardsOverview): number {
  const card = data.cards[0];
  if (!card) return 0;
  return data.config.balanceShowsElectronic
    ? card.elecBalanceYuan
    : card.balanceYuan;
}

function yuan(v: number): string {
  return `¥ ${v.toFixed(2)}`;
}

/** 一句「其余账户」副行：把没在当主口径的那个余额说清楚，不重复主数字。 */
function secondaryLine(data: EcardCardsOverview): string {
  const card = data.cards[0];
  if (!card) return "";
  const other = data.config.balanceShowsElectronic
    ? `卡账户 ${yuan(card.balanceYuan)}`
    : `电子账户 ${yuan(card.elecBalanceYuan)}`;
  return [
    other,
    card.statusLabel,
    card.accountMasked,
    card.cardTypeName,
  ]
    .filter(Boolean)
    .join(" · ");
}

export function EcardHome({
  phase,
  data,
  error,
  onRetry,
  onOpen,
  config,
}: {
  phase: "loading" | "ready" | "error";
  data: EcardCardsOverview | null;
  error: string;
  onRetry: () => void;
  onOpen: (v: EcardView) => void;
  /** 学校配置（门控用）；缺省时不门控、恒显示全部宫格项 */
  config?: EcardClientConfig | null;
}) {
  const overloaded = phase === "loading" || data === null;

  return (
    <div className="space-y-3">
      {/* 主余额带：最常看的一眼数字，放宫格之前；整条可点进账户详情 */}
      <Surface accent="wallet" className="px-5 py-5">
        <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-3">
          <div className="min-w-0">
            <p className="text-caption text-text-2">
              {phase === "ready" && data
                ? data.config.balanceShowsElectronic
                  ? "电子账户余额"
                  : "卡账户余额"
                : "一卡通余额"}
            </p>
            {overloaded ? (
              <div
                aria-hidden
                className="mt-2 h-9 w-32 animate-pulse rounded bg-line"
              />
            ) : (
              <p className="tabular-num mt-2 text-display font-semibold text-text">
                {yuan(primaryBalance(data))}
              </p>
            )}
            {phase === "ready" && data && (
              <p className="mt-3 text-caption text-text-2">{secondaryLine(data)}</p>
            )}
          </div>
          {phase === "ready" && data && data.cards.length > 0 && (
            <Button
              variant="outline"
              size="sm"
              className="shrink-0"
              onClick={() => onOpen("balance")}
            >
              账户详情
              <ArrowRight className="size-3.5" aria-hidden />
            </Button>
          )}
        </div>

        {/* 取数失败只坏这一条带，宫格照常可点（子页各自取数、互不牵连） */}
        {phase === "error" && (
          <div className="mt-3 flex items-center justify-between gap-3">
            <p className="min-w-0 truncate text-caption text-text-2">
              余额获取失败：{error}
            </p>
            <Button variant="outline" size="sm" className="shrink-0" onClick={onRetry}>
              重试
            </Button>
          </div>
        )}
      </Surface>

      {/* 宫格：每一格是一个子页入口；带 visible 判据的项按 config 门控 */}
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {TILES.filter(
          (tile) => !tile.visible || !config || tile.visible(config),
        ).map((tile) => {
          const Icon = tile.icon;
          return (
            <button
              key={tile.id}
              type="button"
              onClick={() => onOpen(tile.id)}
              className={cn(
                "group relative flex min-h-[96px] flex-col items-start gap-1.5 rounded-card border p-4 text-left",
                "transition-[transform,box-shadow] duration-[var(--dur-base)] ease-out-soft",
                "hover:-translate-y-[1px] hover:shadow-lift",
                "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-wallet)] focus-visible:ring-offset-2 focus-visible:ring-offset-bg",
                tile.primary
                  ? "border-transparent shadow-card"
                  : "border-line bg-surface shadow-card",
                tile.span,
              )}
              style={
                tile.primary
                  ? {
                      backgroundColor:
                        "color-mix(in srgb, var(--color-wallet) 14%, var(--color-surface))",
                    }
                  : undefined
              }
            >
              <span
                aria-hidden
                className="flex size-8 items-center justify-center rounded-inner"
                style={{
                  backgroundColor:
                    "color-mix(in srgb, var(--color-wallet) 14%, transparent)",
                  color: domainVar.wallet,
                }}
              >
                <Icon className="size-4" />
              </span>
              <span className="text-body font-medium text-text">{tile.label}</span>
              <span className="text-caption text-text-2">{tile.hint}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
