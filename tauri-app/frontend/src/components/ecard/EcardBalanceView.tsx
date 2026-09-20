import { Wallet } from "lucide-react";
import { useState } from "react";
import { EmptyState } from "@/components/EmptyState";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { cn } from "@/shared/cn";
import type { EcardAccountInfo, EcardCard, EcardCardsOverview } from "@/shared/types";

/**
 * 一卡通「卡包/余额」子页（M4.5）。数据由容器面板取好传进来（`get_ecard_overview`），
 * 本组件只负责渲染四态：loading 骨架 / ready / error 重试 / 空卡列表。
 *
 * 余额口径：`config.balanceShowsElectronic`（本校 type=1 ⇒ true）⇒ 主数字 = 电子账户
 * `elecBalanceYuan`，卡账户 `balanceYuan` 作次级说明；口径为 false 时对调。
 * 只读展示：挂失/改密/转账等写操作一律不在此出现。
 */

/** 三项限额为 0 时显示「未设置」而不是 ¥0.00（0 = 学校侧没设限额）。 */
function limitText(v: number): string {
  return v > 0 ? `¥ ${v.toFixed(2)}` : "未设置";
}

/** 卡状态徽标：挂失 → 域琥珀、冻结 → 告警红、其余 → 域绿「正常」。 */
function StatusBadge({ card }: { card: EcardCard }) {
  const lost = card.lost;
  const frozen = card.frozen;
  const label = lost ? "已挂失" : frozen ? "已冻结" : "正常";
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-control px-2 py-0.5 text-caption font-medium",
        lost
          ? "bg-todo/10 text-todo"
          : frozen
            ? "bg-alert/10 text-alert"
            : "bg-wallet/10 text-wallet",
      )}
    >
      {label}
    </span>
  );
}

/** 子账户明细行（`accinfo[]`）：类型码 + 余额 + 当日已消费 + 三项限额。 */
function AccInfoRow({ info }: { info: EcardAccountInfo }) {
  return (
    <div className="py-2.5">
      <div className="flex items-baseline justify-between gap-3">
        <span className="min-w-0 truncate text-body text-text">子账户 {info.type}</span>
        <span className="tabular-num shrink-0 text-body font-medium text-text">
          ¥ {info.balanceYuan.toFixed(2)}
        </span>
      </div>
      <dl className="mt-1 grid grid-cols-2 gap-x-4 gap-y-1 sm:grid-cols-5">
        {[
          { label: "当日已消费", value: `¥ ${info.dayCostAmtYuan.toFixed(2)}` },
          { label: "单日消费限额", value: limitText(info.dayCostLimitYuan) },
          { label: "免密限额", value: limitText(info.nonpwdLimitYuan) },
          { label: "单笔限额", value: limitText(info.singleLimitYuan) },
        ].map((item) => (
          <div key={item.label} className="min-w-0">
            <dt className="text-caption text-text-2">{item.label}</dt>
            <dd className="tabular-num text-caption text-text">{item.value}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

export function EcardBalanceView({
  phase,
  data,
  error,
  onRetry,
}: {
  phase: "loading" | "ready" | "error";
  data: EcardCardsOverview | null;
  error: string;
  onRetry: () => void;
}) {
  // 多卡时的当前卡下标；换卡 / 数据刷新后由渲染处钳制
  const [cardIdx, setCardIdx] = useState(0);

  if (phase === "loading") {
    return (
      <div aria-hidden className="flex flex-col gap-3">
        <Surface className="px-5 py-5">
          <div className="h-4 w-24 animate-pulse rounded bg-line" />
          <div className="mt-3 h-9 w-40 animate-pulse rounded bg-line" />
          <div className="mt-3 h-4 w-2/3 animate-pulse rounded bg-line" />
        </Surface>
        <Surface className="px-4 py-4">
          <div className="h-4 w-28 animate-pulse rounded bg-line" />
          <div className="mt-3 space-y-2">
            {[0, 1, 2].map((i) => (
              <div key={i} className="h-8 animate-pulse rounded bg-line" />
            ))}
          </div>
        </Surface>
      </div>
    );
  }

  if (phase === "error") {
    return (
      <Surface accent="info" className="flex items-center justify-between gap-3 px-4 py-3">
        <p className="min-w-0 text-body text-text-2">获取失败：{error}</p>
        <Button variant="outline" size="sm" className="shrink-0" onClick={onRetry}>
          重试
        </Button>
      </Surface>
    );
  }

  const cards = data?.cards ?? [];
  const config = data?.config;
  if (cards.length === 0) {
    return (
      <Surface>
        <EmptyState icon={Wallet} domain="wallet" title="没有可用校园卡" />
      </Surface>
    );
  }

  const card = cards[Math.min(cardIdx, cards.length - 1)]!;
  const showElec = config?.balanceShowsElectronic ?? true;
  const mainBalance = showElec ? card.elecBalanceYuan : card.balanceYuan;
  const otherBalance = showElec ? card.balanceYuan : card.elecBalanceYuan;
  const otherLabel = showElec ? "卡账户" : "电子账户";

  return (
    <div className="flex flex-col gap-3">
      {/* 多卡切换：单卡不渲染 */}
      {cards.length > 1 && (
        <div
          role="tablist"
          aria-label="切换校园卡"
          className="flex gap-1 self-start rounded-control border border-line bg-surface-2 p-1"
        >
          {cards.map((c, i) => (
            <button
              key={c.accountMasked}
              type="button"
              role="tab"
              aria-selected={i === cardIdx}
              className={cn(
                "min-h-8 rounded-control px-3 text-caption transition-colors duration-[var(--dur-fast)] ease-out-soft",
                i === cardIdx
                  ? "bg-surface font-medium text-text shadow-card"
                  : "text-text-2 hover:text-text",
              )}
              onClick={() => setCardIdx(i)}
            >
              {c.accountMasked}
            </button>
          ))}
        </div>
      )}

      {/* 主余额大数字卡 */}
      <Surface accent="wallet" className="px-5 py-5">
        <div className="flex items-center justify-between gap-3">
          <p className="text-caption text-text-2">{showElec ? "电子账户余额" : "卡账户余额"}</p>
          <StatusBadge card={card} />
        </div>
        <p
          className={cn(
            "tabular-num mt-2 text-display font-semibold",
            mainBalance < 0 ? "text-alert" : "text-text",
          )}
        >
          ¥ {mainBalance.toFixed(2)}
        </p>
        <p className="mt-3 text-caption text-text-2">
          {[
            `${otherLabel} ¥ ${otherBalance.toFixed(2)}`,
            card.statusLabel,
            card.accountMasked,
            card.cardTypeName,
          ]
            .filter(Boolean)
            .join(" · ")}
        </p>
        {card.expDate && (
          <p className="mt-1 text-caption text-text-2">有效期至 {card.expDate}</p>
        )}
        {card.openDate && (
          <p className="mt-1 text-caption text-text-2">开户时间 {card.openDate}</p>
        )}
        {card.dayCostAmtYuan !== null && (
          <p className="mt-1 text-caption text-text-2">
            当日已消费 ¥ {card.dayCostAmtYuan.toFixed(2)}
          </p>
        )}
      </Surface>

      {/* 子账户明细（accinfo[]） */}
      {card.accInfos.length > 0 && (
        <Surface className="px-4 py-4">
          <p className="text-body font-medium text-text">子账户明细</p>
          <div className="mt-1 divide-y divide-line">
            {card.accInfos.map((info, i) => (
              <AccInfoRow key={`${info.type}-${i}`} info={info} />
            ))}
          </div>
        </Surface>
      )}

      {/* 自动转账（圈存）与已绑银行卡 */}
      <Surface className="px-4 py-4">
        <p className="text-body font-medium text-text">转账与绑定</p>
        <dl className="mt-2 space-y-1.5">
          <div className="flex items-baseline justify-between gap-3">
            <dt className="text-caption text-text-2">自动转账（圈存）</dt>
            <dd className="text-body text-text">
              {card.autotransFlag ? "开启" : "关闭"}
              {card.autotransFlag && (
                <span className="tabular-num text-text-2">
                  {" "}
                  · 每次圈 ¥ {card.autotransAmtYuan.toFixed(2)}
                </span>
              )}
            </dd>
          </div>
          <div className="flex items-baseline justify-between gap-3">
            <dt className="text-caption text-text-2">余额下限</dt>
            <dd className="tabular-num text-body text-text">
              {limitText(card.autotransLimiteYuan)}
            </dd>
          </div>
          <div className="flex items-baseline justify-between gap-3">
            <dt className="text-caption text-text-2">已绑银行卡</dt>
            <dd className="text-body text-text">
              {card.bankaccTail ? `尾号 ${card.bankaccTail}` : "未绑定"}
            </dd>
          </div>
        </dl>
      </Surface>
    </div>
  );
}
