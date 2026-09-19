import { ChevronLeft, Wallet } from "lucide-react";
import { useEffect, useState } from "react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { EcardBalanceView } from "@/components/ecard/EcardBalanceView";
import { EcardBillView } from "@/components/ecard/EcardBillView";
import { EcardHome } from "@/components/ecard/EcardHome";
import { EcardPowerView } from "@/components/ecard/EcardPowerView";
import { EcardRechargeView } from "@/components/ecard/EcardRechargeView";
import { EcardStatsView } from "@/components/ecard/EcardStatsView";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import { invokeCommand } from "@/shared/tauriApi";
import type { EcardCardsOverview, EcardView } from "@/shared/types";

/** 概览三态：只驱动宫格顶部余额带与账户详情子页；子页各自取数，互不牵连。 */
type OverviewState =
  | { phase: "loading" }
  | { phase: "ready"; data: EcardCardsOverview }
  | { phase: "error"; message: string };

const SUB_TITLES: Record<Exclude<EcardView, "home">, string> = {
  balance: "账户详情",
  bill: "流水账单",
  stats: "收支统计",
  recharge: "一卡通充值",
  power: "宿舍电费",
};

/** 充值片区兜底值：实测该校 `frontConfig.recharge = 401`（配置拿不到时仍能进充值页）。 */
const FALLBACK_RECHARGE_FEEITEM = "401";
/** 401 快捷金额兜底：实测 `singleFeeitem.layout = "1,10,50,100"`。 */
const FALLBACK_RECHARGE_LAYOUT = ["1", "10", "50", "100"];

/**
 * 一卡通页（M4.5）：宫格首页 + 子页，融合原「钱包」与「电费」两个面板。
 *
 * - 余额 / 流水 / 统计 / 充值 走一卡通（`get_ecard_*` 系列），电费子页沿用原 `PowerPanel` 的
 *   完整能力（片区级联、自采趋势、缴费记录、充值）；
 * - 充值片区 id 由学校配置给出（`config.rechargeFeeitemId`），一卡通充值是 `/charge` 体系的
 *   **401 无级联片区**（建单不带 `third_party`）；
 * - 宫格入口按 `config` 门控（如 `showLost` 控制挂失入口，`enabledApps` 控制银行卡入口）——
 *   本批先只放已实现的入口，写操作子页随后接入。
 */
export function EcardsPanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const authed = status === "authed";

  const [overview, setOverview] = useState<OverviewState>({ phase: "loading" });
  const [tick, setTick] = useState(0);
  // 当前子页放 uiStore（今日页「查电费 / 卡片充值」要直达子页，见 shared/types.ts 的 EcardView）
  const view = useUiStore((s) => s.ecardView);
  const setView = useUiStore((s) => s.setEcardView);

  useEffect(() => {
    if (!authed) {
      setOverview({ phase: "loading" });
      return;
    }
    let alive = true;
    setOverview({ phase: "loading" });
    invokeCommand<EcardCardsOverview>("get_ecard_overview").then((r) => {
      if (!alive) return;
      if (r.success && r.data) setOverview({ phase: "ready", data: r.data });
      else
        setOverview({
          phase: "error",
          message: r.message ?? "一卡通信息获取失败",
        });
    });
    return () => {
      alive = false;
    };
  }, [authed, tick]);

  if (!authed) {
    return (
      <section className="mx-auto mt-8 max-w-5xl px-4">
        <PanelHeader
          title="一卡通"
          description="校园卡余额 · 账单 · 充值 · 宿舍电费"
          domain="wallet"
        />
        <Surface>
          <EmptyState
            icon={Wallet}
            domain="wallet"
            title="登录后查看一卡通"
            hint="登录后可查余额与流水账单，给校园卡和宿舍电费充值。"
            action={<Button onClick={openLoginDialog}>登录</Button>}
          />
        </Surface>
      </section>
    );
  }

  const cfg = overview.phase === "ready" ? overview.data.config : null;
  // 账单按卡筛选：该校账号只有一张卡（实测 `getCampusCards` 返回 1 张），
  // 且后端卡 DTO 只透出脱敏卡号、不提供可用于查询的原号 ⇒ 不传 account，查本人全量。
  const billAccounts: { account: string; label: string }[] = [];

  return (
    <section className="mx-auto mt-8 max-w-5xl px-4">
      {view === "home" ? (
        <PanelHeader
          title="一卡通"
          description="校园卡余额 · 账单 · 充值 · 宿舍电费"
          domain="wallet"
        />
      ) : (
        <header className="mt-8 mb-5 flex flex-wrap items-center gap-x-3 gap-y-2">
          <Button variant="ghost" size="sm" onClick={() => setView("home")}>
            <ChevronLeft className="size-4" aria-hidden />
            一卡通
          </Button>
          <h2 className="text-display-s font-semibold text-text">
            {SUB_TITLES[view]}
          </h2>
        </header>
      )}

      {view === "home" && (
        <EcardHome
          phase={overview.phase}
          data={overview.phase === "ready" ? overview.data : null}
          error={overview.phase === "error" ? overview.message : ""}
          onRetry={() => setTick((t) => t + 1)}
          onOpen={setView}
        />
      )}

      {view === "balance" && (
        <EcardBalanceView
          phase={overview.phase}
          data={overview.phase === "ready" ? overview.data : null}
          error={overview.phase === "error" ? overview.message : ""}
          onRetry={() => setTick((t) => t + 1)}
        />
      )}

      {view === "bill" && <EcardBillView accounts={billAccounts} />}
      {view === "stats" && <EcardStatsView />}
      {view === "recharge" && (
        <EcardRechargeView
          feeitemId={cfg?.rechargeFeeitemId || FALLBACK_RECHARGE_FEEITEM}
          layout={FALLBACK_RECHARGE_LAYOUT}
        />
      )}
      {view === "power" && (
        <EcardPowerView
          onNavigate={(v) => setView(v === "ecard-recharge" ? "recharge" : "home")}
        />
      )}
    </section>
  );
}
