import { useEffect, useState } from "react";
import { Bell, ListChecks, Newspaper, Zap } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { useUiStore } from "@/stores/uiStore";
import type { NotificationItem, NotificationStateView, PanelId } from "@/shared/types";
import { getNotifications, markNotificationsRead } from "@/shared/tauriApi";
import { cn } from "@/shared/cn";

/** 列表四态（参照 TodoPanel 先例）。 */
type ListState =
  | { phase: "loading" }
  | { phase: "ready"; data: NotificationStateView }
  | { phase: "empty" }
  | { phase: "error"; message: string };

/** kind → 徽标文案 / 域色 / 跳转面板。电费低余额是告警性质，用 alert 红；
 *  未知 kind 不跳转（后端未来加源时前端未跟进的兜底）。 */
const KIND_META: Record<string, { label: string; color: string; panel: PanelId | null }> = {
  info: { label: "公告", color: "var(--color-info)", panel: "info" },
  todo: { label: "待办", color: "var(--color-todo)", panel: "todo" },
  electricity: { label: "电费", color: "var(--color-alert)", panel: "ecard" },
};

function kindIcon(kind: string): LucideIcon {
  switch (kind) {
    case "info":
      return Newspaper;
    case "todo":
      return ListChecks;
    default:
      return Zap;
  }
}

/** createdAt（ISO8601 带偏移，后端秒精度固定格式）→ "YYYY-MM-DD HH:MM"；异常原样展示。 */
function fmtTime(iso: string): string {
  return iso.length >= 16 ? `${iso.slice(0, 10)} ${iso.slice(11, 16)}` : iso;
}

function NotificationCard({ item, onJump }: { item: NotificationItem; onJump: (kind: string) => void }) {
  const meta = KIND_META[item.kind] ?? { label: "通知", color: "var(--color-text-2)", panel: null };
  const Icon = kindIcon(item.kind);
  const jumpable = meta.panel != null;
  return (
    <button
      type="button"
      onClick={() => onJump(item.kind)}
      aria-label={jumpable ? `查看${meta.label}：${item.title}` : item.title}
      className={cn(
        "group flex w-full items-start gap-3 rounded-inner border border-line bg-surface px-4 py-3 text-left transition-all duration-[var(--dur-fast)] ease-out-soft",
        jumpable ? "hover:-translate-y-px hover:border-line-strong hover:shadow-card" : "cursor-default",
      )}
    >
      {/* 未读色标（后端只回未读列表，列表内全部视为未读） */}
      <span
        aria-hidden
        className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-full"
        style={{ backgroundColor: `color-mix(in srgb, ${meta.color} 12%, transparent)` }}
      >
        <Icon className="size-4" style={{ color: meta.color }} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-2">
          <span
            className="shrink-0 rounded-full px-1.5 py-0.5 text-[10px] leading-none font-medium"
            style={{
              color: meta.color,
              backgroundColor: `color-mix(in srgb, ${meta.color} 12%, transparent)`,
            }}
          >
            {meta.label}
          </span>
          <span className="truncate text-body font-medium text-text">{item.title}</span>
        </span>
        <span className="mt-1 flex items-center gap-2">
          <span className="tabular-num truncate text-caption text-text-2">{item.body}</span>
        </span>
        <span className="tabular-num mt-0.5 block text-caption text-text-2">{fmtTime(item.createdAt)}</span>
      </span>
    </button>
  );
}

export function NotificationsPanel() {
  const setActivePanel = useUiStore((s) => s.setActivePanel);
  const [list, setList] = useState<ListState>({ phase: "loading" });
  const [reloadTick, setReloadTick] = useState(0);
  const [marking, setMarking] = useState(false);
  const [markErr, setMarkErr] = useState<string | null>(null);

  // 取未读列表（后端只落未读；升序存放，展示倒序）
  useEffect(() => {
    let alive = true;
    setList({ phase: "loading" });
    getNotifications().then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        setList(r.data.items.length > 0 ? { phase: "ready", data: r.data } : { phase: "empty" });
      } else {
        setList({ phase: "error", message: r.message ?? "通知列表获取失败" });
      }
    });
    return () => {
      alive = false;
    };
  }, [reloadTick]);

  const retry = () => setReloadTick((t) => t + 1);

  // 全部标为已读：mark 返回剩余未读（契约免二次拉取），直接回写本地态
  const markAllRead = () => {
    setMarking(true);
    setMarkErr(null);
    markNotificationsRead()
      .then((r) => {
        if (r.success && r.data) {
          setList(r.data.items.length > 0 ? { phase: "ready", data: r.data } : { phase: "empty" });
        } else {
          setMarkErr(r.message ?? "标记已读失败");
        }
      })
      .finally(() => setMarking(false));
  };

  // 通知项跳转：公告→资讯、待办→待办、电费→一卡通（只切面板，不直达子页）
  const jump = (kind: string) => {
    const panel = (KIND_META[kind] ?? KIND_META.info).panel;
    if (panel) setActivePanel(panel);
  };

  const counts = list.phase === "ready" ? list.data.counts : null;

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      <PanelHeader
        title="通知"
        description="公告、待办与电费提醒 · 后台自动检查"
        domain="info"
        actions={
          counts != null && counts.total > 0 ? (
            <Button variant="outline" size="sm" disabled={marking} onClick={markAllRead}>
              全部标为已读{counts.total > 0 && <span className="tabular-num">（{counts.total}）</span>}
            </Button>
          ) : undefined
        }
      />

      {list.phase === "loading" && (
        <div aria-hidden className="space-y-2">
          {[0, 1, 2].map((i) => (
            <Surface key={i} className="px-4 py-3">
              <span className="block h-4 w-2/3 animate-pulse rounded bg-line" />
              <span className="mt-2 block h-3 w-1/3 animate-pulse rounded bg-line" />
            </Surface>
          ))}
        </div>
      )}

      {list.phase === "error" && (
        <Surface>
          <EmptyState
            icon={Bell}
            domain="info"
            title="通知列表获取失败"
            hint={list.message}
            action={
              <Button variant="outline" onClick={retry}>
                重试
              </Button>
            }
          />
        </Surface>
      )}

      {list.phase === "empty" && (
        <Surface>
          <EmptyState
            icon={Bell}
            domain="info"
            title="暂无通知"
            hint="后台会自动检查公告、待办与电费余额，有新消息会出现在这里。"
          />
        </Surface>
      )}

      {list.phase === "ready" && (
        <>
          {markErr && (
            <p role="alert" className="text-caption text-alert">
              {markErr}
            </p>
          )}
          <div className="space-y-2">
            {[...list.data.items].reverse().map((item) => (
              <NotificationCard key={item.id} item={item} onJump={jump} />
            ))}
          </div>
        </>
      )}
    </section>
  );
}
