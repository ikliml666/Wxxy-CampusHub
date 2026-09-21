import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { LogOut, RefreshCw, Upload, type LucideIcon } from "lucide-react";
import { Avatar } from "@/components/Avatar";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import { cn } from "@/shared/cn";
import type { InfoColumn, NotificationSettings } from "@/shared/types";
import {
  getNotificationSettings,
  saveNotificationSettings,
  invokeCommand,
} from "@/shared/tauriApi";

/** 小号开关：轨道纯色瞬时切换，滑块只做 transform 过渡 */
function ToggleSwitch({
  checked,
  onToggle,
  ariaLabel,
}: {
  checked: boolean;
  onToggle: () => void;
  ariaLabel: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
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

/** 设置分区里的一行（左文案 + 右控件）。 */
function SetRow({
  title,
  desc,
  children,
}: {
  title: string;
  desc: string;
  children: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-inner border border-line bg-surface-2 px-3 py-2.5">
      <div className="min-w-0">
        <p className="text-body text-text">{title}</p>
        <p className="text-caption text-text-2">{desc}</p>
      </div>
      {children}
    </div>
  );
}

/**
 * 通知设置卡片（M5 批 3）：公告/待办开关 + 轮询间隔只读展示。
 * 电费开关与阈值已迁至一卡通 · 电费子页（EcardPowerNotifCard），本卡不再承载。
 *
 * 公告开关的落盘语义 = `infoColumns` 空/非空（后端契约）；打开且列表为空时
 * 拉 `get_info_columns` 全量回填（前端不硬编码栏目 id）。轮询间隔只读——
 * 后端校验范围 5..=720 已在，编辑能力留给后续批次（实现成本低者优先）。
 */
function NotifSettingsCard() {
  const [phase, setPhase] = useState<"loading" | "ready" | "error">("loading");
  const [settings, setSettings] = useState<NotificationSettings | null>(null);
  const [infoOn, setInfoOn] = useState(true);
  const [todoOn, setTodoOn] = useState(true);
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<{ kind: "ok" | "err"; text: string } | null>(null);
  const [reloadTick, setReloadTick] = useState(0);

  useEffect(() => {
    let alive = true;
    setPhase("loading");
    setMsg(null);
    getNotificationSettings().then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        setSettings(r.data);
        setInfoOn(r.data.infoColumns.length > 0);
        setTodoOn(r.data.todoEnabled);
        setPhase("ready");
      } else {
        setPhase("error");
      }
    });
    return () => {
      alive = false;
    };
  }, [reloadTick]);

  if (phase === "loading") {
    return (
      <Surface className="px-4 py-4">
        <p className="text-body font-medium text-text">通知</p>
        <div aria-hidden className="mt-3 space-y-2">
          {[0, 1, 2].map((i) => (
            <span key={i} className="block h-9 animate-pulse rounded bg-line" />
          ))}
        </div>
      </Surface>
    );
  }

  if (phase === "error") {
    return (
      <Surface className="px-4 py-4">
        <p className="text-body font-medium text-text">通知</p>
        <div className="mt-3 flex items-center justify-between gap-3 rounded-inner border border-line bg-surface-2 px-3 py-2.5">
          <p className="text-caption text-text-2">通知设置获取失败</p>
          <Button variant="outline" size="sm" onClick={() => setReloadTick((t) => t + 1)}>
            重试
          </Button>
        </div>
      </Surface>
    );
  }

  const toggleInfo = () => {
    if (infoOn) {
      setInfoOn(false);
      setMsg(null);
      return;
    }
    // 开启公告通知需要至少一个订阅栏目：列表为空时拉全量栏目回填
    const existing = settings?.infoColumns ?? [];
    if (existing.length > 0) {
      setInfoOn(true);
      setMsg(null);
      return;
    }
    invokeCommand<InfoColumn[]>("get_info_columns").then((r) => {
      if (r.success && r.data && r.data.length > 0 && settings) {
        setSettings({ ...settings, infoColumns: r.data.map((c) => c.id) });
        setInfoOn(true);
        setMsg(null);
      } else {
        setMsg({ kind: "err", text: r.message ?? "获取资讯栏目失败，无法开启公告通知" });
      }
    });
  };

  const toggleTodo = () => {
    setTodoOn((v) => !v);
    setMsg(null);
  };

  // 电费字段（electricityEnabled / electricityThresholdYuan）原样回传不改，
  // 编辑入口在一卡通 · 电费子页的「电费通知」卡。
  const save = () => {
    if (!settings) return;
    const next: NotificationSettings = {
      ...settings,
      infoColumns: infoOn ? settings.infoColumns : [],
      todoEnabled: todoOn,
    };
    setSaving(true);
    setMsg(null);
    saveNotificationSettings(next).then((r) => {
      if (r.success) {
        setMsg({ kind: "ok", text: "通知设置已保存" });
      } else {
        setMsg({ kind: "err", text: r.message ?? "保存通知设置失败" });
      }
    }).finally(() => setSaving(false));
  };

  return (
    <Surface className="px-4 py-4">
      <p className="text-body font-medium text-text">通知</p>
      <div className="mt-3 space-y-2">
        <SetRow title="公告通知" desc="订阅栏目有新公告时提醒">
          <ToggleSwitch ariaLabel="公告通知" checked={infoOn} onToggle={toggleInfo} />
        </SetRow>
        <SetRow title="待办通知" desc="办事大厅出现新待办时提醒">
          <ToggleSwitch ariaLabel="待办通知" checked={todoOn} onToggle={toggleTodo} />
        </SetRow>
        {settings && (
          <p className="tabular-num px-1 text-caption text-text-2">
            检查频率：资讯每 {settings.infoIntervalMin} 分钟 · 待办每 {settings.todoIntervalMin} 分钟
          </p>
        )}
        {msg && (
          <p role={msg.kind === "err" ? "alert" : "status"} className={cn("px-1 text-caption", msg.kind === "ok" ? "text-wallet" : "text-alert")}>
            {msg.text}
          </p>
        )}
        <div className="flex justify-end pt-1">
          <Button size="sm" disabled={saving} onClick={save}>
            {saving ? "保存中…" : "保存通知设置"}
          </Button>
        </div>
      </div>
    </Surface>
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
            <ToggleSwitch ariaLabel="深色模式" checked={theme === "dark"} onToggle={toggleTheme} />
          </div>
        </Surface>

        {/* 通知 */}
        <NotifSettingsCard />

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
