import { cn } from "@/shared/cn";

const sizeClasses = {
  sm: "size-6 text-[11px]",
  md: "size-8 text-[13px]",
  lg: "size-10 text-base",
  xl: "size-16 text-2xl",
} as const;

export function Avatar({
  src,
  name,
  size = "md",
  className,
}: {
  src?: string | null;
  name?: string | null;
  size?: "sm" | "md" | "lg" | "xl";
  className?: string;
}) {
  if (src) {
    return (
      <img
        src={src}
        alt={name || "账号头像"}
        className={cn(
          "rounded-full object-cover ring-1 ring-inset ring-black/5",
          sizeClasses[size],
          className,
        )}
      />
    );
  }
  return (
    <span
      role="img"
      aria-label={name || "账号头像"}
      className={cn(
        "flex items-center justify-center rounded-full font-semibold text-white ring-1 ring-inset ring-black/5 select-none",
        sizeClasses[size],
        className,
      )}
      style={{
        backgroundImage:
          "linear-gradient(140deg, var(--color-brand), var(--color-info))",
      }}
    >
      {name?.trim()?.[0] ?? "锡"}
    </span>
  );
}
