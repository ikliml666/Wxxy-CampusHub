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

/**
 * 本机记录的「已取消订单」。
 *
 * 2026-09-19 实测（用户反馈后复核）：学校侧的取消是**物理删除**——取消后该订单在「全部」
 * 查询里也不再返回（`status=0` / `1` / `2` / 不带 status 四种查询都查不到它），所以
 * **学校侧根本没有「已取消」这个状态可供渲染**。用户取消完就再也看不到那笔单的归宿，
 * 故由本机留痕，并在界面上明确标注「本机记录」，与学校侧下发的状态区分开。
 */
type CancelledOrder = {
  orderId: string;
  amountYuan: number | null;
  commitDate: string;
  abstracts: string;
  cancelledAt: string;
};

const CANCELLED_KEY = "campushub-elec-cancelled";
const CANCELLED_MAX = 10;

function loadCancelled(): CancelledOrder[] {
  try {
    const raw = localStorage.getItem(CANCELLED_KEY);
    const v = raw ? JSON.parse(raw) : [];
    return Array.isArray(v) ? (v as CancelledOrder[]) : [];
  } catch {
    // 存储被禁用或内容损坏：静默回空，不影响缴费记录主流程
    return [];
  }
}

function saveCancelled(list: CancelledOrder[]) {
  try {
    localStorage.setItem(CANCELLED_KEY, JSON.stringify(list.slice(0, CANCELLED_MAX)));
  } catch {
    /* 配额满/被禁用：忽略 */
  }
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
  /** 本机留痕的已取消订单（学校侧取消即删除、无状态可查，见 CancelledOrder 注释）。 */
  const [cancelled, setCancelled] = useState<CancelledOrder[]>(() => loadCancelled());

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
  /** 月度区默认只露「全年合计 + 有缴费的月份」，全年 12 个月按需展开（高度协调，M4 点验反馈）。 */
  const [showAllMonths, setShowAllMonths] = useState(false);

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

  /** 取消遗留订单：成功后订单列表与账单各刷一次，并在**本机**留下「已取消」痕迹。副作用命令，不重试。 */
  const cancelOrder = async (orderId: string) => {
    // 先抓下这条单的摘要——取消后学校侧就不再返回它了，之后没机会再取
    const target = orders.find((o) => o.orderId === orderId);
    setCancelBusyId(orderId);
    setCancelMsg("");
    const r = await invokeCommand("recharge_cancel", { orderId });
    setCancelBusyId("");
    if (r.success) {
      const next = [
        {
          orderId,
          amountYuan: target?.amountYuan ?? null,
          commitDate: target?.commitDate ?? "",
          abstracts: target?.abstracts ?? "",
          cancelledAt: new Date().toISOString(),
        },
        ...cancelled.filter((c) => c.orderId !== orderId),
      ].slice(0, CANCELLED_MAX);
      saveCancelled(next);
      setCancelled(next);
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
  /** 有缴费的月份（>0）；空月 = 0 元是事实而非取数失败，默认收起、文案里点明。 */
  const paidMonths = monthly.filter((m) => m.amountYuan > 0);
  const yearTotal = monthly.reduce((s, m) => s + m.amountYuan, 0);
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

      {/* 已取消订单（**本机记录**）：学校侧取消即物理删除、不留状态，故痕迹只能本机留 */}
      {cancelled.length > 0 && (
        <ul className="mt-2 divide-y divide-line rounded-inner border border-line bg-surface-2 px-3">
          {cancelled.map((c) => (
            <li key={c.orderId} className="flex items-center gap-3 py-2.5">
              <span className="min-w-0 flex-1">
                <span className="block text-body text-text-2">
                  已取消
                  {c.amountYuan != null && (
                    <span className="tabular-num ml-1 text-text-2 line-through">
                      ¥ {c.amountYuan.toFixed(2)}
                    </span>
                  )}
                </span>
                <span className="mt-0.5 block truncate text-caption text-text-2">
                  {[c.commitDate ? shortTime(c.commitDate) : "", c.abstracts]
                    .filter(Boolean)
                    .join(" · ")}
                  {c.commitDate || c.abstracts ? " · " : ""}
                  本机记录
                </span>
              </span>
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
            {[0, 1].map((i) => (
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
        ) : (
          <>
            {/* 紧凑形态：全年合计一眼可见，默认只列有缴费的月份；空月 = 0 元是事实，
                在提示文案里点明「非取数失败」。全年 12 个月按需展开（2 列小格）。 */}
            <div className="mt-2 flex items-baseline justify-between gap-3">
              <span className="text-caption text-text-2">全年合计</span>
              <span className="tabular-num text-body font-medium text-text">
                ¥ {yearTotal.toFixed(2)}
              </span>
            </div>
            {paidMonths.length === 0 ? (
              <p className="mt-1.5 text-caption text-text-2">
                12 个月均无缴费记录（学校侧空月返回为空，按 0 元计，不是取数失败）。
              </p>
            ) : (
              <>
                <ul className="mt-1 divide-y divide-line">
                  {paidMonths.map((m) => (
                    <li key={m.month} className="flex items-center gap-2 py-1.5">
                      <span className="tabular-num w-10 shrink-0 text-caption text-text-2">
                        {m.month.slice(5)}月
                      </span>
                      <span className="h-3 min-w-0 flex-1 overflow-hidden rounded-inner bg-surface-2">
                        <span
                          aria-hidden
                          className="bg-wallet block h-full rounded-inner"
                          style={{ width: `${Math.max((m.amountYuan / monthlyMax) * 100, 2)}%` }}
                        />
                      </span>
                      <span className="tabular-num w-14 shrink-0 text-right text-caption text-text">
                        {m.amountYuan.toFixed(2)}
                      </span>
                    </li>
                  ))}
                </ul>
                <p className="mt-1 text-caption text-text-2">
                  其余 {12 - paidMonths.length} 个月无缴费（0 元）。
                </p>
              </>
            )}
            <div className="mt-1">
              <Button
                variant="ghost"
                size="sm"
                aria-expanded={showAllMonths}
                onClick={() => setShowAllMonths((v) => !v)}
              >
                {showAllMonths ? "收起" : "展开全年 12 个月"}
              </Button>
            </div>
            {showAllMonths && (
              <ul className="mt-1 grid grid-cols-2 gap-x-4">
                {monthly.map((m) => (
                  <li
                    key={m.month}
                    className="flex items-baseline justify-between gap-2 border-b border-line py-1"
                  >
                    <span className="tabular-num text-caption text-text-2">
                      {m.month.slice(5)}月
                    </span>
                    <span
                      className={cn(
                        "tabular-num text-caption",
                        m.amountYuan > 0 ? "font-medium text-text" : "text-text-2",
                      )}
                    >
                      {m.amountYuan.toFixed(2)}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </>
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
