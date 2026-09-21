import { useEffect } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { listen } from "@tauri-apps/api/event";
import { appBrowserBack, appBrowserForward } from "@/shared/tauriApi";
import { useBrowserStore } from "@/stores/browserStore";
import BrowserToolbar from "./BrowserToolbar";
import BrowserStatusView from "./BrowserStatusView";

/**
 * 应用内浏览器浮层（挂 AppShell，与 CommandPalette 并列，自联 browserStore）。
 *
 * 布局事实：Rust `open_url_inapp` 把主 webview（React 视口）缩顶为
 * BROWSER_TOOLBAR_H 高的条内（browser.rs relayout / spike 六问 1），副 webview
 * 占其余区域——故本浮层 `fixed inset-0` 实际就是这条高度：Toolbar 占满，
 * StatusView absolute 覆盖，下方 flex-1 空占位是给「未来若放开裁剪」的
 * webview 区预留，本批被视口裁剪属预期（browser.rs:184 注释）。
 *
 * 事件回写（Rust emit，payload camelCase JSON，见 browser.rs 模块头）：
 * - browser://load    { phase: "started"|"finished" } → setLoading
 * - browser://nav     { url }                          → setNav + 清拦截提示
 * - browser://blocked { url }                          → setBlocked
 */
export default function BrowserOverlay() {
  const open = useBrowserStore((s) => s.open);
  const browserClose = useBrowserStore((s) => s.browserClose);
  const setNav = useBrowserStore((s) => s.setNav);
  const setLoading = useBrowserStore((s) => s.setLoading);
  const setBlocked = useBrowserStore((s) => s.setBlocked);

  // 三个 Rust 事件：open 时挂、open=false 时 unlisten（dispose 后 resolve 的
  // listen promise 也要补 unlisten，防 StrictMode 双挂载泄漏）
  useEffect(() => {
    if (!open) return;
    let disposed = false;
    const unlistens: (() => void)[] = [];
    const pending = [
      listen<{ phase?: string }>("browser://load", (e) => {
        setLoading(e.payload.phase === "started");
      }),
      // 导航放行 = 拦截解除（用户刷新/后退重进同理），顺带清掉拦截提示
      listen<{ url?: string }>("browser://nav", (e) => {
        if (e.payload.url) setNav({ url: e.payload.url });
        setBlocked(null);
      }),
      listen<{ url?: string }>("browser://blocked", (e) => {
        setBlocked(e.payload.url ?? null);
      }),
    ];
    for (const p of pending) {
      p.then((un) => {
        if (disposed) un();
        else unlistens.push(un);
      });
    }
    return () => {
      disposed = true;
      for (const un of unlistens) un();
    };
  }, [open, setNav, setLoading, setBlocked]);

  // 快捷键：Esc 关闭、Alt+←/→ 后退/前进（open-gated keydown，惯例同 CommandPalette）
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        void browserClose();
        return;
      }
      if (e.altKey && e.key === "ArrowLeft") {
        e.preventDefault();
        void appBrowserBack();
        return;
      }
      if (e.altKey && e.key === "ArrowRight") {
        e.preventDefault();
        void appBrowserForward();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, browserClose]);

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          key="browser-overlay"
          // 进出场沿用 AppShell 面板切换的 spring（opacity + y）
          initial={{ opacity: 0, y: 8 }}
          animate={{
            opacity: 1,
            y: 0,
            transition: { type: "spring", stiffness: 400, damping: 40 },
          }}
          exit={{ opacity: 0, transition: { duration: 0.04 } }}
          className="fixed inset-0 z-50 flex flex-col bg-bg/95"
        >
          <BrowserToolbar />
          <BrowserStatusView />
          {/* webview 填充区：原生副 webview 由 Rust 占位，本批 React 侧为空 div */}
          <div aria-hidden className="min-h-0 flex-1" />
        </motion.div>
      )}
    </AnimatePresence>
  );
}
