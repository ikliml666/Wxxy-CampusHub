import { Receipt } from "lucide-react";
import { useEffect, useState } from "react";
import { EmptyState } from "@/components/EmptyState";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { invokeCommand } from "@/shared/tauriApi";
import { cn } from "@/shared/cn";
import type {
  ElectricityBill,
  ElectricityBillPage,
  ElectricityMonthTotal,
  ElectricityOrder,
} from "@/shared/types";

/**
 * 缴费记录卡（M4 批 3，右列）：**未支付订单置顶**（取消走既有 `recharge_cancel`，清掉
 * 任务书 §1.6 那笔遗留单）→ 月度缴费横向柱 → 账单列表（「加载更多」按 `total`）。
 *
 * ⚠️ `get_electricity_monthly` 一次调用发 **12 个串行请求并持 synjones 全局锁**（可能数秒），
 * 故月度区有**独立的三态**，绝不让它拖住订单/账单区。
 */

/** 三区各自独立：月度与订单用 phase 三态；账单加页码。 */
type Phase = "loading" | "ready" | "error";

/** "YYYY-MM-DD HH:MM:SS" → "MM-DD HH:MM"（与钱包页同口径，行内保持紧凑）。 */
function shortTime(t: string): string {
  return t.slice(5, 16);
}

export function ElectricityPaymentsCard({
  authed,
  openLoginDialog,
}: {
  authed: boolean;
  openLoginDialog: () => void;
}) {
  // —— 待支付订单（status=0；学校侧实测能查到，取消复用 recharge_cancel）——
  const [orders, setOrders] = useState<ElectricityOrder[]>([]);
  const [ordersPhase, setOrdersPhase] = useState<Phase>("loading");
  const [ordersTick, setOrdersTick] = useState(0);
  const [cancelBusyId, setCancelBusyId] = useState("");
  const [cancelMsg, setCancelMsg] = useState("");

  // —— 月度缴费（慢请求，独立 loading）——
  const [monthly, setMonthly] = useState<ElectricityMonthTotal[]>([]);
  const [monthlyPhase, setMonthlyPhase] = useState<Phase>("loading");
  const [monthlyError, setMonthlyError] = useState("");
  const [monthlyTick, setMonthlyTick] = useState(0);

  // —— 账单列表（分页）——
  const [bills, setBills] = useState<ElectricityBill[]>([]);
  const [billsTotal, setBillsTotal] = useState(0);
  const [billsPage, setBillsPage] = useState(1);
  const [billsPhase, setBillsPhase] = useState<Phase>("loading");
  const [billsError, setBillsError] = useState("");
  const [billsTick, setBillsTick] = useState(0);

  useEffect(() => {
    if (!authed) return;
    let alive = true;
    setOrdersPhase("loading");
    invokeCommand<ElectricityOrder[]>("get_electricity_orders", { status: 0 }).then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        setOrders(r.data);
        setOrdersPhase("ready");
      } else {
        setOrdersPhase("error");
        setCancelMsg(r.message ?? "待支付订单获取失败");
      }
    });
    return () => {
      alive = false;
    };
  }, [authed, ordersTick]);

  useEffect(() => {
    if (!authed) return;
    let alive = true;
    setMonthlyPhase("loading");
    // 全局锁 12 连发，可能数秒：只有本区块转 loading
    invokeCommand<ElectricityMonthTotal[]>("get_electricity_monthly", {}).then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        setMonthly(r.data);
        setMonthlyPhase("ready");
      } else {
        setMonthlyError(r.message ?? "月度缴费获取失败");
        setMonthlyPhase("error");
      }
    });
    return () => {
      alive = false;
    };
  }, [authed, monthlyTick]);

  useEffect(() => {
    if (!authed) return;
    let alive = true;
    setBillsPhase("loading");
    invokeCommand<ElectricityBillPage>("get_electricity_bills", { page: billsPage }).then(
      (r) => {
        if (!alive) return;
        const data = r.data;
        if (r.success && data) {
          setBillsTotal(data.total);
          setBills((prev) => (billsPage === 1 ? data.records : [...prev, ...data.records]));
          setBillsPhase("ready");
        } else {
          setBillsError(r.message ?? "缴费账单获取失败");
          setBillsPhase("error");
        }
      },
    );
    return () => {
      alive = false;
    };
  }, [authed, billsPage, billsTick]);

  /** 取消遗留订单：成功后订单列表与账单各刷一次。副作用命令，不重试。 */
  const cancelOrder = async (orderId: string) => {
    setCancelBusyId(orderId);
    setCancelMsg("");
    const r = await invokeCommand("recharge_cancel", { orderId });
    setCancelBusyId("");
    if (r.success) {
      setOrdersTick((t) => t + 1);
      setBillsTick((t) => t + 1);
      setCancelMsg("订单已取消");
    } else {
      setCancelMsg(r.message ?? "取消订单失败");
    }
  };

  const billsRetry = () => {
    setBillsPage(1);
    setBills([]);
    setBillsTick((t) => t + 1);
  };

  if (!authed) {
    return (
      <Surface className="px-4 py-4">
        <p className="text-body font-medium text-text">缴费记录</p>
        <EmptyState
          compact
          icon={Receipt}
          domain="wallet"
          title="登录后查看缴费记录"
          hint="缴费账单与待支付订单来自学校系统，需登录后获取。"
          action={<Button onClick={openLoginDialog}>登录</Button>}
        />
      </Surface>
    );
  }

  const monthlyMax = Math.max(...monthly.map((m) => m.amountYuan), 0.01);
  const hasBillsMore = bills.length > 0 && bills.length < billsTotal;

  return (
    <Surface accent="wallet" className="px-4 py-4">
      <p className="text-body font-medium text-text">缴费记录</p>

      {/* 未支付订单：置顶 + 可取消（消除历史遗留单） */}
      {ordersPhase === "loading" && (
        <div aria-hidden className="mt-3 h-10 animate-pulse rounded-inner bg-line" />
      )}
      {ordersPhase === "ready" && orders.length > 0 && (
        <ul className="mt-3 divide-y divide-line rounded-inner border border-line bg-surface-2 px-3">
          {orders.map((o) => (
            <li key={o.orderId} className="flex items-center gap-3 py-2.5">
              <span className="min-w-0 flex-1">
                <span className="block text-body text-text">
                  待支付
                  {o.amountYuan != null && (
                    <span className="tabular-num text-alert"> ¥ {o.amountYuan.toFixed(2)}</span>
                  )}
                </span>
                <span className="mt-0.5 block truncate text-caption text-text-2">
                  {[shortTime(o.commitDate), o.abstracts].filter(Boolean).join(" · ")}
                </span>
              </span>
              <Button
                variant="outline"
                size="sm"
                className="shrink-0"
                aria-busy={cancelBusyId === o.orderId}
                disabled={cancelBusyId !== ""}
                onClick={() => void cancelOrder(o.orderId)}
              >
                取消订单
              </Button>
            </li>
          ))}
        </ul>
      )}
      {cancelMsg && <p className="mt-2 text-caption text-text-2">{cancelMsg}</p>}

      {/* 月度缴费：横向 CSS 柱（零依赖）；独立三态 */}
      <div className="mt-3">
        <div className="flex items-baseline justify-between gap-3">
          <p className="text-caption text-text-2">今年每月缴费</p>
          {monthlyPhase === "loading" && (
            <span className="text-caption text-text-2" aria-busy>
              逐月查询中（约数秒）…
            </span>
          )}
        </div>
        {monthlyPhase === "loading" ? (
          <div aria-hidden className="mt-2 space-y-1.5">
            {[0, 1, 2, 3].map((i) => (
              <div key={i} className="h-4 animate-pulse rounded bg-line" />
            ))}
          </div>
        ) : monthlyPhase === "error" ? (
          <div className="mt-2 flex items-center justify-between gap-3">
            <p className="min-w-0 truncate text-caption text-text-2">{monthlyError}</p>
            <Button
              variant="outline"
              size="xs"
              className="shrink-0"
              onClick={() => setMonthlyTick((t) => t + 1)}
            >
              重试
            </Button>
          </div>
        ) : monthly.every((m) => m.amountYuan === 0) ? (
          <p className="mt-2 text-caption text-text-2">
            今年还没有缴费记录（学校侧逐月返回为空）。
          </p>
        ) : (
          <ul className="mt-2">
            {monthly.map((m) => (
              <li key={m.month} className="flex items-center gap-2 py-1">
                <span className="tabular-num w-12 shrink-0 text-caption text-text-2">
                  {m.month.slice(5)}月
                </span>
                <span className="h-3 min-w-0 flex-1 overflow-hidden rounded-inner bg-surface-2">
                  <span
                    aria-hidden
                    className="bg-wallet block h-full rounded-inner"
                    style={{ width: `${Math.max((m.amountYuan / monthlyMax) * 100, 2)}%` }}
                  />
                </span>
                <span className="tabular-num w-16 shrink-0 text-right text-caption text-text">
                  {m.amountYuan.toFixed(2)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>

      {/* 账单列表 */}
      <div className="mt-3">
        <div className="flex items-baseline justify-between gap-3">
          <p className="text-caption text-text-2">缴费账单</p>
          {billsTotal > 0 && (
            <span className="tabular-num shrink-0 text-caption text-text-2">
              共 {billsTotal} 条
            </span>
          )}
        </div>
        {billsPhase === "error" ? (
          <div className="mt-2 flex items-center justify-between gap-3">
            <p className="min-w-0 truncate text-caption text-text-2">{billsError}</p>
            <Button variant="outline" size="xs" className="shrink-0" onClick={billsRetry}>
              重试
            </Button>
          </div>
        ) : bills.length === 0 ? (
          billsPhase === "loading" ? (
            <div aria-hidden className="mt-2 space-y-2">
              {[0, 1, 2].map((i) => (
                <div key={i} className="h-9 animate-pulse rounded bg-line" />
              ))}
            </div>
          ) : (
            <p className="mt-2 text-caption text-text-2">
              暂无缴费账单（学校侧返回为空，充值成功后会出现在这里）。
            </p>
          )
        ) : (
          <ul className={cn("mt-2 divide-y divide-line")}>
            {bills.map((b) => (
              <li key={b.id} className="flex items-baseline gap-3 py-2">
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-body text-text">
                    {b.itemName || "缴费"}
                  </span>
                  <span className="mt-0.5 block truncate text-caption text-text-2">
                    {[shortTime(b.time), b.typeName].filter(Boolean).join(" · ")}
                  </span>
                </span>
                {b.amountYuan != null ? (
                  <span className="tabular-num text-wallet shrink-0 text-body">
                    +{b.amountYuan.toFixed(2)}
                  </span>
                ) : (
                  <span className="shrink-0 text-caption text-text-2">金额未知</span>
                )}
              </li>
            ))}
          </ul>
        )}
        {billsPhase !== "error" && hasBillsMore && (
          <div className="mt-2 flex justify-center">
            <Button
              variant="outline"
              size="sm"
              disabled={billsPhase === "loading"}
              onClick={() => setBillsPage((p) => p + 1)}
            >
              {billsPhase === "loading" ? "加载中…" : "加载更多"}
            </Button>
          </div>
        )}
      </div>
    </Surface>
  );
}
