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
  /** 非本周课程降级显示（2026-09-19 批 7 契约 §13.2）：true = 网格同时渲染非展示周
   *  课程（40% 透明度、可点详情、不可拖）；false/旧数据缺省 = 隐藏 */
  showNonCurrentWeek: boolean;
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

/** get_timetable → data（批次 4 修订契约 §2.3 + 修复轮 §17）。 */
export interface TimetableView {
  timetable: Timetable;
  /** 生效作息（自定义优先、回落内置校本大节表）；作息编辑器的唯一事实源（大节口径） */
  slots: TimeSlot[];
  /** 内置 11 小节作息表（修复轮契约 §17）：恒定下发、不随 config.slots 变化；
   *  时间列与小节网格坐标用 */
  sectionSlots: TimeSlot[];
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
}
