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

// ==================== 资讯附件（tauri commands/portal.rs：download_attachment） ====================

/** 下载校园官网附件（后端白名单校验链接）：成功 data = { fileName, base64 }（裸 base64，无 data: 前缀）。 */
export const downloadAttachment = (url: string) =>
  invokeCommand<{ fileName: string; base64: string }>("download_attachment", { url });
