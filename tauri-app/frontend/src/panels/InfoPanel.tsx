import { useState } from "react";
import { Newspaper, Rss } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import { cn } from "@/shared/cn";

// 融合门户资讯栏目（栏目 id 已实机探明，M2 接 querySimpleInfoCenter）
const INFO_CATEGORIES = [
  "通知公告",
  "校园要闻",
  "校园快讯",
  "教务处",
  "学工处",
  "团委",
  "规章制度",
] as const;

export function InfoPanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const [category, setCategory] = useState<string>(INFO_CATEGORIES[0]);
  const authed = status === "authed";

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      <PanelHeader
        title="资讯"
        description="校园通知、要闻与快讯 · 来自融合门户"
        domain="info"
      />

      {!authed ? (
        <Surface>
          <EmptyState
            icon={Newspaper}
            domain="info"
            title="登录后查看校园资讯"
            hint="登录后展示融合门户的通知公告、校园要闻与校园快讯。"
            action={<Button onClick={openLoginDialog}>登录</Button>}
          />
        </Surface>
      ) : (
        <>
          {/* 分类 chips：纯本地切换，M2 接线后过滤列表 */}
          <div role="tablist" aria-label="资讯分类" className="flex flex-wrap gap-2">
            {INFO_CATEGORIES.map((c) => (
              <button
                key={c}
                type="button"
                role="tab"
                aria-selected={c === category}
                onClick={() => setCategory(c)}
                className={cn(
                  "rounded-control border px-3 py-1.5 text-caption transition-colors duration-[var(--dur-fast)] ease-out-soft",
                  c === category
                    ? "border-info/30 bg-info/10 font-medium text-info"
                    : "border-line bg-surface text-text-2 hover:border-line-strong hover:text-text",
                )}
              >
                {c}
              </button>
            ))}
          </div>

          <Surface className="mt-4">
            <EmptyState
              compact
              icon={Rss}
              domain="info"
              title="数据接入中"
              hint="门户模块开发中 · 该页将在 M2 接入"
            />
          </Surface>

          {/* 列表骨架：标题与日期以 `—` 占位 */}
          <div className="mt-3 space-y-2">
            {[0, 1, 2].map((i) => (
              <Surface key={i} className="flex items-center gap-4 px-4 py-3">
                <span className="shrink-0 text-caption text-info">{category}</span>
                <span className="min-w-0 truncate text-body text-text">—</span>
                <span className="tabular-num ml-auto shrink-0 text-caption text-text-2">—</span>
              </Surface>
            ))}
          </div>
        </>
      )}
    </section>
  );
}
