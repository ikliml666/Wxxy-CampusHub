import type { ReactNode } from "react";

/** 域色语义档：与 index.css 的 --color-<domain> token 一一对应 */
export type PanelDomain =
  | "brand"
  | "info"
  | "todo"
  | "sched"
  | "wallet"
  | "neutral";

/** 域色 → CSS 变量值（供 style 内联取色，深浅主题自动跟随） */
export const domainVar: Record<PanelDomain, string> = {
  brand: "var(--color-brand)",
  info: "var(--color-info)",
  todo: "var(--color-todo)",
  sched: "var(--color-sched)",
  wallet: "var(--color-wallet)",
  neutral: "var(--color-text-2)",
};

export function PanelHeader({
  title,
  description,
  domain = "neutral",
  actions,
}: {
  title: string;
  description?: string;
  domain?: PanelDomain;
  actions?: ReactNode;
}) {
  return (
    // flex-wrap：actions 过多时换行而不是把 min-w-0 标题区挤到 1 字宽（竖排根因修复）
    <header className="mt-8 mb-5 flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
      <div className="flex shrink-0 items-center gap-3">
        <span
          aria-hidden
          className="size-2 shrink-0 rounded-full"
          style={{ backgroundColor: domainVar[domain] }}
        />
        <div className="min-w-0">
          <h2 className="text-display-s font-semibold text-text">{title}</h2>
          {description !== undefined && (
            <p className="text-caption text-text-2">{description}</p>
          )}
        </div>
      </div>
      {actions !== undefined && (
        <div className="flex shrink-0 items-center gap-2">{actions}</div>
      )}
    </header>
  );
}
