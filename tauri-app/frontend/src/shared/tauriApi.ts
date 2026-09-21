import { invoke } from "@tauri-apps/api/core";
import type { CommandResult } from "./types";

/**
 * IPC 唯一出口：所有 Tauri 命令调用必须经过此函数。
 * 命令返回 CommandResult<T>；invoke 抛错时包装为 { success:false, message:String(e) }。
 */
export async function invokeCommand<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<CommandResult<T>> {
  try {
    return await invoke<CommandResult<T>>(cmd, args);
  } catch (e) {
    return { success: false, message: String(e) };
  }
}

// ==================== M5 通知中心（tauri commands/notification.rs） ====================

import type { NotificationStateView, NotificationSettings } from "./types";

/** 通知中心未读列表 + 各类计数。 */
export const getNotifications = () => invokeCommand<NotificationStateView>("get_notifications");

/** 标记已读：ids 缺省/空 = 全部已读；返回剩余未读（免二次拉取）。 */
export const markNotificationsRead = (ids?: string[]) =>
  invokeCommand<NotificationStateView>("mark_notifications_read", ids ? { ids } : {});

/** 读取通知设置（未落盘过返回默认值）。 */
export const getNotificationSettings = () =>
  invokeCommand<NotificationSettings>("get_notification_settings");

/** 保存通知设置（间隔须 5～720 分钟、阈值 ≥ 0，非法走 message 中文原因）。 */
export const saveNotificationSettings = (settings: NotificationSettings) =>
  invokeCommand("save_notification_settings", { settings });

// ==================== 内置浏览器（tauri commands，契约冻结 commit a6358ee） ====================
// 事件（Task 5 在 BrowserOverlay 组件内挂 listen，此处只封装命令）：
// browser://load (phase: "started"|"finished") / browser://blocked (url) / browser://nav (url)

import type { BrowserOpenResult } from "./types";

/** 打开内置浏览器；`inApp=false` = 域外应用，调用方须降级 open_app 走系统浏览器。 */
export const openInAppBrowser = (url: string) =>
  invokeCommand<BrowserOpenResult>("open_in_app_browser", { url });
export const closeAppBrowser = () => invokeCommand("close_app_browser");
export const appBrowserNavigate = (url: string) => invokeCommand("app_browser_navigate", { url });
export const appBrowserReload = () => invokeCommand("app_browser_reload");
export const appBrowserBack = () => invokeCommand("app_browser_back");
export const appBrowserForward = () => invokeCommand("app_browser_forward");

/**
 * 逃生口：在系统浏览器打开 url（Task 5，工具栏 ExternalLink 与拦截提示共用）。
 * 按 URL 判降级：校方域（cwxu.edu.cn 及子域）走 open_in_browser 域名白名单，
 * 其余（含充值内网 IP 10.3.100.110，域名白名单不覆盖）走 open_app 协议白名单
 * （isCas 为旧契约占位参数，恒 false，与 browserStore 降级链路一致）。
 */
export const openExternalBrowser = (url: string) => {
  let host = "";
  try {
    host = new URL(url).host;
  } catch {
    // 解析失败 host 为空 → 走 open_app 协议白名单判定（后端兜底校验）
  }
  const campus = host === "cwxu.edu.cn" || host.endsWith(".cwxu.edu.cn");
  return campus
    ? invokeCommand("open_in_browser", { url })
    : invokeCommand("open_app", { url, isCas: false });
};
