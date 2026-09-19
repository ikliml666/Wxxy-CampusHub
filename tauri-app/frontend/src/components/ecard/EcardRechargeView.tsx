import { Receipt } from "lucide-react";
import { useEffect, useState } from "react";
import { EmptyState } from "@/components/EmptyState";
import { RechargeFlow } from "@/components/RechargeFlow";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import { invokeCommand } from "@/shared/tauriApi";
import type { ElectricityBillPage, FeeItem } from "@/shared/types";

/**
 * 一卡通 · 卡片充值子页（M4.5 批 3）：一卡通充值在官方侧就是 `/charge` 体系的 **401 片区**
 * （`慧新易校一卡通充值`，计划 §1.7）。复用既有 `RechargeFlow`（金额 → 风险声明 → 支付方式 →
 * 免密/安全键盘 → 轮询 → 取消），只做两处适配：
 *
 * - **无级联上下文**：401 的 `getThirdData` 恒 `code=500`（live 实测），建单**不带
 *   `third_party`**——`RechargeFlow` 传 `skipThirdParty`，后端 `recharge_create(noContext)` 分支处理；
 * - **无上下限**：`maxmoney/retain_money` 均为 null（live 实测），服务端两侧都不校验金额，
 *   前端把关「> 0 且至多两位小数」（校验在 `RechargeFlow` 内，对两条流程通用）。
 *
 * 右列充值记录复用 `get_electricity_bills({ feeitemId })`（同一端点
 * `/charge/turnover/app_account`，`feeitemId=401` 即一卡通充值账单）。
 *
 * # 红线（照旧，不因复用而松动）
 *
 * - 不代做真实扣款：开发/点验阶段**绝不提交支付**；
 * - 安全键盘只传**位置下标**（见 `RechargeFlow` 文件头注，红线只此一份，不复制流程）。
 */

/** 充值记录三态（与 `ElectricityPaymentsCard` 的账单区同款四态：loading/ready/空/error+重试）。 */
type Phase = "loading" | "ready" | "error";

/** "YYYY-MM-DD HH:MM:SS" → "MM-DD HH:MM"（与缴费记录卡同口径）。 */
function shortTime(t: string): string {
  return t.slice(5, 16);
}

export function EcardRechargeView({ feeitemId, layout }: { feeitemId: string; layout: string[] }) {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const authed = status === "authed";

  /** 401 没有独立 FeeItem 目录项可复用：按实测字段就地合成（layout 来自 `singleFeeitem`，无上下限）。 */
  const feeitem: FeeItem = {
    id: feeitemId,
    name: "一卡通充值",
    billingUnit: "",
    layout: layout.map(Number),
    retainMoney: null,
    maxmoney: null,
    remark: "",
    lastLevelIsInput: false,
  };

  // —— 充值记录（右列；需登录）——
  const [bills, setBills] = useState<ElectricityBillPage["records"]>([]);
  const [billsTotal, setBillsTotal] = useState(0);
  const [billsPage, setBillsPage] = useState(1);
  const [billsPhase, setBillsPhase] = useState<Phase>("loading");
  const [billsError, setBillsError] = useState("");
  const [billsTick, setBillsTick] = useState(0);

  useEffect(() => {
    if (!authed) return;
    let alive = true;
    setBillsPhase("loading");
    invokeCommand<ElectricityBillPage>("get_electricity_bills", { feeitemId, page: billsPage }).then(
      (r) => {
        if (!alive) return;
        const data = r.data;
        if (r.success && data) {
          setBillsTotal(data.total);
          setBills((prev) => (billsPage === 1 ? data.records : [...prev, ...data.records]));
          setBillsPhase("ready");
        } else {
          setBillsError(r.message ?? "充值记录获取失败");
          setBillsPhase("error");
        }
      },
    );
    return () => {
      alive = false;
    };
  }, [authed, feeitemId, billsPage, billsTick]);

  /** 充值成功/取消后刷新记录（交给 RechargeFlow 完成态后的手动刷新即可，本批不做自动联动）。 */
  const billsRetry = () => {
    setBillsPage(1);
    setBills([]);
    setBillsTick((t) => t + 1);
  };

  const hasMore = bills.length > 0 && bills.length < billsTotal;

  return (
    <div className="grid items-start gap-3 lg:grid-cols-[minmax(0,1fr)_340px] lg:gap-4">
      {/* 左列：充值流程（金额 chips 按官方 layout 下发，无上下限，前端把关两位小数） */}
      <RechargeFlow feeitem={feeitem} roomLabel="一卡通充值" path={[]} skipThirdParty />

      {/* 右列：充值记录（/charge/turnover/app_account?feeitemid=401） */}
      <Surface accent="wallet" className="px-4 py-4">
        <div className="flex items-baseline justify-between gap-3">
          <p className="text-body font-medium text-text">充值记录</p>
          {authed && billsPhase !== "error" && billsTotal > 0 && (
            <span className="tabular-num shrink-0 text-caption text-text-2">
              共 {billsTotal} 条
            </span>
          )}
        </div>

        {!authed ? (
          <EmptyState
            compact
            icon={Receipt}
            domain="wallet"
            title="登录后查看充值记录"
            hint="充值记录来自学校系统，需登录后获取。"
            action={<Button onClick={openLoginDialog}>登录</Button>}
          />
        ) : billsPhase === "error" ? (
          <div className="mt-3 flex items-center justify-between gap-3">
            <p className="min-w-0 truncate text-body text-text-2">{billsError}</p>
            <Button variant="outline" size="xs" className="shrink-0" onClick={billsRetry}>
              重试
            </Button>
          </div>
        ) : bills.length === 0 ? (
          billsPhase === "loading" ? (
            <div aria-hidden className="mt-3 space-y-2">
              {[0, 1, 2].map((i) => (
                <div key={i} className="h-9 animate-pulse rounded bg-line" />
              ))}
            </div>
          ) : (
            <EmptyState
              compact
              icon={Receipt}
              domain="wallet"
              title="暂无充值记录"
              hint="充值成功后会出现在这里（学校侧账单）。"
            />
          )
        ) : (
          <ul className="mt-2 divide-y divide-line">
            {bills.map((b) => (
              <li key={b.id} className="flex items-baseline gap-3 py-2">
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-body text-text">
                    {b.itemName || "一卡通充值"}
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

        {authed && billsPhase !== "error" && hasMore && (
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
      </Surface>
    </div>
  );
}
