import { LayoutGrid, Puzzle } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";

// 分组对齐融合门户应用中心（queryMyStore / selectAppByCardId），M2 接线
const APP_GROUPS = ["服务", "系统"] as const;

export function AppsPanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const authed = status === "authed";

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      <PanelHeader title="应用" description="校内服务与常用系统入口" domain="sched" />

      {!authed ? (
        <Surface>
          <EmptyState
            icon={LayoutGrid}
            domain="sched"
            title="登录后查看校园应用"
            hint="登录后展示融合门户应用中心的校内服务与常用系统。"
            action={<Button onClick={openLoginDialog}>登录</Button>}
          />
        </Surface>
      ) : (
        <>
          <Surface>
            <EmptyState
              compact
              icon={Puzzle}
              domain="sched"
              title="数据接入中"
              hint="门户模块开发中 · 该页将在 M2 接入"
            />
          </Surface>

          {APP_GROUPS.map((group) => (
            <div key={group} className="mt-5">
              <h3 className="text-body font-medium text-text">{group}</h3>
              {/* 应用卡片骨架：名称与部门以 `—` 占位 */}
              <div className="mt-2 grid grid-cols-3 gap-3">
                {[1, 2, 3].map((i) => (
                  <Surface key={i} hover className="flex items-center gap-3 px-3.5 py-3">
                    <span
                      aria-hidden
                      className="flex size-9 shrink-0 items-center justify-center rounded-inner bg-sched/10 text-sched"
                    >
                      <Puzzle className="size-4" />
                    </span>
                    <div className="min-w-0">
                      <p className="truncate text-body font-medium text-text">—</p>
                      <p className="truncate text-caption text-text-2">—</p>
                    </div>
                  </Surface>
                ))}
              </div>
            </div>
          ))}
        </>
      )}
    </section>
  );
}
