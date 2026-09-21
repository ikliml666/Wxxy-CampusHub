import { useEffect, useState } from "react";
import { CalendarDays, ChevronLeft, ChevronRight, Clock, MapPin } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader, domainVar } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import type { ScheduleClassify, ScheduleEvent } from "@/shared/types";
import { invokeCommand } from "@/shared/tauriApi";
import { cn } from "@/shared/cn";

const DAY_HEADERS = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"] as const;

const DAY_MS = 86_400_000;

/** 周历列最小宽（7 × 136px = 952px，窄面板横向滚动）。 */
const WEEK_COL_MIN = 136;
const WEEK_GRID_MIN = WEEK_COL_MIN * 7;

/** 「最近日程」窗口：今天起 30 天。 */
const UPCOMING_DAYS = 30;

/** 分类四态。 */
type ClassifyState =
  | { phase: "loading" }
  | { phase: "ready"; data: ScheduleClassify[] }
  | { phase: "error"; message: string };

/** 事件四态（周内无日程为 empty）。 */
type EventsState =
  | { phase: "loading" }
  | { phase: "ready"; data: ScheduleEvent[] }
  | { phase: "empty" }
  | { phase: "error"; message: string };

/**
 * 月明细四态（月视图角标）：角标不再走 get_schedule_day_counts，改由
 * 过滤课程后的月明细前端自算（月明细本就全量返回，与周列表同源同过滤口径）。
 * 拉取失败只影响角标与空态，网格与跳周不受影响。
 */
type MonthDetailState =
  | { phase: "loading" }
  | { phase: "ready"; map: Record<string, number>; total: number }
  | { phase: "error"; message: string };

/** 最近日程四态。 */
type UpcomingState =
  | { phase: "loading" }
  | { phase: "ready"; data: ScheduleEvent[] }
  | { phase: "error"; message: string };

const pad2 = (n: number) => String(n).padStart(2, "0");

/** 毫秒 → 本地 "HH:MM"。 */
function fmtTime(ms: number): string {
  const d = new Date(ms);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

/** 毫秒 → 本地 "M.D"。 */
function fmtDay(ms: number): string {
  const d = new Date(ms);
  return `${d.getMonth() + 1}.${d.getDate()}`;
}

/** 第 offset 周的 [周一 00:00.000, 周日 23:59.999] 本地区间（毫秒）。 */
function weekRange(offset: number): { startMs: number; endMs: number; start: Date } {
  const now = new Date();
  // 列序周一为 0；getDay() 周日=0，折算到第 6 列
  const dow = (now.getDay() + 6) % 7;
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate() - dow + offset * 7);
  return { startMs: start.getTime(), endMs: start.getTime() + 7 * DAY_MS - 1, start };
}

/**
 * 第 offset 月的月视图网格：首格 = 当月 1 日所在周的周一（列序与周视图一致），
 * 行数按需 5 或 6 行；count 区间覆盖全部网格日期。
 */
function monthGrid(offset: number): {
  startMs: number;
  endMs: number;
  year: number;
  month: number;
  cells: number;
  gridStartMs: number;
} {
  const now = new Date();
  const first = new Date(now.getFullYear(), now.getMonth() + offset, 1);
  const dow = (first.getDay() + 6) % 7;
  const monthDays = new Date(first.getFullYear(), first.getMonth() + 1, 0).getDate();
  const cells = Math.ceil((dow + monthDays) / 7) * 7;
  const gridStart = new Date(first.getFullYear(), first.getMonth(), 1 - dow);
  return {
    startMs: gridStart.getTime(),
    endMs: gridStart.getTime() + cells * DAY_MS - 1,
    year: first.getFullYear(),
    month: first.getMonth(),
    cells,
    gridStartMs: gridStart.getTime(),
  };
}

/** 同一自然日（本地时区）。 */
const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() &&
  a.getMonth() === b.getMonth() &&
  a.getDate() === b.getDate();

/** Date → 计数 day 键 "YYYY-MM-DD"。 */
const fmtDayKey = (d: Date) =>
  `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;

/** 周视图/详情的时间段文案：endMs==startMs（会议结束时刻未知）显示单时刻。 */
function fmtTimeRange(startMs: number, endMs: number): string {
  return endMs > startMs ? `${fmtTime(startMs)} – ${fmtTime(endMs)}` : fmtTime(startMs);
}

/** 事件按开始时间落到本周的列（0=周一…6=周日），越界（跨周事件）返回 null。 */
function dayIndexOf(ms: number, weekStart: Date): number | null {
  const d = new Date(ms);
  const midnight = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const idx = Math.round((midnight - weekStart.getTime()) / DAY_MS);
  return idx >= 0 && idx <= 6 ? idx : null;
}

/**
 * 课程类条目判定：课程归属课表页（TimetablePanel）呈现，日程页在分桶/计数前剔除。
 * 双判据容错：classifyName 恒等「课程」，或 classifyCode 含 "course"（大小写不敏感）。
 */
function isCourseEvent(ev: ScheduleEvent): boolean {
  return ev.classifyName === "课程" || ev.classifyCode.toLowerCase().includes("course");
}

/** 事件自身所在列序（0=周一…6=周日），详情卡不再绑定周视图列序。 */
const dayIndexOfSelf = (ms: number) => (new Date(ms).getDay() + 6) % 7;

/** 「最近日程」分组：按自然日升序，今天/明天/周X + 日期标签。 */
type UpcomingGroup = {
  dayMs: number;
  label: string;
  dateLabel: string;
  isToday: boolean;
  events: ScheduleEvent[];
};

function buildUpcomingGroups(events: ScheduleEvent[]): UpcomingGroup[] {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const buckets = new Map<string, { dayMs: number; events: ScheduleEvent[] }>();
  for (const ev of events) {
    const d = new Date(ev.startMs);
    d.setHours(0, 0, 0, 0);
    const key = fmtDayKey(d);
    const bucket = buckets.get(key);
    if (bucket) bucket.events.push(ev);
    else buckets.set(key, { dayMs: d.getTime(), events: [ev] });
  }
  return [...buckets.values()]
    .sort((a, b) => a.dayMs - b.dayMs)
    .map(({ dayMs, events: evs }) => {
      const d = new Date(dayMs);
      const offsetDays = Math.round((dayMs - today.getTime()) / DAY_MS);
      const label =
        offsetDays === 0 ? "今天" : offsetDays === 1 ? "明天" : DAY_HEADERS[(d.getDay() + 6) % 7];
      return {
        dayMs,
        label,
        dateLabel: fmtDay(dayMs),
        isToday: sameDay(d, today),
        events: evs.sort((a, b) => a.startMs - b.startMs),
      };
    });
}

/** 分类过滤 chip：色点用接口给的 classifyColor，选中态沿用域色 token。 */
function ClassifyChip({
  c,
  active,
  onToggle,
}: {
  c: ScheduleClassify;
  active: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onToggle}
      className={cn(
        "flex items-center gap-1.5 rounded-control border px-3 py-1.5 text-caption transition-colors duration-[var(--dur-fast)] ease-out-soft",
        active
          ? "border-sched/30 bg-sched/10 font-medium text-sched"
          : "border-line bg-surface text-text-2 hover:border-line-strong hover:text-text",
      )}
    >
      <span
        aria-hidden
        className="size-2 shrink-0 rounded-full"
        style={{ backgroundColor: c.color || domainVar.sched }}
      />
      {c.name}
    </button>
  );
}

export function SchedulePanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const authed = status === "authed";

  const [classify, setClassify] = useState<ClassifyState>({ phase: "loading" });
  const [selectedCodes, setSelectedCodes] = useState<string[]>([]);
  const [viewMode, setViewMode] = useState<"week" | "month">("week");
  const [weekOffset, setWeekOffset] = useState(0);
  const [monthOffset, setMonthOffset] = useState(0);
  const [events, setEvents] = useState<EventsState>({ phase: "loading" });
  const [monthDetail, setMonthDetail] = useState<MonthDetailState>({ phase: "loading" });
  const [upcoming, setUpcoming] = useState<UpcomingState>({ phase: "loading" });
  const [selected, setSelected] = useState<ScheduleEvent | null>(null);
  const [reloadTick, setReloadTick] = useState(0);

  // 分类列表：登录后取一次（后端会话内缓存），取回默认全选
  useEffect(() => {
    if (!authed) return;
    let alive = true;
    setClassify({ phase: "loading" });
    setSelectedCodes([]);
    invokeCommand<ScheduleClassify[]>("get_schedule_classify").then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        // 课程类分类整个剔除（2026-09-21 用户裁决：日程页不出现课表，chip 一并
        // 去掉——对面 UI 轮只剔了事件，chip 残留点击即空列表）；事件侧剔除见
        // isCourseEvent（三处入桶前），此处判据与之一致按 name/code 双保险。
        const visible = r.data.filter(
          (c) => !c.name.includes("课表") && !c.code.toLowerCase().includes("course"),
        );
        setClassify({ phase: "ready", data: visible });
        setSelectedCodes(visible.map((c) => c.code));
      } else {
        setClassify({ phase: "error", message: r.message ?? "日程分类获取失败" });
      }
    });
    return () => {
      alive = false;
    };
  }, [authed, reloadTick]);

  // 周区间明细：切周 / 切过滤触发；全不选或不处于周视图时不发请求
  //（服务端空 codes 语义未实测，前端规避）。课程类条目在入桶前剔除。
  const { startMs, endMs, start } = weekRange(weekOffset);
  useEffect(() => {
    if (!authed || viewMode !== "week" || classify.phase !== "ready" || selectedCodes.length === 0)
      return;
    let alive = true;
    setEvents({ phase: "loading" });
    setSelected(null);
    invokeCommand<ScheduleEvent[]>("get_schedule_month", { startMs, endMs, codes: selectedCodes }).then(
      (r) => {
        if (!alive) return;
        if (r.success && r.data) {
          const visible = r.data.filter((ev) => !isCourseEvent(ev));
          setEvents(visible.length > 0 ? { phase: "ready", data: visible } : { phase: "empty" });
        } else {
          setEvents({ phase: "error", message: r.message ?? "日程获取失败" });
        }
      },
    );
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [authed, classify, selectedCodes, viewMode, weekOffset, reloadTick]);

  // 月明细（月视图角标）：与周列表同一接口同一过滤口径，角标由过滤后的明细
  // 前端自算（按开始日落日计数）；失败只降级角标，不白屏。
  const month = viewMode === "month" ? monthGrid(monthOffset) : null;
  useEffect(() => {
    if (!authed || viewMode !== "month" || classify.phase !== "ready" || selectedCodes.length === 0) {
      return;
    }
    let alive = true;
    setMonthDetail({ phase: "loading" });
    const range = monthGrid(monthOffset);
    invokeCommand<ScheduleEvent[]>("get_schedule_month", {
      startMs: range.startMs,
      endMs: range.endMs,
      codes: selectedCodes,
    }).then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        const visible = r.data.filter((ev) => !isCourseEvent(ev));
        const map: Record<string, number> = {};
        for (const ev of visible) {
          const key = fmtDayKey(new Date(ev.startMs));
          map[key] = (map[key] ?? 0) + 1;
        }
        setMonthDetail({ phase: "ready", map, total: visible.length });
      } else {
        setMonthDetail({ phase: "error", message: r.message ?? "月日程获取失败" });
      }
    });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [authed, classify, selectedCodes, viewMode, monthOffset, reloadTick]);

  // 最近日程：今天起 30 天（与日历同一过滤口径，课程已剔除）。全不选时清空。
  useEffect(() => {
    if (!authed || classify.phase !== "ready" || selectedCodes.length === 0) {
      setUpcoming({ phase: "ready", data: [] });
      return;
    }
    let alive = true;
    setUpcoming({ phase: "loading" });
    const today = new Date();
    today.setHours(0, 0, 0, 0);
    invokeCommand<ScheduleEvent[]>("get_schedule_month", {
      startMs: today.getTime(),
      endMs: today.getTime() + UPCOMING_DAYS * DAY_MS - 1,
      codes: selectedCodes,
    }).then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        const visible = r.data.filter((ev) => !isCourseEvent(ev));
        setUpcoming({ phase: "ready", data: visible });
      } else {
        setUpcoming({ phase: "error", message: r.message ?? "最近日程获取失败" });
      }
    });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [authed, classify, selectedCodes, reloadTick]);

  const toggleCode = (code: string) => {
    setSelectedCodes((cs) =>
      cs.includes(code) ? cs.filter((c) => c !== code) : [...cs, code],
    );
  };

  /** 周期箭头：周视图切周，月视图切月。 */
  const shiftPeriod = (dir: -1 | 1) => {
    setSelected(null);
    if (viewMode === "week") setWeekOffset((w) => w + dir);
    else setMonthOffset((m) => m + dir);
  };

  /** 切换视图（周/月），清除选中详情。 */
  const switchView = (mode: "week" | "month") => {
    if (mode === viewMode) return;
    setSelected(null);
    setViewMode(mode);
  };

  /** 月视图点击某天 → 跳到该天所在周（周视图明细按当前过滤取数）。 */
  const gotoWeek = (d: Date) => {
    const base = weekRange(0).start.getTime();
    const monday = new Date(d.getFullYear(), d.getMonth(), d.getDate() - ((d.getDay() + 6) % 7));
    setWeekOffset(Math.round((monday.getTime() - base) / (7 * DAY_MS)));
    setSelected(null);
    setViewMode("week");
  };

  const todayCol = (new Date().getDay() + 6) % 7;
  // 事件按列分组（开始时间落列；跨周事件忽略）
  const byDay: ScheduleEvent[][] = Array.from({ length: 7 }, () => []);
  if (events.phase === "ready") {
    for (const ev of events.data) {
      const idx = dayIndexOf(ev.startMs, start);
      if (idx !== null) byDay[idx].push(ev);
    }
    for (const col of byDay) col.sort((a, b) => a.startMs - b.startMs);
  }

  const upcomingGroups =
    upcoming.phase === "ready" ? buildUpcomingGroups(upcoming.data) : [];
  const hasUpcomingToday = upcomingGroups.some((g) => g.isToday);

  const periodSwitcher = (
    <div className="flex items-center gap-1.5">
      {/* 周/月视图切换 */}
      <div
        className="flex items-center rounded-control border border-line p-0.5"
        role="group"
        aria-label="视图切换"
      >
        {(["week", "month"] as const).map((m) => (
          <button
            key={m}
            type="button"
            aria-pressed={viewMode === m}
            onClick={() => switchView(m)}
            className={cn(
              "rounded-[calc(var(--radius)-2px)] px-2.5 py-1 text-caption transition-colors duration-[var(--dur-fast)] ease-out-soft",
              viewMode === m
                ? "bg-sched/10 font-medium text-sched"
                : "text-text-2 hover:text-text",
            )}
          >
            {m === "week" ? "周" : "月"}
          </button>
        ))}
      </div>
      <Button
        variant="outline"
        size="icon-sm"
        aria-label={viewMode === "week" ? "上一周" : "上一月"}
        onClick={() => shiftPeriod(-1)}
      >
        <ChevronLeft aria-hidden="true" />
      </Button>
      <span className="tabular-num min-w-20 text-center text-body font-medium text-text-2">
        {viewMode === "week"
          ? `${fmtDay(startMs)} – ${fmtDay(endMs - DAY_MS + 1)}`
          : month
            ? `${month.year}年${month.month + 1}月`
            : ""}
      </span>
      <Button
        variant="outline"
        size="icon-sm"
        aria-label={viewMode === "week" ? "下一周" : "下一月"}
        onClick={() => shiftPeriod(1)}
      >
        <ChevronRight aria-hidden="true" />
      </Button>
    </div>
  );

  return (
    <section className="mx-auto mt-8 w-full max-w-6xl px-4">
      <PanelHeader
        title="日程"
        description="个人日程 · 活动 · 会议 · 值班"
        domain="sched"
        actions={authed ? periodSwitcher : undefined}
      />

      {!authed ? (
        <Surface>
          <EmptyState
            icon={CalendarDays}
            domain="sched"
            title="登录后查看日程"
            hint="登录后展示门户日程服务中的个人日程、活动、会议与值班安排。"
            action={<Button onClick={openLoginDialog}>登录</Button>}
          />
        </Surface>
      ) : (
        <>
          {/* 分类过滤 chips（含加载骨架 / 出错重试） */}
          {classify.phase === "error" ? (
            <Surface accent="sched" className="flex items-center justify-between gap-3 py-3 pl-4 pr-2">
              <p className="min-w-0 truncate text-body text-text-2">
                日程分类获取失败：{classify.message}
              </p>
              <Button variant="outline" size="sm" className="shrink-0" onClick={() => setReloadTick((t) => t + 1)}>
                重试
              </Button>
            </Surface>
          ) : classify.phase === "loading" ? (
            <div aria-hidden className="flex flex-wrap gap-2">
              {[0, 1, 2, 3, 4].map((i) => (
                <span
                  key={i}
                  className="h-9 w-16 animate-pulse rounded-control border border-line bg-line"
                />
              ))}
            </div>
          ) : (
            <div className="flex flex-wrap gap-2" role="group" aria-label="日程分类过滤">
              {classify.data.map((c) => (
                <ClassifyChip
                  key={c.code}
                  c={c}
                  active={selectedCodes.includes(c.code)}
                  onToggle={() => toggleCode(c.code)}
                />
              ))}
            </div>
          )}

          {/* 周视图：列最小宽 136px，窄面板横向滚动；事件块两行截断不挤压 */}
          {viewMode === "week" && (
            <Surface className="mt-4 overflow-x-auto">
              <div className="grid grid-cols-7" style={{ minWidth: WEEK_GRID_MIN }}>
                {DAY_HEADERS.map((d, i) => (
                  <div
                    key={d}
                    className={cn(
                      "border-b border-line py-2 text-center",
                      i === todayCol && weekOffset === 0
                        ? "bg-sched/5 font-medium text-sched"
                        : "text-text-2",
                      i > 0 && "border-l border-line",
                    )}
                  >
                    <p className="text-caption">{d}</p>
                    <p className="tabular-num text-caption opacity-70">
                      {fmtDay(startMs + i * DAY_MS)}
                    </p>
                  </div>
                ))}
                {byDay.map((dayEvents, col) => (
                  <div
                    key={col}
                    className={cn(
                      "flex min-h-32 flex-col gap-1.5 p-1.5",
                      col > 0 && "border-l border-line",
                      col === todayCol && weekOffset === 0 && "bg-sched/5",
                    )}
                  >
                    {dayEvents.map((ev) => (
                      <button
                        key={ev.id}
                        type="button"
                        aria-pressed={selected?.id === ev.id}
                        onClick={() => setSelected((s) => (s?.id === ev.id ? null : ev))}
                        className={cn(
                          "rounded-inner border-line bg-line/40 px-2 py-1.5 text-left transition-shadow duration-[var(--dur-fast)] ease-out-soft hover:shadow-card",
                          selected?.id === ev.id && "ring-1 ring-sched",
                        )}
                        style={{ borderLeft: `3px solid ${ev.color || domainVar.sched}` }}
                      >
                        <p className="line-clamp-2 text-caption font-medium text-text">{ev.title}</p>
                        <p className="tabular-num mt-0.5 text-caption text-text-2">
                          {fmtTimeRange(ev.startMs, ev.endMs)}
                        </p>
                      </button>
                    ))}
                  </div>
                ))}
              </div>
            </Surface>
          )}

          {/* 月视图：日期格 + 每日日程数角标（由过滤后的月明细自算）；点击某天跳到该周 */}
          {viewMode === "month" && month && (
            <Surface className="mt-4 overflow-hidden">
              <div className="grid grid-cols-7">
                {DAY_HEADERS.map((d) => (
                  <div key={d} className="border-b border-line py-2 text-center text-caption text-text-2">
                    {d}
                  </div>
                ))}
                {Array.from({ length: month.cells }, (_, i) => {
                  const d = new Date(month.gridStartMs + i * DAY_MS);
                  const inMonth = d.getMonth() === month.month;
                  const isToday = sameDay(d, new Date());
                  const count =
                    monthDetail.phase === "ready" && inMonth
                      ? monthDetail.map[fmtDayKey(d)]
                      : undefined;
                  return (
                    <button
                      key={i}
                      type="button"
                      disabled={!inMonth}
                      aria-label={`${d.getMonth() + 1}月${d.getDate()}日${count ? `，${count} 条日程，点击查看该周` : ""}`}
                      onClick={() => gotoWeek(d)}
                      className={cn(
                        "flex h-16 flex-col items-center justify-start gap-1 border-line p-1.5 transition-colors duration-[var(--dur-fast)] ease-out-soft",
                        i >= 7 && "border-t",
                        i % 7 > 0 && "border-l",
                        inMonth
                          ? isToday
                            ? "bg-sched/5"
                            : "hover:bg-line/40"
                          : "cursor-default",
                      )}
                    >
                      <span
                        className={cn(
                          "tabular-num text-caption",
                          !inMonth && "text-text-2/40",
                          inMonth && isToday && "font-medium text-sched",
                        )}
                      >
                        {d.getDate()}
                      </span>
                      {!!count && (
                        <span className="tabular-num rounded-full bg-sched/10 px-1.5 text-caption font-medium leading-4 text-sched">
                          {count}
                        </span>
                      )}
                    </button>
                  );
                })}
              </div>
            </Surface>
          )}

          {/* 详情卡：选中事件的时间 / 地点 / 分类（周历块与最近日程行共用） */}
          {selected ? (
            <Surface accent="sched" className="mt-3 px-4 py-3">
              <div className="flex items-center gap-2">
                <span
                  aria-hidden
                  className="size-2 shrink-0 rounded-full"
                  style={{ backgroundColor: selected.color || domainVar.sched }}
                />
                <p className="min-w-0 truncate text-body font-medium text-text">{selected.title}</p>
                <span className="ml-auto shrink-0 text-caption text-text-2">
                  {selected.classifyName || "未分类"}
                </span>
              </div>
              <p className="tabular-num mt-1.5 text-caption text-text-2">
                {DAY_HEADERS[dayIndexOfSelf(selected.startMs)]}{" "}
                {fmtDay(selected.startMs)} · {fmtTimeRange(selected.startMs, selected.endMs)}
              </p>
              {selected.place && (
                <p className="mt-1 text-caption text-text-2">地点：{selected.place}</p>
              )}
              {selected.extra && <p className="mt-1 text-caption text-text-2">{selected.extra}</p>}
            </Surface>
          ) : events.phase === "ready" && viewMode === "week" ? (
            <p className="mt-3 text-center text-caption text-text-2">点击日程块查看详情</p>
          ) : null}

          {/* 事件区四态（分类加载中沿用骨架；分类错误已在其上方显示）。
              周视图吃明细，月视图吃月明细，各自独立四态。 */}
          {classify.phase === "ready" && selectedCodes.length === 0 && (
            <Surface className="mt-3">
              <EmptyState
                compact
                icon={CalendarDays}
                domain="sched"
                title="已隐藏全部分类"
                hint="至少选择一个分类即可查看对应日程。"
              />
            </Surface>
          )}
          {viewMode === "week" && events.phase === "loading" && (
            <div aria-hidden className="mt-3 space-y-2">
              {[0, 1, 2].map((i) => (
                <Surface key={i} className="flex items-center gap-3 px-4 py-3">
                  <span className="h-4 w-14 shrink-0 animate-pulse rounded bg-line" />
                  <span className="h-4 min-w-0 flex-1 animate-pulse rounded bg-line" />
                </Surface>
              ))}
            </div>
          )}
          {viewMode === "week" && events.phase === "error" && (
            <Surface className="mt-3">
              <EmptyState
                icon={Clock}
                domain="sched"
                title="日程获取失败"
                hint={events.message}
                action={
                  <Button variant="outline" onClick={() => setReloadTick((t) => t + 1)}>
                    重试
                  </Button>
                }
              />
            </Surface>
          )}
          {viewMode === "week" && events.phase === "empty" && selectedCodes.length > 0 && (
            <Surface className="mt-3">
              <EmptyState
                compact
                icon={CalendarDays}
                domain="sched"
                title="本周暂无日程"
                hint="当前筛选的分类在本周没有日程安排。"
              />
            </Surface>
          )}
          {/* 月视图：月明细失败不白屏（角标缺失仍可点日期跳周），给错误提示可重试 */}
          {viewMode === "month" && monthDetail.phase === "error" && (
            <Surface accent="sched" className="mt-3 flex items-center justify-between gap-3 py-3 pl-4 pr-2">
              <p className="min-w-0 truncate text-body text-text-2">
                月日程获取失败：{monthDetail.message}
              </p>
              <Button variant="outline" size="sm" className="shrink-0" onClick={() => setReloadTick((t) => t + 1)}>
                重试
              </Button>
            </Surface>
          )}
          {viewMode === "month" &&
            monthDetail.phase === "ready" &&
            selectedCodes.length > 0 &&
            monthDetail.total === 0 && (
              <Surface className="mt-3">
                <EmptyState
                  compact
                  icon={CalendarDays}
                  domain="sched"
                  title="本月暂无日程"
                  hint="当前月份没有日程安排，点击日期可查看对应周的明细。"
                />
              </Surface>
            )}

          {/* 最近日程：今天起 30 天，按日分组升序；行点击联动上方详情卡 */}
          {classify.phase === "ready" && selectedCodes.length > 0 && (
            <Surface className="mt-4 px-4 py-4">
              <div className="flex items-baseline justify-between gap-3">
                <p className="text-body font-medium text-text">最近日程</p>
                {upcoming.phase === "ready" && upcoming.data.length > 0 && (
                  <span className="shrink-0 text-caption text-text-2">
                    今天起 {UPCOMING_DAYS} 天 · 共 {upcoming.data.length} 条
                  </span>
                )}
              </div>

              {upcoming.phase === "loading" ? (
                <div aria-hidden className="mt-3 space-y-2">
                  {[0, 1, 2].map((i) => (
                    <div key={i} className="flex items-center gap-3">
                      <span className="h-4 w-12 shrink-0 animate-pulse rounded bg-line" />
                      <span className="h-8 min-w-0 flex-1 animate-pulse rounded bg-line" />
                    </div>
                  ))}
                </div>
              ) : upcoming.phase === "error" ? (
                <div className="mt-3 flex items-center justify-between gap-3">
                  <p className="min-w-0 truncate text-body text-text-2">
                    最近日程获取失败：{upcoming.message}
                  </p>
                  <Button variant="outline" size="sm" className="shrink-0" onClick={() => setReloadTick((t) => t + 1)}>
                    重试
                  </Button>
                </div>
              ) : upcoming.data.length === 0 ? (
                <EmptyState
                  compact
                  icon={CalendarDays}
                  domain="sched"
                  title="最近暂无日程"
                  hint={`未来 ${UPCOMING_DAYS} 天内没有日程安排。`}
                />
              ) : (
                <div className="mt-3 space-y-4">
                  {/* 今天无事件：单独给空态提示，其后仍列出后续日期 */}
                  {!hasUpcomingToday && (
                    <EmptyState
                      compact
                      icon={CalendarDays}
                      domain="sched"
                      title="今日无日程"
                      hint="今天没有日程安排，看看接下来的日期。"
                    />
                  )}
                  {upcomingGroups.map((g) => (
                    <div key={g.dayMs}>
                      <div className="flex items-baseline gap-2">
                        <p
                          className={cn(
                            "text-caption font-medium",
                            g.isToday ? "text-sched" : "text-text-2",
                          )}
                        >
                          {g.label}
                        </p>
                        <p className="tabular-num text-caption text-text-2">{g.dateLabel}</p>
                      </div>
                      <ul className="mt-1.5 space-y-1.5">
                        {g.events.map((ev) => (
                          <li key={ev.id}>
                            <button
                              type="button"
                              aria-pressed={selected?.id === ev.id}
                              onClick={() => setSelected((s) => (s?.id === ev.id ? null : ev))}
                              className={cn(
                                "flex w-full items-center gap-3 rounded-inner border border-line bg-line/40 py-2 pr-3 pl-2.5 text-left transition-shadow duration-[var(--dur-fast)] ease-out-soft hover:shadow-card",
                                selected?.id === ev.id && "ring-1 ring-sched",
                              )}
                              style={{ borderLeft: `3px solid ${ev.color || domainVar.sched}` }}
                            >
                              <span className="min-w-0 flex-1">
                                <span className="block truncate text-body font-medium text-text">
                                  {ev.title}
                                </span>
                                {ev.place && (
                                  <span className="mt-0.5 flex items-center gap-1 text-caption text-text-2">
                                    <MapPin aria-hidden className="size-3 shrink-0" />
                                    <span className="truncate">{ev.place}</span>
                                  </span>
                                )}
                              </span>
                              <span className="tabular-num shrink-0 text-caption text-text-2">
                                {fmtTimeRange(ev.startMs, ev.endMs)}
                              </span>
                              <span className="inline-flex shrink-0 items-center gap-1 rounded-full border border-line bg-surface px-2 py-0.5 text-caption text-text-2">
                                <span
                                  aria-hidden
                                  className="size-1.5 rounded-full"
                                  style={{ backgroundColor: ev.color || domainVar.sched }}
                                />
                                {ev.classifyName || "未分类"}
                              </span>
                            </button>
                          </li>
                        ))}
                      </ul>
                    </div>
                  ))}
                </div>
              )}
            </Surface>
          )}
        </>
      )}
    </section>
  );
}
