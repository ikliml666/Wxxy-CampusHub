import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  CalendarRange,
  ChevronLeft,
  ChevronRight,
  ClipboardPaste,
  Download,
  Pencil,
  Plus,
  RefreshCw,
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
  TimetableView,
} from "@/shared/types";

const DAY_HEADERS = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"] as const;

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

/** 单大节行高（px）。5 大节 = 360px 网格主体。 */
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

    // 停课优先：该周该次被停（newDay 未提及时按该课当次全停处理）
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

  const weekBlocks = useMemo(
    () => (ready ? buildWeekBlocks(ready, week) : { blocks: [], columns: Array.from({ length: 7 }, () => []) }),
    [ready, week],
  );
  const layouts = useMemo(
    () => weekBlocks.columns.map((col) => layoutColumn(col)),
    [weekBlocks],
  );

  /** 视图周各列日期（开学日锚定；未设置开学日为 null，列头只显示星期）。 */
  const weekDates: (Date | null)[] = useMemo(() => {
    const start = tt?.config.semesterStartDate ? parseDay(tt.config.semesterStartDate) : null;
    if (!start) return Array.from({ length: 7 }, () => null);
    return Array.from({ length: 7 }, (_, i) => {
      const d = new Date(start);
      d.setDate(d.getDate() + (week - 1) * 7 + i);
      return d;
    });
  }, [tt?.config.semesterStartDate, week]);
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
    const r = await invokeCommand<string>("export_ics");
    setIcsBusy(false);
    if (!r.success || typeof r.data !== "string") {
      setIcsMsg(r.message ?? "导出失败");
      return;
    }
    const url = URL.createObjectURL(new Blob([r.data], { type: "text/calendar;charset=utf-8" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = "课表.ics";
    a.click();
    URL.revokeObjectURL(url);
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
              style={{ gridTemplateColumns: "56px repeat(7, minmax(0, 1fr))" }}
            >
              {/* 表头行 */}
              <div className="border-b border-line" />
              {DAY_HEADERS.map((d, i) => (
                <div
                  key={d}
                  className={cn(
                    "border-b border-line py-2 text-center",
                    i === todayCol ? "bg-sched/5 font-medium text-sched" : "text-text-2",
                    i > 0 && "border-l border-line",
                  )}
                >
                  <p className="text-caption">{d}</p>
                  {weekDates[i] && (
                    <p className="tabular-num text-caption opacity-70">{fmtDay(weekDates[i]!)}</p>
                  )}
                </div>
              ))}
              {/* 时间列：一律取后端 slots（校本大节作息），前端不硬编码时间 */}
              <div>
                {slots.map((s) => (
                  <div
                    key={s.number}
                    className="flex flex-col items-center justify-center border-b border-line px-1 text-center last:border-b-0"
                    style={{ height: ROW_H }}
                  >
                    <span className="tabular-num text-caption font-medium text-text-2">
                      {s.number}
                    </span>
                    <span className="tabular-num text-caption opacity-60 text-text-2">
                      {s.startTime}
                    </span>
                  </div>
                ))}
              </div>
              {/* 7 天列 */}
              {weekBlocks.columns.map((col, dayIdx) => {
                const layout = layouts[dayIdx];
                return (
                  <div
                    key={dayIdx}
                    className={cn(
                      "relative border-line",
                      dayIdx > 0 && "border-l",
                      dayIdx === todayCol && "bg-sched/5",
                    )}
                    style={{ height: ROW_H * slots.length }}
                  >
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
                          <p className="tabular-num truncate text-caption leading-tight text-text-2">
                            {b.room}
                          </p>
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
