import { History, Zap } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";

export function PowerPanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const authed = status === "authed";

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      <PanelHeader title="电费" description="宿舍电费查询与充值" domain="wallet" />

      {!authed ? (
        <Surface>
          <EmptyState
            icon={Zap}
            domain="wallet"
            title="登录后查看电费"
            hint="登录后绑定宿舍房间，查询电费余额与缴费历史。"
            action={<Button onClick={openLoginDialog}>登录</Button>}
          />
        </Surface>
      ) : (
        <>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            {/* 房间绑定卡：绑定流程 M2 接门户后开放 */}
            <Surface accent="wallet" className="flex items-center justify-between gap-3 px-4 py-4">
              <div className="min-w-0">
                <p className="text-caption text-text-2">宿舍房间</p>
                <p className="tabular-num mt-1 text-display-s font-semibold text-text">—</p>
              </div>
              <TooltipProvider delayDuration={150}>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span className="inline-flex shrink-0">
                      <Button variant="outline" size="sm" disabled>
                        绑定房间
                      </Button>
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>门户接入后开放</TooltipContent>
                </Tooltip>
              </TooltipProvider>
            </Surface>

            <Surface accent="wallet" className="px-4 py-4">
              <p className="text-caption text-text-2">电费余额</p>
              <p className="tabular-num mt-1 text-display-s font-semibold text-text">¥ —</p>
            </Surface>
          </div>

          <Surface className="mt-3 flex items-center justify-between px-4 py-3">
            <span className="text-body text-text-2">电价单价</span>
            <span className="tabular-num text-body font-medium text-text">—</span>
          </Surface>

          <Surface className="mt-3">
            <p className="px-4 pt-4 text-body font-medium text-text">缴费历史</p>
            <EmptyState
              compact
              icon={History}
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
