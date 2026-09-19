import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { motion, useReducedMotion } from "framer-motion";
import {
  ImagePlus,
  LogIn,
  LogOut,
  Moon,
  RefreshCw,
  Search,
  SearchX,
  Settings,
  Sun,
  UserRound,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/shared/cn";
import { EmptyState } from "@/components/EmptyState";
import { DOCK_ITEMS } from "@/components/DockNav";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";

/** 平台修饰键提示（macOS 显示 ⌘K，其余显示 Ctrl K）——顶栏触发条复用同一常量。 */
export const MOD_KEY_LABEL = /Mac/i.test(navigator.platform || navigator.userAgent)
  ? "⌘K"
  : "Ctrl K";

type Group = "页面" | "动作" | "设置";

/** 分组渲染顺序（固定，不随匹配得分变化）。 */
const GROUPS: Group[] = ["页面", "动作", "设置"];

interface Command {
  id: string;
  group: Group;
  label: string;
  /** 附加搜索词（面板 id、别名等，与 label 同口径匹配） */
  keywords?: string;
  icon: LucideIcon;
  /** 危险动作行用告警色（退出登录） */
  danger?: boolean;
  run: () => void;
}

/** 模糊匹配得分：0 = 前缀命中 < 1 = 子串命中 < 2 = 顺序子序列命中；不命中返回 null。
 *  统一小写比较，中文天然按字符包含（"课" 命中「课表」）。 */
function matchScore(text: string, q: string): number | null {
  const t = text.toLowerCase();
  if (!q) return 2;
  const at = t.indexOf(q);
  if (at === 0) return 0;
  if (at > 0) return 1;
  let k = 0;
  for (const ch of t) if (k < q.length && ch === q[k]) k++;
  return k === q.length ? 2 : null;
}

export default function CommandPalette() {
  const open = useUiStore((s) => s.commandPaletteOpen);
  const openPalette = useUiStore((s) => s.openCommandPalette);
  const close = useUiStore((s) => s.closeCommandPalette);
  const setActivePanel = useUiStore((s) => s.setActivePanel);
  const toggleTheme = useUiStore((s) => s.toggleTheme);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const openAvatarDialog = useUiStore((s) => s.openAvatarDialog);
  const theme = useUiStore((s) => s.theme);

  const status = useAuthStore((s) => s.status);
  const accounts = useAuthStore((s) => s.accounts);
  const username = useAuthStore((s) => s.username);
  const loginSaved = useAuthStore((s) => s.loginSaved);
  const logout = useAuthStore((s) => s.logout);
  const syncOfficialAvatar = useAuthStore((s) => s.syncOfficialAvatar);

  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const reduceMotion = useReducedMotion();

  const authed = status === "authed";

  // 命令表：三类数据源全部来自现有 store（无新增后端命令、无硬编码接口）
  const commands = useMemo<Command[]>(() => {
    const pages: Command[] = DOCK_ITEMS.map((item) => ({
      id: `panel-${item.id}`, group: "页面", label: item.label,
      keywords: `${item.id} 页面 跳转`, icon: item.icon,
      run: () => setActivePanel(item.id),
    }));

    const actions: Command[] = [
      { id: "theme-toggle", group: "动作", icon: theme === "dark" ? Sun : Moon, run: toggleTheme,
        label: theme === "dark" ? "切换到浅色模式" : "切换到深色模式", keywords: "主题 外观 深色 浅色 dark light" },
      { id: "login-dialog", group: "动作", icon: LogIn, run: openLoginDialog,
        label: authed ? "使用其他账号登录" : "登录", keywords: "重新登录 账号密码 login" },
    ];

    if (authed) {
      actions.push(
        { id: "avatar-dialog", group: "动作", label: "上传头像", icon: ImagePlus, run: openAvatarDialog,
          keywords: "头像 本地图片 avatar" },
        { id: "sync-avatar", group: "动作", label: "同步学校头像", icon: RefreshCw, run: () => void syncOfficialAvatar(),
          keywords: "头像 学校 官方 avatar sync" },
        { id: "logout", group: "动作", label: "退出登录", icon: LogOut, danger: true, run: () => void logout(),
          keywords: "登出 注销 logout" },
      );
      // 已保存账号 → 一键免密切换（当前账号不重复出条目）
      for (const acc of accounts) {
        if (acc.username === username) continue;
        actions.push({
          id: `switch-${acc.username}`, group: "动作", icon: UserRound, run: () => void loginSaved(acc.username),
          label: `切换到 ${acc.displayName ?? acc.username}`, keywords: `切换账号 ${acc.username}`,
        });
      }
    }

    // 设置页暂无分组锚点机制 → 只做「打开设置」一项（不改造设置页）
    const settings: Command[] = [
      { id: "settings-open", group: "设置", icon: Settings, run: () => setActivePanel("settings"),
        label: "打开设置（外观 / 账号 / 关于）", keywords: "设置 外观 账号 关于 settings" },
    ];

    return [...pages, ...actions, ...settings];
  }, [
    accounts,
    authed,
    loginSaved,
    logout,
    openAvatarDialog,
    openLoginDialog,
    setActivePanel,
    syncOfficialAvatar,
    theme,
    toggleTheme,
    username,
  ]);

  const q = query.trim().toLowerCase();
  const filtered = useMemo(() => {
    const hits: { c: Command; score: number }[] = [];
    for (const c of commands) {
      const score = matchScore(`${c.label} ${c.keywords ?? ""}`, q);
      if (score !== null) hits.push({ c, score });
    }
    // 同分组内按得分排序（Array.sort 稳定，同分保持声明顺序）
    hits.sort((a, b) => a.score - b.score);
    return hits.map((h) => h.c);
  }, [commands, q]);

  const indexOf = useMemo(
    () => new Map(filtered.map((c, i) => [c.id, i])),
    [filtered],
  );
  /** 选中项下标：结果变短时夹取到末项，无结果时 -1 */
  const activeIndex = filtered.length === 0 ? -1 : Math.min(active, filtered.length - 1);

  // 选中项滚入可视区（列表可滚动）
  useEffect(() => {
    if (activeIndex < 0) return;
    document
      .getElementById(`cp-opt-${activeIndex}`)
      ?.scrollIntoView({ block: "nearest" });
  }, [activeIndex]);

  // 全局快捷键：Cmd/Ctrl+K 开关（在输入框内按同样生效，故 preventDefault 拦掉原生行为）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        open ? close() : openPalette();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [close, open, openPalette]);

  // 开启：记录来源焦点 → 聚焦输入框；Esc 关闭；关闭时焦点回归原处
  useEffect(() => {
    if (!open) return;
    const prev = document.activeElement as HTMLElement | null;
    inputRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      prev?.focus?.();
    };
  }, [close, open]);

  const exec = (c: Command | undefined) => {
    if (!c) return;
    close();
    c.run();
  };

  const onKeyDown = (e: ReactKeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (filtered.length === 0) return;
      const step = e.key === "ArrowDown" ? 1 : -1;
      const cur = Math.max(activeIndex, 0);
      setActive((cur + step + filtered.length) % filtered.length);
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      exec(filtered[activeIndex]);
    }
  };

  if (!open) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="命令面板"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) close();
      }}
      onKeyDown={onKeyDown}
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/30 p-4 pt-[12vh]"
    >
      <motion.div
        initial={reduceMotion ? { opacity: 0 } : { opacity: 0, y: -8, scale: 0.98 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={
          reduceMotion ? { duration: 0.12 } : { duration: 0.18, ease: [0.22, 1, 0.36, 1] }
        }
        className="flex max-h-[70vh] w-full max-w-lg flex-col overflow-hidden rounded-card border border-line bg-surface shadow-pop"
      >
        <div className="flex shrink-0 items-center gap-2.5 border-b border-line px-4">
          <Search className="size-4 shrink-0 text-text-2" aria-hidden="true" />
          <input
            ref={inputRef}
            role="combobox"
            aria-expanded="true"
            aria-controls="cp-list"
            aria-activedescendant={
              activeIndex >= 0 ? `cp-opt-${activeIndex}` : undefined
            }
            aria-autocomplete="list"
            aria-label="搜索命令"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0); // 查询变化回到首项
            }}
            placeholder="搜索页面、动作或设置…"
            className="h-12 min-w-0 flex-1 bg-transparent text-body text-text outline-none placeholder:text-text-2/60"
          />
          <kbd className="shrink-0 rounded-[5px] border border-line bg-bg px-1.5 py-0.5 text-[10px] leading-none text-text-2">
            Esc
          </kbd>
        </div>

        <ul
          id="cp-list"
          role="listbox"
          aria-label="命令结果"
          className="min-h-0 flex-1 overflow-y-auto p-1.5"
        >
          {filtered.length === 0 && (
            <li role="presentation">
              <EmptyState
                compact
                icon={SearchX}
                title="没有匹配的命令"
                hint="试试「课表」「深色」「头像」或「退出登录」"
              />
            </li>
          )}
          {GROUPS.map((group) => {
            const items = filtered.filter((c) => c.group === group);
            if (items.length === 0) return null;
            return (
              <Fragment key={group}>
                <li
                  role="presentation"
                  className="px-2.5 pb-1 pt-2 text-caption font-medium text-text-2"
                >
                  {group}
                </li>
                {items.map((c) => {
                  const i = indexOf.get(c.id) ?? 0;
                  const selected = i === activeIndex;
                  const Icon = c.icon;
                  return (
                    <li
                      key={c.id}
                      id={`cp-opt-${i}`}
                      role="option"
                      aria-selected={selected}
                      // 阻止默认以获得无闪烁的 hover/点击：焦点始终留在输入框（键盘导航不断链）
                      onMouseDown={(e) => e.preventDefault()}
                      onMouseMove={() => setActive(i)}
                      onClick={() => exec(c)}
                      className={cn(
                        "flex cursor-pointer items-center gap-2.5 rounded-control px-2.5 py-2 text-body transition-colors duration-[var(--dur-fast)] ease-out-soft",
                        selected ? "bg-surface-2 text-text" : "text-text-2",
                        c.danger && "hover:text-alert",
                      )}
                    >
                      <Icon
                        className="size-4 shrink-0"
                        aria-hidden="true"
                        style={c.danger ? { color: "var(--color-alert)" } : undefined}
                      />
                      <span className="min-w-0 flex-1 truncate">{c.label}</span>
                      {selected && (
                        <span
                          aria-hidden="true"
                          className="shrink-0 text-caption text-text-2"
                        >
                          ↵
                        </span>
                      )}
                    </li>
                  );
                })}
              </Fragment>
            );
          })}
        </ul>
      </motion.div>
    </div>
  );
}
