import { useState } from "react";
import { ListChecks } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import { cn } from "@/shared/cn";

// 三分区与融合门户事务中心 tab 一致（queryTabItems / querySimpleFlowItems）
const TODO_TABS = ["我的待办", "我的已办", "我的申请"] as const;
type TodoTab = (typeof TODO_TABS)[number];

export function TodoPanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const [tab, setTab] = useState<TodoTab>("我的待办");
  const authed = status === "authed";

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      <PanelHeader title="待办" description="待办、已办与我的申请" domain="todo" />

      {/* 三分区切换：本地 state，M2 接线后按 tabId 拉取 */}
      <div role="tablist" aria-label="待办分区" className="flex gap-2">
        {TODO_TABS.map((t) => (
          <button
            key={t}
            type="button"
            role="tab"
            aria-selected={t === tab}
            onClick={() => setTab(t)}
            className={cn(
              "rounded-control border px-3 py-1.5 text-body transition-colors duration-[var(--dur-fast)] ease-out-soft",
              t === tab
                ? "border-todo/30 bg-todo/10 font-medium text-todo"
                : "border-line bg-surface text-text-2 hover:border-line-strong hover:text-text",
            )}
          >
            {t}
          </button>
        ))}
      </div>

      <Surface className="mt-4">
        <EmptyState
          icon={ListChecks}
          domain="todo"
          compact={authed}
          title={authed ? "数据接入中" : `登录后查看${tab.slice(2)}`}
          hint={
            authed
              ? "门户模块开发中 · 该页将在 M2 接入"
              : "登录后同步融合门户的待办、已办与申请记录。"
          }
          action={authed ? undefined : <Button onClick={openLoginDialog}>登录</Button>}
        />
      </Surface>
    </section>
  );
}
