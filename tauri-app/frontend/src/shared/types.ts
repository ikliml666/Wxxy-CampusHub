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
