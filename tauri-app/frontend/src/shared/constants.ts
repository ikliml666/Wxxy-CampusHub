/**
 * 内置浏览器（in-app browser）前端共享常量。
 */

/**
 * 内置浏览器工具栏高度（逻辑 px）。
 *
 * ⚠️ 与 Rust 侧 `tauri-app/src-tauri/src/commands/browser.rs` 的 `TOPBAR_LOGICAL = 48.0`
 * 互为镜像：Rust 的 `relayout` / `open_url_inapp` 按此值把主 webview（React 视口）
 * 缩顶，给前端工具栏留位——**改一处必改两处**。React 视口实际高度等于该值，
 * 超出部分（如工具栏下方的流内元素）会被 webview 边界裁剪不可见（spike 六问 1）。
 */
export const BROWSER_TOOLBAR_H = 48;

/** 提取 url 的 host 供地址 chip / 提示文案展示；解析失败兜底空串，不抛错。 */
export const browserHostOf = (url: string): string => {
  try {
    return new URL(url).host;
  } catch {
    return "";
  }
};
