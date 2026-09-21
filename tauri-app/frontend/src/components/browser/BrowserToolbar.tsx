import { useEffect, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  ExternalLink,
  RotateCw,
  ShieldAlert,
  ShieldCheck,
  X,
} from "lucide-react";
import { cn } from "@/shared/cn";
import { BROWSER_TOOLBAR_H, browserHostOf } from "@/shared/constants";
import {
  appBrowserBack,
  appBrowserForward,
  appBrowserReload,
  openExternalBrowser,
} from "@/shared/tauriApi";
import { useBrowserStore } from "@/stores/browserStore";
import { Button } from "@/components/ui/button";

/**
 * 内置浏览器工具栏（高 = BROWSER_TOOLBAR_H，与 Rust 侧 TOPBAR_LOGICAL 互指）。
 *
 * 布局事实：React 视口被 Rust relayout 裁剪为该高度的条内（spike 六问 1），
 * 故进度条放工具栏底边（absolute bottom-0，视觉等价设计稿的「工具栏下方 2px」）；
 * 拦截态地址 chip 图标变 alert 红，host 显示被拦截目标。
 */
export default function BrowserToolbar() {
  const nav = useBrowserStore((s) => s.nav);
  const loading = useBrowserStore((s) => s.loading);
  const blockedUrl = useBrowserStore((s) => s.blockedUrl);
  const browserClose = useBrowserStore((s) => s.browserClose);

  // 进度条：loading 结束后保底 300ms 淡出再卸载（brief：完成后 300ms 淡出）
  const [barMounted, setBarMounted] = useState(loading);
  useEffect(() => {
    if (loading) {
      setBarMounted(true);
      return;
    }
    const t = setTimeout(() => setBarMounted(false), 300);
    return () => clearTimeout(t);
  }, [loading]);

  // 地址 chip：被拦截时显示拦截目标（alert 红），否则当前页 host（品牌紫）
  const blocked = blockedUrl != null;
  const chipUrl = blockedUrl ?? nav.url;
  const chipHost = browserHostOf(chipUrl) || "—";

  return (
    // 高度消费 BROWSER_TOOLBAR_H 单点常量（h-12 与常量是同一个值的两种写法，取常量）
    <div
      className="relative flex shrink-0 items-center gap-2 border-b border-line bg-surface px-3"
      style={{ height: BROWSER_TOOLBAR_H }}
    >
      {/* 左：导航三钮（canBack/canForward 首批恒 false，禁用态渲染；下批接导航栈后启用） */}
      <div className="flex shrink-0 items-center gap-1">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="后退"
          disabled={!nav.canBack}
          onClick={() => void appBrowserBack()}
        >
          <ChevronLeft className="size-4" />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="前进"
          disabled={!nav.canForward}
          onClick={() => void appBrowserForward()}
        >
          <ChevronRight className="size-4" />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="刷新"
          onClick={() => void appBrowserReload()}
        >
          <RotateCw className="size-4" />
        </Button>
      </div>

      {/* 中：只读地址标识（锁形图标 = 白名单状态色，host 只显域名防长 URL 误导） */}
      <div className="flex h-8 min-w-0 flex-1 items-center gap-1.5 rounded-full border border-line bg-bg px-3 text-caption text-text-2">
        {blocked ? (
          <ShieldAlert aria-hidden className="size-3.5 shrink-0 text-alert" />
        ) : (
          <ShieldCheck aria-hidden className="size-3.5 shrink-0 text-brand" />
        )}
        <span className="truncate">{chipHost}</span>
      </div>

      {/* 右：逃生口（外部浏览器）+ 关闭 */}
      <div className="flex shrink-0 items-center gap-1">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="在外部浏览器打开"
          disabled={!chipUrl}
          onClick={() => void openExternalBrowser(chipUrl)}
        >
          <ExternalLink className="size-4" />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="关闭内置浏览器"
          onClick={() => void browserClose()}
        >
          <X className="size-4" />
        </Button>
      </div>

      {/* 进度条（indeterminate 横向位移 keyframes 见 index.css；reduced-motion 下静态显示） */}
      {barMounted && (
        <div
          aria-hidden
          className={cn(
            "absolute bottom-0 left-0 h-0.5 w-1/6 rounded-full bg-brand",
            loading
              ? "animate-[browser-progress_1.2s_ease-in-out_infinite] motion-reduce:animate-none"
              : "opacity-0 transition-opacity duration-300",
          )}
        />
      )}
    </div>
  );
}
