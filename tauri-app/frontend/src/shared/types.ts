export type CommandResult<T> = { success: boolean; message?: string; data?: T };

// ---------- M2 批次 1：get_portal_overview（tauri commands/portal.rs::PortalOverview） ----------

/** 学期与当前周（服务端字段均为字符串）。 */
export interface SemesterInfo {
  grade: string;
  semester: string;
  currentWeek: string;
  weekCount: string;
  /** "YYYYMMDD" */
  startDate: string;
  endDate: string;
  currentWeekDay: string;
}

/** 钱包三卡摘要；单项取失败为 null（前端显示 "—"，不伪造数字）。 */
export interface WalletSummary {
  cardBalance: number | null;
  bookBorrowed: number | null;
  mailUnread: number | null;
}

/** 「下一节课」简报（格子第 4 段教师名后端已丢弃）。 */
export interface CourseBrief {
  name: string;
  room: string;
  teachingClass: string;
  /** 1-based 节次号 */
  slot: number;
  /** "HH:MM"，默认节次表查不到为 null */
  startTime: string | null;
}

/** get_portal_overview → data；子字段取失败不阻塞其余（null 即空态）。 */
export interface PortalOverview {
  semester: SemesterInfo | null;
  wallet: WalletSummary | null;
  nextCourse: CourseBrief | null;
  fetchedAt: number;
}

// ---------- M2 批次 2：资讯页 / 待办页（tauri commands/portal.rs，契约冻结于计划 §2.1） ----------

/** 资讯栏目（后端 = 订阅接口 + 实测全量兜底，固定 7 栏）。 */
export interface InfoColumn {
  id: string;
  name: string;
  sortNum: number;
}

/** 资讯条目（仅契约字段；dept 服务端可能为 null）。 */
export interface InfoItem {
  id: string;
  title: string;
  columnTitle: string;
  /** "YYYY-MM-DD HH:MM:SS" */
  publishTime: string;
  dept: string | null;
  /** 官网正文页 URL（抓取经后端域名白名单校验） */
  url: string;
}

/**
 * 资讯分页。⚠️ total/pageCount 实测不可靠（pageSize=1 时返回 0）原样透传，
 * **分页以 items.length == pageSize 判断可能有下一页**。
 */
export interface InfoPage {
  page: number;
  pageSize: number;
  pageCount: number;
  total: number;
  items: InfoItem[];
}

/**
 * 资讯正文（计划契约的兼容扩展）。后端已完成白名单清洗，前端直接渲染、
 * 不再二次清洗；`needsBrowser == true` 时正文受官网鉴权保护（html 为 null），
 * 前端引导在浏览器中打开 `url`，**不显示错误态/重试**。
 */
export interface InfoDetail {
  title: string;
  html: string | null;
  needsBrowser: boolean;
  url: string;
}

/** 待办分栏（接口实际返回 6 个 tab；前端按契约只展示 todo/done/apply 三栏）。 */
export interface TodoTab {
  id: string;
  name: string;
  desc: string;
  count: number;
}

/** 待办条目（字段形态未实测——账号无待办数据，后端按候选键映射，缺省空串）。 */
export interface TodoItem {
  id: string;
  title: string;
  applicant: string;
  applyTime: string;
  source: string;
  node: string;
  urgency: string;
}

/** 待办分页（分页字段口径同 InfoPage：total/pageCount 不可靠）。 */
export interface TodoPage {
  page: number;
  pageSize: number;
  pageCount: number;
  total: number;
  items: TodoItem[];
}

// PanelId 现为 9 项（2026-09-18 M2.5 批次 4 追加 "timetable"）：三处同步 = 本类型 +
// DockNav DOCK_ITEMS + AppShell PANEL_MAP；uiStore persist 的 migrate 校验非法值兜底。
export type PanelId =
  | "today"
  | "timetable"
  | "info"
  | "todo"
  | "schedule"
  | "apps"
  | "wallet"
  | "power"
  | "settings";

// ---------- M2 批次 3：应用页 / 日程页（tauri commands/portal.rs，契约冻结于计划 §2.1） ----------

/** 应用可达性（后端按附录 A 实测表按 host 推导，不信任门户 isCas；未命中回落 external）。 */
export type AppAccess = "cas" | "webvpn" | "external" | "unavailable";

/** 门户应用条目；iconUrl 为后端代拉并转好的 data URL，失败/无图标为 null。 */
export interface AppItem {
  id: string;
  name: string;
  iconUrl: string | null;
  /** 服务端下发的应用链接（open_app 后端按协议白名单校验） */
  link: string;
  isCas: boolean;
  showType: string;
  /** 可达性分类：cas=直达 / webvpn=需校园网或 WebVPN / external=外链 / unavailable=暂不可用 */
  access: AppAccess;
}

/** 应用分组（部门维度，name 即部门名）。 */
export interface AppGroup {
  id: string;
  name: string;
  apps: AppItem[];
}

/** 应用目录（pinned = queryMyStore 收藏/常用，契约的兼容扩展）。 */
export interface AppCatalog {
  groups: AppGroup[];
  pinned: AppItem[];
}

/** 日程分类（实测 5 类；color 为服务端给的色值，过滤 chip 与日程块色标直接用）。 */
export interface ScheduleClassify {
  name: string;
  code: string;
  color: string;
}

/** 日程条目（毫秒时间戳；classifyName/color 由后端按分类 code 映射补全）。 */
export interface ScheduleEvent {
  id: string;
  title: string;
  startMs: number;
  endMs: number;
  place: string;
  classifyCode: string;
  classifyName: string;
  color: string;
  /** 附加信息（会议的主持人/参会人员/承办单位拼接文本；其余来源为 null）。 */
  extra: string | null;
}

/**
 * 每日日程计数（月视图角标）。⚠️ bs-schedule 接口无分类过滤参数，计数为当日
 * 全量日程数（如实呈现服务端计数）。
 */
export interface ScheduleDayCount {
  day: string;
  count: number;
}

// ---------- M2.5：课表页（tauri commands/timetable.rs，契约冻结于计划 §2.1–§2.5；
// 镜像 crates/campus-schedule/src/model.rs 与 src/notice.rs 的 camelCase 序列化） ----------

/** 课程来源：导入（自动更新可触碰）/ 手动添加（永不触碰）。 */
export type CourseSource = "import" | "manual";

/** 一门课程（同一课程多周复用同一条记录，周次见 weeks）。 */
export interface Course {
  id: string;
  courseTableId: string;
  name: string;
  teacher: string;
  position: string;
  /** 星期几，1=周一 … 7=周日 */
  day: number;
  /** 起始/结束小节（1-based 教务小节号）；导入课程恒有值 */
  startSection: number | null;
  endSection: number | null;
  isCustomTime: boolean;
  customStartTime: string | null;
  customEndTime: string | null;
  /** 卡片颜色索引：导入课程为课名哈希大数，**取色必须 % 色板长度** */
  colorIndex: number;
  /** 导入课程 = 课程性质·考核方式（kcxz·khfsmc）；手动课程 = 用户备注 */
  remark: string | null;
  source: CourseSource;
  /** 出现周次（1-based 显式列表） */
  weeks: number[];
  /** 教学班 ID（正方 jxb_id，diff 匹配键之一） */
  classId: string | null;
  /** 「已停开」：导入课程消失时后端置 true（不删记录）；手动课程永不置位 */
  disabled: boolean;
}

/** 课表配置。 */
export interface CourseTableConfig {
  courseTableId: string;
  showWeekends: boolean;
  /** 学期开学日 "YYYY-MM-DD"（周次锚点；null = 未设置） */
  semesterStartDate: string | null;
  semesterTotalWeeks: number;
  /** 一周起始日：1=周一 … 7=周日 */
  firstDayOfWeek: number;
  /** 自定义作息（M2.5 收尾轮追加）：null/空 = 内置校本大节表；有值 = 用户编辑过的作息（唯一事实源） */
  slots: TimeSlot[] | null;
  /** 跳过日期（2026-09-19 批 2 契约 §8.1）：全校性停课日 "YYYY-MM-DD" 列表；
   *  网格该列不渲染课程 +「休」徽标，ICS 剔除该日 VEVENT */
  skippedDates: string[];
  /** 按日期生效的作息规则（2026-09-19 批 3 契约 §9.1）：区间含端点、重叠取先声明；
   *  无命中回落 slots → 内置 */
  slotRules: SlotRule[];
}

/** 按日期区间的作息规则（契约 §9.1）。slots 恒非空（保存时校验）。 */
export interface SlotRule {
  /** 生效起始日期（含）"YYYY-MM-DD" */
  startDate: string;
  /** 生效结束日期（含）"YYYY-MM-DD" */
  endDate: string;
  slots: TimeSlot[];
}

/** 调整类型：调课 / 停课 / 补课（Rust OverrideKind snake_case）。 */
export type OverrideKind = "rescheduled" | "cancelled" | "extra";

/** 单次调课叠加（叠加于导入课程之上，原数据保留；撤销按 sourceNoticeId 整批）。 */
export interface CourseOverride {
  id: string;
  courseId: string;
  /** 生效周次（1-based） */
  weeks: number[];
  changeType: OverrideKind;
  /** 调课/补课 = 新时间；停课 = 被停那次的星期（供定位），通知未提及时 null */
  newDay: number | null;
  newStartSection: number | null;
  newEndSection: number | null;
  newPosition: string | null;
  /** 来源通知 ID（撤销与去重键） */
  sourceNoticeId: string;
  /** 高置信自动应用 = true；低置信人工采纳 = false */
  autoApplied: boolean;
}

/** 一份本地课表（timetable.json 顶层结构）。 */
export interface Timetable {
  config: CourseTableConfig;
  courses: Course[];
  overrides: CourseOverride[];
  /** 最近更新时刻 RFC3339；空串 = 从未更新 */
  updatedAt: string;
}

/** 节次时间段（后端下发，前端不得硬编码时间）。 */
export interface TimeSlot {
  number: number;
  /** "HH:MM" */
  startTime: string;
  endTime: string;
  alias: string | null;
}

/** get_timetable → data（批次 4 修订契约 §2.3）。 */
export interface TimetableView {
  timetable: Timetable;
  /** 生效作息（自定义优先、回落内置校本大节表），时间标签唯一事实源；行数不固定，按 slots.len() 渲染 */
  slots: TimeSlot[];
  /** 当前教学周；null = 未设置开学日或今天不在学期内 */
  currentWeek: number | null;
  /** 本机今天 "YYYY-MM-DD" */
  today: string;
}

/** import_timetable → data（变更摘要，changes 为人类可读条目）。 */
export interface ImportResult {
  added: number;
  changed: number;
  removed: number;
  /** 合并后本地课程总数（含已停开保留记录） */
  total: number;
  changes: string[];
}

/** add_course_manual 入参（冻结契约 §2.3）。 */
export interface ManualCourseInput {
  name: string;
  teacher: string;
  position: string;
  /** 1=周一 … 7=周日 */
  day: number;
  startSection: number;
  endSection: number;
  /** 出现周次（1-based） */
  weeks: number[];
  /** 前端色板下标 */
  colorIndex: number;
  remark?: string | null;
}

/** 解析置信度。high = 要素齐全且课程名唯一精确匹配（自动应用）；low = 待确认。 */
export type NoticeConfidence = "high" | "low";

/** save_semester_config 入参（2026-09-19 批 1 修订 §7.1）。
 *  Rust 侧容器级 serde default：前端可省略 currentWeekHint。 */
export interface SemesterConfigInput {
  /** "YYYY-MM-DD" | null（null = 清空开学日即假期态）；被 currentWeekHint 反推覆盖 */
  semesterStartDate: string | null;
  /** 1..=30 */
  semesterTotalWeeks: number;
  /** 一周起始日：1=周一 … 7=周日 */
  firstDayOfWeek: number;
  showWeekends: boolean;
  /** 「今天是第 N 周」手动锚点：后端按此反推开学日（口径单点在后端） */
  currentWeekHint?: number | null;
}

/** 调课通知候选（parse_notice → data，不入库）。
 *  ⚠️ Rust 侧 Option 字段 skip_serializing_if 缺省省略 → TS 用可选属性（非 null）。 */
export interface NoticeCandidate {
  noticeId: string;
  courseId?: string;
  courseName: string;
  changeType: OverrideKind;
  weeks: number[];
  newDay?: number;
  newStartSection?: number;
  newEndSection?: number;
  newPosition?: string;
  confidence: NoticeConfidence;
  /** 降级原因（high 时为空串） */
  reason: string;
  /** 原文摘录（命中课程所在行） */
  excerpt: string;
}
