export type CommandResult<T> = { success: boolean; message?: string; data?: T };

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
