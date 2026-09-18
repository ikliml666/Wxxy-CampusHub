import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/shared/cn";
import { domainVar, type PanelDomain } from "./PanelHeader";

export function EmptyState({
  icon: Icon,
  domain = "neutral",
  title,
  hint,
  action,
  compact = false,
}: {
  icon?: LucideIcon;
  domain?: PanelDomain;
  title: string;
  hint?: string;
  action?: ReactNode;
  compact?: boolean;
}) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center text-center",
        compact ? "gap-2 px-4 py-8" : "gap-3 px-6 py-14",
      )}
    >
      {Icon && (
        <span
          aria-hidden
          className={cn(
            "flex shrink-0 items-center justify-center rounded-full",
            compact ? "size-10" : "size-14",
          )}
          style={{
            backgroundColor: `color-mix(in srgb, ${domainVar[domain]} 12%, transparent)`,
          }}
        >
          <Icon
            className={compact ? "size-5" : "size-6"}
            style={{ color: domainVar[domain] }}
          />
        </span>
      )}
      <p className="text-title font-semibold text-text">{title}</p>
      {hint !== undefined && (
        <p className="text-caption max-w-[36ch] text-text-2">{hint}</p>
      )}
      {action !== undefined && (
        <div className="mt-2 flex items-center gap-2">{action}</div>
      )}
    </div>
  );
}
