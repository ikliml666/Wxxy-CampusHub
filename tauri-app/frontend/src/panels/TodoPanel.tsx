import { useEffect, useState } from "react";
import { ChevronLeft, ChevronRight, ListChecks } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import type { TodoPage, TodoTab } from "@/shared/types";
import { invokeCommand } from "@/shared/tauriApi";
import { cn } from "@/shared/cn";

const PAGE_SIZE = 10;

// 契约三栏：接口实际返回 6 个 tab（unread/read/focus 不在契约内），
// rail 顺序按 TODO_TAB_IDS；名称优先接口返回，失败回落此处兜底
const TODO_TAB_IDS = ["todo", "done", "apply"] as const;
const TODO_TAB_FALLBACK: Record<(typeof TODO_TAB_IDS)[number], string> = {
  todo: "我的待办",
  done: "我的已办",
  apply: "我的申请",
};

/** 列表四态。 */
type ListState =
  | { phase: "loading" }
  | { phase: "ready"; data: TodoPage }
  | { phase: "empty" }
  | { phase: "error"; message: string };

/** 副行元信息：空段省略，段间 " · "。 */
function metaLine(parts: (string | null | undefined)[]): string {
  return parts.filter(Boolean).join(" · ");
}

export function TodoPanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const authed = status === "authed";

  const [tabNames, setTabNames] = useState<TodoTab[] | null>(null);
  const [activeTab, setActiveTab] = useState<(typeof TODO_TAB_IDS)[number]>("todo");
  const [page, setPage] = useState(1);
  const [list, setList] = useState<ListState>({ phase: "loading" });
  const [reloadTick, setReloadTick] = useState(0);

  // 分栏名称与待办数（增强信息；失败时 rail 回落兜底名，不阻塞列表）
  useEffect(() => {
    if (!authed) return;
    let alive = true;
    invokeCommand<TodoTab[]>("get_todo_tabs").then((r) => {
      if (!alive) return;
      setTabNames(r.success && r.data ? r.data : null);
    });
    return () => {
      alive = false;
    };
  }, [authed, reloadTick]);

  // 当前分栏列表：切栏回第 1 页；「下一页」按满页判断（total/pageCount 不可靠）
  useEffect(() => {
    if (!authed) return;
    let alive = true;
    setList({ phase: "loading" });
    invokeCommand<TodoPage>("get_todo_list", {
      tabId: activeTab,
      page,
      pageSize: PAGE_SIZE,
    }).then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        setList(r.data.items.length > 0 ? { phase: "ready", data: r.data } : { phase: "empty" });
      } else {
        setList({ phase: "error", message: r.message ?? "待办列表获取失败" });
      }
    });
    return () => {
      alive = false;
    };
  }, [authed, activeTab, page, reloadTick]);

  const retry = () => setReloadTick((t) => t + 1);
  const switchTab = (id: (typeof TODO_TAB_IDS)[number]) => {
    if (id === activeTab) return;
    setPage(1);
    setActiveTab(id);
  };
  const nameOf = (id: (typeof TODO_TAB_IDS)[number]): string =>
    tabNames?.find((t) => t.id === id)?.name ?? TODO_TAB_FALLBACK[id];
  const countOf = (id: (typeof TODO_TAB_IDS)[number]): number =>
    tabNames?.find((t) => t.id === id)?.count ?? 0;

  const listData = list.phase === "ready" ? list.data : null;
  const hasNext = listData != null && listData.items.length === PAGE_SIZE;

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      <PanelHeader title="待办" description="待办、已办与我的申请 · 来自办事大厅" domain="todo" />

      {!authed ? (
        <Surface>
          <EmptyState
            icon={ListChecks}
            domain="todo"
            title="登录后查看我的待办"
            hint="登录后同步融合门户的待办、已办与申请记录。"
            action={<Button onClick={openLoginDialog}>登录</Button>}
          />
        </Surface>
      ) : (
        <>
          {/* 三栏 rail：接口名称优先；count>0 显示待办数徽标 */}
          <div role="tablist" aria-label="待办分区" className="flex gap-2">
            {TODO_TAB_IDS.map((id) => (
              <button
                key={id}
                type="button"
                role="tab"
                aria-selected={id === activeTab}
                onClick={() => switchTab(id)}
                className={cn(
                  "rounded-control border px-3 py-1.5 text-body transition-colors duration-[var(--dur-fast)] ease-out-soft",
                  id === activeTab
                    ? "border-todo/30 bg-todo/10 font-medium text-todo"
                    : "border-line bg-surface text-text-2 hover:border-line-strong hover:text-text",
                )}
              >
                {nameOf(id)}
                {countOf(id) > 0 && (
                  <span className="tabular-num ml-1.5 text-caption">{countOf(id)}</span>
                )}
              </button>
            ))}
          </div>

          {/* 列表：加载骨架 */}
          {list.phase === "loading" && (
            <div aria-hidden className="mt-4 space-y-2">
              {[0, 1, 2].map((i) => (
                <Surface key={i} className="px-4 py-3">
                  <span className="block h-4 w-2/3 animate-pulse rounded bg-line" />
                  <span className="mt-2 block h-3 w-1/3 animate-pulse rounded bg-line" />
                </Surface>
              ))}
            </div>
          )}
          {/* 列表：出错可重试 */}
          {list.phase === "error" && (
            <Surface className="mt-4">
              <EmptyState
                icon={ListChecks}
                domain="todo"
                title="待办列表获取失败"
                hint={list.message}
                action={
                  <Button variant="outline" onClick={retry}>
                    重试
                  </Button>
                }
              />
            </Surface>
          )}
          {/* 列表：空态（当前账号无待办数据时的常态） */}
          {list.phase === "empty" && (
            <Surface className="mt-4">
              <EmptyState
                icon={ListChecks}
                domain="todo"
                title={`${nameOf(activeTab)}暂无事项`}
                hint="当前分区下没有可展示的记录。"
              />
            </Surface>
          )}
          {/* 列表：有数据 */}
          {list.phase === "ready" && (
            <>
              <div className="mt-4 space-y-2">
                {listData?.items.map((item) => {
                  const meta = metaLine([item.applicant, item.applyTime, item.node, item.urgency]);
                  return (
                    <Surface key={item.id} className="px-4 py-3">
                      <p className="min-w-0 truncate text-body text-text">{item.title}</p>
                      <p className="tabular-num mt-1 truncate text-caption text-text-2">
                        {meta || item.source || "—"}
                      </p>
                    </Surface>
                  );
                })}
              </div>
              <div className="mt-4 flex items-center justify-center gap-3">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={page <= 1}
                  onClick={() => setPage((p) => Math.max(1, p - 1))}
                >
                  <ChevronLeft aria-hidden="true" />
                  上一页
                </Button>
                <span className="tabular-num text-caption text-text-2">第 {page} 页</span>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!hasNext}
                  onClick={() => setPage((p) => p + 1)}
                >
                  下一页
                  <ChevronRight aria-hidden="true" />
                </Button>
              </div>
            </>
          )}
        </>
      )}
    </section>
  );
}
