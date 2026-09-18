import { Receipt, Wallet } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";

export function WalletPanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const authed = status === "authed";

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
          {/* 余额大数字卡：M3 接慧新E校实时数据 */}
          <Surface accent="wallet" className="px-5 py-5">
            <p className="text-caption text-text-2">一卡通余额</p>
            <p className="tabular-num mt-2 text-display font-semibold text-text">¥ —</p>
            <p className="mt-3 text-caption text-text-2">校园卡账户 · 支持卡片充值</p>
          </Surface>

          <div className="mt-3 grid grid-cols-2 gap-3">
            <Surface className="px-4 py-4">
              <p className="text-caption text-text-2">今日消费</p>
              <p className="tabular-num mt-2 text-title font-semibold text-text">¥ —</p>
            </Surface>
            <Surface className="px-4 py-4">
              <p className="text-caption text-text-2">本月消费</p>
              <p className="tabular-num mt-2 text-title font-semibold text-text">¥ —</p>
            </Surface>
          </div>

          <Surface className="mt-3">
            <p className="px-4 pt-4 text-body font-medium text-text">交易记录</p>
            <EmptyState
              compact
              icon={Receipt}
              domain="wallet"
              title="数据接入中"
              hint="门户模块开发中 · 该页将在 M2 接入"
            />
          </Surface>
        </>
      )}
    </section>
  );
}
