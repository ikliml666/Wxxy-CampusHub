import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { CircleAlert, X } from "lucide-react";
import { useBrowserStore } from "@/stores/browserStore";
import { Button } from "@/components/ui/button";

/**
 * 浏览器事件桥（挂 AppShell，常驻不随 overlay 开关卸载）+ 弹层外失败兜底。
 *
 * C1：Rust 侧直建副 webview 的入口（电费 open_recharge_in_browser →
 * open_url_inapp）不经前端 store——browser://opened 的 listen 必须常驻：若挂
 * BrowserOverlay 随 open 开关卸载，overlay 渲染前事件就丢了，仍是「无 chrome
 * 死路」。收到后调 storeSyncOpen 同步 store，BrowserOverlay 随即正常渲染。
 *
 * I1：errorMsg 在弹层未开（open=false）时 BrowserStatusView 不存在、无渲染口
 * ——这里补一条底部固定提示条；弹层开着时交给 BrowserStatusView（互斥不重复）。
 *
 * dispose 纪律同 BrowserOverlay：cleanup 后 resolve 的 listen promise 也要补
 * unlisten，防 StrictMode 双挂载泄漏。
 */
export default function BrowserEventsBridge() {
  const open = useBrowserStore((s) => s.open);
  const errorMsg = useBrowserStore((s) => s.errorMsg);
  const storeSyncOpen = useBrowserStore((s) => s.storeSyncOpen);
  const clearError = useBrowserStore((s) => s.clearError);

  // browser://opened { url } → storeSyncOpen（设 open/loading/pushHistory）
  useEffect(() => {
    const pending = listen<{ url?: string }>("browser://opened", (e) => {
      if (e.payload.url) storeSyncOpen(e.payload.url);
    });
    return () => {
      void pending.then((un) => un());
    };
  }, [storeSyncOpen]);

  // I1：弹层外的失败反馈口（z-40 与 DockNav 同层；弹层开着时让位 z-50 overlay）
  if (open || !errorMsg) return null;
  return (
    <div
      role="status"
      aria-live="polite"
      className="fixed inset-x-0 bottom-0 z-40 flex h-10 items-center gap-2 border-t border-line bg-surface-2 px-3"
    >
      <CircleAlert aria-hidden className="size-4 shrink-0 text-alert" />
      <p className="min-w-0 flex-1 truncate text-caption text-text-2">{errorMsg}</p>
      <Button variant="ghost" size="icon-xs" aria-label="关闭提示" onClick={clearError}>
        <X className="size-3.5" />
      </Button>
    </div>
  );
}
