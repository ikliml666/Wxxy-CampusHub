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
  Upload,
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
  JsonImportResult,
  NoticeCandidate,
  OverrideKind,
  SemesterConfigInput,
  SlotRule,
  TimeSlot,
  TimetableView,
} from "@/shared/types";

/** 课程固定色板（8 档，域色系 token：6 个既有域色 + index.css 新增的 2 档扩展）。
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

/** 固定段长度（批 7 §13.3：自定义色段下标从这起）。 */
const BASE_PALETTE_LEN = COURSE_PALETTE.length;

/** 合成色板（批 7 契约 §13.3）= 8 档固定色 + uiStore.customCourseColors 自定义段；
 *  固定段在前 → 旧数据 colorIndex 0..7 取色行为不变。所有色板下标消费点
 *  （courseColor / CourseForm 色板）统一经本函数，禁止直接下标 COURSE_PALETTE。 */
const coursePalette = (custom: string[]): string[] => [...COURSE_PALETTE, ...custom];

/** 取色：导入课哈希大数与手动课下标同口径 `% 合成长度`（新增自定义色后导入课
 *  取色可能整体位移，批 7 冻结的已知语义）。 */
const courseColor = (c: Course, custom: string[]) => {
  const palette = coursePalette(custom);
  return palette[c.colorIndex % palette.length];
};

const KIND_LABEL: Record<OverrideKind, string> = {
  rescheduled: "调课",
  cancelled: "停课",
  extra: "补课",
};

/** 单大节行高（px）；网格行数 = slots.length（作息可编辑后行数不固定为 5）。 */
const ROW_H = 72;

const DAY_NAMES = ["", "周一", "周二", "周三", "周四", "周五", "周六", "周日"];

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

/** 周次列表 → 紧凑文案："1-16" / "1,3,5(单周)" / "1-8,10"。
 *  单双周后缀（批 5 §11.6，与 Rust crates/campus-schedule/src/diff.rs::format_weeks
 *  同语义互锚，改一处必须同步另一处）：全奇数且 ≥3 项 → `(单周)`；全偶数
 *  （任意项数）→ `(双周)`；其余不变。 */
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
  const out = parts.join(",");
  if (sorted.length >= 3 && sorted.every((w) => w % 2 === 1)) return `${out}(单周)`;
  if (sorted.every((w) => w % 2 === 0)) return `${out}(双周)`;
  return out;
}

/** 课程的时刻标签（批 5 §11.2）：custom 课显示自定义起止时刻，节次课显示小节范围。 */
function courseTimeLabel(c: Course): string {
  if (c.isCustomTime && c.customStartTime && c.customEndTime)
    return `${c.customStartTime}-${c.customEndTime}`;
  return `${c.startSection ?? "?"}-${c.endSection ?? "?"} 节`;
}

/** 表单周次文本 → 显式周次列表（"1-8,10" / "1、3" 混排）；非法返回 null。
 *  容忍 fmtWeeks 回显的尾随 `(单周)/(双周)` 后缀（批 5 §11.6）。 */
function parseWeeksInput(text: string): number[] | null {
  const cleaned = text.replace(/\((单周|双周)\)\s*$/, "").trim();
  const out = new Set<number>();
  for (const part of cleaned.split(/[,，、\s]+/).filter(Boolean)) {
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
  /** 非本周来源（批 7 §13.2）：渲染 40% 降级、不可拖；ghost 样式优先级不变
   *  （降级只作用于 solid 块，风险 R7） */
  nonCurrent: boolean;
}

/** 逆序取最后一条匹配（后采纳的通知覆盖先采纳的，upsert 语义与之呼应）。 */
function lastOverride(
  overrides: CourseOverride[],
  pred: (o: CourseOverride) => boolean,
): CourseOverride | null {
  for (let i = overrides.length - 1; i >= 0; i--) if (pred(overrides[i])) return overrides[i];
  return null;
}

/** "HH:MM" → 分钟数；非法返回 null（批 5 §11.2 custom 相交判定用）。 */
function parseHmMinutes(hm: string): number | null {
  const m = hm.match(/^(\d{2}):(\d{2})$/);
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  if (h > 23 || min > 59) return null;
  return h * 60 + min;
}

/** custom 课（按时刻，无节次）→ 网格落块（批 5 §11.2）：取与各大节区间
 *  **闭区间相交**（端点相触算相交：`start <= slot.endTime && slot.startTime <= end`）
 *  的大节号的 min..max（中间空档一并覆盖）。无相交大节（如整段落在课间空隙）
 *  或时间非法（非 HH:MM / end <= start）→ null：网格不渲染，仅详情/列表可见。 */
function customBlockRange(
  startHm: string,
  endHm: string,
  slots: TimeSlot[],
): { startBlock: number; endBlock: number } | null {
  const start = parseHmMinutes(startHm);
  const end = parseHmMinutes(endHm);
  if (start == null || end == null || end <= start) return null;
  let startBlock: number | null = null;
  let endBlock: number | null = null;
  for (const s of slots) {
    const ss = parseHmMinutes(s.startTime);
    const se = parseHmMinutes(s.endTime);
    if (ss == null || se == null) continue;
    if (start <= se && ss <= end) {
      if (startBlock == null || s.number < startBlock) startBlock = s.number;
      if (endBlock == null || s.number > endBlock) endBlock = s.number;
    }
  }
  return startBlock != null && endBlock != null ? { startBlock, endBlock } : null;
}

function buildWeekBlocks(
  view: TimetableView,
  week: number,
): { blocks: PlacedBlock[]; columns: PlacedBlock[][] } {
  const { courses, overrides } = view.timetable;
  const blocks: PlacedBlock[] = [];
  const byId = new Map(courses.map((c) => [c.id, c]));
  // 非本周降级开关（批 7 契约 §13.2）：关闭 = 现状隐藏；开启 = 非本周课照常
  // 展开（块标记 nonCurrent，渲染层降级、不可拖）。
  const showNonCurrent = view.timetable.config.showNonCurrentWeek;

  for (const course of courses) {
    if (course.disabled) continue;
    const inWeek = course.weeks.includes(week);
    if (!inWeek && !showNonCurrent) continue;
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
      nonCurrent: !inWeek,
    });

    /** 原时段落块（批 5 §11.2）：custom 课按 custom 时刻与各大节相交取 min..max
     *  大节，节次课按小节折算大节；null = 无可渲染时段（custom 无相交/时间非法，
     *  或无节次）→ 不渲染仅详情/列表可见。 */
    const origRange: { startBlock: number; endBlock: number } | null = course.isCustomTime
      ? course.customStartTime && course.customEndTime
        ? customBlockRange(course.customStartTime, course.customEndTime, view.slots)
        : null
      : course.startSection != null && course.endSection != null
        ? { startBlock: blockOf(course.startSection), endBlock: blockOf(course.endSection) }
        : null;

    // 停课优先（冻结契约 §2.5.1 两档）：newDay 有值 = 只停「该周 · 星期 newDay」
    // 那一次——该课当天有排课才渲染虚线「已停」占位，本周其他星期的同课不受影响
    // （同课另一天的记录由挂在其 courseId 上的 override 单独处理）；
    // newDay = null = 通知未提星期 → 该课在 weeks 列出的周次内整周全停，
    // 该周该课所有原时段渲染虚线「已停」。custom 课按相交大节出占位。
    if (cancel && (cancel.newDay == null || cancel.newDay === course.day)) {
      if (origRange)
        blocks.push(
          mk(course.day, origRange.startBlock, origRange.endBlock, course.position, cancel, "cancelled"),
        );
      continue;
    }
    // 调课且新时间 ≠ 原时间 → 原时段虚线占位 + 新时段实体块（custom 课被调
    // 也走新节次——契约 §8.4：新节次取 override 的 new_*，不取 custom 时刻）
    if (
      resched &&
      resched.newDay != null &&
      resched.newStartSection != null &&
      (resched.newDay !== course.day ||
        !origRange ||
        blockOf(resched.newStartSection) !== origRange.startBlock)
    ) {
      if (origRange)
        blocks.push(
          mk(course.day, origRange.startBlock, origRange.endBlock, course.position, resched, "moved-out"),
        );
      const newStart = blockOf(resched.newStartSection);
      // 单节补调：结束 = 起始（复核 P2 修复：缺省必须用已折算的大节 newStart，
      // 误用 raw 小节号会把块拉高数倍并挤压同列分列——与 Rust
      // occurrence.rs `unwrap_or(start)` 同语义，两处注释互锚）
      const newEnd =
        resched.newEndSection != null
          ? blockOf(resched.newEndSection)
          : newStart;
      blocks.push(mk(resched.newDay, newStart, newEnd, resched.newPosition ?? course.position, resched, null));
      continue;
    }
    // 原地（可能仅换教室）；custom 课无相交大节 → origRange 为 null，仅列表可见
    if (origRange)
      blocks.push(
        mk(course.day, origRange.startBlock, origRange.endBlock, resched?.newPosition ?? course.position, resched, null),
      );
  }

  // 补课叠加：新时段新增实体块（独立于上方课程实体块分支——停课/调课周的补课
  // 照常渲染，且不看出 course.weeks；⚠️ 语义互锚 crates/campus-schedule/src/
  // occurrence.rs::expand_occurrences 的 extra 循环，改一处必须同步另一处。
  // 课程删除时后端级联清理 override，正常必命中）
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
      nonCurrent: false,
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

// ---------------- 网格拖拽改课（契约 §10，批 4：纯前端几何 + 落库分叉） ----------------

/** 点击/拖拽阈值：曼哈顿距离超过该值才算拖拽（契约 §10.1）。 */
const DRAG_THRESHOLD = 4;

/** 一次拖拽会话。col/startBlock = 落点；-1 = 无效落点（跳过日期列/网格外）。 */
interface DragState {
  block: PlacedBlock;
  /** 发起拖拽的 pointerId（move/up/cancel 校验，防多指覆盖会话） */
  pointerId: number;
  /** pointerdown 起点（client 坐标） */
  startX: number;
  startY: number;
  /** 移动超过阈值后进入拖拽（此前 pointerup 视为纯点击） */
  active: boolean;
  /** 落点显示列下标（0-based，经 displayDayOf 反映射回星期） */
  col: number;
  /** 落点起始大节（1-based，末位对齐 clamp） */
  startBlock: number;
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
    isCustomTime: boolean;
    customStartTime: string | null;
    customEndTime: string | null;
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
  const customColors = useUiStore((s) => s.customCourseColors);
  const [colorIndex, setColorIndex] = useState(
    typeof initial.colorIndex === "number"
      ? initial.colorIndex % Math.max(1, BASE_PALETTE_LEN + customColors.length)
      : 0,
  );
  // 原生 <input type="color"> 的取色器在拖动时连续触发 input 事件（React onChange
  // 同名），会把中间色塞进自定义段——只监听原生 change（选择器关闭才提交），
  // 取值走 getState 防闭包过期（批 7 §13.3，不引取色器库）。
  const colorInputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const el = colorInputRef.current;
    if (!el) return;
    const onCommit = () => {
      const hex = el.value.toLowerCase();
      const { customCourseColors, addCustomCourseColor } = useUiStore.getState();
      const exist = customCourseColors.indexOf(hex);
      if (exist >= 0) {
        setColorIndex(BASE_PALETTE_LEN + exist);
        return;
      }
      addCustomCourseColor(hex);
      setColorIndex(BASE_PALETTE_LEN + customCourseColors.length);
    };
    el.addEventListener("change", onCommit);
    return () => el.removeEventListener("change", onCommit);
  }, []);
  const [remark, setRemark] = useState(initial.remark ?? "");
  // 「按时刻」模式（批 5 §11.1）：导入课编辑不出现开关（永远节次制）
  const [isCustomTime, setIsCustomTime] = useState(initial.isCustomTime ?? false);
  const [customStartTime, setCustomStartTime] = useState(initial.customStartTime ?? "");
  const [customEndTime, setCustomEndTime] = useState(initial.customEndTime ?? "");
  const [localErr, setLocalErr] = useState<string | null>(null);

  // dirty 检测（批 5 §11.4）：state 与打开时 initial 快照比较；取消时确认放弃
  const snapshot = () =>
    JSON.stringify({
      name,
      teacher,
      position,
      day,
      startSection,
      endSection,
      weeksText,
      colorIndex,
      remark,
      isCustomTime,
      customStartTime,
      customEndTime,
    });
  const initialSnapRef = useRef<string | null>(null);
  if (initialSnapRef.current === null) initialSnapRef.current = snapshot();
  const dirty = snapshot() !== initialSnapRef.current;
  const requestCancel = () => {
    if (dirty && !window.confirm("放弃未保存的修改？")) return;
    onCancel();
  };

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
    if (isCustomTime && (!customStartTime || !customEndTime || customEndTime <= customStartTime))
      return setLocalErr("自定义时间需填写起止，且结束须晚于开始");
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
      isCustomTime,
      customStartTime: isCustomTime ? customStartTime : null,
      customEndTime: isCustomTime ? customEndTime : null,
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
        {/* 「按时刻」开关（批 5 §11.1）：导入课编辑不出现（永远节次制） */}
        {!(editing && initial.source === "import") && (
          <div className={cn(field, "sm:col-span-2 lg:col-span-3")}>
            <label className="flex items-center gap-2 text-body text-text">
              <input
                type="checkbox"
                checked={isCustomTime}
                onChange={(e) => setIsCustomTime(e.target.checked)}
                className="size-4 accent-sched"
              />
              按时刻
              <span className="text-caption text-text-2">
                自定义起止时间（不按小节）；网格按与大节相交的时段落块
              </span>
            </label>
          </div>
        )}
        {isCustomTime ? (
          <>
            <div className={field}>
              <label className={label} htmlFor="tf-custom-start">开始时刻</label>
              <input
                id="tf-custom-start"
                type="time"
                value={customStartTime}
                onChange={(e) => setCustomStartTime(e.target.value)}
                className="tabular-num h-9 rounded-control border border-line bg-surface px-2 text-body text-text"
              />
            </div>
            <div className={field}>
              <label className={label} htmlFor="tf-custom-end">结束时刻</label>
              <input
                id="tf-custom-end"
                type="time"
                value={customEndTime}
                onChange={(e) => setCustomEndTime(e.target.value)}
                className="tabular-num h-9 rounded-control border border-line bg-surface px-2 text-body text-text"
              />
            </div>
          </>
        ) : (
          <>
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
          </>
        )}
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
            {coursePalette(customColors).map((color, i) => (
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
            {/* 自定义色段「+」（批 7 §13.3）：弹原生取色器，选中即入 uiStore 自定义段 */}
            <button
              type="button"
              aria-label="添加自定义颜色"
              title="添加自定义颜色"
              onClick={() => colorInputRef.current?.click()}
              className="size-6 rounded-full border border-dashed border-line text-caption leading-none text-text-2 transition-colors hover:border-text-2 hover:text-text"
            >
              +
            </button>
            <input
              ref={colorInputRef}
              type="color"
              defaultValue="#5b8def"
              aria-hidden="true"
              tabIndex={-1}
              className="hidden"
            />
          </div>
        </div>
        <div className={cn(field, "sm:col-span-2 lg:col-span-3")}>
          <div className="flex items-center justify-between">
            <label className={label} htmlFor="tf-remark">
              {editing && initial.source === "import" ? "性质 · 考核方式" : "备注"}
            </label>
            <span className="tabular-num text-caption text-text-2">{remark.length}/300</span>
          </div>
          <textarea
            id="tf-remark"
            value={remark}
            maxLength={300}
            rows={2}
            onChange={(e) => setRemark(e.target.value)}
            className="min-h-[56px] rounded-control border border-line bg-surface px-3 py-2 text-body text-text placeholder:text-text-2/60"
          />
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
        <Button variant="outline" onClick={requestCancel} disabled={busy}>
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

/** 作息行编辑器（主作息与每条日期规则复用；大节号 = 行序，保存时生成）。 */
function SlotRowsEditor({
  rows,
  onChange,
  disabled,
}: {
  rows: SlotRow[];
  onChange: (rows: SlotRow[]) => void;
  disabled: boolean;
}) {
  const update = (i: number, patch: Partial<SlotRow>) =>
    onChange(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const addRow = () => {
    const last = rows[rows.length - 1];
    const start = last ? last.endTime : "08:00";
    onChange([...rows, { startTime: start, endTime: addMinutes(start, 100), alias: null }]);
  };
  return (
    <>
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
            disabled={disabled}
            className="tabular-num h-9 flex-1 rounded-control border border-line bg-surface px-2 text-body text-text disabled:opacity-50"
          />
          <span aria-hidden className="text-caption text-text-2">–</span>
          <input
            type="time"
            value={r.endTime}
            onChange={(e) => update(i, { endTime: e.target.value })}
            aria-label={`第 ${i + 1} 大节结束时间`}
            disabled={disabled}
            className="tabular-num h-9 flex-1 rounded-control border border-line bg-surface px-2 text-body text-text disabled:opacity-50"
          />
          {/* 别名（批 5 §11.3，maxlength 5 选填；保存随 TimeSlot.alias 透传，后端零改动） */}
          <input
            type="text"
            value={r.alias ?? ""}
            maxLength={5}
            placeholder="别名"
            aria-label={`第 ${i + 1} 大节别名（选填）`}
            onChange={(e) => update(i, { alias: e.target.value || null })}
            disabled={disabled}
            className="h-9 w-20 shrink-0 rounded-control border border-line bg-surface px-2 text-caption text-text placeholder:text-text-2/60 disabled:opacity-50"
          />
          <button
            type="button"
            aria-label={`删除第 ${i + 1} 大节`}
            disabled={disabled}
            onClick={() => onChange(rows.filter((_, j) => j !== i))}
            className="shrink-0 rounded px-1.5 text-caption text-alert hover:underline disabled:opacity-50"
          >
            删除
          </button>
        </div>
      ))}
      {rows.length === 0 && (
        <p className="py-3 text-center text-caption text-text-2">暂无作息行，点击下方新增。</p>
      )}
      <button
        type="button"
        disabled={disabled}
        onClick={addRow}
        className="mt-2 flex items-center gap-1 text-caption text-sched hover:underline disabled:opacity-50"
      >
        <Plus aria-hidden="true" className="size-3.5" />
        新增大节
      </button>
    </>
  );
}

function SlotsEditor({
  initial,
  initialRules,
  usingCustom,
  busy,
  rulesBusy,
  error,
  rulesError,
  onSave,
  onSaveRules,
  onReset,
  onClose,
}: {
  /** 当前生效作息（自定义或内置默认），打开时快照 */
  initial: TimeSlot[];
  /** 日期作息规则快照（契约 §9，批 3） */
  initialRules: SlotRule[];
  usingCustom: boolean;
  busy: boolean;
  /** 规则保存独立 busy（save_slot_rules 整体替换提交） */
  rulesBusy: boolean;
  error: string | null;
  rulesError: string | null;
  onSave: (slots: TimeSlot[]) => void;
  /** null = 清空全部规则（契约 §9.3） */
  onSaveRules: (rules: SlotRule[] | null) => void;
  onReset: () => void;
  onClose: () => void;
}) {
  const [rows, setRows] = useState<SlotRow[]>(
    initial.map((s) => ({ startTime: s.startTime, endTime: s.endTime, alias: s.alias })),
  );
  const [localErr, setLocalErr] = useState<string | null>(null);
  // 日期规则草稿：起止日期 + 作息行，独立整体替换保存
  const [rules, setRules] = useState<{ startDate: string; endDate: string; rows: SlotRow[] }[]>(
    initialRules.map((r) => ({
      startDate: r.startDate,
      endDate: r.endDate,
      rows: r.slots.map((s) => ({ startTime: s.startTime, endTime: s.endTime, alias: s.alias })),
    })),
  );
  const [rulesLocalErr, setRulesLocalErr] = useState<string | null>(null);

  // dirty 检测（批 5 §11.4）：state 与打开时快照比较（主作息与规则任一变化即 dirty）；
  // 取消 / Esc / 遮罩关闭三条路径统一走 requestClose 确认放弃
  const dirty =
    JSON.stringify(rows) !==
      JSON.stringify(
        initial.map((s) => ({ startTime: s.startTime, endTime: s.endTime, alias: s.alias })),
      ) ||
    JSON.stringify(rules) !==
      JSON.stringify(
        initialRules.map((r) => ({
          startDate: r.startDate,
          endDate: r.endDate,
          rows: r.slots.map((s) => ({ startTime: s.startTime, endTime: s.endTime, alias: s.alias })),
        })),
      );
  const requestClose = () => {
    if (dirty && !window.confirm("放弃未保存的修改？")) return;
    onClose();
  };

  // Esc 关闭（busy 时忽略）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy && !rulesBusy) requestClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [busy, rulesBusy, requestClose]);

  const updateRule = (i: number, patch: Partial<{ startDate: string; endDate: string; rows: SlotRow[] }>) =>
    setRules((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));

  const submitRules = () => {
    if (rules.length === 0) {
      setRulesLocalErr(null);
      return onSaveRules(null); // 全部删除 = 清空规则（契约 §9.3）
    }
    for (const [i, r] of rules.entries()) {
      const n = i + 1;
      if (!r.startDate || !r.endDate) return setRulesLocalErr(`第 ${n} 条规则需填写起止日期`);
      if (r.startDate > r.endDate) return setRulesLocalErr(`第 ${n} 条规则的开始日期不能晚于结束日期`);
      if (r.rows.length === 0) return setRulesLocalErr(`第 ${n} 条规则的作息至少需要一行`);
      for (const row of r.rows) {
        if (!row.startTime || !row.endTime) return setRulesLocalErr(`第 ${n} 条规则每行都需要开始与结束时间`);
        if (row.endTime <= row.startTime) return setRulesLocalErr(`第 ${n} 条规则的结束时间必须晚于开始时间`);
      }
    }
    setRulesLocalErr(null);
    onSaveRules(
      rules.map((r) => ({
        startDate: r.startDate,
        endDate: r.endDate,
        slots: r.rows.map((row, j) => ({
          number: j + 1,
          startTime: row.startTime,
          endTime: row.endTime,
          alias: row.alias,
        })),
      })),
    );
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
        if (e.target === e.currentTarget && !busy) requestClose();
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
          <SlotRowsEditor rows={rows} onChange={setRows} disabled={busy} />
        </div>

        {/* 按日期生效的作息规则（契约 §9，批 3）：独立整体替换保存，主作息作回落 */}
        <div className="mt-3 border-t border-line pt-3">
          <p className="text-body font-semibold text-text">按日期生效的作息规则</p>
          <p className="mt-1 text-caption text-text-2">
            命中日期区间（含端点）时优先于上方主作息；区间重叠取先声明的规则。
          </p>
          <div className="mt-2 space-y-2">
            {rules.map((r, i) => (
              <div key={i} className="rounded-control border border-line p-2">
                <div className="flex items-center gap-2">
                  <input
                    type="date"
                    value={r.startDate}
                    aria-label={`规则 ${i + 1} 开始日期`}
                    onChange={(e) => updateRule(i, { startDate: e.target.value })}
                    disabled={busy || rulesBusy}
                    className="tabular-num h-9 flex-1 rounded-control border border-line bg-surface px-2 text-body text-text disabled:opacity-50"
                  />
                  <span aria-hidden className="text-caption text-text-2">–</span>
                  <input
                    type="date"
                    value={r.endDate}
                    aria-label={`规则 ${i + 1} 结束日期`}
                    onChange={(e) => updateRule(i, { endDate: e.target.value })}
                    disabled={busy || rulesBusy}
                    className="tabular-num h-9 flex-1 rounded-control border border-line bg-surface px-2 text-body text-text disabled:opacity-50"
                  />
                  <button
                    type="button"
                    aria-label={`删除规则 ${i + 1}`}
                    disabled={busy || rulesBusy}
                    onClick={() => setRules((rs) => rs.filter((_, j) => j !== i))}
                    className="shrink-0 rounded px-1.5 text-caption text-alert hover:underline disabled:opacity-50"
                  >
                    删除
                  </button>
                </div>
                <div className="mt-2">
                  <SlotRowsEditor
                    rows={r.rows}
                    onChange={(rows) => updateRule(i, { rows })}
                    disabled={busy || rulesBusy}
                  />
                </div>
              </div>
            ))}
            {rules.length === 0 && (
              <p className="text-caption text-text-2/70">暂无规则，日期区间外使用上方主作息。</p>
            )}
          </div>
          <div className="mt-2 flex items-center gap-3">
            <button
              type="button"
              disabled={busy || rulesBusy}
              onClick={() => setRules((rs) => [...rs, { startDate: "", endDate: "", rows: [] }])}
              className="flex items-center gap-1 text-caption text-sched hover:underline disabled:opacity-50"
            >
              <Plus aria-hidden="true" className="size-3.5" />
              新增规则
            </button>
            <Button type="button" variant="outline" size="sm" disabled={rulesBusy} onClick={submitRules}>
              {rulesBusy ? "保存中…" : "保存规则"}
            </Button>
          </div>
          {(rulesLocalErr ?? rulesError) && (
            <p className="mt-2 text-caption text-alert" role="alert">
              {rulesLocalErr ?? rulesError}
            </p>
          )}
        </div>

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
          <Button variant="outline" size="sm" disabled={busy} onClick={requestClose}>
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
  initialSkippedDates,
  busy,
  skippedBusy,
  error,
  skippedError,
  onSave,
  onSaveSkippedDates,
  onClose,
}: {
  initial: {
    semesterStartDate: string | null;
    semesterTotalWeeks: number;
    firstDayOfWeek: number;
    showWeekends: boolean;
    /** 非本周降级显示开关（批 7 契约 §13.2，随 save_semester_config 一并落库） */
    showNonCurrentWeek: boolean;
  };
  /** 跳过日期快照（契约 §8.1，批 2） */
  initialSkippedDates: string[];
  busy: boolean;
  skippedBusy: boolean;
  error: string | null;
  skippedError: string | null;
  onSave: (input: SemesterConfigInput) => void;
  /** 跳过日期独立保存（save_skipped_dates，整体替换），与学期设置分开提交 */
  onSaveSkippedDates: (dates: string[]) => void;
  onClose: () => void;
}) {
  const [startDate, setStartDate] = useState(initial.semesterStartDate ?? "");
  const [totalWeeks, setTotalWeeks] = useState(String(initial.semesterTotalWeeks));
  /** 空 = 不设置；保存时后端按此反推开学日（覆盖上方开学日） */
  const [weekHint, setWeekHint] = useState("");
  const [firstDay, setFirstDay] = useState(initial.firstDayOfWeek);
  const [showWeekends, setShowWeekends] = useState(initial.showWeekends);
  const [showNonCurrentWeek, setShowNonCurrentWeek] = useState(initial.showNonCurrentWeek);
  const [localErr, setLocalErr] = useState<string | null>(null);
  // 跳过日期：本地列表增删，一次整体替换保存（YAGNI：不做日历面板）
  const [skipped, setSkipped] = useState<string[]>(initialSkippedDates);
  const [skipInput, setSkipInput] = useState("");
  const [skipLocalErr, setSkipLocalErr] = useState<string | null>(null);

  // Esc 关闭（busy 时忽略）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy && !skippedBusy) onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [busy, skippedBusy, onClose]);

  const addSkipped = () => {
    if (!skipInput) return;
    if (skipped.includes(skipInput)) return setSkipLocalErr("该日期已在列表中");
    setSkipLocalErr(null);
    setSkipped((ds) => [...ds, skipInput].sort());
    setSkipInput("");
  };

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
      showNonCurrentWeek, // 批 7 §13.2：随学期设置一并落库
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
          {/* 非本周课程降级显示（批 7 契约 §13.2） */}
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              checked={showNonCurrentWeek}
              onChange={(e) => setShowNonCurrentWeek(e.target.checked)}
              disabled={busy}
              className="size-4 accent-sched"
            />
            <span className="text-body text-text">显示非本周课程</span>
            <span className="text-caption text-text-2">以半透明样式叠加，可点击查看、不可拖动</span>
          </label>
          {/* 联动提示（契约 §7.2：前端只提示、不禁用，实际联动由后端保存时收口） */}
          {firstDay === 7 && (
            <p className="text-caption text-text-2">每周起始日为周日时，将始终显示周末列。</p>
          )}
          {!showWeekends && firstDay !== 1 && firstDay !== 7 && (
            <p className="text-caption text-text-2">隐藏周末后，每周起始日将被重置为周一。</p>
          )}

          {/* 跳过日期区块（契约 §8.1，批 2）：独立保存，整体替换 */}
          <div className={cn(field, "border-t border-line pt-3")}>
            <span className={label}>跳过日期（全校停课日）</span>
            <p className="text-caption text-text-2">
              标记后网格该列不显示课程并加「休」标，ICS 导出剔除当天事件。
            </p>
            <div className="flex items-center gap-2">
              <input
                type="date"
                value={skipInput}
                onChange={(e) => setSkipInput(e.target.value)}
                disabled={skippedBusy}
                aria-label="选择要跳过的日期"
                className={cn(inputCls, "flex-1")}
              />
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={skippedBusy || !skipInput}
                onClick={addSkipped}
              >
                <Plus aria-hidden="true" className="size-3.5" />
                添加
              </Button>
            </div>
            {skipped.length > 0 ? (
              <ul className="flex flex-wrap gap-1.5">
                {skipped.map((d) => (
                  <li key={d}>
                    <span className="tabular-num inline-flex items-center gap-1 rounded bg-line px-1.5 py-0.5 text-caption text-text-2">
                      {d}
                      <button
                        type="button"
                        aria-label={`移除 ${d}`}
                        disabled={skippedBusy}
                        onClick={() => setSkipped((ds) => ds.filter((x) => x !== d))}
                        className="text-alert hover:underline disabled:opacity-50"
                      >
                        ✕
                      </button>
                    </span>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-caption text-text-2/70">暂无跳过日期。</p>
            )}
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={skippedBusy}
                onClick={() => onSaveSkippedDates(skipped)}
              >
                {skippedBusy ? "保存中…" : "保存跳过日期"}
              </Button>
              {(skipLocalErr ?? skippedError) && (
                <p className="text-caption text-alert" role="alert">
                  {skipLocalErr ?? skippedError}
                </p>
              )}
            </div>
          </div>
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
  /** 合成色板自定义段（批 7 §13.3）：所有 courseColor 消费点共用 */
  const customColors = useUiStore((s) => s.customCourseColors);
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
  /** ICS 课前提醒（契约 §12.3）：null = 不加 VALARM */
  const [remindMinutes, setRemindMinutes] = useState<number | null>(null);
  const importJsonRef = useRef<HTMLInputElement>(null);

  const [noticeText, setNoticeText] = useState("");
  const [parsing, setParsing] = useState(false);
  const [candidates, setCandidates] = useState<NoticeCandidate[] | null>(null);
  const [noticeMsg, setNoticeMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);

  /** 周次选择弹层（蓝图批 7 小件 1）：顶栏「第 N 周」按钮的下拉网格 */
  const [weekPickerOpen, setWeekPickerOpen] = useState(false);

  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<Course | null>(null);
  const [formInitial, setFormInitial] = useState<Partial<Course>>({});
  const [formError, setFormError] = useState<string | null>(null);

  const [slotsOpen, setSlotsOpen] = useState(false);
  const [slotsBusy, setSlotsBusy] = useState(false);
  const [slotsErr, setSlotsErr] = useState<string | null>(null);
  const [rulesBusy, setRulesBusy] = useState(false);
  const [rulesErr, setRulesErr] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsBusy, setSettingsBusy] = useState(false);
  const [settingsErr, setSettingsErr] = useState<string | null>(null);
  const [skippedBusy, setSkippedBusy] = useState(false);
  const [skippedErr, setSkippedErr] = useState<string | null>(null);

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
  /** 顶栏标题态（批 7 契约 §13.1）：后端按 weeks.rs 对齐式口径判定 */
  const weekState = ready?.weekState ?? "unset";
  /** before 态文案的 N = 开学日 − today 天数（前端日差计算，契约 §13.1） */
  const daysUntilStart = useMemo(() => {
    if (!tt?.config.semesterStartDate || !ready) return null;
    const start = parseDay(tt.config.semesterStartDate);
    const today = parseDay(ready.today);
    if (!start || !today) return null;
    return Math.round((start.getTime() - today.getTime()) / 86400000);
  }, [tt?.config.semesterStartDate, ready]);

  // 周次选择弹层：Esc / 点击弹层以外区域关闭
  useEffect(() => {
    if (!weekPickerOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setWeekPickerOpen(false);
    };
    const onDown = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest("[data-week-picker]")) setWeekPickerOpen(false);
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
    };
  }, [weekPickerOpen]);

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

  /** 跳过日期集合（契约 §8.1）：命中的显示列课程不渲染 +「休」徽标 + 日期置灰
   *  （与周末裁剪同层——只在渲染层过滤，buildWeekBlocks 保持自有实现不动）。 */
  const skippedSet = useMemo(
    () => new Set(tt?.config.skippedDates ?? []),
    [tt?.config.skippedDates],
  );
  const isSkippedCol = (i: number) =>
    weekDates[i] != null && skippedSet.has(dayKeyOf(weekDates[i]!));

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

  // ---------------- 网格拖拽改课（契约 §10，批 4） ----------------

  const [drag, setDrag] = useState<DragState | null>(null);
  const dragRef = useRef<DragState | null>(null);
  /** 进入过拖拽后吃掉后续 click（pointer capture 后 click 仍触发，契约 §10.1 最常见坑） */
  const suppressClickRef = useRef(false);
  /** 天列容器（显示列下标 → DOM），pointerdown 时缓存 rect 快照用于命中测试 */
  const dayColRefs = useRef(new Map<number, HTMLElement>());
  const dragRectsRef = useRef<{ left: number; top: number; width: number }[]>([]);

  const updateDrag = useCallback((d: DragState | null) => {
    dragRef.current = d;
    setDrag(d);
  }, []);

  /** 可拖块：实体块且非 extra 补课；ghost（已停/已调出）不可拖（契约 §10.3）；
   *  非本周来源块不可拖（批 7 §13.2）；custom 课（startSection 为 null）无法生成
   *  override 节次，不可拖（契约 §11.2）。 */
  const isDraggable = (b: PlacedBlock) =>
    b.ghost === null &&
    !b.nonCurrent &&
    b.override?.changeType !== "extra" &&
    b.course.startSection != null;

  /** 落点命中测试（契约 §10.2 纯前端几何，不走 grid.rs 互转）：显示列 = 天列 rect
   *  命中（网格外 clamp 到首/末列）；目标大节 = clamp(floor((y-网格顶)/ROW_H)+1,
   *  1, slots.length-跨度)（末位对齐，整块不超作息行数）。跳过日期列无效。 */
  const hitTest = (
    x: number,
    y: number,
    block: PlacedBlock,
  ): { col: number; startBlock: number } | null => {
    const rects = dragRectsRef.current;
    if (rects.length === 0) return null;
    let col = rects.findIndex((r) => x < r.left + r.width);
    if (col === -1) col = rects.length - 1; // 越过右缘 → 末列
    if (isSkippedCol(col)) return null;
    const span = block.endBlock - block.startBlock;
    const raw = Math.floor((y - rects[col].top) / ROW_H) + 1;
    const startBlock = Math.min(Math.max(1, slots.length - span), Math.max(1, raw));
    return { col, startBlock };
  };

  const onBlockPointerDown = (b: PlacedBlock, e: React.PointerEvent<HTMLButtonElement>) => {
    suppressClickRef.current = false; // 新会话起手清残留，限制 suppress 寿命
    if (busyKey === "drag") return;
    if (e.pointerType === "mouse" && e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    // 拖拽期间的命中测试快照——拖拽中滚动/resize 会用旧快照，已知取舍，
    // pointermove 按需重算 rect 是升级路径（ponytail: 单用户桌面，滚动拖拽极罕见）
    dragRectsRef.current = Array.from({ length: displayDays }, (_, i) => {
      const rect = dayColRefs.current.get(i)?.getBoundingClientRect();
      return rect
        ? { left: rect.left, top: rect.top, width: rect.width }
        : { left: Number.MAX_SAFE_INTEGER, top: 0, width: 0 };
    });
    updateDrag({
      block: b,
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      active: false,
      col: -1,
      startBlock: -1,
    });
  };

  const onBlockPointerMove = (e: React.PointerEvent<HTMLButtonElement>) => {
    const d = dragRef.current;
    if (!d || e.pointerId !== d.pointerId) return; // 非会话发起指（多指）忽略
    const dist = Math.abs(e.clientX - d.startX) + Math.abs(e.clientY - d.startY);
    if (!d.active) {
      if (dist <= DRAG_THRESHOLD) return;
      suppressClickRef.current = true; // 进入拖拽：吃掉 pointerup 后的 click
    }
    const hit = hitTest(e.clientX, e.clientY, d.block);
    updateDrag({ ...d, active: true, col: hit?.col ?? -1, startBlock: hit?.startBlock ?? -1 });
  };

  const onBlockPointerUp = async (e: React.PointerEvent<HTMLButtonElement>) => {
    const d = dragRef.current;
    if (!d || e.pointerId !== d.pointerId) return;
    updateDrag(null);
    if (!d.active) return; // 纯点击：click 正常触发详情浮层
    suppressClickRef.current = true;
    if (d.col < 0 || d.startBlock < 0) return; // 无效落点（跳过日列/网格外）：回弹不落库
    await commitDrag(d);
  };

  const onBlockPointerCancel = (e: React.PointerEvent<HTMLButtonElement>) => {
    const d = dragRef.current;
    if (!d || e.pointerId !== d.pointerId) return;
    updateDrag(null);
    if (d.active) suppressClickRef.current = true;
  };

  /** 意外丢失 pointer capture（元素移除/浏览器接管等）时复位，防 drag 常驻 active。
   *  正常 pointerup 先于本事件复位 dragRef，此处判空自然跳过、不干扰落库。 */
  const onBlockLostCapture = () => {
    if (dragRef.current) updateDrag(null);
  };

  /** 拖拽落库分叉（契约 §10.4）：多周/单周导入 → 单周 Rescheduled override
   *  （sourceNoticeId = "drag:<courseId>:<week>"，upsert 幂等键 = noticeId+courseId，
   *  同课同周反复拖拽覆盖为最后位置）；单周手动 → update_course 直改（先 revoke
   *  本周残留的 drag: 链路 override，否则渲染层仍按 override 移位、与直改冲突）。
   *  位置未变短路不产生记录。跨度按课程原始小节差保持，教室按显示值保持。 */
  const commitDrag = async (d: DragState) => {
    if (d.col < 0 || d.startBlock < 0) return; // 兜底守卫：无效落点不落库（day=0 会炸渲染）
    const b = d.block;
    const course = b.course;
    if (course.startSection == null || course.endSection == null) return; // custom 课无小节（不渲染块，理论不可达）
    const day = displayDayOf(d.col);
    const newStartSection = d.startBlock * 2 - 1; // 大节 → 小节口径（契约 §10.4）
    // 跨度按课程原始小节差保持（冻结公式，契约 §10.4）：如 3-4 节拖到第 3 大节 → 5-6
    const newEndSection = newStartSection + (course.endSection - course.startSection);
    if (day === b.day && d.startBlock === b.startBlock) return; // 位置未变短路
    setBusyKey("drag");
    setNoticeMsg(null);
    try {
      if (course.weeks.length === 1 && course.source === "manual") {
        // 不变量：只清理拖拽链路自身（drag: 前缀）的残留 override，不按通知
        // noticeId 整批删——避免 M5 公告流一文多候选时误删其他候选的调整
        const stale = (tt?.overrides ?? []).filter(
          (o) =>
            o.courseId === course.id &&
            o.weeks.includes(week) &&
            o.changeType === "rescheduled" &&
            o.sourceNoticeId.startsWith("drag:"),
        );
        for (const o of stale) {
          await invokeCommand("revoke_notice", { noticeId: o.sourceNoticeId });
        }
        const r = await invokeCommand<Course>("update_course", {
          course: {
            ...course,
            day,
            startSection: newStartSection,
            endSection: newEndSection,
            position: b.room, // 保持当前显示教室（原教室或通知已改的新教室）
          },
        });
        if (!r.success) setNoticeMsg({ ok: false, text: r.message ?? "拖拽保存失败" });
      } else {
        const candidate: NoticeCandidate = {
          noticeId: `drag:${course.id}:${week}`,
          courseId: course.id,
          courseName: course.name,
          changeType: "rescheduled",
          weeks: [week],
          newDay: day,
          newStartSection,
          newEndSection,
          newPosition: b.room,
          confidence: "low",
          reason: "拖拽调整",
          excerpt: "",
        };
        const r = await invokeCommand<CourseOverride>("apply_override", { candidate });
        if (!r.success) setNoticeMsg({ ok: false, text: r.message ?? "拖拽保存失败" });
      }
      setReloadTick((t) => t + 1);
    } finally {
      setBusyKey(null);
    }
  };

  // 拖拽中 Esc 取消（契约 §10.1）：复位状态机，suppress 吃掉随后 pointerup 的 click
  useEffect(() => {
    if (!drag?.active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        updateDrag(null);
        suppressClickRef.current = true;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [drag?.active, updateDrag]);

  // 状态机复位面：window 失焦 / 展示周或视图相位变化（网格可能卸载）时复位 drag，
  // 防止拖拽会话跨越视图变更后 drag 常驻 active
  useEffect(() => {
    const onBlur = () => {
      if (dragRef.current) updateDrag(null);
    };
    window.addEventListener("blur", onBlur);
    return () => window.removeEventListener("blur", onBlur);
  }, [updateDrag]);
  useEffect(() => {
    if (dragRef.current) updateDrag(null);
  }, [week, view.phase, updateDrag]);

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
    const r = await invokeCommand<string>("export_ics", { remindMinutes });
    setIcsBusy(false);
    if (r.success && typeof r.data === "string") {
      setIcsMsg(`已导出到 ${r.data}`);
    } else {
      setIcsMsg(r.message ?? "导出失败");
    }
  };

  // ---------------- JSON 导入导出（契约 §12：后端写下载目录 / 前端读文件传文本） ----------------

  const exportTimetableJson = async () => {
    setIcsBusy(true);
    setIcsMsg(null);
    const r = await invokeCommand<string>("export_timetable_json");
    setIcsBusy(false);
    setIcsMsg(
      r.success && typeof r.data === "string" ? `已导出到 ${r.data}` : r.message ?? "导出失败",
    );
  };

  const importTimetableJson = async (file: File) => {
    if (!window.confirm("导入将覆盖当前课表的课程与调整记录，确认继续？")) return;
    setIcsBusy(true);
    setIcsMsg(null);
    const json = await file.text();
    const r = await invokeCommand<JsonImportResult>("import_timetable_json", { json });
    setIcsBusy(false);
    if (r.success && r.data) {
      setIcsMsg(`导入完成：课程 ${r.data.courses} 门 · 调整记录 ${r.data.overrides} 条`);
      setReloadTick((t) => t + 1);
    } else {
      setIcsMsg(r.message ?? "导入失败");
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
    isCustomTime: boolean;
    customStartTime: string | null;
    customEndTime: string | null;
  }) => {
    setFormError(null);
    setBusyKey(null);
    if (editing) {
      // custom 课无节次（契约 §8.4/§11.1）；节次课自定义时刻强制清空（§11.1 不变式）
      const updated: Course = {
        ...editing,
        name: payload.name,
        teacher: payload.teacher,
        position: payload.position,
        day: payload.day,
        weeks: payload.weeks,
        colorIndex: payload.colorIndex,
        remark: payload.remark,
        startSection: payload.isCustomTime ? null : payload.startSection,
        endSection: payload.isCustomTime ? null : payload.endSection,
        isCustomTime: payload.isCustomTime,
        customStartTime: payload.customStartTime,
        customEndTime: payload.customEndTime,
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

  // ---------------- 日期作息规则保存（契约 §9.3：整体替换，null = 清空） ----------------

  const saveSlotRules = async (rules: SlotRule[] | null) => {
    setRulesBusy(true);
    setRulesErr(null);
    const r = await invokeCommand<TimetableView>("save_slot_rules", { rules });
    setRulesBusy(false);
    if (r.success && r.data) {
      setView({ phase: "ready", data: r.data });
    } else {
      setRulesErr(r.message ?? "保存失败");
    }
  };

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

  // ---------------- 跳过日期保存（契约 §8.2：整体替换，命令返回刷新后的 TimetableView） ----------------

  const saveSkippedDates = async (dates: string[]) => {
    setSkippedBusy(true);
    setSkippedErr(null);
    const r = await invokeCommand<TimetableView>("save_skipped_dates", { dates });
    setSkippedBusy(false);
    if (r.success && r.data) {
      setView({ phase: "ready", data: r.data });
    } else {
      setSkippedErr(r.message ?? "保存失败");
    }
  };

  // ---------------- 渲染 ----------------

  const weekSwitcher = ready && (
    <div className="flex flex-wrap items-center justify-end gap-1.5">
      {/* 顶栏标题态机（批 7 蓝图小件 2，契约 §13.1）：unset → 点按开设置弹层；
          before → 距开学天数；vacation → 假期；normal → 周次按钮（开选择弹层） */}
      {weekState === "unset" ? (
        <button
          type="button"
          onClick={() => {
            setSettingsErr(null);
            setSkippedErr(null);
            setSettingsOpen(true);
          }}
          className="mr-1 rounded text-body font-medium text-alert underline decoration-dotted underline-offset-4 hover:text-text"
          title="点击打开课表设置"
        >
          尚未设置开学日
        </button>
      ) : weekState === "before" ? (
        <span className="tabular-num mr-1 text-body font-medium text-text-2">
          {daysUntilStart != null ? `距离开学还有 ${Math.max(daysUntilStart, 0)} 天` : "尚未设置开学日"}
        </span>
      ) : weekState === "vacation" ? (
        <span className="mr-1 text-body font-medium text-text-2">假期</span>
      ) : (
        <div className="relative mr-1" data-week-picker>
          <button
            type="button"
            aria-haspopup="dialog"
            aria-expanded={weekPickerOpen}
            onClick={() => setWeekPickerOpen((o) => !o)}
            className="tabular-num rounded text-body font-medium text-text-2 hover:text-text"
            title="点击选择周次"
          >
            第 {week} 周 / 共 {totalWeeks} 周
          </button>
          {weekPickerOpen && (
            <div
              role="dialog"
              aria-label="选择周次"
              className="absolute right-0 top-8 z-40 rounded-card border border-line bg-surface p-3 shadow-pop"
            >
              <div className="grid grid-cols-10 gap-1">
                {Array.from({ length: totalWeeks }, (_, i) => i + 1).map((w) => (
                  <button
                    key={w}
                    type="button"
                    aria-label={`第 ${w} 周${w === currentWeek ? "（本周）" : ""}`}
                    aria-current={w === currentWeek ? "date" : undefined}
                    onClick={() => {
                      setViewWeek(w);
                      setWeekPickerOpen(false);
                    }}
                    className={cn(
                      "tabular-num size-7 rounded-full text-caption leading-none text-text-2 hover:bg-sched/10",
                      w === currentWeek && "bg-sched font-medium text-white hover:bg-sched",
                      w === week && "ring-2 ring-sched ring-offset-1 ring-offset-surface",
                    )}
                  >
                    {w}
                  </button>
                ))}
              </div>
            </div>
          )}
        </div>
      )}
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
      {/* ICS 课前提醒（契约 §12.3）：无/15/30/60，随「导出 ICS」按钮传参 */}
      <select
        aria-label="课前提醒"
        value={remindMinutes ?? 0}
        onChange={(e) => setRemindMinutes(Number(e.target.value) || null)}
        className="h-8 rounded-control border border-line bg-surface px-2 text-caption text-text"
      >
        <option value={0}>无提醒</option>
        <option value={15}>提前 15 分钟</option>
        <option value={30}>提前 30 分钟</option>
        <option value={60}>提前 60 分钟</option>
      </select>
      <Button variant="outline" size="sm" disabled={icsBusy} onClick={exportIcs}>
        <Download aria-hidden="true" className="size-3.5" />
        导出 ICS
      </Button>
      <Button variant="outline" size="sm" disabled={icsBusy} onClick={exportTimetableJson}>
        <Download aria-hidden="true" className="size-3.5" />
        导出 JSON
      </Button>
      <Button
        variant="outline"
        size="sm"
        disabled={icsBusy}
        onClick={() => importJsonRef.current?.click()}
      >
        <Upload aria-hidden="true" className="size-3.5" />
        导入 JSON
      </Button>
      <input
        ref={importJsonRef}
        type="file"
        accept=".json,application/json"
        className="hidden"
        onChange={async (e) => {
          const f = e.target.files?.[0];
          e.target.value = ""; // 允许重复选择同一文件
          if (f) await importTimetableJson(f);
        }}
      />
      <Button variant="outline" size="sm" onClick={() => { setSlotsErr(null); setSlotsOpen(true); }}>
        <Clock aria-hidden="true" className="size-3.5" />
        作息
      </Button>
      <Button variant="outline" size="sm" onClick={() => { setSettingsErr(null); setSkippedErr(null); setSettingsOpen(true); }}>
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

          {/* 未设置开学日提示（批 7 §13.1：仅 unset 态显示——vacation/before 由顶栏文案承载，
              currentWeek 越界为 null 不再误报「未设置」） */}
          {weekState === "unset" && (
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
              className={cn(
                "grid min-w-[640px]",
                drag?.active && (drag.col < 0 ? "cursor-not-allowed select-none" : "cursor-grabbing select-none"),
              )}
              style={{ gridTemplateColumns: `56px repeat(${displayDays}, minmax(0, 1fr))` }}
            >
              {/* 表头行：列头从 firstDay 起旋转（firstDay=7 → 周日起） */}
              <div className="border-b border-line" />
              {Array.from({ length: displayDays }, (_, i) => {
                const day = displayDayOf(i);
                const skipped = isSkippedCol(i);
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
                      <p
                        className={cn(
                          "tabular-num text-caption",
                          skipped ? "text-text-2/40 line-through" : "opacity-70",
                        )}
                      >
                        {fmtDay(weekDates[i]!)}
                      </p>
                    )}
                    {skipped && (
                      <p className="mt-0.5 inline-block rounded bg-line px-1 text-caption font-medium text-text-2">
                        休
                      </p>
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
                    {/* 节次别名（批 5 §11.3）：有 alias 时节号下显示小字 */}
                    {s.alias && (
                      <span className="max-w-[52px] truncate text-caption text-sched" title={s.alias}>
                        {s.alias}
                      </span>
                    )}
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
                const skipped = isSkippedCol(i);
                const col = skipped ? [] : weekBlocks.columns[day - 1];
                const layout = layouts[day - 1];
                return (
                  <div
                    key={day}
                    ref={(el) => {
                      // 天列 DOM：拖拽命中测试用（显示列下标 → rect 快照）
                      if (el) dayColRefs.current.set(i, el);
                      else dayColRefs.current.delete(i);
                    }}
                    className={cn(
                      "relative border-line",
                      i > 0 && "border-l",
                      i === todayCol && "bg-sched/5",
                      drag?.active && drag.col === i && "bg-sched/15", // 落点列高亮（契约 §10.3）
                    )}
                    style={{ height: ROW_H * slots.length }}
                  >
                    {/* 跳过日期列（契约 §8.1）：课程与空位按钮都不渲染，居中「休」标 */}
                    {skipped ? (
                      <p className="absolute inset-0 flex items-center justify-center text-caption text-text-2/50">
                        休
                      </p>
                    ) : (
                      <>
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
                        {/* 拖拽落点预览：按原跨度画虚线 ghost div（契约 §10.3） */}
                        {drag?.active && drag.col === i && drag.startBlock >= 1 && (
                          <div
                            aria-hidden
                            className="pointer-events-none absolute left-0 w-full rounded-inner border-2 border-dashed border-sched bg-sched/10"
                            style={{
                              top: (drag.startBlock - 1) * ROW_H + 2,
                              height:
                                (drag.block.endBlock - drag.block.startBlock + 1) * ROW_H - 4,
                            }}
                          />
                        )}
                      </>
                    )}
                    {col.map((b) => {
                      const pos = layout.get(b.key) ?? { lane: 0, lanes: 1 };
                      const color = courseColor(b.course, customColors);
                      const top = (b.startBlock - 1) * ROW_H + 2;
                      const height = (b.endBlock - b.startBlock + 1) * ROW_H - 4;
                      const draggable = isDraggable(b);
                      const dragging = drag?.active === true && drag.block.key === b.key;
                      const dragInvalid = dragging && drag.col < 0; // 无效落点 = 不可放置态
                      return (
                        <button
                          key={b.key}
                          type="button"
                          data-course-block
                          ref={(el) => {
                            if (el) blockRefs.current.set(b.key, el);
                            else blockRefs.current.delete(b.key);
                          }}
                          aria-label={`${b.course.name}，${DAY_NAMES[b.day]}第${b.startBlock}至${b.endBlock}大节${b.ghost ? `（${b.ghost === "cancelled" ? "已停" : "已调出"}）` : ""}${b.nonCurrent ? "（非本周）" : ""}${draggable ? "，可拖拽调整位置" : ""}`}
                          onClick={() => {
                            // 拖拽结束/取消后的 click 必须吃掉（pointer capture 后 click
                            // 仍触发，契约 §10.1）；未进入拖拽的纯点击照常开详情
                            if (suppressClickRef.current) {
                              suppressClickRef.current = false;
                              return;
                            }
                            openBlockDetail(b);
                          }}
                          onPointerDown={draggable ? (e) => onBlockPointerDown(b, e) : undefined}
                          onPointerMove={draggable ? onBlockPointerMove : undefined}
                          onPointerUp={draggable ? (e) => void onBlockPointerUp(e) : undefined}
                          onPointerCancel={draggable ? onBlockPointerCancel : undefined}
                          onLostPointerCapture={draggable ? onBlockLostCapture : undefined}
                          className={cn(
                            "absolute overflow-hidden rounded-inner px-1.5 py-1 text-left transition-shadow duration-[var(--dur-fast)] ease-out-soft hover:shadow-card focus-visible:shadow-card",
                            b.ghost ? "border border-dashed" : "border",
                            draggable && "cursor-grab touch-none", // touch-none：拖拽不被触屏滚动吞掉
                            dragging && "opacity-40",
                            // 非本周降级（批 7 §13.2）：只作用 solid 块，ghost 样式优先
                            b.nonCurrent && !b.ghost && !dragging && "opacity-40",
                            dragInvalid && "cursor-not-allowed ring-2 ring-alert", // 不可放置反馈（契约 §10.3 复核采纳）
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
                const color = courseColor(c, customColors);
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
                        {DAY_NAMES[c.day]} {courseTimeLabel(c)}
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
                                {o.sourceNoticeId.startsWith("drag:") && (
                                  <span className="rounded bg-line px-1 text-caption text-text-2">拖拽</span>
                                )}{" "}
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

          {/* 作息时间表编辑弹层（打开时快照当前生效作息与日期规则） */}
          {slotsOpen && ready && tt && (
            <SlotsEditor
              initial={ready.slots}
              initialRules={tt.config.slotRules}
              usingCustom={tt?.config.slots != null && tt.config.slots.length > 0}
              busy={slotsBusy}
              rulesBusy={rulesBusy}
              error={slotsErr}
              rulesError={rulesErr}
              onSave={(slots) => void saveSlots(slots)}
              onSaveRules={(rules) => void saveSlotRules(rules)}
              onReset={() => void resetSlots()}
              onClose={() => setSlotsOpen(false)}
            />
          )}

          {/* 课表设置弹层（契约 §7.1 + §8 跳过日期区块） */}
          {settingsOpen && ready && tt && (
            <SettingsEditor
              initial={{
                semesterStartDate: tt.config.semesterStartDate,
                semesterTotalWeeks: tt.config.semesterTotalWeeks,
                firstDayOfWeek: tt.config.firstDayOfWeek,
                showWeekends: tt.config.showWeekends,
                showNonCurrentWeek: tt.config.showNonCurrentWeek,
              }}
              initialSkippedDates={tt.config.skippedDates}
              busy={settingsBusy}
              skippedBusy={skippedBusy}
              error={settingsErr}
              skippedError={skippedErr}
              onSave={(input) => void saveSemesterConfig(input)}
              onSaveSkippedDates={(dates) => void saveSkippedDates(dates)}
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
                            {o.sourceNoticeId.startsWith("drag:") && (
                              <span className="rounded bg-line px-1 text-caption text-text-2">拖拽</span>
                            )}{" "}
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
                      style={{ backgroundColor: courseColor(c, customColors) }}
                    />
                    <p className="min-w-0 truncate text-body font-medium text-text">{c.name}</p>
                    {c.disabled && (
                      <span className="shrink-0 rounded bg-line px-1 text-caption text-text-2">已停开</span>
                    )}
                    <span className="tabular-num min-w-0 truncate text-caption text-text-2">
                      {DAY_NAMES[c.day]} {courseTimeLabel(c)} · 第 {fmtWeeks(c.weeks)} 周
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
