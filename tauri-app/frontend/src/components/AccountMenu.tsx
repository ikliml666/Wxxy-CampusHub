import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { motion, useReducedMotion } from "framer-motion";
import {
  Check,
  ChevronDown,
  ChevronRight,
  ImagePlus,
  LogIn,
  LogOut,
  Moon,
  RefreshCw,
  Settings,
  Sun,
  UserRound,
  X,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/shared/cn";
import { Avatar } from "@/components/Avatar";
import { Button } from "@/components/ui/button";
import { useAuthStore, type SavedAccount } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";

/** 菜单行通用样式（内层 hover 底色，全菜单统一节奏） */
const rowCls =
  "flex w-full items-center gap-2.5 rounded-control px-2.5 py-2 text-left text-body text-text transition-colors duration-[var(--dur-fast)] ease-out-soft hover:bg-surface-2 disabled:pointer-events-none disabled:opacity-50";

/** 上次登录时间：epoch 毫秒字符串 → YYYY-MM-DD HH:mm；缺失/非数字不显示（不回退原文） */
function fmtLastLogin(raw?: string): string | null {
  if (!raw) return null;
  const ms = Number(raw);
  if (!Number.isFinite(ms)) return null;
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return null;
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** 图标 + 文案的动作行（busy 时行尾转圈） */
function MenuRow({
  icon: Icon,
  label,
  onClick,
  danger,
  disabled,
  busy,
  trailing,
  ariaExpanded,
}: {
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  danger?: boolean;
  disabled?: boolean;
  busy?: boolean;
  trailing?: ReactNode;
  ariaExpanded?: boolean;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      aria-expanded={ariaExpanded}
      disabled={disabled}
      onClick={onClick}
      className={rowCls}
    >
      <Icon
        className="size-4 shrink-0 text-text-2"
        style={danger ? { color: "var(--color-alert)" } : undefined}
        aria-hidden="true"
      />
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {busy && (
        <span
          aria-hidden="true"
          className="size-3.5 shrink-0 animate-spin rounded-full border-2 border-current border-t-transparent text-text-2"
        />
      )}
      {trailing}
    </button>
  );
}

/** 外观行：深色模式 Switch（两种登录态共用） */
function AppearanceRow({
  checked,
  onToggle,
}: {
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitemcheckbox"
      aria-checked={checked}
      onClick={onToggle}
      className={rowCls}
    >
      {checked ? (
        <Moon className="size-4 shrink-0 text-text-2" aria-hidden="true" />
      ) : (
        <Sun className="size-4 shrink-0 text-text-2" aria-hidden="true" />
      )}
      <span className="min-w-0 flex-1 truncate">深色模式</span>
      <span
        aria-hidden="true"
        className={cn(
          "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors duration-[var(--dur-fast)]",
          checked ? "bg-brand" : "bg-line",
        )}
      >
        <span
          className={cn(
            "block size-4 rounded-full bg-white shadow-sm transition-transform duration-[var(--dur-fast)] ease-out-soft",
            checked ? "translate-x-[18px]" : "translate-x-[2px]",
          )}
        />
      </span>
    </button>
  );
}

/** 已保存账号行：点行免密登录（pending 就地转圈），hover 出 X 删除 */
function SavedAccountRow({
  account,
  busy,
  disabled,
  onLogin,
  onRemove,
}: {
  account: SavedAccount;
  busy: boolean;
  disabled: boolean;
  onLogin: () => void;
  onRemove: () => void;
}) {
  const last = fmtLastLogin(account.lastLogin);
  return (
    <div className="group relative flex items-center">
      <button
        type="button"
        role="menuitem"
        aria-busy={busy}
        disabled={disabled || busy}
        onClick={onLogin}
        className={cn(rowCls, "pr-10")}
      >
        <UserRound className="size-4 shrink-0 text-text-2" aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate">
          {account.displayName ?? account.username}
        </span>
        {busy ? (
          <span
            aria-hidden="true"
            className="size-3.5 shrink-0 animate-spin rounded-full border-2 border-current border-t-transparent text-text-2"
          />
        ) : (
          last && (
            <span className="shrink-0 text-caption text-text-2 tabular-num">
              {last}
            </span>
          )
        )}
      </button>
      <button
        type="button"
        aria-label={`删除已保存账号 ${account.username}`}
        disabled={disabled}
        onClick={onRemove}
        className="absolute right-1.5 top-1/2 flex size-6 -translate-y-1/2 items-center justify-center rounded-full text-text-2 opacity-0 transition-[opacity,background-color] duration-[var(--dur-fast)] hover:bg-bg hover:text-alert focus-visible:opacity-100 group-hover:opacity-100 disabled:pointer-events-none disabled:opacity-40"
      >
        <X className="size-3.5" aria-hidden="true" />
      </button>
    </div>
  );
}

export default function AccountMenu() {
  const status = useAuthStore((s) => s.status);
  const username = useAuthStore((s) => s.username);
  const displayName = useAuthStore((s) => s.displayName);
  const avatarBase64 = useAuthStore((s) => s.avatarBase64);
  const avatarSource = useAuthStore((s) => s.avatarSource);
  const accounts = useAuthStore((s) => s.accounts);
  const loginSaved = useAuthStore((s) => s.loginSaved);
  const syncOfficialAvatar = useAuthStore((s) => s.syncOfficialAvatar);
  const removeAccount = useAuthStore((s) => s.removeAccount);
  const logout = useAuthStore((s) => s.logout);

  const theme = useUiStore((s) => s.theme);
  const toggleTheme = useUiStore((s) => s.toggleTheme);
  const setActivePanel = useUiStore((s) => s.setActivePanel);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const openAvatarDialog = useUiStore((s) => s.openAvatarDialog);

  const [open, setOpen] = useState(false);
  const [pendingLogin, setPendingLogin] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [syncMsg, setSyncMsg] = useState<string | null>(null);
  const [syncOk, setSyncOk] = useState(false);
  const [loginError, setLoginError] = useState<string | null>(null);
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const [loggingOut, setLoggingOut] = useState(false);

  const wrapRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const reduceMotion = useReducedMotion();

  const authed = status === "authed";
  const avatarSrc = avatarBase64
    ? `data:image/png;base64,${avatarBase64}`
    : null;
  const triggerName = displayName ?? username ?? "未登录";

  // 关闭并清掉一次性反馈（导航类动作与点外/Esc 关闭共用）
  const close = () => {
    setOpen(false);
    setPendingLogin(null);
    setSyncMsg(null);
    setSyncOk(false);
    setLoginError(null);
    setSwitcherOpen(false);
  };

  // 打开时聚焦菜单容器；点外部关闭
  useEffect(() => {
    if (!open) return;
    menuRef.current?.focus();
    const onDocMouseDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        close();
      }
    };
    document.addEventListener("mousedown", onDocMouseDown);
    return () => document.removeEventListener("mousedown", onDocMouseDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // 已保存账号免密登录：pending 就地显示，失败内联红字（菜单不关，便于重试或换号）
  const handleSavedLogin = async (saved: string) => {
    setLoginError(null);
    setPendingLogin(saved);
    const r = await loginSaved(saved);
    if (r.success) {
      close();
      return;
    }
    setPendingLogin(null);
    setLoginError(r.message ?? "登录失败，请稍后重试");
  };

  // 删除已保存账号：确认后执行；管理型动作，菜单保持打开以便连续操作
  const handleRemove = async (saved: string) => {
    if (
      !window.confirm(
        `确定删除已保存账号 ${saved} 吗？删除后需重新输入密码登录。`,
      )
    )
      return;
    setLoginError(null);
    setRemoving(saved);
    const r = await removeAccount(saved);
    setRemoving(null);
    if (!r.success) setLoginError(r.message ?? "删除失败，请稍后重试");
  };

  // 同步学校头像：pending + 内联结果（失败时 message 原文展示，不弹窗）
  const handleSync = async () => {
    setSyncMsg(null);
    setSyncing(true);
    const r = await syncOfficialAvatar();
    setSyncing(false);
    setSyncOk(r.success);
    setSyncMsg(
      r.success
        ? (r.message ?? "已同步学校头像")
        : (r.message ?? "同步失败，请稍后重试"),
    );
  };

  const handleLogout = async () => {
    setLoggingOut(true);
    try {
      await logout();
    } finally {
      setLoggingOut(false);
      close();
    }
  };

  return (
    <div className="relative shrink-0" ref={wrapRef}>
      {/* 触发器：36px 胶囊（头像 + 名字 + chevron） */}
      <button
        ref={triggerRef}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`账号菜单（${triggerName}）`}
        onClick={() => (open ? close() : setOpen(true))}
        className="flex h-9 items-center gap-2 rounded-full border border-line bg-surface pl-1 pr-2.5 transition-all duration-[var(--dur-fast)] ease-out-soft hover:border-line-strong hover:shadow-card"
      >
        <Avatar size="md" src={avatarSrc} name={displayName ?? username} />
        <span className="max-w-[9rem] truncate text-body text-text">
          {triggerName}
        </span>
        <ChevronDown
          aria-hidden="true"
          className={cn(
            "size-4 shrink-0 text-text-2 transition-transform duration-[var(--dur-fast)]",
            open && "rotate-180",
          )}
        />
      </button>

      {open && (
        <motion.div
          ref={menuRef}
          role="menu"
          aria-label="账号"
          tabIndex={-1}
          initial={
            reduceMotion
              ? { opacity: 0 }
              : { opacity: 0, y: 6, scale: 0.98 }
          }
          animate={{ opacity: 1, y: 0, scale: 1 }}
          transition={
            reduceMotion
              ? { duration: 0.12 }
              : { duration: 0.18, ease: [0.22, 1, 0.36, 1] }
          }
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              close();
              triggerRef.current?.focus();
            }
          }}
          className="absolute right-0 top-full z-50 mt-2 w-72 origin-top-right rounded-card border border-line bg-surface p-1.5 shadow-pop outline-none"
        >
          {authed ? (
            <>
              {/* 头部：姓名 + 学号（相同时只留一行）+ 头像来源标注 */}
              <div className="flex items-center gap-3 px-2.5 pb-2.5 pt-2">
                <Avatar size="lg" src={avatarSrc} name={displayName ?? username} />
                <div className="min-w-0">
                  <p className="truncate text-title font-semibold text-text">
                    {displayName ?? username}
                  </p>
                  {displayName && displayName !== username && (
                    <p className="truncate text-caption text-text-2">
                      {username}
                    </p>
                  )}
                  {avatarSource === "official" && (
                    <p className="text-caption text-text-2">学校头像</p>
                  )}
                </div>
              </div>
              <div className="mx-1.5 my-1 h-px bg-line" />

              <MenuRow
                icon={ImagePlus}
                label="上传头像"
                onClick={() => {
                  close();
                  openAvatarDialog();
                }}
              />
              <MenuRow
                icon={RefreshCw}
                label="同步学校头像"
                busy={syncing}
                disabled={syncing}
                onClick={() => void handleSync()}
              />
              {syncMsg && (
                <p
                  role={syncOk ? "status" : "alert"}
                  aria-live="polite"
                  className={cn(
                    "px-2.5 pb-1 pt-0.5 text-caption",
                    syncOk ? "text-wallet" : "text-alert",
                  )}
                >
                  {syncMsg}
                </p>
              )}

              {/* 切换账号（折叠区）：当前账号 ✓ 禁用，其余一键免密切换 */}
              <MenuRow
                icon={UserRound}
                label="切换账号"
                ariaExpanded={switcherOpen}
                trailing={
                  <ChevronRight
                    aria-hidden="true"
                    className={cn(
                      "size-4 shrink-0 text-text-2 transition-transform duration-[var(--dur-fast)]",
                      switcherOpen && "rotate-90",
                    )}
                  />
                }
                onClick={() => setSwitcherOpen((v) => !v)}
              />
              {switcherOpen && (
                <div className="mx-0.5 mt-1 space-y-0.5 rounded-inner bg-surface-2 p-1">
                  {accounts.map((acc) => {
                    const current = acc.username === username;
                    return (
                      <button
                        key={acc.username}
                        type="button"
                        role="menuitem"
                        aria-busy={pendingLogin === acc.username}
                        disabled={current || pendingLogin !== null}
                        onClick={() => void handleSavedLogin(acc.username)}
                        className={cn(rowCls, "py-1.5 hover:bg-bg")}
                      >
                        {current ? (
                          <Check
                            className="size-3.5 shrink-0 text-brand"
                            aria-hidden="true"
                          />
                        ) : pendingLogin === acc.username ? (
                          <span
                            aria-hidden="true"
                            className="size-3.5 shrink-0 animate-spin rounded-full border-2 border-current border-t-transparent text-text-2"
                          />
                        ) : (
                          <span aria-hidden="true" className="size-3.5 shrink-0" />
                        )}
                        <span className="min-w-0 flex-1 truncate">
                          {acc.displayName ?? acc.username}
                        </span>
                        {current && (
                          <span className="shrink-0 text-caption text-text-2">
                            当前
                          </span>
                        )}
                      </button>
                    );
                  })}
                  <button
                    type="button"
                    role="menuitem"
                    disabled={pendingLogin !== null}
                    onClick={() => {
                      close();
                      openLoginDialog();
                    }}
                    className={cn(rowCls, "py-1.5 hover:bg-bg")}
                  >
                    <LogIn
                      className="size-3.5 shrink-0 text-text-2"
                      aria-hidden="true"
                    />
                    <span className="min-w-0 flex-1 truncate text-text-2">
                      使用其他账号登录
                    </span>
                  </button>
                </div>
              )}
              {loginError && (
                <p
                  role="alert"
                  aria-live="polite"
                  className="px-2.5 pb-1 pt-1 text-caption text-alert"
                >
                  {loginError}
                </p>
              )}

              {/* 外观 / 设置 / 退出登录 */}
              <div className="mx-1.5 my-1 h-px bg-line" />
              <AppearanceRow checked={theme === "dark"} onToggle={toggleTheme} />
              <MenuRow
                icon={Settings}
                label="设置"
                onClick={() => {
                  close();
                  setActivePanel("settings");
                }}
              />
              <MenuRow
                icon={LogOut}
                label="退出登录"
                danger
                busy={loggingOut}
                disabled={loggingOut}
                onClick={() => void handleLogout()}
              />
            </>
          ) : (
            <>
              {/* 头部：占位头像 + 登录引导 */}
              <div className="flex items-center gap-3 px-2.5 pb-2.5 pt-2">
                <Avatar size="lg" name="锡" />
                <div className="min-w-0">
                  <p className="text-title font-semibold text-text">未登录</p>
                  <p className="text-caption text-text-2">
                    登录后查看课表、余额与待办
                  </p>
                </div>
              </div>
              {/* 登录主按钮 */}
              <div className="px-1 pb-1.5">
                <Button
                  className="w-full"
                  onClick={() => {
                    close();
                    openLoginDialog();
                  }}
                >
                  <LogIn className="size-4" aria-hidden="true" />
                  登录
                </Button>
              </div>

              {/* 已保存账号快捷登录 */}
              {accounts.length > 0 && (
                <>
                  <div className="mx-1.5 my-1 h-px bg-line" />
                  <p className="px-2.5 pb-1 pt-0.5 text-caption font-medium text-text-2">
                    已保存账号
                  </p>
                  {accounts.map((acc) => (
                    <SavedAccountRow
                      key={acc.username}
                      account={acc}
                      busy={pendingLogin === acc.username}
                      disabled={pendingLogin !== null || removing !== null}
                      onLogin={() => void handleSavedLogin(acc.username)}
                      onRemove={() => void handleRemove(acc.username)}
                    />
                  ))}
                </>
              )}
              {loginError && (
                <p
                  role="alert"
                  aria-live="polite"
                  className="px-2.5 pb-1.5 pt-1 text-caption text-alert"
                >
                  {loginError}
                </p>
              )}

              {/* 外观 / 设置 */}
              <div className="mx-1.5 my-1 h-px bg-line" />
              <AppearanceRow
                checked={theme === "dark"}
                onToggle={toggleTheme}
              />
              <MenuRow
                icon={Settings}
                label="设置"
                onClick={() => {
                  close();
                  setActivePanel("settings");
                }}
              />
            </>
          )}
        </motion.div>
      )}
    </div>
  );
}
