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
