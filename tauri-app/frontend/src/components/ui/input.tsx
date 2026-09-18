import * as React from "react";
import { cn } from "@/shared/cn";

function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      type={type}
      data-slot="input"
      className={cn(
        "h-9 w-full min-w-0 rounded-md border border-input bg-transparent px-3 py-1 text-base shadow-xs transition-[color,box-shadow,border-color] duration-[var(--dur-fast)] ease-out-soft selection:bg-primary selection:text-primary-foreground file:inline-flex file:h-7 file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-foreground placeholder:text-muted-foreground hover:border-line-strong disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 md:text-sm dark:bg-input/30",
        "focus-visible:shadow-[inset_0_1px_2px_rgb(31_27_41/0.08)] dark:focus-visible:shadow-[inset_0_1px_2px_rgb(0_0_0/0.30)]",
        "aria-invalid:border-alert",
        className,
      )}
      {...props}
    />
  );
}

export { Input };
