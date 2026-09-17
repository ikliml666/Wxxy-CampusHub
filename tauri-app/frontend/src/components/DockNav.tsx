import { useCallback, useEffect, useRef } from "react";
import {
  CalendarDays,
  LayoutGrid,
  ListChecks,
  Newspaper,
  Settings,
  SunMedium,
  Wallet,
  Zap,
  type LucideIcon,
} from "lucide-react";
import { motion } from "framer-motion";
import { gsap } from "gsap";
import { cn } from "@/shared/cn";
import type { PanelId } from "@/shared/types";
import { useUiStore } from "@/stores/uiStore";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";

export const DOCK_ITEMS: {
  id: PanelId;
  label: string;
  icon: LucideIcon;
  color: string;
}[] = [
  { id: "today",    label: "今日", icon: SunMedium,    color: "var(--color-brand)" },
  { id: "info",     label: "资讯", icon: Newspaper,    color: "var(--color-info)" },
  { id: "todo",     label: "待办", icon: ListChecks,   color: "var(--color-todo)" },
  { id: "schedule", label: "日程", icon: CalendarDays, color: "var(--color-sched)" },
  { id: "apps",     label: "应用", icon: LayoutGrid,   color: "var(--color-sched)" },
  { id: "wallet",   label: "钱包", icon: Wallet,       color: "var(--color-wallet)" },
  { id: "power",    label: "电费", icon: Zap,          color: "var(--color-wallet)" },
  { id: "settings", label: "设置", icon: Settings,     color: "var(--color-text-2)" },
];

const MAGNETIC_RANGE = 80;
const MAX_SCALE = 1.35;
const MAX_LIFT = -14;

interface MagnetTween {
  scale: gsap.QuickToFunc;
  y: gsap.QuickToFunc;
  center: number;
}

export default function DockNav() {
  const activePanel = useUiStore((s) => s.activePanel);
  const setActivePanel = useUiStore((s) => s.setActivePanel);

  const btnRefs = useRef(new Map<PanelId, HTMLButtonElement>());
  const tweenRefs = useRef(new Map<PanelId, MagnetTween>());
  const magnetEnabled = useRef(false);
  const rafRef = useRef(0);

  // gsap 磁吸注册：prefers-reduced-motion 命中则不注册（动画降级档位禁用磁吸）
  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    magnetEnabled.current = true;

    for (const item of DOCK_ITEMS) {
      const btn = btnRefs.current.get(item.id);
      if (!btn) continue;
      tweenRefs.current.set(item.id, {
        scale: gsap.quickTo(btn, "scale", {
          duration: 0.35,
          ease: "expo.out",
          force3D: true,
        }),
        y: gsap.quickTo(btn, "y", {
          duration: 0.35,
          ease: "expo.out",
          force3D: true,
        }),
        center: 0,
      });
    }

    const updateCenters = () => {
      for (const [id, tween] of tweenRefs.current) {
        const btn = btnRefs.current.get(id);
        if (btn) {
          const rect = btn.getBoundingClientRect();
          tween.center = rect.left + rect.width / 2;
        }
      }
    };
    updateCenters();
    window.addEventListener("resize", updateCenters);

    return () => {
      window.removeEventListener("resize", updateCenters);
      for (const [id] of tweenRefs.current) {
        const btn = btnRefs.current.get(id);
        if (btn) gsap.killTweensOf(btn);
      }
      tweenRefs.current.clear();
      magnetEnabled.current = false;
    };
  }, []);

  // 容器 onMouseMove：RAF 节流，按每项中心距离插值 scale/y
  const handleMouseMove = useCallback((e: React.MouseEvent) => {
    if (!magnetEnabled.current) return;
    const x = e.clientX;
    cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      for (const tween of tweenRefs.current.values()) {
        const distance = Math.abs(x - tween.center);
        if (distance < MAGNETIC_RANGE) {
          const progress = 1 - distance / MAGNETIC_RANGE;
          tween.scale(1 + (MAX_SCALE - 1) * progress);
          tween.y(MAX_LIFT * progress);
        } else {
          tween.scale(1);
          tween.y(0);
        }
      }
    });
  }, []);

  const handleMouseLeave = useCallback(() => {
    cancelAnimationFrame(rafRef.current);
    for (const tween of tweenRefs.current.values()) {
      tween.scale(1);
      tween.y(0);
    }
  }, []);

  return (
    <TooltipProvider delayDuration={250}>
      <nav
        aria-label="主导航"
        onMouseMove={handleMouseMove}
        onMouseLeave={handleMouseLeave}
        className="fixed bottom-5 left-1/2 z-30 flex -translate-x-1/2 items-center gap-1 rounded-[18px] border border-white/60 bg-white/80 px-3 py-2 shadow-[0_8px_30px_rgb(0_0_0/0.12),0_1px_0_rgb(255_255_255/0.6)_inset] dark:border-white/10 dark:bg-[#201d28]/85"
      >
        {DOCK_ITEMS.map((item) => {
          const active = activePanel === item.id;
          const Icon = item.icon;
          return (
            <Tooltip key={item.id}>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  aria-label={item.label}
                  aria-current={active ? "page" : undefined}
                  ref={(el) => {
                    if (el) btnRefs.current.set(item.id, el);
                    else btnRefs.current.delete(item.id);
                  }}
                  onClick={() => setActivePanel(item.id)}
                  className="relative flex size-10 items-center justify-center rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  {active && (
                    <motion.span
                      layoutId="dock-pill"
                      className="absolute inset-0 rounded-full"
                      style={{
                        backgroundColor: `color-mix(in srgb, ${item.color} 14%, transparent)`,
                      }}
                      transition={{ type: "spring", stiffness: 500, damping: 34 }}
                    />
                  )}
                  <Icon
                    size={18}
                    aria-hidden="true"
                    className={cn(
                      "relative transition-colors hover:text-text",
                      !active && "text-text-2",
                    )}
                    style={active ? { color: item.color } : undefined}
                  />
                  {active && (
                    <span className="pointer-events-none absolute inset-x-0 bottom-0 flex justify-center">
                      <motion.span
                        layoutId="dock-dot"
                        className="h-[3px] w-4 rounded-full"
                        style={{ backgroundColor: item.color }}
                        transition={{ type: "spring", stiffness: 500, damping: 34 }}
                      />
                    </span>
                  )}
                </button>
              </TooltipTrigger>
              <TooltipContent side="top">{item.label}</TooltipContent>
            </Tooltip>
          );
        })}
      </nav>
    </TooltipProvider>
  );
}
