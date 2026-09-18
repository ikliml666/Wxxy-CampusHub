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

// PanelId 冻结 8 项；M2.5 课表页届时追加 "timetable" 需同步改此处 + DOCK_ITEMS + persist 兼容
export type PanelId =
  | "today"
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
