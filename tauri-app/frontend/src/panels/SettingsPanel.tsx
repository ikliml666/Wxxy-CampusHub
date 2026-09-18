import { LogOut, RefreshCw, Upload, type LucideIcon } from "lucide-react";
import { Avatar } from "@/components/Avatar";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import { cn } from "@/shared/cn";

/** 小号开关：轨道纯色瞬时切换，滑块只做 transform 过渡 */
function ToggleSwitch({ checked, onToggle }: { checked: boolean; onToggle: () => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label="深色模式"
      onClick={onToggle}
      className={cn(
        "relative h-6 w-10 shrink-0 rounded-full border transition-colors duration-[var(--dur-fast)] ease-out-soft",
        checked ? "border-brand bg-brand" : "border-line bg-surface-2 hover:border-line-strong",
      )}
    >
      <span
        aria-hidden
        className={cn(
          "absolute top-1/2 left-[3px] size-4 -translate-y-1/2 rounded-full bg-surface shadow-card transition-transform duration-[var(--dur-base)] ease-out-soft",
          checked && "translate-x-[18px]",
        )}
      />
    </button>
  );
}

function RowItem({
  icon: Icon,
  label,
  onClick,
  danger = false,
}: {
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex w-full items-center gap-2 rounded-control px-3 py-2 text-body transition-colors duration-[var(--dur-fast)] ease-out-soft",
        danger ? "text-alert hover:bg-alert/10" : "text-text hover:bg-surface-2",
      )}
    >
      <Icon className="size-4 shrink-0" aria-hidden="true" />
      {label}
    </button>
  );
}

export function SettingsPanel() {
  const status = useAuthStore((s) => s.status);
  const displayName = useAuthStore((s) => s.displayName);
  const username = useAuthStore((s) => s.username);
  const avatarBase64 = useAuthStore((s) => s.avatarBase64);
  const syncOfficialAvatar = useAuthStore((s) => s.syncOfficialAvatar);
  const logout = useAuthStore((s) => s.logout);
  // theme / toggleTheme 由 B1 并行写入 uiStore；字段暂缺时 tsc 报错属预期
  const theme = useUiStore((s) => s.theme);
  const toggleTheme = useUiStore((s) => s.toggleTheme);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const openAvatarDialog = useUiStore((s) => s.openAvatarDialog);
  const authed = status === "authed";

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      <PanelHeader title="设置" description="外观、账号与应用信息" domain="neutral" />

      <div className="space-y-3">
        {/* 外观 */}
        <Surface className="px-4 py-4">
          <p className="text-body font-medium text-text">外观</p>
          <div className="mt-3 flex items-center justify-between gap-3 rounded-inner border border-line bg-surface-2 px-3 py-2.5">
            <div>
              <p className="text-body text-text">深色模式</p>
              <p className="text-caption text-text-2">切换应用的深浅主题</p>
            </div>
            <ToggleSwitch checked={theme === "dark"} onToggle={toggleTheme} />
          </div>
        </Surface>

        {/* 账号 */}
        <Surface className="px-4 py-4">
          <p className="text-body font-medium text-text">账号</p>
          {authed ? (
            <>
              <div className="mt-3 flex items-center gap-3">
                <Avatar
                  size="lg"
                  name={displayName ?? username}
                  src={avatarBase64 ? `data:image/png;base64,${avatarBase64}` : null}
                />
                <div className="min-w-0">
                  <p className="truncate text-body font-medium text-text">
                    {displayName ?? username ?? "已登录"}
                  </p>
                  <p className="tabular-num truncate text-caption text-text-2">
                    {username ?? "—"}
                  </p>
                </div>
              </div>
              <div className="mt-3 border-t border-line pt-2">
                <RowItem icon={Upload} label="上传头像" onClick={openAvatarDialog} />
                <RowItem
                  icon={RefreshCw}
                  label="同步学校头像"
                  onClick={() => void syncOfficialAvatar()}
                />
                <RowItem icon={LogOut} label="退出登录" danger onClick={() => void logout()} />
              </div>
            </>
          ) : (
            <div className="mt-3 flex items-center justify-between gap-3 rounded-inner border border-line bg-surface-2 px-3 py-2.5">
              <div className="flex min-w-0 items-center gap-3">
                <Avatar size="lg" />
                <div className="min-w-0">
                  <p className="text-body font-medium text-text">未登录</p>
                  <p className="text-caption text-text-2">登录后可使用校园账号相关功能</p>
                </div>
              </div>
              <Button size="sm" onClick={openLoginDialog}>
                登录
              </Button>
            </div>
          )}
        </Surface>

        {/* 关于 */}
        <Surface className="px-4 py-4">
          <p className="text-body font-medium text-text">关于</p>
          <div className="mt-3 flex items-center gap-3">
            <span
              aria-hidden
              className="flex size-9 shrink-0 items-center justify-center rounded-inner bg-brand text-sm font-semibold text-white"
            >
              锡
            </span>
            <div>
              <p className="text-title font-semibold text-text">锡院助手</p>
              <p className="text-caption text-text-2">无锡学院校园助手客户端</p>
            </div>
          </div>
          <p className="mt-3 text-caption text-text-2">
            版本 0.1.0 · 数据存于本机 %APPDATA%/campushub
          </p>
          <p className="mt-1 text-caption text-text-2">门户数据来自无锡学院融合门户</p>
        </Surface>
      </div>
    </section>
  );
}
