import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  CalendarRange,
  ChevronLeft,
  ChevronRight,
  ClipboardPaste,
  Clock,
  Download,
  Pencil,
  Plus,
  RefreshCw,
  Settings,
} from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import { invokeCommand } from "@/shared/tauriApi";
import { cn } from "@/shared/cn";
import type {
  Course,
  CourseOverride,
  ImportResult,
  NoticeCandidate,
  OverrideKind,
  SemesterConfigInput,
  TimeSlot,
  TimetableView,
} from "@/shared/types";

/** 课程色板（8 档，域色系 token：6 个既有域色 + index.css 新增的 2 档扩展）。
 *  ⚠️ 导入课程的 colorIndex 是课名哈希大数（zhengfang::stable_color），
 *  取色必须 % 色板长度；手动课程 colorIndex 为本表下标。 */
const COURSE_PALETTE = [
  "var(--color-brand)",
  "var(--color-sched)",
  "var(--color-todo)",
  "var(--color-wallet)",
  "var(--color-alert)",
  "var(--color-info)",
  "var(--color-aqua)",
  "var(--color-rose)",
] as const;

const KIND_LABEL: Record<OverrideKind, string> = {
  rescheduled: "调课",
  cancelled: "停课",
  extra: "补课",
};

/** 单大节行高（px）；网格行数 = slots.length（作息可编辑后行数不固定为 5）。 */
const ROW_H = 72;

const DAY_NAMES = ["", "周一", "周二", "周三", "周四", "周五", "周六", "周日"];

const courseColor = (c: Course) => COURSE_PALETTE[c.colorIndex % COURSE_PALETTE.length];

/** 小节号（教务 1-based 小节）→ 大节号：ceil(小节/2)，与后端 ICS 展开口径一致。 */
const blockOf = (section: number) => Math.ceil(section / 2);

/** "YYYY-MM-DD" → 本地 Date（避免时区漂移，解析即当日 00:00）。 */
function parseDay(s: string): Date | null {
  const m = s.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!m) return null;
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
}

const fmtDay = (d: Date) => `${d.getMonth() + 1}.${d.getDate()}`;

const dayKeyOf = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;

/** 把日期对齐到「本周（按 firstDayOfWeek 划分）的首日」，不足则回退。
 *  JS 镜像 crates/campus-schedule/src/weeks.rs::previous_or_same_day_of_week——
 *  两处注释互锚，改一处必须同步另一处（契约 §7.4）。 */
function previousOrSame(d: Date, firstDay: number): Date {
  // JS getDay(): 0=周日 … 6=周六，折算为 1=周一 … 7=周日（与 chrono number_from_monday 同口径）
  const cur = ((d.getDay() + 6) % 7) + 1;
  const out = new Date(d);
  out.setDate(out.getDate() - ((cur - firstDay + 7) % 7));
  return out;
}

/** 周次列表 → 紧凑文案："1-16" / "1,3,5" / "1-8,10"。 */
function fmtWeeks(weeks: number[]): string {
  if (weeks.length === 0) return "—";
  const sorted = [...weeks].sort((a, b) => a - b);
  const parts: string[] = [];
  let start = sorted[0];
  let prev = sorted[0];
  for (const w of sorted.slice(1)) {
    if (w === prev + 1) {
      prev = w;
      continue;
    }
    parts.push(start === prev ? `${start}` : `${start}-${prev}`);
    start = w;
    prev = w;
  }
  parts.push(start === prev ? `${start}` : `${start}-${prev}`);
  return parts.join(",");
}

/** 表单周次文本 → 显式周次列表（"1-8,10" / "1、3" 混排）；非法返回 null。 */
function parseWeeksInput(text: string): number[] | null {
  const out = new Set<number>();
  for (const part of text.split(/[,，、\s]+/).filter(Boolean)) {
    const m = part.match(/^(\d+)(?:[-–~](\d+))?$/);
    if (!m) return null;
    const a = Number(m[1]);
    const b = m[2] ? Number(m[2]) : a;
    if (a < 1 || b < a || b > 52) return null;
    for (let w = a; w <= b; w++) out.add(w);
  }
  return out.size > 0 ? [...out].sort((x, y) => x - y) : null;
}

/** override/候选共用摘要（"调课 第5周 · 调至周四 · 3-4节 · D4-305"）。 */
type SummaryInput = {
  weeks: number[];
  changeType: OverrideKind;
  newDay?: number | null;
  newStartSection?: number | null;
  newEndSection?: number | null;
  newPosition?: string | null;
};

function overrideSummary(o: SummaryInput): string {
  const parts = [`第${fmtWeeks(o.weeks)}周`];
  if (o.changeType !== "cancelled") {
    const day = o.newDay ?? null;
    if (day != null) parts.push(`调至${DAY_NAMES[day] ?? `周${day}`}`);
    if (o.newStartSection != null && o.newEndSection != null)
      parts.push(`${o.newStartSection}-${o.newEndSection}节`);
    if (o.newPosition) parts.push(o.newPosition);
  } else {
    const day = o.newDay ?? null;
    if (day != null) parts.push(DAY_NAMES[day] ?? `周${day}`);
  }
  return parts.join(" · ");
}

// ---------------- 周视图块构建（对齐后端 grid::merge_courses 的「同日重叠分列」语义，
// 但在纯前端重写：教务课表天然无同时段冲突，重叠主要来自手动课程/补课叠加） ----------------

interface PlacedBlock {
  key: string;
  course: Course;
  /** 该块关联的 override（显示【调】与详情信息）；普通块/占位为 null */
  override: CourseOverride | null;
  day: number;
  startBlock: number;
  endBlock: number;
  room: string;
  /** ghost = 非实体块：moved-out（已调走）/ cancelled（已停） */
  ghost: null | "moved-out" | "cancelled";
}

/** 逆序取最后一条匹配（后采纳的通知覆盖先采纳的，upsert 语义与之呼应）。 */
function lastOverride(
  overrides: CourseOverride[],
  pred: (o: CourseOverride) => boolean,
): CourseOverride | null {
  for (let i = overrides.length - 1; i >= 0; i--) if (pred(overrides[i])) return overrides[i];
  return null;
}

function buildWeekBlocks(
  view: TimetableView,
  week: number,
): { blocks: PlacedBlock[]; columns: PlacedBlock[][] } {
  const { courses, overrides } = view.timetable;
  const blocks: PlacedBlock[] = [];
  const byId = new Map(courses.map((c) => [c.id, c]));

  for (const course of courses) {
    if (course.disabled || !course.weeks.includes(week)) continue;
    if (course.startSection == null || course.endSection == null) continue;
    const startBlock = blockOf(course.startSection);
    const endBlock = blockOf(course.endSection);
    const resched = lastOverride(
      overrides,
      (o) => o.courseId === course.id && o.changeType === "rescheduled" && o.weeks.includes(week),
    );
    const cancel = lastOverride(
      overrides,
      (o) => o.courseId === course.id && o.changeType === "cancelled" && o.weeks.includes(week),
    );
    const mk = (
      day: number,
      s: number,
      e: number,
      room: string,
      override: CourseOverride | null,
      ghost: PlacedBlock["ghost"],
    ): PlacedBlock => ({
      key: `${course.id}-${week}-${day}-${s}-${ghost ?? "solid"}`,
      course,
      override,
      day,
      startBlock: s,
      endBlock: e,
      room,
      ghost,
    });

    // 停课优先（冻结契约 §2.5.1 两档）：newDay 有值 = 只停「该周 · 星期 newDay」
    // 那一次——该课当天有排课才渲染虚线「已停」占位，本周其他星期的同课不受影响
    // （同课另一天的记录由挂在其 courseId 上的 override 单独处理）；
    // newDay = null = 通知未提星期 → 该课在 weeks 列出的周次内整周全停，
    // 该周该课所有原时段渲染虚线「已停」。
    if (cancel && (cancel.newDay == null || cancel.newDay === course.day)) {
      blocks.push(mk(course.day, startBlock, endBlock, course.position, cancel, "cancelled"));
      continue;
    }
    // 调课且新时间 ≠ 原时间 → 原时段虚线占位 + 新时段实体块
    if (
      resched &&
      resched.newDay != null &&
      resched.newStartSection != null &&
      (resched.newDay !== course.day || blockOf(resched.newStartSection) !== startBlock)
    ) {
      blocks.push(mk(course.day, startBlock, endBlock, course.position, resched, "moved-out"));
      const newStart = blockOf(resched.newStartSection);
      const newEnd =
        resched.newEndSection != null
          ? blockOf(resched.newEndSection)
          : resched.newStartSection; // 单节补调：结束=起始
      blocks.push(mk(resched.newDay, newStart, newEnd, resched.newPosition ?? course.position, resched, null));
      continue;
    }
    // 原地（可能仅换教室）
    blocks.push(
      mk(course.day, startBlock, endBlock, resched?.newPosition ?? course.position, resched, null),
    );
  }

  // 补课叠加：新时段新增实体块（课程删除时后端级联清理 override，正常必命中）
  for (const ov of overrides) {
    if (ov.changeType !== "extra" || !ov.weeks.includes(week)) continue;
    const course = byId.get(ov.courseId);
    if (!course || course.disabled) continue;
    const day = ov.newDay ?? course.day;
    const s = ov.newStartSection != null ? blockOf(ov.newStartSection) : 1;
    const e =
      ov.newEndSection != null
        ? blockOf(ov.newEndSection)
        : ov.newStartSection != null
          ? blockOf(ov.newStartSection)
          : 1;
    blocks.push({
      key: `${ov.id}-${week}`,
      course,
      override: ov,
      day,
      startBlock: s,
      endBlock: e,
      room: ov.newPosition ?? course.position,
      ghost: null,
    });
  }

  // 同日重叠分列：按开始大节排序 → 连通簇 → 簇内贪心占道
  const columns: PlacedBlock[][] = Array.from({ length: 7 }, () => []);
  for (const b of blocks) columns[b.day - 1].push(b);
  for (const col of columns) {
    col.sort((a, b) => a.startBlock - b.startBlock || a.endBlock - b.endBlock);
  }
  return { blocks, columns };
}

/** 簇内分列结果：block → (lane, lanes)。 */
function layoutColumn(col: PlacedBlock[]): Map<string, { lane: number; lanes: number }> {
  const out = new Map<string, { lane: number; lanes: number }>();
  let i = 0;
  while (i < col.length) {
    // 连通簇：下一个块开始于当前簇最晚结束之前（同一大节内相邻视为重叠）
    let endMax = col[i].endBlock;
    let j = i + 1;
    while (j < col.length && col[j].startBlock <= endMax) {
      endMax = Math.max(endMax, col[j].endBlock);
      j++;
    }
    const cluster = col.slice(i, j);
    const laneEnds: number[] = [];
    const laneOf = new Map<string, number>();
    for (const b of cluster) {
      let lane = laneEnds.findIndex((e) => e < b.startBlock);
      if (lane === -1) {
        laneEnds.push(b.endBlock);
        lane = laneEnds.length - 1;
      } else {
        laneEnds[lane] = b.endBlock;
      }
      laneOf.set(b.key, lane);
    }
    for (const b of cluster) out.set(b.key, { lane: laneOf.get(b.key) ?? 0, lanes: laneEnds.length });
    i = j;
  }
  return out;
}

// ---------------- 手动添加 / 编辑表单 ----------------

const DAY_OPTIONS = DAY_NAMES.slice(1);

function CourseForm({
  initial,
  editing,
  totalWeeks,
  busy,
  error,
  onSubmit,
  onCancel,
}: {
  initial: Partial<Course>;
  editing: boolean;
  totalWeeks: number;
  busy: boolean;
  error: string | null;
  onSubmit: (payload: {
    name: string;
    teacher: string;
    position: string;
    day: number;
    startSection: number;
    endSection: number;
    weeks: number[];
    colorIndex: number;
    remark: string | null;
  }) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(initial.name ?? "");
  const [teacher, setTeacher] = useState(initial.teacher ?? "");
  const [position, setPosition] = useState(initial.position ?? "");
  const [day, setDay] = useState(initial.day ?? 1);
  const [startSection, setStartSection] = useState(initial.startSection ?? 1);
  const [endSection, setEndSection] = useState(initial.endSection ?? 2);
  const [weeksText, setWeeksText] = useState(
    initial.weeks?.length ? fmtWeeks(initial.weeks) : `1-${totalWeeks}`,
  );
  const [colorIndex, setColorIndex] = useState(
    typeof initial.colorIndex === "number" ? initial.colorIndex % COURSE_PALETTE.length : 0,
  );
  const [remark, setRemark] = useState(initial.remark ?? "");
  const [localErr, setLocalErr] = useState<string | null>(null);

  const quickWeeks = (kind: "all" | "odd" | "even") => {
    const ws: number[] = [];
    for (let w = 1; w <= totalWeeks; w++) {
      if (kind === "odd" && w % 2 === 0) continue;
      if (kind === "even" && w % 2 === 1) continue;
      ws.push(w);
    }
    setWeeksText(fmtWeeks(ws));
  };

  const submit = () => {
    const weeks = parseWeeksInput(weeksText);
    if (!name.trim()) return setLocalErr("课程名不能为空");
    if (!weeks) return setLocalErr("周次格式无法识别，示例：1-16 或 1,3,5-8");
    setLocalErr(null);
    onSubmit({
      name: name.trim(),
      teacher: teacher.trim(),
      position: position.trim(),
      day,
      startSection,
      endSection: Math.max(endSection, startSection),
      weeks,
      colorIndex,
      remark: remark.trim() || null,
    });
  };

  const field = "grid gap-1.5";
  const label = "text-caption text-text-2";

  return (
    <Surface accent="sched" className="mt-4 px-4 py-4">
      <div className="mb-3 flex items-center justify-between">
        <p className="text-body font-semibold text-text">
          {editing ? "编辑课程" : "手动添加课程"}
        </p>
        {editing && (
          <p className="text-caption text-text-2">
            导入课程的修改会在下次导入时被教务数据覆盖
          </p>
        )}
      </div>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        <div className={field}>
          <label className={label} htmlFor="tf-name">课程名 *</label>
          <Input id="tf-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="如：信息安全" />
        </div>
        <div className={field}>
          <label className={label} htmlFor="tf-teacher">教师</label>
          <Input id="tf-teacher" value={teacher} onChange={(e) => setTeacher(e.target.value)} />
        </div>
        <div className={field}>
          <label className={label} htmlFor="tf-position">教室</label>
          <Input id="tf-position" value={position} onChange={(e) => setPosition(e.target.value)} placeholder="如：D4-207" />
        </div>
        <div className={field}>
          <label className={label} htmlFor="tf-day">星期</label>
          <select
            id="tf-day"
            value={day}
            onChange={(e) => setDay(Number(e.target.value))}
            className="h-9 rounded-control border border-line bg-surface px-2 text-body text-text"
          >
            {DAY_OPTIONS.map((d, i) => (
              <option key={d} value={i + 1}>{d}</option>
            ))}
          </select>
        </div>
        <div className={field}>
          <label className={label} htmlFor="tf-start">起始小节</label>
          <select
            id="tf-start"
            value={startSection}
            onChange={(e) => setStartSection(Number(e.target.value))}
            className="h-9 rounded-control border border-line bg-surface px-2 text-body text-text"
          >
            {Array.from({ length: 12 }, (_, i) => i + 1).map((n) => (
              <option key={n} value={n}>第 {n} 小节</option>
            ))}
          </select>
        </div>
        <div className={field}>
          <label className={label} htmlFor="tf-end">结束小节（含）</label>
          <select
            id="tf-end"
            value={Math.max(endSection, startSection)}
            onChange={(e) => setEndSection(Number(e.target.value))}
            className="h-9 rounded-control border border-line bg-surface px-2 text-body text-text"
          >
            {Array.from({ length: 12 }, (_, i) => i + 1)
              .filter((n) => n >= startSection)
              .map((n) => (
                <option key={n} value={n}>第 {n} 小节</option>
              ))}
          </select>
        </div>
        <div className={cn(field, "sm:col-span-2")}>
          <label className={label} htmlFor="tf-weeks">周次（示例 1-16 或 1,3,5-8）</label>
          <div className="flex items-center gap-1.5">
            <Input
              id="tf-weeks"
              value={weeksText}
              onChange={(e) => setWeeksText(e.target.value)}
              className="flex-1"
            />
            <Button type="button" variant="outline" size="sm" onClick={() => quickWeeks("all")}>
              全部
            </Button>
            <Button type="button" variant="outline" size="sm" onClick={() => quickWeeks("odd")}>
              单周
            </Button>
            <Button type="button" variant="outline" size="sm" onClick={() => quickWeeks("even")}>
              双周
            </Button>
          </div>
        </div>
        <div className={field}>
          <span className={label}>颜色</span>
          <div className="flex flex-wrap items-center gap-1.5" role="radiogroup" aria-label="课程颜色">
            {COURSE_PALETTE.map((color, i) => (
              <button
                key={color}
                type="button"
                role="radio"
                aria-checked={colorIndex === i}
                aria-label={`颜色 ${i + 1}`}
                onClick={() => setColorIndex(i)}
                className={cn(
                  "size-6 rounded-full transition-shadow duration-[var(--dur-fast)] ease-out-soft",
                  colorIndex === i && "ring-2 ring-ring ring-offset-2",
                )}
                style={{ backgroundColor: color }}
              />
            ))}
          </div>
        </div>
        <div className={cn(field, "sm:col-span-2 lg:col-span-3")}>
          <label className={label} htmlFor="tf-remark">
            {editing && initial.source === "import" ? "性质 · 考核方式" : "备注"}
          </label>
          <Input id="tf-remark" value={remark} onChange={(e) => setRemark(e.target.value)} />
        </div>
      </div>
      {(localErr ?? error) && (
        <p className="mt-3 text-caption text-alert" role="alert">
          {localErr ?? error}
        </p>
      )}
      <div className="mt-3 flex items-center gap-2">
        <Button onClick={submit} disabled={busy}>
          {busy ? "保存中…" : editing ? "保存修改" : "添加课程"}
        </Button>
        <Button variant="outline" onClick={onCancel} disabled={busy}>
          取消
        </Button>
      </div>
    </Surface>
  );
}

// ---------------- 作息时间表编辑（冻结契约 §2.3 save_time_slots，M2.5 收尾轮） ----------------

/** "HH:MM" + 分钟 → "HH:MM"（不跨 24:00，溢出截到 23:59）。 */
function addMinutes(hm: string, minutes: number): string {
  const [h, m] = hm.split(":").map(Number);
  const total = Math.min((h || 0) * 60 + (m || 0) + minutes, 23 * 60 + 59);
  return `${String(Math.floor(total / 60)).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`;
}

/** 编辑态作息行：大节号由保存时的行序生成（1..n，天然严格递增），不手填。 */
interface SlotRow {
  startTime: string;
  endTime: string;
  alias: string | null;
}

function SlotsEditor({
  initial,
  usingCustom,
  busy,
  error,
  onSave,
  onReset,
  onClose,
}: {
  /** 当前生效作息（自定义或内置默认），打开时快照 */
  initial: TimeSlot[];
  usingCustom: boolean;
  busy: boolean;
  error: string | null;
  onSave: (slots: TimeSlot[]) => void;
  onReset: () => void;
  onClose: () => void;
}) {
  const [rows, setRows] = useState<SlotRow[]>(
    initial.map((s) => ({ startTime: s.startTime, endTime: s.endTime, alias: s.alias })),
  );
  const [localErr, setLocalErr] = useState<string | null>(null);

  // Esc 关闭（busy 时忽略）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  const update = (i: number, patch: Partial<SlotRow>) =>
    setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));

  const addRow = () => {
    const last = rows[rows.length - 1];
    const start = last ? last.endTime : "08:00";
    setRows((rs) => [...rs, { startTime: start, endTime: addMinutes(start, 100), alias: null }]);
  };

  const submit = () => {
    if (rows.length === 0) return setLocalErr("作息至少需要一条");
    for (const r of rows) {
      if (!r.startTime || !r.endTime) return setLocalErr("每行都需要开始与结束时间");
      if (r.endTime <= r.startTime) return setLocalErr("结束时间必须晚于开始时间");
    }
    setLocalErr(null);
    onSave(rows.map((r, i) => ({ number: i + 1, startTime: r.startTime, endTime: r.endTime, alias: r.alias })));
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && !busy) onClose();
      }}
    >
      <div
        role="dialog"
        aria-label="作息时间表"
        className="w-full max-w-md rounded-card border border-line bg-surface p-4 shadow-pop"
      >
        <div className="flex items-center justify-between">
          <p className="text-body font-semibold text-text">作息时间表</p>
          <span className="rounded bg-line px-1.5 text-caption text-text-2">
            {usingCustom ? "自定义" : "本校默认"}
          </span>
        </div>
        <p className="mt-1 text-caption text-text-2">
          按行即大节（第 1 行 = 第 1 大节），课表网格与 ICS 导出都会按此展开。
        </p>

        <div className="mt-3 max-h-72 space-y-2 overflow-y-auto pr-1">
          {rows.map((r, i) => (
            <div key={i} className="flex items-center gap-2">
              <span className="tabular-num w-14 shrink-0 text-caption font-medium text-text-2">
                第 {i + 1} 大节
              </span>
              <input
                type="time"
                value={r.startTime}
                onChange={(e) => update(i, { startTime: e.target.value })}
                aria-label={`第 ${i + 1} 大节开始时间`}
                disabled={busy}
                className="tabular-num h-9 flex-1 rounded-control border border-line bg-surface px-2 text-body text-text disabled:opacity-50"
              />
              <span aria-hidden className="text-caption text-text-2">–</span>
              <input
                type="time"
                value={r.endTime}
                onChange={(e) => update(i, { endTime: e.target.value })}
                aria-label={`第 ${i + 1} 大节结束时间`}
                disabled={busy}
                className="tabular-num h-9 flex-1 rounded-control border border-line bg-surface px-2 text-body text-text disabled:opacity-50"
              />
              <button
                type="button"
                aria-label={`删除第 ${i + 1} 大节`}
                disabled={busy}
                onClick={() => setRows((rs) => rs.filter((_, j) => j !== i))}
                className="shrink-0 rounded px-1.5 text-caption text-alert hover:underline disabled:opacity-50"
              >
                删除
              </button>
            </div>
          ))}
          {rows.length === 0 && (
            <p className="py-3 text-center text-caption text-text-2">暂无作息行，点击下方新增。</p>
          )}
        </div>

        <button
          type="button"
          disabled={busy}
          onClick={addRow}
          className="mt-2 flex items-center gap-1 text-caption text-sched hover:underline disabled:opacity-50"
        >
          <Plus aria-hidden="true" className="size-3.5" />
          新增大节
        </button>

        {(localErr ?? error) && (
          <p className="mt-2 text-caption text-alert" role="alert">
            {localErr ?? error}
          </p>
        )}

        <div className="mt-3 flex items-center gap-2 border-t border-line pt-3">
          <Button variant="ghost" size="sm" disabled={busy || !usingCustom} onClick={onReset} title={usingCustom ? "清空自定义作息，恢复内置校本大节表" : "当前已是内置默认"}>
            恢复本校默认
          </Button>
          <span className="flex-1" />
          <Button variant="outline" size="sm" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button size="sm" disabled={busy} onClick={submit}>
            {busy ? "保存中…" : "保存"}
          </Button>
        </div>
      </div>
    </div>
  );
}

// ---------------- 课表设置弹层（契约 §7.1 save_semester_config，2026-09-19 批 1） ----------------

function SettingsEditor({
  initial,
  busy,
  error,
  onSave,
  onClose,
}: {
  initial: {
    semesterStartDate: string | null;
    semesterTotalWeeks: number;
    firstDayOfWeek: number;
    showWeekends: boolean;
  };
  busy: boolean;
  error: string | null;
  onSave: (input: SemesterConfigInput) => void;
  onClose: () => void;
}) {
  const [startDate, setStartDate] = useState(initial.semesterStartDate ?? "");
  const [totalWeeks, setTotalWeeks] = useState(String(initial.semesterTotalWeeks));
  /** 空 = 不设置；保存时后端按此反推开学日（覆盖上方开学日） */
  const [weekHint, setWeekHint] = useState("");
  const [firstDay, setFirstDay] = useState(initial.firstDayOfWeek);
  const [showWeekends, setShowWeekends] = useState(initial.showWeekends);
  const [localErr, setLocalErr] = useState<string | null>(null);

  // Esc 关闭（busy 时忽略）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  const submit = () => {
    if (totalWeeks.trim() === "" || !Number.isInteger(Number(totalWeeks)))
      return setLocalErr("请填写学期总周数（1-30）");
    const hint = weekHint.trim() === "" ? null : Number(weekHint);
    if (hint !== null && !Number.isInteger(hint))
      return setLocalErr("「今天是第几周」需为正整数");
    setLocalErr(null);
    onSave({
      semesterStartDate: startDate || null, // 清空开学日 = 假期态（契约 §7.1）
      semesterTotalWeeks: Number(totalWeeks),
      firstDayOfWeek: firstDay,
      showWeekends,
      currentWeekHint: hint,
    });
  };

  const field = "grid gap-1.5";
  const label = "text-caption text-text-2";
  const inputCls =
    "tabular-num h-9 rounded-control border border-line bg-surface px-2 text-body text-text disabled:opacity-50";

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && !busy) onClose();
      }}
    >
      <div
        role="dialog"
        aria-label="课表设置"
        className="w-full max-w-md rounded-card border border-line bg-surface p-4 shadow-pop"
      >
        <p className="text-body font-semibold text-text">课表设置</p>
        <p className="mt-1 text-caption text-text-2">
          学期锚点决定周次与日期列；周首日与周末列的联动由后端保存时统一处理。
        </p>

        <div className="mt-3 grid gap-3">
          <div className={field}>
            <label className={label} htmlFor="ts-start">学期开学日</label>
            <div className="flex items-center gap-2">
              <input
                id="ts-start"
                type="date"
                value={startDate}
                onChange={(e) => setStartDate(e.target.value)}
                disabled={busy}
                className={cn(inputCls, "flex-1")}
              />
              {startDate && (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => setStartDate("")}
                  className="shrink-0 text-caption text-alert hover:underline disabled:opacity-50"
                >
                  清空
                </button>
              )}
            </div>
            <p className="text-caption text-text-2">清空开学日即回到假期态（不显示周次）。</p>
          </div>
          <div className={field}>
            <label className={label} htmlFor="ts-weeks">学期总周数（1-30）</label>
            <input
              id="ts-weeks"
              type="number"
              min={1}
              max={30}
              value={totalWeeks}
              onChange={(e) => setTotalWeeks(e.target.value)}
              disabled={busy}
              className={inputCls}
            />
          </div>
          <div className={field}>
            <label className={label} htmlFor="ts-hint">今天是第几周（选填）</label>
            <input
              id="ts-hint"
              type="number"
              min={1}
              max={30}
              value={weekHint}
              placeholder="填写后保存时自动反推开学日"
              onChange={(e) => setWeekHint(e.target.value)}
              disabled={busy}
              className={inputCls}
            />
          </div>
          <div className={field}>
            <label className={label} htmlFor="ts-firstday">每周起始日</label>
            <select
              id="ts-firstday"
              value={firstDay}
              onChange={(e) => setFirstDay(Number(e.target.value))}
              disabled={busy}
              className={inputCls}
            >
              <option value={1}>周一</option>
              <option value={7}>周日</option>
            </select>
          </div>
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              checked={showWeekends}
              onChange={(e) => setShowWeekends(e.target.checked)}
              disabled={busy}
              className="size-4 accent-sched"
            />
            <span className="text-body text-text">显示周末列</span>
          </label>
          {/* 联动提示（契约 §7.2：前端只提示、不禁用，实际联动由后端保存时收口） */}
          {firstDay === 7 && (
            <p className="text-caption text-text-2">每周起始日为周日时，将始终显示周末列。</p>
          )}
          {!showWeekends && firstDay !== 1 && firstDay !== 7 && (
            <p className="text-caption text-text-2">隐藏周末后，每周起始日将被重置为周一。</p>
          )}
        </div>

        {(localErr ?? error) && (
          <p className="mt-2 text-caption text-alert" role="alert">
            {localErr ?? error}
          </p>
        )}

        <div className="mt-3 flex items-center justify-end gap-2 border-t border-line pt-3">
          <Button variant="outline" size="sm" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button size="sm" disabled={busy} onClick={submit}>
            {busy ? "保存中…" : "保存"}
          </Button>
        </div>
      </div>
    </div>
  );
}

// ---------------- 主面板 ----------------

/** 面板四态。 */
type TtState =
  | { phase: "loading" }
  | { phase: "ready"; data: TimetableView }
  | { phase: "error"; message: string };

interface DetailPos {
  course: Course;
  top: number;
  left: number;
}

export function TimetablePanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const authed = status === "authed";

  const [view, setView] = useState<TtState>({ phase: "loading" });
  const [reloadTick, setReloadTick] = useState(0);
  /** null = 跟随 currentWeek（currentWeek 也为 null 时按第 1 周展示） */
  const [viewWeek, setViewWeek] = useState<number | null>(null);

  const [importing, setImporting] = useState(false);
  const [importMsg, setImportMsg] = useState<
    { ok: boolean; text: string; result?: ImportResult } | null
  >(null);

  const [icsBusy, setIcsBusy] = useState(false);
  const [icsMsg, setIcsMsg] = useState<string | null>(null);

  const [noticeText, setNoticeText] = useState("");
  const [parsing, setParsing] = useState(false);
  const [candidates, setCandidates] = useState<NoticeCandidate[] | null>(null);
  const [noticeMsg, setNoticeMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);

  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<Course | null>(null);
  const [formInitial, setFormInitial] = useState<Partial<Course>>({});
  const [formError, setFormError] = useState<string | null>(null);

  const [slotsOpen, setSlotsOpen] = useState(false);
  const [slotsBusy, setSlotsBusy] = useState(false);
  const [slotsErr, setSlotsErr] = useState<string | null>(null);

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsBusy, setSettingsBusy] = useState(false);
  const [settingsErr, setSettingsErr] = useState<string | null>(null);

  const [detail, setDetail] = useState<DetailPos | null>(null);
  const blockRefs = useRef(new Map<string, HTMLElement>());

  const reload = useCallback(() => {
    setView({ phase: "loading" });
    invokeCommand<TimetableView>("get_timetable").then((r) => {
      if (r.success && r.data) {
        setView({ phase: "ready", data: r.data });
      } else {
        setView({ phase: "error", message: r.message ?? "课表读取失败" });
      }
    });
  }, []);

  useEffect(() => {
    if (!authed) return;
    reload();
  }, [authed, reloadTick, reload]);

  // 详情浮层：Esc / 点击浮层与课程块以外区域关闭
  useEffect(() => {
    if (!detail) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setDetail(null);
    };
    const onDown = (e: MouseEvent) => {
      const t = e.target as HTMLElement;
      if (!t.closest("[data-course-detail]") && !t.closest("[data-course-block]")) setDetail(null);
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
    };
  }, [detail]);

  const ready = view.phase === "ready" ? view.data : null;
  const tt = ready?.timetable ?? null;
  const slots = ready?.slots ?? [];
  const currentWeek = ready?.currentWeek ?? null;
  /** 展示周：用户切换 > 后端当前周 > 第 1 周 */
  const week = viewWeek ?? currentWeek ?? 1;
  const totalWeeks = Math.max(tt?.config.semesterTotalWeeks ?? 20, currentWeek ?? 1);

  // 周首日与显示列（契约 §7）：列头从 firstDay 起旋转；不显示周末时裁成 5 列。
  // buildWeekBlocks 内部恒按 7 天计算，裁剪只发生在渲染层。
  const firstDay = tt?.config.firstDayOfWeek ?? 1;
  const showWeekends = tt?.config.showWeekends ?? false;
  const displayDays = showWeekends ? 7 : 5;
  /** 显示列下标（0-based）→ 实际星期（1=周一 … 7=周日）。 */
  const displayDayOf = (i: number) => ((firstDay - 1 + i) % 7) + 1;

  const weekBlocks = useMemo(
    () => (ready ? buildWeekBlocks(ready, week) : { blocks: [], columns: Array.from({ length: 7 }, () => []) }),
    [ready, week],
  );
  const layouts = useMemo(
    () => weekBlocks.columns.map((col) => layoutColumn(col)),
    [weekBlocks],
  );

  /** 视图周各显示列日期（契约 §7）：周首 = previousOrSame(开学日, firstDay)，
   *  第 i 列 = 周首 + (week-1)×7 + i；未设置开学日为 null，列头只显示星期。
   *  ⚠️ todayCol 必须对「显示列的 weekDates」findIndex——旋转后按 day 数学映射必错。 */
  const weekDates: (Date | null)[] = useMemo(() => {
    const start = tt?.config.semesterStartDate ? parseDay(tt.config.semesterStartDate) : null;
    if (!start) return Array.from({ length: displayDays }, () => null);
    const weekFirst = previousOrSame(start, firstDay);
    return Array.from({ length: displayDays }, (_, i) => {
      const d = new Date(weekFirst);
      d.setDate(d.getDate() + (week - 1) * 7 + i);
      return d;
    });
  }, [tt?.config.semesterStartDate, week, firstDay, displayDays]);
  const todayCol = weekDates.findIndex((d) => d && dayKeyOf(d) === ready?.today);

  const openBlockDetail = (block: PlacedBlock) => {
    const el = blockRefs.current.get(block.key);
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const W = 288;
    const left = Math.min(Math.max(rect.left, 12), window.innerWidth - W - 12);
    // 下方放不下则上翻（浮层 max-h 320 估高）
    const top = rect.bottom + 8 > window.innerHeight - 200 ? Math.max(12, rect.top - 328) : rect.bottom + 8;
    setDetail((d) =>
      d?.course.id === block.course.id && d.top === top && d.left === left
        ? null
        : { course: block.course, top, left },
    );
  };

  // ---------------- 动作 ----------------

  const doImport = async () => {
    setImporting(true);
    setImportMsg(null);
    const r = await invokeCommand<ImportResult>("import_timetable");
    setImporting(false);
    if (r.success && r.data) {
      setImportMsg({
        ok: true,
        text: `导入完成：新增 ${r.data.added} · 更新 ${r.data.changed} · 停开 ${r.data.removed} · 共 ${r.data.total} 门`,
        result: r.data,
      });
      setViewWeek(null); // 回到当前教学周（周次锚点可能刚被导入刷新）
      setReloadTick((t) => t + 1);
    } else {
      setImportMsg({ ok: false, text: r.message ?? "导入失败" });
    }
  };

  const exportIcs = async () => {
    setIcsBusy(true);
    setIcsMsg(null);
    // WebView2 不处理下载（DownloadStarting 未接管），前端 Blob/a[download] 不可用，
    // 交付由后端 export_ics 直接写入下载目录，这里只展示结果。
    const r = await invokeCommand<string>("export_ics");
    setIcsBusy(false);
    if (r.success && typeof r.data === "string") {
      setIcsMsg(`已导出到 ${r.data}`);
    } else {
      setIcsMsg(r.message ?? "导出失败");
    }
  };

  const parseNotice = async () => {
    if (!noticeText.trim()) return;
    setParsing(true);
    setNoticeMsg(null);
    const r = await invokeCommand<NoticeCandidate[]>("parse_notice", { text: noticeText });
    setParsing(false);
    if (r.success && r.data) {
      setCandidates(r.data);
      if (r.data.length === 0) setNoticeMsg({ ok: false, text: "未从该文本中识别出调课/停课/补课信息" });
    } else {
      setNoticeMsg({ ok: false, text: r.message ?? "解析失败" });
    }
  };

  const adoptCandidate = async (c: NoticeCandidate) => {
    setBusyKey(`adopt-${c.noticeId}-${c.courseName}`);
    setNoticeMsg(null);
    const r = await invokeCommand<CourseOverride>("apply_override", { candidate: c });
    setBusyKey(null);
    if (r.success) {
      setCandidates((cs) => (cs ? cs.filter((x) => x !== c) : cs));
      setNoticeMsg({ ok: true, text: `已采纳：${c.courseName} ${KIND_LABEL[c.changeType]}` });
      setReloadTick((t) => t + 1);
    } else {
      setNoticeMsg({ ok: false, text: r.message ?? "采纳失败" });
    }
  };

  const revokeOverride = async (o: CourseOverride) => {
    setBusyKey(`revoke-${o.id}`);
    setNoticeMsg(null);
    const r = await invokeCommand<number>("revoke_notice", { noticeId: o.sourceNoticeId });
    setBusyKey(null);
    if (r.success) {
      setNoticeMsg({ ok: true, text: `已撤销该通知产生的 ${r.data ?? 0} 条调整` });
      setReloadTick((t) => t + 1);
    } else {
      setNoticeMsg({ ok: false, text: r.message ?? "撤销失败" });
    }
  };

  const submitCourse = async (payload: {
    name: string;
    teacher: string;
    position: string;
    day: number;
    startSection: number;
    endSection: number;
    weeks: number[];
    colorIndex: number;
    remark: string | null;
  }) => {
    setFormError(null);
    setBusyKey(null);
    if (editing) {
      const updated: Course = {
        ...editing,
        ...payload,
        isCustomTime: false,
      };
      const r = await invokeCommand<Course>("update_course", { course: updated });
      if (r.success) {
        closeForm();
        setReloadTick((t) => t + 1);
      } else {
        setFormError(r.message ?? "保存失败");
      }
    } else {
      const r = await invokeCommand<Course>("add_course_manual", { input: payload });
      if (r.success) {
        closeForm();
        setReloadTick((t) => t + 1);
      } else {
        setFormError(r.message ?? "添加失败");
      }
    }
  };

  const closeForm = () => {
    setFormOpen(false);
    setEditing(null);
    setFormInitial({});
    setFormError(null);
  };

  /** 手动添加（可预填「同款」）；表单组件内部 state 由 key 重挂重置。 */
  const openForm = (initial: Partial<Course>, editTarget: Course | null) => {
    setFormInitial(initial);
    setEditing(editTarget);
    setFormOpen(true);
    setDetail(null);
  };

  const removeCourse = async (c: Course) => {
    if (!window.confirm(`删除课程「${c.name}」？其挂载的调课调整将一并清除。`)) return;
    setBusyKey(`del-${c.id}`);
    const r = await invokeCommand("delete_course", { id: c.id });
    setBusyKey(null);
    if (r.success) {
      if (detail?.course.id === c.id) setDetail(null);
      setReloadTick((t) => t + 1);
    } else {
      setNoticeMsg({ ok: false, text: r.message ?? "删除失败" });
    }
  };

  // ---------------- 作息保存 / 恢复默认（命令返回刷新后的 TimetableView，免二次拉取） ----------------

  const saveSlots = async (slots: TimeSlot[]) => {
    setSlotsBusy(true);
    setSlotsErr(null);
    const r = await invokeCommand<TimetableView>("save_time_slots", { slots });
    setSlotsBusy(false);
    if (r.success && r.data) {
      setView({ phase: "ready", data: r.data });
      setSlotsOpen(false);
    } else {
      setSlotsErr(r.message ?? "保存失败");
    }
  };

  const resetSlots = async () => {
    setSlotsBusy(true);
    setSlotsErr(null);
    const r = await invokeCommand<TimetableView>("save_time_slots", { slots: null });
    setSlotsBusy(false);
    if (r.success && r.data) {
      setView({ phase: "ready", data: r.data });
      setSlotsOpen(false);
    } else {
      setSlotsErr(r.message ?? "恢复失败");
    }
  };

  // ---------------- 学期设置保存（契约 §7.1：命令返回刷新后的 TimetableView） ----------------

  const saveSemesterConfig = async (input: SemesterConfigInput) => {
    setSettingsBusy(true);
    setSettingsErr(null);
    const r = await invokeCommand<TimetableView>("save_semester_config", { input });
    setSettingsBusy(false);
    if (r.success && r.data) {
      setView({ phase: "ready", data: r.data });
      setSettingsOpen(false);
    } else {
      setSettingsErr(r.message ?? "保存失败");
    }
  };

  // ---------------- 渲染 ----------------

  const weekSwitcher = ready && (
    <div className="flex flex-wrap items-center justify-end gap-1.5">
      <span className="tabular-num mr-1 text-body font-medium text-text-2">
        第 {week} 周 / 共 {totalWeeks} 周
      </span>
      <Button
        variant="outline"
        size="icon-sm"
        aria-label="上一周"
        disabled={week <= 1}
        onClick={() => setViewWeek(Math.max(1, week - 1))}
      >
        <ChevronLeft aria-hidden="true" />
      </Button>
      <Button
        variant={viewWeek !== null && viewWeek === currentWeek ? "default" : "outline"}
        size="sm"
        disabled={currentWeek === null}
        aria-label="回到本周"
        onClick={() => setViewWeek(null)}
      >
        本周
      </Button>
      <Button
        variant="outline"
        size="icon-sm"
        aria-label="下一周"
        disabled={week >= totalWeeks}
        onClick={() => setViewWeek(Math.min(totalWeeks, week + 1))}
      >
        <ChevronRight aria-hidden="true" />
      </Button>
      <span aria-hidden className="mx-1 h-5 w-px bg-line" />
      <Button variant="outline" size="sm" disabled={importing} onClick={doImport}>
        <RefreshCw aria-hidden="true" className={cn("size-3.5", importing && "animate-spin")} />
        {importing ? "同步中…" : "导入 / 同步"}
      </Button>
      <Button variant="outline" size="sm" disabled={icsBusy} onClick={exportIcs}>
        <Download aria-hidden="true" className="size-3.5" />
        导出 ICS
      </Button>
      <Button variant="outline" size="sm" onClick={() => { setSlotsErr(null); setSlotsOpen(true); }}>
        <Clock aria-hidden="true" className="size-3.5" />
        作息
      </Button>
      <Button variant="outline" size="sm" onClick={() => { setSettingsErr(null); setSettingsOpen(true); }}>
        <Settings aria-hidden="true" className="size-3.5" />
        设置
      </Button>
    </div>
  );

  return (
    <section className="mx-auto mt-8 max-w-5xl px-4">
      <PanelHeader
        title="课表"
        description="教务导入 · 调课通知 · 手动课程 · ICS 导出"
        domain="sched"
        actions={authed ? weekSwitcher : undefined}
      />

      {!authed ? (
        <Surface>
          <EmptyState
            icon={CalendarRange}
            domain="sched"
            title="登录后同步课表"
            hint="登录后可从教务系统一键导入课表，并支持调课通知解析与手动添加课程。"
            action={<Button onClick={openLoginDialog}>登录</Button>}
          />
        </Surface>
      ) : view.phase === "loading" ? (
        <div aria-hidden className="space-y-3">
          <div className="h-10 w-64 animate-pulse rounded bg-line" />
          <Surface className="h-96 p-4">
            <div className="h-full w-full animate-pulse rounded bg-line" />
          </Surface>
        </div>
      ) : view.phase === "error" ? (
        <Surface>
          <EmptyState
            icon={CalendarRange}
            domain="sched"
            title="课表读取失败"
            hint={view.message}
            action={
              <Button variant="outline" onClick={() => setReloadTick((t) => t + 1)}>
                重试
              </Button>
            }
          />
        </Surface>
      ) : ready && tt && tt.courses.length === 0 ? (
        <Surface>
          <EmptyState
            icon={CalendarRange}
            domain="sched"
            title="还没有课表"
            hint="登录教务后一键导入本学期课表；也可以先手动添加课程。"
            action={
              <>
                <Button disabled={importing} onClick={doImport}>
                  {importing ? "导入中…" : "导入课表"}
                </Button>
                <Button variant="outline" onClick={() => openForm({}, null)}>
                  手动添加
                </Button>
              </>
            }
          />
        </Surface>
      ) : ready && tt ? (
        <>
          {/* 导入摘要 / 失败提示（可关闭） */}
          {importMsg && (
            <Surface
              accent={importMsg.ok ? "sched" : undefined}
              className="mb-3 px-4 py-3"
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <p className={cn("text-body font-medium", importMsg.ok ? "text-sched" : "text-alert")}>
                    {importMsg.text}
                  </p>
                  {importMsg.ok && importMsg.result && importMsg.result.changes.length > 0 && (
                    <ul className="mt-1.5 space-y-0.5">
                      {importMsg.result.changes.map((c, i) => (
                        <li key={i} className="text-caption text-text-2">{c}</li>
                      ))}
                    </ul>
                  )}
                </div>
                <button
                  type="button"
                  aria-label="关闭提示"
                  onClick={() => setImportMsg(null)}
                  className="shrink-0 rounded px-1.5 text-caption text-text-2 hover:text-text"
                >
                  ✕
                </button>
              </div>
            </Surface>
          )}

          {/* 未设置开学日提示（周次不可靠） */}
          {currentWeek === null && (
            <Surface accent="sched" className="mb-3 px-4 py-3">
              <p className="text-body text-text-2">
                尚未设置学期开学日，当前按第 {week} 周展示；完成一次「导入 / 同步」可自动设置。
              </p>
            </Surface>
          )}

          {/* ICS 导出失败提示 */}
          {icsMsg && (
            <p className="mb-3 text-caption text-alert" role="alert">{icsMsg}</p>
          )}

          {/* 周视图网格 */}
          <Surface className="overflow-x-auto">
            <div
              className="grid min-w-[640px]"
              style={{ gridTemplateColumns: `56px repeat(${displayDays}, minmax(0, 1fr))` }}
            >
              {/* 表头行：列头从 firstDay 起旋转（firstDay=7 → 周日起） */}
              <div className="border-b border-line" />
              {Array.from({ length: displayDays }, (_, i) => {
                const day = displayDayOf(i);
                return (
                  <div
                    key={day}
                    className={cn(
                      "border-b border-line py-2 text-center",
                      i === todayCol ? "bg-sched/5 font-medium text-sched" : "text-text-2",
                      i > 0 && "border-l border-line",
                    )}
                  >
                    <p className="text-caption">{DAY_NAMES[day]}</p>
                    {weekDates[i] && (
                      <p className="tabular-num text-caption opacity-70">{fmtDay(weekDates[i]!)}</p>
                    )}
                  </div>
                );
              })}
              {/* 时间列：一律取后端 slots（校本大节作息），前端不硬编码时间 */}
              <div>
                {slots.map((s) => (
                  <div
                    key={s.number}
                    className="flex flex-col items-center justify-center border-b border-line px-1 text-center last:border-b-0"
                    style={{ height: ROW_H }}
                  >
                    <span className="tabular-num text-body font-medium text-text-2">
                      {s.number}
                    </span>
                    <span className="tabular-num text-caption opacity-60 text-text-2">
                      {s.startTime}
                    </span>
                    <span className="tabular-num text-caption opacity-60 text-text-2">
                      {s.endTime}
                    </span>
                  </div>
                ))}
              </div>
              {/* 课程列：按显示列渲染（buildWeekBlocks 恒按 7 天计算，这里只取
                  displayDays 列；colIdx ≥ displayDays 的星期六/日课静默不渲染） */}
              {Array.from({ length: displayDays }, (_, i) => {
                const day = displayDayOf(i);
                const col = weekBlocks.columns[day - 1];
                const layout = layouts[day - 1];
                return (
                  <div
                    key={day}
                    className={cn(
                      "relative border-line",
                      i > 0 && "border-l",
                      i === todayCol && "bg-sched/5",
                    )}
                    style={{ height: ROW_H * slots.length }}
                  >
                    {/* 空位按钮：点击空白格新建课程（预填星期/大节/展示周），渲染在课程块之下 */}
                    {slots.map((s) => (
                      <button
                        key={`slot-${s.number}`}
                        type="button"
                        aria-label={`${DAY_NAMES[day]}第${s.number}大节空位，点击添加课程`}
                        onClick={() => {
                          if (currentWeek === null) {
                            setNoticeMsg({ ok: false, text: "请在学期内添加课程" });
                            return;
                          }
                          openForm(
                            {
                              day,
                              startSection: s.number * 2 - 1,
                              endSection: s.number * 2,
                              weeks: [week],
                            },
                            null,
                          );
                        }}
                        className="absolute left-0 w-full border-b border-line/50 last:border-b-0 hover:bg-sched/10 focus-visible:bg-sched/10"
                        style={{ top: (s.number - 1) * ROW_H, height: ROW_H }}
                      />
                    ))}
                    {col.map((b) => {
                      const pos = layout.get(b.key) ?? { lane: 0, lanes: 1 };
                      const color = courseColor(b.course);
                      const top = (b.startBlock - 1) * ROW_H + 2;
                      const height = (b.endBlock - b.startBlock + 1) * ROW_H - 4;
                      return (
                        <button
                          key={b.key}
                          type="button"
                          data-course-block
                          ref={(el) => {
                            if (el) blockRefs.current.set(b.key, el);
                            else blockRefs.current.delete(b.key);
                          }}
                          aria-label={`${b.course.name}，${DAY_NAMES[b.day]}第${b.startBlock}至${b.endBlock}大节${b.ghost ? `（${b.ghost === "cancelled" ? "已停" : "已调出"}）` : ""}`}
                          onClick={() => openBlockDetail(b)}
                          className={cn(
                            "absolute overflow-hidden rounded-inner px-1.5 py-1 text-left transition-shadow duration-[var(--dur-fast)] ease-out-soft hover:shadow-card focus-visible:shadow-card",
                            b.ghost ? "border border-dashed" : "border",
                          )}
                          style={{
                            top,
                            height,
                            left: `${(pos.lane / pos.lanes) * 100}%`,
                            width: `calc(${(1 / pos.lanes) * 100}% - 3px)`,
                            borderColor: b.ghost ? undefined : color,
                            backgroundColor: b.ghost
                              ? `color-mix(in srgb, ${color} 6%, transparent)`
                              : `color-mix(in srgb, ${color} 14%, transparent)`,
                          }}
                        >
                          <p
                            className={cn(
                              "truncate text-caption font-medium leading-tight text-text-2",
                              b.ghost && "line-through",
                            )}
                            style={b.ghost ? undefined : { color }}
                          >
                            {b.ghost === "cancelled" ? "已停 · " : b.ghost === "moved-out" ? "已调出 · " : ""}
                            {b.course.name}
                          </p>
                          {b.course.teacher && (
                            <p className="truncate text-caption leading-tight text-text-2">
                              {b.course.teacher}
                            </p>
                          )}
                          {b.room && (
                            <p className="tabular-num truncate text-caption leading-tight text-text-2">
                              @{b.room}
                            </p>
                          )}
                          {!b.ghost && (
                            <p className="absolute right-1 top-1 flex gap-0.5 text-[10px] leading-none">
                              {b.course.source === "import" && (
                                <span
                                  className="rounded px-0.5 py-px"
                                  style={{
                                    backgroundColor: `color-mix(in srgb, ${color} 16%, transparent)`,
                                    color,
                                  }}
                                >
                                  导
                                </span>
                              )}
                              {b.override && (
                                <span
                                  className="rounded px-0.5 py-px"
                                  style={{
                                    backgroundColor: `color-mix(in srgb, ${color} 16%, transparent)`,
                                    color,
                                  }}
                                >
                                  调
                                </span>
                              )}
                            </p>
                          )}
                        </button>
                      );
                    })}
                  </div>
                );
              })}
            </div>
          </Surface>

          {/* 课程详情浮层 */}
          {detail && (
            <div
              data-course-detail
              role="dialog"
              aria-label="课程详情"
              className="fixed z-40 w-72 max-h-[60vh] overflow-y-auto rounded-card border border-line bg-surface p-4 shadow-pop"
              style={{ top: detail.top, left: detail.left }}
            >
              {(() => {
                const c = detail.course;
                const myOverrides = tt.overrides.filter((o) => o.courseId === c.id);
                const color = courseColor(c);
                return (
                  <>
                    <div className="flex items-center gap-2">
                      <span
                        aria-hidden
                        className="size-2.5 shrink-0 rounded-full"
                        style={{ backgroundColor: color }}
                      />
                      <p className="min-w-0 truncate text-body font-semibold text-text">{c.name}</p>
                      {c.source === "import" && (
                        <span className="shrink-0 rounded bg-sched/10 px-1 text-caption text-sched">导</span>
                      )}
                      {c.disabled && (
                        <span className="shrink-0 rounded bg-line px-1 text-caption text-text-2">已停开</span>
                      )}
                    </div>
                    <dl className="mt-2 space-y-1 text-caption text-text-2">
                      {c.teacher && <div>教师：{c.teacher}</div>}
                      {c.classId && <div className="truncate">教学班：{c.classId}</div>}
                      <div className="tabular-num">
                        {DAY_NAMES[c.day]} {c.startSection ?? "?"}-{c.endSection ?? "?"} 小节
                        （第 {fmtWeeks(c.weeks)} 周）
                      </div>
                      {c.position && <div>教室：{c.position}</div>}
                      {c.remark && (
                        <div>
                          {c.source === "import" ? "性质 · 考核：" : "备注："}
                          {c.remark}
                        </div>
                      )}
                    </dl>
                    {myOverrides.length > 0 && (
                      <div className="mt-2 border-t border-line pt-2">
                        <p className="text-caption font-medium text-text">生效中的调整</p>
                        <ul className="mt-1 space-y-1">
                          {myOverrides.map((o) => (
                            <li key={o.id} className="flex items-center justify-between gap-2">
                              <span className="min-w-0 text-caption text-text-2">
                                <span className={cn("font-medium", o.autoApplied ? "text-sched" : "text-todo")}>
                                  {KIND_LABEL[o.changeType]}
                                </span>{" "}
                                {overrideSummary(o)}
                                {o.autoApplied && <span className="ml-1 opacity-70">（自动）</span>}
                              </span>
                              <button
                                type="button"
                                disabled={busyKey === `revoke-${o.id}`}
                                onClick={() => revokeOverride(o)}
                                className="shrink-0 text-caption text-alert hover:underline disabled:opacity-50"
                              >
                                撤销
                              </button>
                            </li>
                          ))}
                        </ul>
                      </div>
                    )}
                    <div className="mt-3 flex flex-wrap items-center gap-2">
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => openForm({ ...c }, null)}
                        title="以此课程为模板预填手动添加表单"
                      >
                        手动添加同款
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => openForm({ ...c }, c)}>
                        <Pencil aria-hidden="true" className="size-3.5" />
                        编辑
                      </Button>
                    </div>
                  </>
                );
              })()}
            </div>
          )}

          {/* 手动添加 / 编辑表单（key 重挂重置内部 state） */}
          {formOpen && (
            <CourseForm
              key={editing?.id ?? "new"}
              initial={formInitial}
              editing={editing !== null}
              totalWeeks={totalWeeks}
              busy={busyKey === "form"}
              error={formError}
              onSubmit={(p) => {
                setBusyKey("form");
                void submitCourse(p);
              }}
              onCancel={closeForm}
            />
          )}

          {/* 作息时间表编辑弹层（打开时快照当前生效作息） */}
          {slotsOpen && ready && (
            <SlotsEditor
              initial={ready.slots}
              usingCustom={tt?.config.slots != null && tt.config.slots.length > 0}
              busy={slotsBusy}
              error={slotsErr}
              onSave={(slots) => void saveSlots(slots)}
              onReset={() => void resetSlots()}
              onClose={() => setSlotsOpen(false)}
            />
          )}

          {/* 课表设置弹层（契约 §7.1） */}
          {settingsOpen && ready && tt && (
            <SettingsEditor
              initial={{
                semesterStartDate: tt.config.semesterStartDate,
                semesterTotalWeeks: tt.config.semesterTotalWeeks,
                firstDayOfWeek: tt.config.firstDayOfWeek,
                showWeekends: tt.config.showWeekends,
              }}
              busy={settingsBusy}
              error={settingsErr}
              onSave={(input) => void saveSemesterConfig(input)}
              onClose={() => setSettingsOpen(false)}
            />
          )}

          {/* 调课通知区（有课程才显示） */}
          {tt.courses.length > 0 && (
            <Surface className="mt-4 px-4 py-4">
              <div className="flex items-center gap-2">
                <ClipboardPaste aria-hidden="true" className="size-4 text-sched" />
                <p className="text-body font-semibold text-text">调课 / 停课 / 补课通知</p>
              </div>
              <p className="mt-1 text-caption text-text-2">
                粘贴教务处或学院通知原文，自动提取调课信息；要素不全的会列出原因待你确认。
              </p>
              <div className="mt-2.5 flex items-start gap-2">
                <textarea
                  value={noticeText}
                  onChange={(e) => setNoticeText(e.target.value)}
                  rows={3}
                  placeholder="粘贴通知文本，例如：第5周周四3-4节 信息安全 调整到 D4-305"
                  className="min-h-[72px] flex-1 rounded-control border border-line bg-surface px-3 py-2 text-body text-text placeholder:text-text-2/60"
                />
                <Button disabled={parsing || !noticeText.trim()} onClick={parseNotice}>
                  {parsing ? "解析中…" : "解析"}
                </Button>
              </div>
              {noticeMsg && (
                <p
                  className={cn("mt-2 text-caption", noticeMsg.ok ? "text-sched" : "text-alert")}
                  role="status"
                >
                  {noticeMsg.text}
                </p>
              )}

              {/* 候选列表：高置信可自动应用，低置信显示 reason */}
              {candidates && candidates.length > 0 && (
                <ul className="mt-3 space-y-2">
                  {candidates.map((c) => (
                    <li
                      key={`${c.noticeId}-${c.courseName}`}
                      className="rounded-inner border border-line bg-surface-2 px-3 py-2.5"
                    >
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="rounded bg-sched/10 px-1.5 text-caption font-medium text-sched">
                          {KIND_LABEL[c.changeType]}
                        </span>
                        <p className="text-body font-medium text-text">{c.courseName || "未识别课程"}</p>
                        {c.confidence === "high" ? (
                          <span className="rounded bg-wallet/10 px-1.5 text-caption font-medium text-wallet">
                            可自动应用
                          </span>
                        ) : (
                          <span className="rounded bg-todo/10 px-1.5 text-caption text-todo" title={c.reason}>
                            待确认
                          </span>
                        )}
                        <Button
                          size="sm"
                          className="ml-auto"
                          disabled={busyKey === `adopt-${c.noticeId}-${c.courseName}`}
                          onClick={() => adoptCandidate(c)}
                        >
                          采纳
                        </Button>
                      </div>
                      <p className="tabular-num mt-1 text-caption text-text-2">{overrideSummary(c)}</p>
                      {c.confidence === "low" && c.reason && (
                        <p className="mt-1 text-caption text-todo">{c.reason}</p>
                      )}
                      {c.excerpt && (
                        <p className="mt-1 border-l-2 border-line pl-2 text-caption text-text-2/80">
                          {c.excerpt}
                        </p>
                      )}
                    </li>
                  ))}
                </ul>
              )}

              {/* 已生效 override 列表（按通知可整批撤销） */}
              {tt.overrides.length > 0 && (
                <div className="mt-4 border-t border-line pt-3">
                  <p className="text-caption font-medium text-text">
                    已生效调整 · {tt.overrides.length} 条
                  </p>
                  <ul className="mt-1.5 space-y-1">
                    {tt.overrides.map((o) => {
                      const course = tt.courses.find((c) => c.id === o.courseId);
                      return (
                        <li key={o.id} className="flex items-center justify-between gap-2">
                          <span className="min-w-0 text-caption text-text-2">
                            <span className={cn("font-medium", o.autoApplied ? "text-sched" : "text-todo")}>
                              {KIND_LABEL[o.changeType]}
                            </span>{" "}
                            {course?.name ?? "（课程已删除）"} · {overrideSummary(o)}
                            {o.autoApplied && <span className="ml-1 opacity-70">（自动）</span>}
                          </span>
                          <button
                            type="button"
                            disabled={busyKey === `revoke-${o.id}`}
                            onClick={() => revokeOverride(o)}
                            className="shrink-0 text-caption text-alert hover:underline disabled:opacity-50"
                          >
                            撤销此通知调整
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                </div>
              )}
            </Surface>
          )}

          {/* 全部课程列表（停开灰显；编辑/删除入口） */}
          <div className="mt-6 mb-4 flex items-center justify-between">
            <p className="text-body font-semibold text-text">
              全部课程 · {tt.courses.length} 门
            </p>
            <Button variant="outline" size="sm" onClick={() => openForm({}, null)}>
              <Plus aria-hidden="true" className="size-3.5" />
              手动添加
            </Button>
          </div>
          <ul className="space-y-2">
            {[...tt.courses]
              .sort((a, b) => a.day - b.day || (a.startSection ?? 0) - (b.startSection ?? 0))
              .map((c) => (
                <li key={c.id}>
                  <Surface className={cn("flex flex-wrap items-center gap-x-3 gap-y-1 px-4 py-2.5", c.disabled && "opacity-55")}>
                    <span
                      aria-hidden
                      className="size-2.5 shrink-0 rounded-full"
                      style={{ backgroundColor: courseColor(c) }}
                    />
                    <p className="min-w-0 truncate text-body font-medium text-text">{c.name}</p>
                    {c.disabled && (
                      <span className="shrink-0 rounded bg-line px-1 text-caption text-text-2">已停开</span>
                    )}
                    <span className="tabular-num min-w-0 truncate text-caption text-text-2">
                      {DAY_NAMES[c.day]} {c.startSection ?? "?"}-{c.endSection ?? "?"} 节 · 第 {fmtWeeks(c.weeks)} 周
                      {c.position && ` · ${c.position}`}
                      {c.teacher && ` · ${c.teacher}`}
                    </span>
                    <span className="ml-auto flex shrink-0 items-center gap-2">
                      <span className="rounded bg-line px-1 text-caption text-text-2">
                        {c.source === "import" ? "导入" : "手动"}
                      </span>
                      <button
                        type="button"
                        onClick={() => openForm({ ...c }, c)}
                        className="text-caption text-text-2 hover:text-text"
                      >
                        编辑
                      </button>
                      <button
                        type="button"
                        disabled={busyKey === `del-${c.id}`}
                        onClick={() => removeCourse(c)}
                        className="text-caption text-alert hover:underline disabled:opacity-50"
                      >
                        删除
                      </button>
                    </span>
                  </Surface>
                </li>
              ))}
          </ul>
        </>
      ) : null}
    </section>
  );
}
