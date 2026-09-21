//! 应用内浏览器（in-app browser）：打开去向决策 + webview 生命周期命令 + 注入脚本。
//!
//! 结构：
//! - [`decide_open`]：打开去向三态纯函数（校园域/内网充值 IP → 应用内；域外公网 →
//!   前端降级走旧 open_app 系统浏览器；非 http/https → 拒绝），单测钉死；
//! - [`open_url_inapp`]：共用入口（Task 6 电费入口复用）——串行化清理旧实例 →
//!   主 webview 缩顶 48 逻辑px（给前端工具栏留位）→ `add_child` 副 webview
//!   （注入 [`INJECT_JS`] + on_navigation 白名单拦截 + on_page_load 进度事件）；
//! - `close_app_browser` / `app_browser_navigate` / `app_browser_reload` /
//!   `app_browser_back` / `app_browser_forward`：副 webview 生命周期与导航；
//! - [`relayout`]：窗口尺寸变化时按「有无副 webview」重算两块 bounds（lib.rs
//!   的 `WindowEvent::Resized` 调用）。
//!
//! API 事实（tauri 2.11.5 真机 spike 钉死，详见 `.codewiki/learnings/inapp-webview-spike.md`）：
//! - 多 webview API（get_window/get_webview/add_child）需 tauri feature "unstable"；
//! - `Window::add_child(builder, position, size)` 三参数直接带位置与尺寸；
//! - 无 `remove_webview` / `remove_webview_by_label`，销毁副 webview 用
//!   `Webview::close()`——**异步落定**，close 后立即同 label 重建有竞态窗口，
//!   故 open 侧统一 [`close_stale_child`] 轮询等待（超时 1s 放行）；
//! - `Webview::set_bounds(tauri::Rect)`，Rect { position, size } 字段非 Option；
//! - `Webview::eval` / `Webview::reload` 可用；无 `Webview::navigate`（导航用
//!   eval `location.href`），白名单外导航由 on_navigation 返回 false 拦下。
//!
//! 事件（Rust → 前端 emit，载荷 camelCase JSON；前端 Task 4 listen）：
//! - `browser://load`    `{"phase":"started"|"finished"}`（on_page_load）
//! - `browser://nav`     `{"url":"..."}`（on_navigation 放行）
//! - `browser://blocked` `{"url":"..."}`（on_navigation 拒绝）
//!
//! 敏感纪律：本模块不持有会话/token；副 webview 页面无 IPC capability（不能
//! invoke Tauri 命令），注入脚本只做页面侧治理，宿主通信只走 Rust 侧回调。

use serde_json::json;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, Webview,
    WebviewUrl, WebviewBuilder,
};

use super::auth::CommandResult;

// ---------------- 打开去向决策层（Task 2） ----------------

/// 充值页内网 IP（慧新E校官网 base）：校园域语义，[`campus_portal::is_allowed_info_url`]
/// 的 `*.cwxu.edu.cn` 白名单不覆盖，decide/导航白名单需额外放行。
pub(crate) const RECHARGE_HOST: &str = "10.3.100.110";

/// 打开去向三态（纯函数判定，单测钉死）。
pub(crate) enum OpenDecision {
    /// 进应用内 webview（校园域或内网充值 IP）
    InApp,
    /// 域外公网站点 → 前端降级走旧 open_app 系统浏览器
    External,
    /// 非 http/https，拒绝
    Blocked,
}

/// 应用内 webview 的**导航白名单**（与 [`decide_open`] 的 InApp 判据同源，
/// `on_navigation` 拦截也用它）：校园域（`*.cwxu.edu.cn`）或内网充值 IP。
fn inapp_url_allowed(url: &str) -> bool {
    if campus_portal::is_allowed_info_url(url) {
        return true;
    }
    // SSRF 面不变：该内网形态 URL 仅由后端 `browser_recharge_url` 拼装后下发，
    // 前端无法借 decide_open/on_navigation 的内网放行开任意地址。
    matches!(
        reqwest::Url::parse(url),
        Ok(u) if u.host_str() == Some(RECHARGE_HOST)
    )
}

/// 打开去向判定（纯函数）：协议白名单 → 导航白名单 → 域外。
pub(crate) fn decide_open(url: &str) -> OpenDecision {
    if !campus_portal::is_http_url(url) {
        return OpenDecision::Blocked;
    }
    if inapp_url_allowed(url) {
        OpenDecision::InApp
    } else {
        OpenDecision::External
    }
}

// ---------------- webview 生命周期命令（Task 3） ----------------

/// 副 webview 标签（单 webview，无多标签）。
const BROWSER_LABEL: &str = "app-browser";
/// 顶栏占位高度（逻辑 px）：主 webview 缩顶后留给前端工具栏的区域。
const TOPBAR_LOGICAL: f64 = 48.0;
/// 注入脚本（`include_str!` 打进二进制，不进 resources 打包配置）。
const INJECT_JS: &str = include_str!("../browser_inject.js");

/// `open_in_app_browser` → data（前端据 `in_app=false` 降级走旧 open_app 系统浏览器）。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOpenResult {
    pub in_app: bool,
    pub url: String,
}

/// 主 webview 复原为整窗（物理坐标，与 `window.inner_size()` 同源最稳，spike 结论）。
fn restore_main_full(window: &tauri::Window) {
    if let Some(main_wv) = window.get_webview("main") {
        if let Ok(size) = window.inner_size() {
            let _ = main_wv.set_bounds(tauri::Rect {
                position: PhysicalPosition::new(0, 0).into(),
                size: size.into(),
            });
        }
    }
}

/// 窗口尺寸变化时重算 webview bounds：有副 webview → 主缩顶 48 + 副占其余；
/// 无 → 主复原整窗。lib.rs setup 的 `WindowEvent::Resized` 高频调用——全走
/// 轻量 bounds 计算，无锁无分配；零尺寸（最小化）跳过。
pub(crate) fn relayout(window: &tauri::Window) {
    let Ok(size) = window.inner_size() else { return };
    if size.width == 0 || size.height == 0 {
        return; // 最小化/遮挡时的零尺寸事件不必布局
    }
    let Ok(scale) = window.scale_factor() else { return };
    let Some(main_wv) = window.get_webview("main") else { return };
    if window.get_webview(BROWSER_LABEL).is_none() {
        restore_main_full(window);
        return;
    }
    let w = size.width as f64 / scale;
    let h = size.height as f64 / scale;
    let _ = main_wv.set_bounds(tauri::Rect {
        position: LogicalPosition::new(0.0, 0.0).into(),
        size: LogicalSize::new(w, TOPBAR_LOGICAL).into(),
    });
    if let Some(child) = window.get_webview(BROWSER_LABEL) {
        let _ = child.set_bounds(tauri::Rect {
            position: LogicalPosition::new(0.0, TOPBAR_LOGICAL).into(),
            size: LogicalSize::new(w, (h - TOPBAR_LOGICAL).max(0.0)).into(),
        });
    }
}

/// 串行化保护（Task 1 评审 Minor 4）：对残留的副 webview 执行 close 并轮询等待
/// 销毁落定（`Webview::close` 异步——立即同 label `add_child` 会撞 label 冲突）。
/// 超时 1s 放行（不无限阻塞；真撞上时由 add_child 报错给调用方）。
async fn close_stale_child(app: &AppHandle) {
    let Some(stale) = app.get_webview(BROWSER_LABEL) else { return };
    if let Err(e) = stale.close() {
        log::warn!("[browser] close stale child: {e}");
    }
    for _ in 0..20 {
        if app.get_webview(BROWSER_LABEL).is_none() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    log::warn!("[browser] 旧副 webview 未在 1s 内销毁落定，放行继续");
}

/// 共用入口（`open_in_app_browser` 命令与 Task 6 电费入口复用）：确保「单
/// app-browser webview」加载 url。实现采用**先清理旧实例再重建**而非导航复用：
/// `Webview::close()` 异步销毁且无存活态判定，复用无法区分「活的」与「销毁中」，
/// 统一 close_stale + 重建是杜绝 label 冲突的确定路径（控制器裁决 4）；
/// 「单 webview、无多标签」契约语义不变。调用方自行保证 url 已过
/// [`decide_open`]（InApp 分支）。
pub(crate) async fn open_url_inapp(
    app: AppHandle,
    url: String,
) -> CommandResult<BrowserOpenResult> {
    let Some(window) = app.get_window("main") else {
        return CommandResult::err("主窗口不存在");
    };
    let parsed: tauri::Url = match url.parse() {
        Ok(u) => u,
        Err(e) => return CommandResult::err(&format!("URL 解析失败：{e}")),
    };
    close_stale_child(&app).await;

    let Ok(inner) = window.inner_size() else {
        return CommandResult::err("读取窗口尺寸失败");
    };
    let Ok(scale) = window.scale_factor() else {
        return CommandResult::err("读取窗口缩放比失败");
    };
    let w = inner.width as f64 / scale;
    let h = inner.height as f64 / scale;

    // 主 webview 缩顶 48 逻辑px，给前端工具栏留位（Task 4）；React UI 暂被
    // 裁剪进条内属预期（spike 六问 1 实锤不崩）
    if let Some(main_wv) = app.get_webview("main") {
        if let Err(e) = main_wv.set_bounds(tauri::Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(w, TOPBAR_LOGICAL).into(),
        }) {
            return CommandResult::err(&format!("主窗口布局失败：{e}"));
        }
    }

    let app_for_nav = app.clone();
    let app_for_load = app.clone();
    let child = window.add_child(
        WebviewBuilder::new(BROWSER_LABEL, WebviewUrl::External(parsed))
            .initialization_script(INJECT_JS)
            // 导航白名单（与 decide_open 同源）：放行 emit browser://nav，拒绝
            // emit browser://blocked 并拦下（返回 false）
            .on_navigation(move |u| {
                let url_str = u.as_str().to_string();
                let allow = inapp_url_allowed(&url_str);
                let event = if allow { "browser://nav" } else { "browser://blocked" };
                log::info!("[browser] on_navigation {} allowed={}", url_str, allow);
                if let Err(e) = app_for_nav.emit(event, json!({ "url": url_str })) {
                    log::warn!("[browser] emit {event}: {e}");
                }
                allow
            })
            // 加载进度事件：前端 Task 4 据此显示 loading 态
            .on_page_load(move |wv, payload| {
                let phase = match payload.event() {
                    tauri::webview::PageLoadEvent::Started => "started",
                    tauri::webview::PageLoadEvent::Finished => "finished",
                };
                log::info!("[browser] page load {phase}: {}", payload.url());
                if let Err(e) = app_for_load.emit("browser://load", json!({ "phase": phase })) {
                    log::warn!("[browser] emit browser://load: {e}");
                }
                let _ = wv; // 回调签名要求，本批不用
            }),
        LogicalPosition::new(0.0, TOPBAR_LOGICAL),
        LogicalSize::new(w, (h - TOPBAR_LOGICAL).max(0.0)),
    );
    let child = match child {
        Ok(c) => c,
        Err(e) => {
            // 失败回滚：主 webview 复原整窗，不留下裁剪态
            restore_main_full(&window);
            return CommandResult::err(&format!("创建应用内浏览器失败：{e}"));
        }
    };
    // add_child 传参之外的显式 bounds 复验（spike 六问 5 形态，运行期通道双保险）
    if let Err(e) = child.set_bounds(tauri::Rect {
        position: LogicalPosition::new(0.0, TOPBAR_LOGICAL).into(),
        size: LogicalSize::new(w, (h - TOPBAR_LOGICAL).max(0.0)).into(),
    }) {
        log::warn!("[browser] child set_bounds: {e}");
    }
    log::info!(
        "[browser] open label={BROWSER_LABEL} rect=(0,48 {w}x{}) url={url}",
        (h - TOPBAR_LOGICAL).max(0.0)
    );
    CommandResult::ok(BrowserOpenResult { in_app: true, url })
}

/// 应用内打开 url：校园域/内网充值 IP 进 webview（`in_app=true`）；域外公网
/// 返回 `in_app=false`，前端降级走旧 open_app 系统浏览器；非 http/https 拒绝。
#[tauri::command]
pub async fn open_in_app_browser(
    app: AppHandle,
    url: String,
) -> CommandResult<BrowserOpenResult> {
    match decide_open(&url) {
        OpenDecision::Blocked => CommandResult::err("仅支持 http/https 链接"),
        OpenDecision::External => CommandResult::ok(BrowserOpenResult { in_app: false, url }),
        OpenDecision::InApp => open_url_inapp(app, url).await,
    }
}

/// 关闭应用内浏览器：销毁副 webview + 主 webview 复原整窗。
#[tauri::command]
pub async fn close_app_browser(app: AppHandle) -> CommandResult<()> {
    let Some(window) = app.get_window("main") else {
        return CommandResult::err("主窗口不存在");
    };
    if let Some(child) = app.get_webview(BROWSER_LABEL) {
        if let Err(e) = child.close() {
            return CommandResult::err(&format!("关闭应用内浏览器失败：{e}"));
        }
    }
    restore_main_full(&window);
    log::info!("[browser] close: done");
    CommandResult::empty()
}

/// 副 webview 已打开时的句柄（未打开给中文错误）。
fn child_or_err(app: &AppHandle) -> Result<Webview, CommandResult<()>> {
    app.get_webview(BROWSER_LABEL)
        .ok_or_else(|| CommandResult::err("应用内浏览器未打开"))
}

/// 工具栏导航：副 webview 跳到 url。tauri 2 无 `Webview::navigate`，用 eval
/// `location.href`（spike 定案）；url 经 JSON 序列化成安全 JS 字符串字面量
///（防注入拆串），白名单外导航由 on_navigation 返回 false 拦下兜底。
#[tauri::command]
pub async fn app_browser_navigate(app: AppHandle, url: String) -> CommandResult<()> {
    let child = match child_or_err(&app) {
        Ok(c) => c,
        Err(ret) => return ret,
    };
    let literal = serde_json::to_string(&url).unwrap_or_else(|_| "\"\"".into());
    match child.eval(&format!("location.href = {literal};")) {
        Ok(()) => CommandResult::empty(),
        Err(e) => CommandResult::err(&format!("导航失败：{e}")),
    }
}

/// 刷新副 webview 当前页（`Webview::reload`）。
#[tauri::command]
pub async fn app_browser_reload(app: AppHandle) -> CommandResult<()> {
    let child = match child_or_err(&app) {
        Ok(c) => c,
        Err(ret) => return ret,
    };
    match child.reload() {
        Ok(()) => CommandResult::empty(),
        Err(e) => CommandResult::err(&format!("刷新失败：{e}")),
    }
}

/// 副 webview 后退（eval `history.back()`）。
#[tauri::command]
pub async fn app_browser_back(app: AppHandle) -> CommandResult<()> {
    let child = match child_or_err(&app) {
        Ok(c) => c,
        Err(ret) => return ret,
    };
    match child.eval("history.back();") {
        Ok(()) => CommandResult::empty(),
        Err(e) => CommandResult::err(&format!("后退失败：{e}")),
    }
}

/// 副 webview 前进（eval `history.forward()`）。
#[tauri::command]
pub async fn app_browser_forward(app: AppHandle) -> CommandResult<()> {
    let child = match child_or_err(&app) {
        Ok(c) => c,
        Err(ret) => return ret,
    };
    match child.eval("history.forward();") {
        Ok(()) => CommandResult::empty(),
        Err(e) => CommandResult::err(&format!("前进失败：{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 校园域（`*.cwxu.edu.cn`）命中白名单 → 进应用内 webview。
    #[test]
    fn campus_domain_goes_inapp() {
        assert!(matches!(decide_open("https://my.cwxu.edu.cn/xx"), OpenDecision::InApp));
    }

    /// 充值页内网 IP（慧新E校官网）是校园域语义；`is_allowed_info_url` 不覆盖，
    /// decide_open 需额外放行该 host。
    #[test]
    fn recharge_internal_ip_goes_inapp() {
        assert!(matches!(
            decide_open("http://10.3.100.110/charge-pc/pays/7"),
            OpenDecision::InApp
        ));
    }

    /// 域外公网站点 → 前端降级走旧 open_app 系统浏览器。
    #[test]
    fn external_domain_goes_external() {
        assert!(matches!(decide_open("https://kns.cnki.net/"), OpenDecision::External));
    }

    /// 非 http/https 协议一律拒绝。
    #[test]
    fn javascript_url_blocked() {
        assert!(matches!(decide_open("javascript:alert(1)"), OpenDecision::Blocked));
    }
}
