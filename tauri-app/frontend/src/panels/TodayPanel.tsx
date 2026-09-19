import { Fragment, useEffect, useState } from "react";
import {
  ArrowRight,
  CreditCard,
  Landmark,
  LayoutGrid,
  Wrench,
  X,
  Zap,
  type LucideIcon,
} from "lucide-react";
import type { PanelDomain } from "@/components/PanelHeader";
import type {
  CourseBrief,
  PanelId,
  PortalOverview,
  TodayCourse,
  TodayCoursesView,
  WalletCards,
} from "@/shared/types";
import { invokeCommand } from "@/shared/tauriApi";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import { Avatar } from "@/components/Avatar";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/shared/cn";

const WEEKDAYS = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"] as const;

function greetingByHour(hour: number): string {
  if (hour >= 5 && hour < 11) return "早安";
  if (hour >= 11 && hour < 18) return "午安";
  return "晚上好";
}

/** 今日页总览四态：加载中（骨架）/ 有数据 / 出错（可重试）；游客不取数。 */
type OverviewState =
  | { phase: "loading" }
  | { phase: "ready"; data: PortalOverview }
  | { phase: "error"; message: string };

// 钱包三卡：非对称 12 栏（5/4/3），窄窗口自然堆叠；数值与脚注由 walletCell 计算，
// 单项取失败显示 "—"（不伪造数字）
const WALLET_CARDS: {
  key: "ecard" | "mail" | "library";
  label: string;
  span: string;
  accent: PanelDomain;
}[] = [
  { key: "ecard", label: "一卡通", span: "sm:col-span-5", accent: "wallet" },
  { key: "mail", label: "邮箱", span: "sm:col-span-4", accent: "wallet" },
  { key: "library", label: "图书借阅", span: "sm:col-span-3", accent: "info" },
];

/**
 * 单格取值与脚注（M3 §2.4）：一卡通格**优先实时值**（`get_wallet_cards`，
 * 脚注标「实时 / 门户快照」）；实时未到或失败时回落门户总览的数字（脚注「实时余额」）。
 * 门户数据先渲染、实时到后替换，`walletCards` 为 null 即静默降级——**不阻塞首屏、不弹错**。
 * 邮箱未读与图书借阅仍以门户总览为唯一来源。
 */
function walletCell(
  key: (typeof WALLET_CARDS)[number]["key"],
  wallet: PortalOverview["wallet"],
  cards: WalletCards | null,
): { value: string | null; note: string } {
  if (key === "ecard") {
    const realtime = cards?.ecard;
    if (realtime?.valueYuan != null) {
      return {
        value: realtime.valueYuan.toFixed(2),
        note: realtime.source === "realtime" ? "实时" : "门户快照",
      };
    }
    return {
      value: wallet?.cardBalance != null ? wallet.cardBalance.toFixed(2) : null,
      note: "实时余额",
    };
  }
  if (key === "mail") {
    return {
      value: wallet?.mailUnread != null ? String(wallet.mailUnread) : null,
      note: "未读邮件",
    };
  }
  return {
    value: wallet?.bookBorrowed != null ? String(wallet.bookBorrowed) : null,
    note: "在借图书",
  };
}

// 快捷动作：panel 有值则切对应面板；enabled=false 为未落地门户模块（M2+ 接上）
const QUICK_ACTIONS: {
  label: string;
  icon: LucideIcon;
  panel?: PanelId;
  enabled: boolean;
  arrow?: boolean;
}[] = [
  { label: "查电费", icon: Zap, panel: "power", enabled: true },
  { label: "卡片充值", icon: CreditCard, panel: "wallet", enabled: true },
  { label: "网络报修", icon: Wrench, enabled: false },
  { label: "办事大厅", icon: Landmark, enabled: false },
  { label: "全部应用", icon: LayoutGrid, panel: "apps", enabled: true, arrow: true },
];

/** 下一节课横幅文案：「HH:MM · 课程 · 教室（教学班）」，空段自动省略。 */
function nextCourseLabel(c: CourseBrief): string {
  const time = c.startTime ?? `第${c.slot}节`;
  const parts = [time, c.name, c.room].filter(Boolean);
  if (c.teachingClass) parts.push(`（${c.teachingClass}）`);
  return parts.join(" · ");
}

/** 本地「下一节课」横幅文案（契约 §15.2）：「HH:MM-HH:MM 课名 @教室」，空段自动省略。 */
function todayCourseLabel(c: TodayCourse): string {
  return [`${c.startHm}-${c.endHm}`, c.name, c.room ? `@${c.room}` : ""]
    .filter(Boolean)
    .join(" ");
}

/**
 * 列表行的展示态（契约 §15.2）：ongoing 由后端下发；「已结束」不由前端读时钟
 * 判断，而是由列表与 next 的相对位置推导——next 是升序序列中第一门未结束的课，
 * 位于它之前的行都已结束，next 为 null 则全部已结束。
 */
function localRowState(
  c: TodayCourse,
  list: TodayCourse[],
  next: TodayCourse | null,
): "ongoing" | "ended" | "upcoming" {
  if (c.ongoing) return "ongoing";
  if (!next) return "ended";
  const nextIdx = list.findIndex(
    (x) => x.courseId === next.courseId && x.startHm === next.startHm && x.name === next.name,
  );
  if (nextIdx === -1) return "ended";
  return list.indexOf(c) < nextIdx ? "ended" : "upcoming";
}

/** 今日课程取数四态（与门户总览互不阻塞，风险 R9：各自 alive 标志）。 */
type LocalCoursesState =
  | { phase: "loading" }
  | { phase: "ready"; data: TodayCoursesView }
  | { phase: "error"; message: string };

export function TodayPanel() {
  const status = useAuthStore((s) => s.status);
  const displayName = useAuthStore((s) => s.displayName);
  const avatarBase64 = useAuthStore((s) => s.avatarBase64);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const setActivePanel = useUiStore((s) => s.setActivePanel);
  const [guideDismissed, setGuideDismissed] = useState(false);
  const [overview, setOverview] = useState<OverviewState>({ phase: "loading" });
  const [localCourses, setLocalCourses] = useState<LocalCoursesState>({ phase: "loading" });
  // 首页钱包卡的实时一卡通（后端聚合 + 门户降级）：到达后替换门户数字，失败静默为 null
  const [walletCards, setWalletCards] = useState<WalletCards | null>(null);
  const [reloadTick, setReloadTick] = useState(0);

  const authed = status === "authed";
  const now = new Date();
  // 本地日期行（不编造教学周：教学周信息待后续批次接入展示）
  const dateLabel = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")} ${WEEKDAYS[now.getDay()]}`;

  // 已登录时取门户总览（学期/钱包/下一节课）；登出或失败不阻塞本页其余区块
  useEffect(() => {
    if (!authed) {
      setOverview({ phase: "ready", data: { semester: null, wallet: null, nextCourse: null, fetchedAt: 0 } });
      return;
    }
    let alive = true;
    setOverview({ phase: "loading" });
    invokeCommand<PortalOverview>("get_portal_overview").then((r) => {
      if (!alive) return;
      if (r.success && r.data) setOverview({ phase: "ready", data: r.data });
      else setOverview({ phase: "error", message: r.message ?? "门户数据获取失败" });
    });
    return () => {
      alive = false;
    };
  }, [authed, reloadTick]);

  // 本地今日课程（批 9，契约 §15.2）：与门户总览并行发起、各自 alive 标志、
  // 互不阻塞；失败回落门户 nextCourse 路径（useLocal 为 false 即现状）
  useEffect(() => {
    if (!authed) {
      setLocalCourses({
        phase: "ready",
        data: { date: "", currentWeek: null, state: "normal", hasLocal: false, courses: [], next: null, swapWeekday: null },
      });
      return;
    }
    let alive = true;
    setLocalCourses({ phase: "loading" });
    invokeCommand<TodayCoursesView>("get_today_courses").then((r) => {
      if (!alive) return;
      if (r.success && r.data) setLocalCourses({ phase: "ready", data: r.data });
      else setLocalCourses({ phase: "error", message: r.message ?? "本地课表获取失败" });
    });
    return () => {
      alive = false;
    };
  }, [authed, reloadTick]);

  // 首页钱包卡实时一卡通（M3 §2.4）：与门户总览**并行**发起——门户数字先渲染，
  // 实时值到达后替换脚注为「实时 / 门户快照」；失败（校外/桥失败/未登录）静默，
  // 不弹错、不阻塞首屏（回落门户数字）
  useEffect(() => {
    if (!authed) {
      setWalletCards(null);
      return;
    }
    let alive = true;
    invokeCommand<WalletCards>("get_wallet_cards").then((r) => {
      if (!alive) return;
      setWalletCards(r.success && r.data ? r.data : null);
    });
    return () => {
      alive = false;
    };
  }, [authed, reloadTick]);

  const loading = overview.phase === "loading";
  const wallet = overview.phase === "ready" ? overview.data.wallet : null;
  const nextCourse = overview.phase === "ready" ? overview.data.nextCourse : null;
  // 本地优先（契约 §15.2）：has_local 才消费本地结果；no_semester 时 has_local
  // 必为 false，自然走门户现状
  const local = localCourses.phase === "ready" ? localCourses.data : null;
  const useLocal = local != null && local.hasLocal;

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      {/* 欢迎区：头像 + 时段问候 + 日期行 */}
      <div className="flex items-center gap-4">
        <Avatar
          size="lg"
          src={avatarBase64 ? `data:image/png;base64,${avatarBase64}` : null}
          name={displayName}
        />
        <div className="min-w-0">
          <h2 className="text-display font-semibold text-text">
            {authed
              ? `${greetingByHour(now.getHours())}，${displayName ?? "同学"}`
              : "你好，同学"}
          </h2>
          <p className="tabular-num mt-1 text-caption text-text-2">{dateLabel}</p>
        </div>
      </div>

      {/* 游客登录引导条：本地 state 可关闭，不持久化 */}
      {!authed && !guideDismissed && (
        <Surface
          accent="brand"
          className="mt-4 flex items-center justify-between gap-3 py-3 pl-4 pr-2"
        >
          <p className="text-body text-text">登录后同步课表、一卡通余额与待办</p>
          <div className="flex shrink-0 items-center gap-1">
            <Button size="sm" onClick={openLoginDialog}>
              登录
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="关闭登录引导"
              onClick={() => setGuideDismissed(true)}
            >
              <X aria-hidden="true" />
            </Button>
          </div>
        </Surface>
      )}

      {/* 钱包三卡：非对称栅格；加载骨架 → 真实数字，单项取失败回落 "—" */}
      <div className="mt-5 grid grid-cols-12 gap-3">
        {WALLET_CARDS.map((card) => {
          const cell = walletCell(card.key, wallet, walletCards);
          return (
            <Surface
              key={card.key}
              accent={card.accent}
              hover
              className={cn("col-span-12 px-4 py-4", card.span)}
            >
              <p className="text-caption text-text-2">{card.label}</p>
              {loading ? (
                <div
                  aria-hidden
                  className="mt-2 h-7 w-20 animate-pulse rounded bg-line"
                />
              ) : (
                <p className="tabular-num mt-2 text-display font-semibold text-text">
                  {cell.value ?? "—"}
                </p>
              )}
              <p className="mt-3 text-caption text-text-2">{cell.note}</p>
            </Surface>
          );
        })}
      </div>

      {/* 总览取数失败：可重试（钱包三卡已回落 "—"，不白屏） */}
      {overview.phase === "error" && (
        <Surface
          accent="info"
          className="mt-4 flex items-center justify-between gap-3 py-3 pl-4 pr-2"
        >
          <p className="min-w-0 truncate text-body text-text-2">
            门户数据获取失败：{overview.message}
          </p>
          <Button
            variant="outline"
            size="sm"
            className="shrink-0"
            onClick={() => setReloadTick((t) => t + 1)}
          >
            重试
          </Button>
        </Surface>
      )}

      {/* 今日课程（批 9，契约 §15.2）：has_local 时本地优先——skipped → 今日放假、
          vacation → 假期中、normal → 下一节课横幅 + 今日课程列表（ongoing 高亮、
          已结束置灰；不做时间轴刻度）；next 为 null 如实显示今日无课/已结束 */}
      {useLocal && local && local.state === "skipped" && (
        <Surface accent="sched" className="mt-4 px-4 py-3">
          <p className="text-body text-text">今日放假</p>
        </Surface>
      )}
      {useLocal && local && local.state === "vacation" && (
        <Surface accent="sched" className="mt-4 px-4 py-3">
          <p className="text-body text-text">假期中</p>
        </Surface>
      )}
      {useLocal && local && local.state === "normal" && (
        <>
          {local.swapWeekday != null && (
            <Surface accent="sched" className="mt-4 flex items-center gap-2 px-4 py-3">
              <span className="rounded bg-sched/10 px-1.5 py-0.5 text-caption font-medium text-sched">
                班
              </span>
              <span className="text-body text-text">
                今日调休补课，按{"周一二三四五六日".charAt(local.swapWeekday)}课表上课
              </span>
            </Surface>
          )}
          {local.next && (
            <Surface accent="sched" className="mt-4 flex items-center gap-2 px-4 py-3">
              <span className="shrink-0 text-body font-medium text-text">下一节课</span>
              <span className="truncate text-body text-text-2">
                {todayCourseLabel(local.next)}
              </span>
            </Surface>
          )}
          <Surface className="mt-4 px-4 py-4">
            <p className="text-body font-medium text-text">今日课程</p>
            {local.courses.length === 0 ? (
              <p className="mt-3 text-body text-text-2">今日无课</p>
            ) : (
              <ul className="mt-2 divide-y divide-line">
                {local.courses.map((c) => {
                  const row = localRowState(c, local.courses, local.next);
                  return (
                    <li
                      key={`${c.courseId}-${c.startHm}-${c.name}`}
                      className={cn(
                        "flex items-baseline gap-3 py-2.5 text-body",
                        row === "ended" && "opacity-50",
                      )}
                    >
                      <span
                        className={cn(
                          "tabular-num shrink-0",
                          row === "ongoing" ? "font-medium text-text" : "text-text-2",
                        )}
                      >
                        {c.startHm}-{c.endHm}
                      </span>
                      <span
                        className={cn(
                          "min-w-0 truncate",
                          row === "ongoing" ? "font-medium text-text" : "text-text-2",
                        )}
                      >
                        {c.name}
                      </span>
                      {row === "ongoing" && (
                        <span className="shrink-0 text-caption text-brand">进行中</span>
                      )}
                      {c.room && (
                        <span className="ml-auto shrink-0 truncate text-caption text-text-2">
                          @{c.room}
                        </span>
                      )}
                    </li>
                  );
                })}
              </ul>
            )}
          </Surface>
        </>
      )}

      {/* 下一节课横幅（门户兜底）：has_local 时本地横幅已接管，门户 nextCourse
          不再展示（契约 §15.2 本地优先/教务兜底）；无课 / 取数失败时整行隐藏 */}
      {nextCourse && !useLocal && (
        <Surface accent="sched" className="mt-4 flex items-center gap-2 px-4 py-3">
          <span className="shrink-0 text-body font-medium text-text">下一节课</span>
          <span className="truncate text-body text-text-2">
            {nextCourseLabel(nextCourse)}
          </span>
        </Surface>
      )}

      {/* 快捷动作：游客点击已落地项先登录；未落地项禁用 + tooltip */}
      <Surface className="mt-4 px-4 py-4">
        <p className="text-body font-medium text-text">快捷动作</p>
        <TooltipProvider delayDuration={150}>
          <div className="mt-3 flex flex-wrap gap-2">
            {QUICK_ACTIONS.map((action) => {
              const button = (
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!action.enabled}
                  onClick={() => {
                    if (!authed) {
                      openLoginDialog();
                      return;
                    }
                    if (action.panel) setActivePanel(action.panel);
                  }}
                >
                  <action.icon aria-hidden="true" />
                  {action.label}
                  {action.arrow && <ArrowRight aria-hidden="true" />}
                </Button>
              );
              if (action.enabled) return <Fragment key={action.label}>{button}</Fragment>;
              return (
                <Tooltip key={action.label}>
                  <TooltipTrigger asChild>
                    <span className="inline-flex">{button}</span>
                  </TooltipTrigger>
                  <TooltipContent>门户接入后开放</TooltipContent>
                </Tooltip>
              );
            })}
          </div>
        </TooltipProvider>
      </Surface>

      <p className="mt-6 text-center text-caption text-text-2">今日没有更多安排</p>
    </section>
  );
}
