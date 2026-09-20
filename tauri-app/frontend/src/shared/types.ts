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

// ---------- 批 9：get_today_courses（tauri commands/timetable.rs::TodayCoursesView） ----------

/** 今日单节课（ongoing/next 由后端按本机时钟算好下发，前端不自算时钟）。 */
export interface TodayCourse {
  courseId: string;
  name: string;
  room: string;
  teacher: string;
  /** "HH:MM" */
  startHm: string;
  /** "HH:MM" */
  endHm: string;
  /** start <= now < end（恰在开始时刻 = 进行中） */
  ongoing: boolean;
}

/** get_today_courses → data；hasLocal=false（未导入/未配置）回落门户 nextCourse。 */
export interface TodayCoursesView {
  /** "YYYY-MM-DD" */
  date: string;
  currentWeek: number | null;
  /** "normal" | "no_semester" | "vacation" | "skipped" */
  state: string;
  /** 本地课表已导入且有课程（课程数 > 0 且已设开学日） */
  hasLocal: boolean;
  /** 按开始时刻升序；停课实例不进列表 */
  courses: TodayCourse[];
  /** 第一门未结束（end > now）；全部已结束为 null */
  next: TodayCourse | null;
  /** 今日是置换日：值 = 被补日星期（1=周一…7=周日），前端显示「补周X课」 */
  swapWeekday: number | null;
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

// PanelId 现为 9 项（M5：追加 "notifications" 通知中心；M4.5：原 "wallet" + "power" 合并为 "ecard"）：
// 四处同步 = 本类型 + DockNav DOCK_ITEMS + AppShell PANEL_MAP + uiStore 的 PANEL_IDS；
// uiStore persist 的 migrate 负责把旧值 "wallet"/"power" 迁移到 "ecard"。
export type PanelId =
  | "today"
  | "timetable"
  | "info"
  | "todo"
  | "schedule"
  | "apps"
  | "ecard"
  | "notifications"
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
  /** 非本周课程降级显示（2026-09-19 批 7 契约 §13.2）：true = 网格同时渲染非展示周
   *  课程（40% 透明度、可点详情、不可拖）；false/旧数据缺省 = 隐藏 */
  showNonCurrentWeek: boolean;
  /** 置换日（2026-09-19 节假日轮契约 §22）：该日期按 weekday 的课表执行
   *  （调休补课）。网格该列显示 weekday 列课程 +「班」徽标 */
  swapDays: SwapDay[];
  /** 法定节假日名（timor.tech 拉取，仅显示用） */
  holidayNames: NamedDate[];
}

/** 置换日（契约 §22）：某日期按某星期的课表执行。sourceNoticeId = 公告撤销键。 */
export interface SwapDay {
  /** "YYYY-MM-DD" */
  date: string;
  /** 1=周一 … 7=周日 */
  weekday: number;
  sourceNoticeId: string | null;
}

/** 法定节假日名（契约 §22，仅显示用；无课判定以 skippedDates 为准）。 */
export interface NamedDate {
  date: string;
  name: string;
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

/** 顶栏标题态（2026-09-19 批 7 契约 §13.1）：后端按 weeks.rs 对齐式周次口径判定。 */
export type WeekState = "unset" | "before" | "vacation" | "normal";

/** get_timetable → data（批次 4 修订契约 §2.3 + 重设计轮批 A 契约 §18 小节化）。 */
export interface TimetableView {
  timetable: Timetable;
  /** 生效作息（小节口径，重设计轮批 A §18）：slot_rules 命中 → config.slots →
   *  内置 11 小节表；时间列与小节网格坐标的唯一事实源（sectionSlots 字段已删） */
  slots: TimeSlot[];
  /** 当前教学周；null = 未设置开学日或今天不在学期内 */
  currentWeek: number | null;
  /** 顶栏标题态（批 7 §13.1）：细分 currentWeek 为 null 的原因 */
  weekState: WeekState;
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

/** import_timetable_json → data（契约 §12.2；与 diff 语义的 ImportResult 是两个结构）。 */
export interface JsonImportResult {
  /** 导入课程数 */
  courses: number;
  /** 导入调课/停课/补课记录数 */
  overrides: number;
}

/** move_day_courses → data（2026-09-19 批 8 契约 §14.1：调课搬迁，统一走 override）。 */
export interface MoveResult {
  /** 本次搬迁的课程数（= 生成/覆盖的 override 数） */
  moved: number;
  /** 整批撤销句柄：revoke_notice(noticeId) 一次撤销本批全部搬迁 */
  noticeId: string;
}

/** add_course_manual 入参（冻结契约 §2.3 + 批 5 §11.1）。
 *  ⚠️ Rust 侧新增字段 serde default：旧调用方可省略 isCustomTime/customStartTime/customEndTime。 */
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
  /** 「按时刻」模式（批 5 §11.1）：true 时后端忽略节次、落库节次 null */
  isCustomTime?: boolean;
  /** "HH:MM"；custom 模式必填 */
  customStartTime?: string | null;
  /** "HH:MM"；custom 模式必填 */
  customEndTime?: string | null;
}

/** 解析置信度。high = 要素齐全且课程名唯一精确匹配（自动应用）；low = 待确认。 */
export type NoticeConfidence = "high" | "low";

/** save_semester_config 入参（2026-09-19 批 1 修订 §7.1 + 修复轮 §17）。
 *  Rust 侧容器级 serde default：前端可省略 currentWeekHint。
 *  §17 起：开学日/总周数随教务导入自动维护——前端保存设置时原样回传当前值、
 *  不再传 currentWeekHint；后端契约保持兼容（showWeekends/firstDayOfWeek/
 *  showNonCurrentWeek 仍经此命令落库）。 */
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
  /** 非本周课程降级显示（批 7 §13.2）：缺省 = 保留旧值（容忍旧调用方） */
  showNonCurrentWeek?: boolean | null;
}

/** 公告简报（list_schedule_notices → data，重设计轮批 B 契约 §19；镜像
 *  crates/campus-portal/src/lib.rs::ScheduleNoticeBrief 的 camelCase 序列化）。 */
export interface ScheduleNoticeBrief {
  title: string;
  /** "YYYY-MM-DD HH:MM:SS"（列表原样透传，倒序排序键） */
  date: string;
  /** 官网正文页 URL（parse_notice_from_url 的入参） */
  url: string;
  /** 所属栏目名（服务端 columnTitle，缺失回落扫描常量表名） */
  column: string;
  /** 标题命中的关键词（强词在前；展示着色用，命中判定在后端） */
  matchedKeywords: string[];
}

/** 一键自动解析结果（auto_parse_notices → data；契约 §21）：
 *  NoticeAutoParse 镜像 Rust camelCase 序列化，error 非 null 时 candidates 为空。 */
export interface NoticeAutoParse {
  title: string;
  date: string;
  url: string;
  column: string;
  error: string | null;
  candidates: NoticeCandidate[];
  /** 全校日期置换候选（契约 §22）：命中置换格式时非空（candidates 为空） */
  swaps: NoticeSwapCandidate[];
}

/** 全校日期置换候选（契约 §22）：采纳走 apply_swap_day 写 config.swapDays。 */
export interface NoticeSwapCandidate {
  noticeId: string;
  /** 上课日 "YYYY-MM-DD" */
  date: string;
  /** 被补日星期（1=周一…7=周日）；null = 缺星期（Low，不可采纳） */
  weekday: number | null;
  confidence: "high" | "low";
  reason: string;
  excerpt: string;
  sourceTitle: string;
}
export interface NoticeAutoParse {
  title: string;
  date: string;
  url: string;
  column: string;
  error: string | null;
  candidates: NoticeCandidate[];
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

// ---------- M3 批 2：一卡通（tauri commands/synjones.rs，计划 §2.4/§2.5；后端已把单位换算成元） ----------

/**
 * 一张一卡通。**余额口径**（后端批 1 live 实测修订）：
 * `elecAccamtYuan`（电子账户，`elec_accamt`）是页面主口径——`elec` 指 electronic，
 * **不是电费余额**；`balanceYuan` 是卡账户 `(db_balance + unsettle_amount)/100`。
 */
export interface CardInfo {
  account: string;
  cardname: string;
  /** 卡账户余额（元） */
  balanceYuan: number;
  /** 电子账户余额（元） */
  elecAccamtYuan: number;
  statusLabel: string;
}

/** 一条流水（`amountYuan` 带符号：收入为正、支出为负）。 */
export interface EcardTransaction {
  /** 服务端原文 "YYYY-MM-DD HH:MM:SS" */
  time: string;
  summary: string;
  amountYuan: number;
  isIncome: boolean;
  payName: string;
  locationName: string;
  /** 流水单号（`orderId`）——账单详情按它单查一条 */
  orderId: string;
  /** 分类 id（实测 1 消费 / 2 充值 / 3 退款 / 4 扫码付 / 5 补贴，字典见 `get_ecard_types`） */
  typeId: string;
  /** 分类中文名（服务端已给，如「消费」） */
  turnoverType: string;
  /** 标签（常为空串） */
  labelName: string;
  /** 标签备注（常为空串） */
  labelRemark: string;
  /**
   * **该笔交易后的余额快照**（元）。服务端未给时为 `null`（**不是 0**）——
   * 这是「某时刻余额」的可回溯来源，与电费自采快照同语义。
   */
  cardBalanceYuan: number | null;
}

/** 流水一页（不带方向过滤 = 全量；`total` 供「加载更多」判断）。 */
export interface EcardTransactions {
  total: number;
  records: EcardTransaction[];
}

/**
 * get_ecard → data。`balanceYuan` 为电子账户（主数字）、`cardBalanceYuan` 为卡账户。
 * ⚠️ `todaySpend` / `monthSpend` 目前恒 `null`：慧新E校
 * `statistics/turnover/sum/user` 五参齐备仍恒返回空数据（2026-09-19 live 实测），
 * 前端按「暂不可用」呈现，**不伪造数字**。
 */
export interface EcardOverview {
  balanceYuan: number;
  cardBalanceYuan: number;
  account: string;
  cards: CardInfo[];
  todaySpend: number | null;
  monthSpend: number | null;
}

/** 钱包卡数据来源：慧新E校实时 / 门户快照 / 两者都失败。 */
export type WalletSource = "realtime" | "portal" | "none";

/** get_wallet_cards → data（首页钱包卡：后端聚合 + 门户降级，计划 §2.4）。 */
export interface WalletCards {
  ecard: { valueYuan: number | null; source: WalletSource; updatedAt: string };
  mail: { unread: number | null };
  library: { borrowed: number | null };
  /** 电费余额：走 `/charge/*` 级联（批 3 接入），当前恒 `null` + `source:"none"`。 */
  elec: { valueYuan: number | null; source: WalletSource };
}

// ─────────────── 电费（M3 批 3，`campus_synjones::charge` 契约） ───────────────

/**
 * 一个缴费片区（`list_feeitems` → data）。
 * ⚠️ `maxmoney` 是**服务端字段原文拼写**（非 camelCase 的 `maxMoney`），与后端 serde 保持一致。
 */
export interface FeeItem {
  id: string;
  name: string;
  billingUnit: string;
  /** 快捷金额档（`"10,50,100"` → `[10,50,100]`） */
  layout: number[];
  /** 单次充值下限（元），服务端为字符串形态 */
  retainMoney: number | null;
  maxmoney: number | null;
  remark: string;
  /**
   * 末级是否为**输入级**（官方 `flag[4]=='3'`「先选择再输入」）。
   * true 时前 N-1 级是下拉、末级（房间）由用户输入房间号——服务端在末级前一档不下发选项，
   * 故 `options` 为空不代表出错（2026-09-19 live 实测）。
   */
  lastLevelIsInput: boolean;
}

/** 级联的一步（回传后端 / 存常用房间共用）。`name` 仅展示用，不参与请求。 */
export interface RoomStep {
  level: number;
  /** 该级的 form 参数名（服务端 `total[].code`） */
  code: string;
  /** 该级已选值（选项 `value`，或用户输入的房间号） */
  value: string;
  name: string;
}

/** 一级候选（`query_electricity` → `options`）。 */
export interface ElectricityChoice {
  label: string;
  value: string;
  code: string;
  level: number;
}

/** 服务端层级定义（`map.total`）：UI 用它给每级做标签（校区/楼栋/房间）。 */
export interface ElectricityLevel {
  level: number;
  code: string;
  name: string;
}

/** 末级展示信息的一项：**键名由服务端下发**（实测恒为「信息」），通用渲染，禁止硬编码。 */
export interface ElectricityField {
  label: string;
  value: string;
}

/**
 * 末级视图。⚠️ 2026-09-19 live 实测：448/449/450 都**不下发** `money`/`iectranamt`
 * （恒 `null`），余额与单价是塞在 `fields[].value` 那句**各片区格式互不相同**的自由文本里
 * （如 450「房间号：101,剩余金额：-545.70，单价：0.5400」、448「当前余额517.05元,当前剩余电量957.50度」），
 * 故前端**只做通用字典渲染 + 按分隔符折行**，不解构、不硬编码任何标签。
 */
export interface ElectricityView {
  fields: ElectricityField[];
  money: number | null;
  /**
   * **结构化余额（元）**——结果卡主数字与趋势/统计一律用它，**前端绝不自己解析 `fields` 文本**
   * （解析实现只有后端 `campus_synjones::charge::balance_from_text` 一处，复制到前端会随校方文案漂移）。
   *
   * - 数值来自后端「关键词 + 分隔符 + 数字」的**相邻形态**提取（三片区原文都覆盖）；
   * - `null`/缺省 = **未提取到**（无「剩余金额/余额/剩余电费」关键词，或关键词后不是数字）⇒
   *   显示「无数据」，**绝不当 0**（`0.00` 是合法余额，与「没提取到」语义完全不同）；
   * - 刻意**不认「剩余电量」**：448 原文同一句里有 `当前剩余电量957.50度`（kWh），当钱显示就是错报。
   */
  balanceYuan?: number | null;
  tip: string | null;
}

/** `query_electricity` → data。`options` 为空且 `isFinal === false` ⇒ 该级无下拉（末级输入级）。 */
export interface ElectricityQuery {
  levels: ElectricityLevel[];
  options: ElectricityChoice[];
  isFinal: boolean;
  view: ElectricityView | null;
}

/** 一个常用房间（本地存 `%APPDATA%/campushub/electricity_rooms.json`，计划 §2.3）。 */
export interface SavedRoom {
  /** 本机 id（新增时留空字符串，后端补） */
  id: string;
  feeitemId: string;
  feeitemName: string;
  path: RoomStep[];
  label: string;
  /**
   * 是否「我的宿舍」（M4 批 2：`run_electricity_snapshot` 的采集对象，同一时刻最多一个）。
   *
   * 读（`get_electricity_rooms`）时**恒在场**（后端 `serde` 正常序列化）；写成可选只是为了让
   * 「新增房间」的调用不用带它——后端 `#[serde(default)]` 收 `false`，且**保存已有房间时沿用存量绑定**
   * （改名/刷新元信息不会静默丢绑定）。改绑走 `bind_electricity_room`，不要靠 save 修改。
   */
  bound?: boolean;
}

// ─────────── 电费充值（M3.1 批 D 前端；命令契约 = 计划 §2.2，后端 `commands/electricity.rs`） ───────────
//
// 全部命令出入参 camelCase（Tauri 自动转 snake_case）。**密码相关字段（passwordSeq/uuid）只在一次
// 调用内存在**：不进 localStorage、不进日志、不回填输入框（计划 Global Constraints 1）。

/** 一笔充值订单。`status`：0=待支付、1=已完成（计划 §1.3 状态机）。 */
export interface RechargeOrder {
  orderId: string;
  status: number;
  /** 订单过期时刻（服务端原文，可能带空格："YYYY-MM-DD HH:MM:SS"）；缺省 null */
  payExpDate: string | null;
  /** 订单金额（元）；缺省 null（前端不伪造，回落到用户输入值展示） */
  tranamt: number | null;
}

/**
 * 一种支付方式（`recharge_pay_methods` → methods 项）。
 * `nopassword` 后端已按计划 §1.3 的**唯一判据**归一成布尔（原文 `=== 1` ⇒ true）；
 * 其他真值是官方死分支，后端按「需密码」处理——前端只消费布尔，不重判原文。
 */
export interface RechargePayMethod {
  /** 后端只透出 ACCOUNT / ACCOUNTTSM（校园卡、电子账户） */
  code: string;
  /** `paytypeid`，与 code 一起回传给后续所有支付请求 */
  payid: string;
  name: string;
  nopassword: boolean;
  remark: string | null;
}

/** `recharge_pay_methods` → data。`methods` 为空 ⇒ 无可用账户类渠道（不展示第三方渠道）。 */
export interface RechargePayMethods {
  order: RechargeOrder;
  methods: RechargePayMethod[];
}

/**
 * 安全键盘数据。**`keys` 只用于渲染**：提交的是「点击的键位下标序列」而非键上的字符
 * （App 口径 `password = 下标数组.join("")` + `uuid`），见 `RechargeFlow.tsx` 红线注释。
 *
 * ⚠️ 服务端 `passwordMap[uuid]` 实测是**10 个字符的字符串**（不是数组），官方前端逐字符渲染；
 * 批 C 的 crate 已把它按字符拆成数组后下发（`recharge.rs::parse_password_pad`），故这里收到的是
 * `string[]`，**下标语义不变**（第 i 个元素 = 第 i 个键）。
 */
export interface PasswordPad {
  uuid: string;
  keys: string[];
}

/**
 * `recharge_query_account` → data。**两步协议**（crate `query_account` 的 `accountno` 参数，
 * 官方 `getAccountno` → `getAccounttype`）：不带 `accountno` ⇒ 回**账号列表**；带已选 `accountno`
 * ⇒ 回该账号的**账户类型 + 安全键盘**。故免密分支同样要跑完这两步（提交体必须带 accountno/ccctype）。
 *
 * `ccctypes` 是**裸类型码字符串数组**（crate `parse_ccctypes` → `Vec<String>`，命令层同名透出）；
 * 没有 balance（官方 UI 才用，本项目不展示）。
 */
export interface RechargeAccounts {
  accounts: string[];
  ccctypes: string[];
  /** 只有**需密码**且服务端下发键盘时才有值；免密分支为 null/缺省 */
  pad?: PasswordPad | null;
}

/** `recharge_create` → data（命令层 `RechargeCreated`）。 */
export interface RechargeCreated {
  orderId: string;
}

/** `recharge_status` → data：与 `recharge_pay_methods` **同一个结构**（同一端点同一解析），
 *  轮询时只关心 `order.status`。故不另立类型，避免一个 payload 两种形状。 */
export type RechargeStatus = RechargePayMethods;

// ──────── 电费历史（M4 批 2；命令层 `commands/electricity_history.rs`） ────────
//
// 历史三源并存（任务书 §2）：① 学校账单（权威缴费记录）② 订单（含待支付）
// ③ **自采日余额快照**——学校侧没有每日余额序列（`turnover` 的 `balance_amount` 恒 null），
// 只能客户端自己采。三个源金额口径都是**元**（`/charge/*` 侧实测单位；一卡通侧才是分）。

/**
 * 一条缴费账单（`get_electricity_bills` → `records` 项；服务端 `/charge/turnover/app_account`）。
 * ⚠️ `amountYuan` 单位是**元**（服务端 `TRANAMT` 原文即元，**不要再除 100**）；缺失为 `null`。
 */
export interface ElectricityBill {
  /** 流水号（列表 key 用；后端不落盘、不记日志） */
  id: string;
  /** 成功时间（服务端原文，形如 `2026-09-19 17:08:16`） */
  time: string;
  /** 缴费项名（如 `桃园1号-李园8号`） */
  itemName: string;
  /** 摘要（含房间路径等学校侧原文） */
  abstracts: string;
  /** 渠道名（实测 `移动服务平台`） */
  typeName: string;
  amountYuan: number | null;
}

/** `get_electricity_bills` → data。`total` 是**符合条件的全量条数**（不是本页条数），用于「加载更多」。 */
export interface ElectricityBillPage {
  total: number;
  records: ElectricityBill[];
}

/**
 * 某月缴费合计（`get_electricity_monthly` → 12 项，`month` 形如 `2026-09`）。
 * **空月 = 0**（学校侧回空数组 ⇒ 该月确实没缴费），故曲线保留完整 12 个月 x 轴；
 * 而**请求失败是整年失败**（不会把「取不到」显示成 0 元）。
 */
export interface ElectricityMonthTotal {
  month: string;
  amountYuan: number;
}

/**
 * 一条订单（`get_electricity_orders` → 项；`/charge/order/personal_data`，**含待支付**）。
 * `status`：0 待支付 / 1 已完成 / 2。
 *
 * 待支付单的 `orderId` 可直接交给已有的 `recharge_status` / `recharge_cancel`
 *（取消遗留订单复用充值那条链路，本批不新增取消命令）。**无任何户号/姓名等 PII 字段**。
 */
export interface ElectricityOrder {
  orderId: string;
  status: number;
  amountYuan: number | null;
  /** 实付金额；**待支付单为 `null`** */
  actualYuan: number | null;
  /** 下单时间（服务端原文） */
  commitDate: string;
  /** 成功时间（待支付单为空串） */
  successDate: string;
  /** 来源（实测 `app`） */
  source: string;
  /** 摘要（如 `无锡学院 1号楼 101`） */
  abstracts: string;
  /** 片区 id（取自 `feeitemlist[0]`；顶层 `feeitemid` 实测恒 0） */
  feeitemId: string;
}

/**
 * 一条自采余额快照（`get_electricity_history` → 项，**时间升序**，图表直接消费）。
 *
 * - 去重键 = `roomKey` + `date`：同房间同日重复采集**覆盖**当天那条（保留 `collectedAt` 最新的）；
 * - `balance` 为 `null` = 学校那句自由文本里**提不到金额**（如只给了「剩余电量」）⇒ UI 显示「无数据」，
 *   **不要当 0**（0 元与「没采到」在曲线上语义完全不同）；
 * - `raw` 是学校侧那句原文（三片区格式互不相同，前端**不要解构**它，展示只在需要时作为副标题）。
 */
export interface ElectricityHistoryEntry {
  /** 稳定主键 `roomKey@date`（跨端合并的唯一依据；前端只读不构造） */
  id: string;
  /** 观察到该快照的设备名（多端合并时区分来源；本机采集时在场，导入的记录可能缺省） */
  device?: string;
  /** 房间稳定键（片区 id + 级联路径；与 `SavedRoom.id` 无关，删除重建房间不改变历史归属） */
  roomKey: string;
  /** 房间显示名（保存时的 `label`） */
  roomName: string;
  feeitemId: string;
  feeitemName: string;
  /** 采集时刻（ISO8601 本地带偏移，如 `2026-09-19T17:20:31+08:00`） */
  collectedAt: string;
  /** 采集日期 `YYYY-MM-DD`（本地时区） */
  date: string;
  balance: number | null;
  raw: string;
  /** `"auto"`（启动补采）/ `"manual"`（用户手动采集） */
  source: string;
}

/**
 * `run_electricity_snapshot` → data。
 *
 * 失败不是这个形状：未登录 / 未绑定宿舍 / 房间号无效都会走 `CommandResult.message`
 * （可读中文原因，如「尚未绑定宿舍房间：…」），前端据此给引导而不是弹技术错误。
 */
export interface ElectricitySnapshot {
  /** 本次写入的那条快照（同日已有则被覆盖） */
  entry: ElectricityHistoryEntry;
  /** 是否覆盖了当天已有的那条（提示文案用「已更新今日记录」而非「已记录」） */
  replaced: boolean;
}

// ==================== M4.5 一卡通页契约（2026-09-19，live 实测钉住） ====================
//
// 单位口径：后端已把**分**换算成元（一卡通侧一切金额字段都是分），前端一律按元展示、不再除 100。
// 脱敏：卡号只给 `accountMasked`；持卡人姓名/手机号/证件/户号后端不透出，前端也没有。
// 详见 `.codewiki/modules/campus-synjones.md` 与 `docs/superpowers/plans/2026-09-19-ecard-full-replica.md`。

/** 卡上一个子账户（`accinfo[]`）：电子账户 / 卡账户。 */
export interface EcardAccountInfo {
  /** 子账户类型码（实测形如 `42940-000`） */
  type: string;
  balanceYuan: number;
  /** 当日已消费 */
  dayCostAmtYuan: number;
  dayCostLimitYuan: number;
  nonpwdLimitYuan: number;
  singleLimitYuan: number;
}

/** 一张一卡通（`get_ecard_overview` → `cards[]`）。 */
export interface EcardCard {
  /** 脱敏卡号（前 5 + `****` + 后 2）；全号只在查询密码校验通过后由 `ecard_check_pwd` 单独返回 */
  accountMasked: string;
  cardTypeName: string;
  /** 中文状态标签（正常 / 已挂失 / 已冻结…） */
  statusLabel: string;
  /** 卡账户余额（`db_balance + unsettle_amount`） */
  balanceYuan: number;
  /** 电子账户余额（主口径，`elec_accamt`） */
  elecBalanceYuan: number;
  lost: boolean;
  frozen: boolean;
  accStatus: number | null;
  expDate: string;
  /** 开户时间（后端多键名回落探测，未命中为 null ⇒ 该行不显示） */
  openDate: string | null;
  /** 当天支付累计（卡级 daycostamt，未下发为 null ⇒ 该行不显示） */
  dayCostAmtYuan: number | null;
  /** 自动转账（圈存）开关（官方档位 1/2 都算开启） */
  autotransFlag: boolean;
  /** 圈存档位原值（0=禁止 1=只允许自助 2=自助及自动） */
  autotransFlagKind: number;
  autotransAmtYuan: number;
  autotransLimiteYuan: number;
  dayCostLimitYuan: number;
  nonpwdLimitYuan: number;
  singleLimitYuan: number;
  /** 已绑银行卡尾号（未绑为空串） */
  bankaccTail: string;
  accInfos: EcardAccountInfo[];
}

/**
 * 学校侧下发的一卡通客户端配置（`frontInfo` 白名单键 + 应用清单）。
 *
 * `enabledApps` 是官方「服务大厅」应用清单里的 `appCode`（status=1）——宫格入口按它门控
 * （如该校清单里**没有** `bind-campus-card` ⇒ 不显示多卡绑定）。
 */
export interface EcardClientConfig {
  /** `getEcardConfig.type !== "2"` ⇒ 主余额口径为电子账户（本校 `type=1`，即电子账户） */
  balanceShowsElectronic: boolean;
  showSno: boolean;
  /** 是否展示「挂失·解挂」入口（本校 1） */
  showLost: boolean;
  freezeRecharge: boolean;
  manageFee: boolean;
  /** 一卡通充值片区 id（本校 `401`，来自 `getFrontConfig.recharge`） */
  rechargeFeeitemId: string;
  /** 扫码付片区 id（本校 `407`） */
  scanFeeitemId: string;
  /** 查询密码规则（本校 `A/a/Num/#/leng_6`，6 位） */
  passwordRule: string;
  /** 电子账户服务时间（`["05:00","23:50"]`） */
  serviceTime: string[];
  enabledApps: string[];
}

/** `get_ecard_overview` → data。 */
export interface EcardCardsOverview {
  cards: EcardCard[];
  config: EcardClientConfig;
}

/**
 * 一卡通页内的视图（宫格首页 + 子页）。
 *
 * 存在 uiStore 而不是面板局部 state：今日页「查电费」「卡片充值」两个快捷动作要
 * **直达子页**（合并成一个 Dock 入口后，否则要多点一次）。
 */
export type EcardView =
  | "home"
  | "balance"
  | "bill"
  | "stats"
  | "recharge"
  | "power"
  | "cardops"
  | "bank"
  | "paycode"
  | "profile"
  | "face";

/** 流水分类字典（`get_ecard_types` → 项；id 语义实测 1 消费 2 充值 3 退款 4 扫码付 5 补贴）。 */
export interface EcardTurnoverType {
  id: number;
  name: string;
  nameEn: string;
  icon: string;
  showOrder: number;
}

/** `get_ecard_stats_summary` → data（区间收支合计）。 */
export interface EcardStatsSummary {
  expensesYuan: number;
  incomeYuan: number;
}

/** `get_ecard_stats_series` → 项（后端已按 key 升序、**零值保留**）。 */
export interface EcardStatsPoint {
  /** 月视图为 `YYYY-MM-DD`，年视图为 `YYYY-MM` */
  label: string;
  amountYuan: number;
}

/** `get_ecard_stats_assort` → 项（按分类聚合）。 */
export interface EcardStatsAssortItem {
  typeId: string;
  turnoverType: string;
  nameEn: string;
  amountYuan: number;
}

/**
 * `get_ecard_secure_keyboard` → data：**安全键盘**。
 *
 * 红线（与电费充值 `passwordMap` 同构）：前端只拿 `keys`（位置 → 显示字符）渲染、
 * 只回传 `padId` + **用户点击的位置下标序列**；密码明文只在后端拼装，用完即弃。
 */
export interface EcardSecurePad {
  /** 后端随机化的一次性键盘 id（真实 `uuid` 不外泄） */
  padId: string;
  /** 位置 i → 该位置显示的字符（渲染用；提交的是**位置**不是字符） */
  keys: string[];
  /** 位置 i 对应的官方键盘图片（data URL / base64 片段，可能为空） */
  images: string[];
}

/** `ecard_check_pwd` → data。 */
export interface EcardCheckResult {
  ok: boolean;
  /**
   * 校验通过后学校返回的银行卡全号。**本校恒为 null**（学校未提供「校验密码查卡号」
   * 能力）——前端必须如实提示「学校未返回卡号」，绝不伪造号码或显示脱敏假数据。
   */
  bankCardNo: string | null;
}

/** 发码类命令（`ecard_send_find_pwd_code` / `ecard_send_bind_bank_code`）→ data。 */
export interface EcardCodeSent {
  /** 学校侧 `data.account`，后续提交命令（`ecard_find_pwd` / `ecard_bind_bank`）原样回传的 `id` */
  id: string;
}

/**
 * `ecard_face_detail` → data：人脸采集状态与基础信息（fapi 智慧校园服务）。
 */
export interface EcardFaceDetail {
  name: string;
  number: string;
  schoolName: string;
  /** 是否已采集（学校侧头像非空） */
  collected: boolean;
}

/**
 * `get_ecard_paycode` → data：动态付款码（一期，对齐官方 H5 plat/pay）。
 *
 * 红线：`barcode` 是**动态支付凭据**——不写 console.log、不进 localStorage、
 * 不进错误文案 / aria-label；除条码图与「查看数字」主动展开外不留存。
 */
export interface EcardPaycode {
  /** 条码内容（官方 barcode 数组首段，20 位数字串；条码图与二维码同串） */
  barcode: string;
  /**
   * 官方 `expires` 原样透传：**秒级时间戳或有效期秒数**（前端兼容判定）；
   * `<= 0` = 未拿到 ⇒ 不显示倒计时、不自动重取，只保留手动刷新。
   */
  expires: number;
  /** 支付方式名（实测「一卡通电子钱包」） */
  payName: string;
  /** 电子账户 + 卡账户合计余额（元，后端已换算） */
  balanceYuan: number;
}

/** `get_ecard_paycode_settings` → data（脱机二维码开关状态，一期只读展示）。 */
export interface EcardPaycodeSettings {
  offlineSwitch: boolean;
}


/** `get_plat_profile` → data（plat 用户资料；`idNumber` 服务端已掩码，未列出的键前端不消费）。 */
export interface PlatProfile {
  account: string;
  sno: string;
  name: string;
  identityName: string;
  departmentName: string;
  sex: string;
  /** 头像 URL（plat 资源地址，可能为空串） */
  avatar: string;
  mobile: string | null;
  email: string;
}

/** `get_plat_equipment` → 项（plat 绑定设备；`status="1"` 在线 / `"0"` 已授权）。 */
export interface PlatDevice {
  id: string;
  name: string | null;
  type: string | null;
  status: number;
  createTime: string;
  updateTime: string;
}
// plat 设备写操作（官方 bundle 取证，POST JSON `{equipmentUserBh: PlatDevice.id}`，
// 报文镜像见 crates/campus-synjones/src/plat.rs）：无独立 DTO，均返回 CommandResult<void>——
// - `plat_offline_device`：下线「已登录在线」设备（status="1" 列表项）；
// - `plat_remove_device`：移除「已授权」设备授权（status="0" 列表项）。

/** `get_plat_login_logs` → data（MyBatis-Plus 分页；本校实测恒空列表）。 */
export interface PlatLoginLogs {
  records: { id: string | null; createTime: string | null; ip: string | null }[];
  total: number;
  size: number;
  current: number;
  pages: number;
}
// ==================== M5 通知中心契约（2026-09-20，tauri commands/notification.rs 镜像） ====================
//
// 后台 poll_tick 产出三类通知：info（门户资讯）/ todo（门户待办）/ electricity（电费低余额）；
// 系统通知由 Rust 侧发送，前端只消费通知中心列表与设置。

/** 通知中心一条未读通知（`get_notifications` → `items[]`）。 */
export interface NotificationItem {
  /** 确定性主键：`info:{栏目id}:{条目id}` / `todo:{条目id}` / `elec:{YYYY-MM-DD}`（前端只读不构造） */
  id: string;
  /** `"info"` / `"todo"` / `"electricity"` */
  kind: string;
  /** 公告标题 / 待办标题 / 「{房间} 余额不足」 */
  title: string;
  /** 副文案（栏目名·发布时间 / 申请人·申请时间 / 余额与阈值明细） */
  body: string;
  /** 产生时刻（ISO8601 本地带偏移） */
  createdAt: string;
  /** 门户资讯的官网原文 URL（可直接跳转）；待办 / 电费为 null */
  url: string | null;
}

/** 各类未读计数（后端按 kind 归好类，前端不自算）。 */
export interface NotificationCounts {
  total: number;
  info: number;
  todo: number;
  electricity: number;
}

/** `get_notifications` / `mark_notifications_read` → data（mark 返回剩余未读，免二次拉取）。 */
export interface NotificationStateView {
  /** 后端按产生时间升序存放，前端倒序展示（最新的在前） */
  items: NotificationItem[];
  counts: NotificationCounts;
}

/**
 * 通知设置（`get_notification_settings` / `save_notification_settings`）。
 * 全字段有默认值：未落盘过时后端返回默认（全 7 栏订阅、门户/待办 10 分钟、电费 30 分钟）。
 * 校验规则（save 非法返回中文 message）：三个间隔必须在 5～720 分钟、阈值 ≥ 0。
 */
export interface NotificationSettings {
  /** 门户订阅栏目 id 列表（空数组 = 资讯通知关闭） */
  infoColumns: string[];
  todoEnabled: boolean;
  electricityEnabled: boolean;
  /** 电费提醒阈值（元）：余额 < 阈值触发（24h 节流，后端控制） */
  electricityThresholdYuan: number;
  infoIntervalMin: number;
  todoIntervalMin: number;
  electricityIntervalMin: number;
  /** 静音：true = 只进通知中心，不发系统通知 */
  muteSystemNotify: boolean;
}
