import { useEffect, useState } from "react";
import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { CircleAlert, Info, ShieldAlert, Timer, X } from "lucide-react";
import { cn } from "@/shared/cn";
import { browserHostOf } from "@/shared/constants";
import { appBrowserReload, openExternalBrowser } from "@/shared/tauriApi";
import { useBrowserStore } from "@/stores/browserStore";
import { Button } from "@/components/ui/button";

/**
 * 浏览器状态区：errorMsg / 拦截 / 加载慢 / CAS 登录 四类提示，按优先级互斥显示一条。
 *
 * ⚠️ 布局事实：React 视口被 Rust relayout 裁剪为 BROWSER_TOOLBAR_H 高的条内，
 * brief 里「工具栏下方的提示条 / EmptyState compact」会落在裁剪区不可见——
 * 故这里以 absolute 覆盖工具栏整条（可关闭）的横条形态呈现，语义等价：
 * 拦截态 = EmptyState 的 alert 色语义（ShieldAlert + 目标 host + 外部打开 action），
 * CAS/slow = sched 域色的网络环境提示条。关闭后工具栏即时恢复可点。
 */

/** loading 连续超此时长（ms）出现「加载较慢」提示（brief：10s）。 */
const SLOW_LOAD_MS = 10_000;

/** 单条提示的统一横条骨架（图标 + 文案 + 可选动作 + 关闭）。 */
function StatusBar({
  icon: Icon,
  iconClass,
  msg,
  onClose,
  children,
}: {
  icon: LucideIcon;
  iconClass: string;
  msg: string;
  onClose: () => void;
  children?: ReactNode;
}) {
  return (
    <div
      role="status"
      aria-live="polite"
      className="absolute inset-x-0 top-0 z-10 flex h-12 items-center gap-2 border-b border-line bg-surface-2 px-3"
    >
      <Icon aria-hidden className={cn("size-4 shrink-0", iconClass)} />
      <p className="min-w-0 flex-1 truncate text-caption text-text-2">{msg}</p>
      {children}
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label="关闭提示"
        onClick={onClose}
      >
        <X className="size-3.5" />
      </Button>
    </div>
  );
}

export default function BrowserStatusView() {
  const errorMsg = useBrowserStore((s) => s.errorMsg);
  const clearError = useBrowserStore((s) => s.clearError);
  const blockedUrl = useBrowserStore((s) => s.blockedUrl);
  const setBlocked = useBrowserStore((s) => s.setBlocked);
  const nav = useBrowserStore((s) => s.nav);
  const loading = useBrowserStore((s) => s.loading);

  // slow 提示：loading 连续超 10s 才出现；loading 或页面切换即重置计时
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    if (!loading) {
      setSlow(false);
      return;
    }
    const t = setTimeout(() => setSlow(true), SLOW_LOAD_MS);
    return () => clearTimeout(t);
  }, [loading, nav.url]);

  // CAS 登录提示的关闭记忆：同一页面关过就不再弹，换页（nav.url 变化）重新提示
  const [casDismissed, setCasDismissed] = useState(false);
  useEffect(() => {
    setCasDismissed(false);
  }, [nav.url]);

  // CAS 登录页判定：host 含 cas.cwxu.edu.cn（实测 CAS 域 wxcas.cwxu.edu.cn 亦命中）
  // 或路径含 /login（WebVPN 登录页）
  const isCasPage =
    nav.url !== "" &&
    (browserHostOf(nav.url).includes("cas.cwxu.edu.cn") ||
      nav.url.includes("/login"));

  // 互斥优先级：失败 > 拦截 > 加载慢 > 需登录；全无 → 不渲染
  if (errorMsg) {
    return (
      <StatusBar
        icon={CircleAlert}
        iconClass="text-alert"
        msg={errorMsg}
        onClose={clearError}
      />
    );
  }
  if (blockedUrl != null) {
    const host = browserHostOf(blockedUrl) || blockedUrl;
    return (
      <StatusBar
        icon={ShieldAlert}
        iconClass="text-alert"
        msg={`已拦截非校园网链接：${host}`}
        onClose={() => setBlocked(null)}
      >
        <Button
          variant="ghost"
          size="xs"
          onClick={async () => {
            const r = await openExternalBrowser(blockedUrl);
            if (r.success) setBlocked(null); // 降级链路成功后才关闭提示，失败可重试
          }}
        >
          仍在外部浏览器打开
        </Button>
      </StatusBar>
    );
  }
  if (slow) {
    return (
      <StatusBar
        icon={Timer}
        iconClass="text-sched"
        msg="加载较慢，可刷新或在外部浏览器打开"
        onClose={() => setSlow(false)}
      >
        <Button variant="ghost" size="xs" onClick={() => void appBrowserReload()}>
          刷新
        </Button>
        <Button
          variant="ghost"
          size="xs"
          onClick={() => void openExternalBrowser(nav.url)}
        >
          在外部打开
        </Button>
      </StatusBar>
    );
  }
  if (isCasPage && !casDismissed) {
    return (
      <StatusBar
        icon={Info}
        iconClass="text-text-2"
        msg="该系统需登录，可在页面内直接登录；登录一次后本应用将记住状态"
        onClose={() => setCasDismissed(true)}
      />
    );
  }
  return null;
}
