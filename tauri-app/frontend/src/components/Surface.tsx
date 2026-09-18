import type { ReactNode } from "react";
import { cn } from "@/shared/cn";
import { domainVar, type PanelDomain } from "./PanelHeader";

export function Surface({
  accent,
  hover = false,
  className,
  children,
}: {
  /** 域色 CSS 变量名（如 "wallet"），渲染左上角 8px 域色角块 */
  accent?: PanelDomain;
  hover?: boolean;
  className?: string;
  children: ReactNode;
}) {
  return (
    <div
      className={cn(
        "relative rounded-card border border-line bg-surface shadow-card",
        hover &&
          "transition-[transform,box-shadow] duration-[var(--dur-base)] ease-out-soft hover:-translate-y-[1px] hover:shadow-lift",
        className,
      )}
    >
      {accent && (
        <span
          aria-hidden
          className="absolute top-0 left-0 size-2 rounded-tl-card rounded-br-inner"
          style={{ backgroundColor: domainVar[accent] }}
        />
      )}
      {children}
    </div>
  );
}
