import { Receipt, Wallet } from "lucide-react";
import { useEffect, useState } from "react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import { invokeCommand } from "@/shared/tauriApi";
import { cn } from "@/shared/cn";
import type { EcardOverview, EcardTransaction, EcardTransactions } from "@/shared/types";

/** 余额/卡信息三态：加载骨架 → 数据 / 出错（可重试，余额显示 "—"）。 */
type OverviewState =
  | { phase: "loading" }
  | { phase: "ready"; data: EcardOverview }
  | { phase: "error"; message: string };

/** 流水三态：loading = 本页在取（首屏骨架 / 「加载更多」按钮态）/ idle / error（只影响列表区）。 */
type ListPhase = "loading" | "idle" | "error";

/** "YYYY-MM-DD HH:MM:SS" → "MM-DD HH:MM"（交易记录不跨年，行内保持紧凑）。 */
function shortTime(t: string): string {
  return t.slice(5, 16);
}

/** 带符号金额："+3.50" / "-11.50"。 */
function signedAmount(v: number): string {
  return `${v > 0 ? "+" : ""}${v.toFixed(2)}`;
}

/**
 * 钱包页（M3 批 2）：余额走慧新E校实时接口（`get_ecard`）。
 * 主数字 = **电子账户**余额（批 1 实测口径，见 `CardInfo` 注释），卡账户次级显示；
 * 今日/本月消费实测无解 → 「暂不可用」；交易记录分页（「加载更多」按 total 判断）。
 */
export function WalletPanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const authed = status === "authed";

  const [overview, setOverview] = useState<OverviewState>({ phase: "loading" });
  const [records, setRecords] = useState<EcardTransaction[]>([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(1);
  const [listPhase, setListPhase] = useState<ListPhase>("loading");
  const [listError, setListError] = useState("");
  const [reloadTick, setReloadTick] = useState(0);

  const account = overview.phase === "ready" ? overview.data.account : "";

  // 余额 + 卡信息（登录后取一次；失败可重试，不白屏）
  useEffect(() => {
    if (!authed) {
      setOverview({ phase: "loading" });
      return;
    }
    let alive = true;
    setOverview({ phase: "loading" });
    invokeCommand<EcardOverview>("get_ecard").then((r) => {
      if (!alive) return;
      if (r.success && r.data) setOverview({ phase: "ready", data: r.data });
      else setOverview({ phase: "error", message: r.message ?? "一卡通余额获取失败" });
    });
    return () => {
      alive = false;
    };
  }, [authed, reloadTick]);

  // 流水：卡号就绪后取；page === 1 替换、page > 1 追加（「加载更多」）
  useEffect(() => {
    if (!authed || !account) return;
    let alive = true;
    setListPhase("loading");
    invokeCommand<EcardTransactions>("get_ecard_transactions", { account, page }).then((r) => {
      if (!alive) return;
      const data = r.data;
      if (r.success && data) {
        setTotal(data.total);
        setRecords((prev) => (page === 1 ? data.records : [...prev, ...data.records]));
        setListPhase("idle");
      } else {
        setListError(r.message ?? "交易记录获取失败");
        setListPhase("error");
      }
    });
    return () => {
      alive = false;
    };
  }, [authed, account, page, reloadTick]);

  const retry = () => {
    setPage(1);
    setRecords([]);
    setReloadTick((t) => t + 1);
  };

  const card = overview.phase === "ready" ? overview.data.cards[0] : undefined;
  const balanceYuan = overview.phase === "ready" ? overview.data.balanceYuan : null;
  const cardBalanceYuan = overview.phase === "ready" ? overview.data.cardBalanceYuan : null;
  const todaySpend = overview.phase === "ready" ? overview.data.todaySpend : null;
  const monthSpend = overview.phase === "ready" ? overview.data.monthSpend : null;
  const hasMore = records.length > 0 && records.length < total;

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      <PanelHeader title="钱包" description="一卡通余额与消费记录" domain="wallet" />

      {!authed ? (
        <Surface>
          <EmptyState
            icon={Wallet}
            domain="wallet"
            title="登录后查看一卡通"
            hint="登录后查看一卡通余额与消费记录。"
            action={<Button onClick={openLoginDialog}>登录</Button>}
          />
        </Surface>
      ) : (
        <>
          {/* 余额大数字卡：电子账户为主口径，卡账户与卡状态作次级说明 */}
          <Surface accent="wallet" className="px-5 py-5">
            <p className="text-caption text-text-2">一卡通余额</p>
            {overview.phase === "loading" ? (
              <div aria-hidden className="mt-2 h-9 w-32 animate-pulse rounded bg-line" />
            ) : (
              <p className="tabular-num mt-2 text-display font-semibold text-text">
                {balanceYuan != null ? `¥ ${balanceYuan.toFixed(2)}` : "¥ —"}
              </p>
            )}
            <p className="mt-3 text-caption text-text-2">
              {[
                "电子账户",
                cardBalanceYuan != null ? `卡账户 ¥ ${cardBalanceYuan.toFixed(2)}` : null,
                card ? card.statusLabel : null,
              ]
                .filter(Boolean)
                .join(" · ")}
            </p>
          </Surface>

          {/* 余额取数失败：可重试（余额显示 "—"，流水区另有自己的多态，不整页报错） */}
          {overview.phase === "error" && (
            <Surface
              accent="info"
              className="mt-3 flex items-center justify-between gap-3 py-3 pl-4 pr-2"
            >
              <p className="min-w-0 truncate text-body text-text-2">
                一卡通数据获取失败：{overview.message}
              </p>
              <Button variant="outline" size="sm" className="shrink-0" onClick={retry}>
                重试
              </Button>
            </Surface>
          )}

          {/* 今日/本月消费：学校统计接口无数据（见 types.ts 注释）→ 暂不可用 */}
          <div className="mt-3 grid grid-cols-2 gap-3">
            {[
              { label: "今日消费", value: todaySpend },
              { label: "本月消费", value: monthSpend },
            ].map((item) => (
              <Surface key={item.label} className="px-4 py-4">
                <p className="text-caption text-text-2">{item.label}</p>
                {item.value != null ? (
                  <p className="tabular-num mt-2 text-title font-semibold text-text">
                    ¥ {item.value.toFixed(2)}
                  </p>
                ) : (
                  <p className="mt-2 text-body text-text-2">暂不可用</p>
                )}
              </Surface>
            ))}
          </div>

          {/* 交易记录：分页追加 + 加载/空/错误态；收入绿（域色 wallet） */}
          <Surface className="mt-3 px-4 py-4">
            <div className="flex items-baseline justify-between gap-3">
              <p className="text-body font-medium text-text">交易记录</p>
              {total > 0 && (
                <span className="tabular-num shrink-0 text-caption text-text-2">
                  共 {total} 条
                </span>
              )}
            </div>

            {listPhase === "error" ? (
              <div className="mt-3 flex items-center justify-between gap-3">
                <p className="min-w-0 truncate text-body text-text-2">{listError}</p>
                <Button variant="outline" size="sm" className="shrink-0" onClick={retry}>
                  重试
                </Button>
              </div>
            ) : records.length === 0 ? (
              listPhase === "loading" ? (
                <div aria-hidden className="mt-3 space-y-2">
                  {[0, 1, 2].map((i) => (
                    <div key={i} className="h-10 animate-pulse rounded bg-line" />
                  ))}
                </div>
              ) : (
                <EmptyState
                  compact
                  icon={Receipt}
                  domain="wallet"
                  title="暂无交易记录"
                  hint="还没有一卡通消费或充值记录。"
                />
              )
            ) : (
              <ul className="mt-2 divide-y divide-line">
                {records.map((r, i) => (
                  <li key={`${r.time}-${i}`} className="flex items-baseline gap-3 py-2.5">
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-body text-text">
                        {r.summary || r.payName || "交易"}
                      </span>
                      <span className="mt-0.5 block truncate text-caption text-text-2">
                        {shortTime(r.time)}
                        {r.locationName ? ` · ${r.locationName}` : ""}
                      </span>
                    </span>
                    <span
                      className={cn(
                        "tabular-num shrink-0 text-body",
                        r.isIncome ? "text-wallet" : "text-text",
                      )}
                    >
                      {signedAmount(r.amountYuan)}
                    </span>
                  </li>
                ))}
              </ul>
            )}

            {listPhase !== "error" && hasMore && (
              <div className="mt-3 flex justify-center">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={listPhase === "loading"}
                  onClick={() => setPage((p) => p + 1)}
                >
                  {listPhase === "loading" ? "加载中…" : "加载更多"}
                </Button>
              </div>
            )}
          </Surface>
        </>
      )}
    </section>
  );
}
