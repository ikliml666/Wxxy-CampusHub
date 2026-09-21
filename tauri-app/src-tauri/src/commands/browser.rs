//! 应用内浏览器（in-app browser）命令。
//!
//! Task 1（2026-09-21）：本文件当前仅为 spike——在真机验证 tauri 2 多 webview API 面
//! （主 webview 缩顶 / add_child / initialization_script / on_navigation / close /
//! WebView2 profile 持久化），验证入口有三条：
//! 1. setup 定时任务自动时序（t+5s run → t+15s close → t+35s 换域名 run，见 lib.rs）；
//! 2. 前端 `invoke("spike_inapp_webview")` / `invoke("spike_close_inapp")`；
//! 3. 真机控制台 `[spike]` 打点逐条比对六问。
//! 正式的 open_in_app_browser 等命令由 Task 3 在本文件重写，勿在本版上做正式功能。
//!
//! API 事实（tauri 2.11.5，以 cargo registry 源码核对 + 编译为准）：
//! - 多 webview API（get_window/get_webview/add_child）需 tauri feature "unstable"；
//! - `Window::add_child(builder, position: impl Into<Position>, size: impl Into<Size>)`
//!   三参数直接带位置与尺寸（brief 骨架的一参数版不存在）；
//! - 无 `remove_webview` / `remove_webview_by_label`，销毁副 webview 用 `Webview::close()`；
//! - `Webview::set_bounds(tauri::Rect)`，Rect { position: Position, size: Size }，字段非 Option。

use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewBuilder,
};

/// Result 打点格式化：ok / err: 原因
fn fmt_res(r: &tauri::Result<()>) -> String {
    match r {
        Ok(()) => "ok".into(),
        Err(e) => format!("err: {e}"),
    }
}

/// spike 核心逻辑（命令与 setup 定时任务共用）：主 webview 缩顶 48 逻辑px +
/// 第二 webview 加载 url 占剩余区 + initialization_script 注入 + on_navigation
/// 拦截 baidu.com。每步成功/失败都 eprintln! 打点（`[spike]` 前缀），真机只看控制台。
pub fn spike_run_with_url(app: &AppHandle, url: &str) -> Result<(), String> {
    let window = app.get_window("main").ok_or("no main window")?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let w = size.width as f64 / scale;
    let h = size.height as f64 / scale;

    // 可重入保护：上次 spike 残留的 app-browser 先移除，保证 run 可重复触发
    match app.get_webview("app-browser") {
        Some(stale) => {
            let r = stale.close();
            eprintln!("[spike] step=close_stale: {}", fmt_res(&r));
        }
        None => eprintln!("[spike] step=close_stale: skip (no stale child)"),
    }

    // 1) 主 webview 缩顶（React UI 暂被裁剪到 48px 条内属预期；六问 1）
    match app.get_webview("main") {
        Some(main_wv) => {
            let r = main_wv.set_bounds(tauri::Rect {
                position: LogicalPosition::new(0.0, 0.0).into(),
                size: LogicalSize::new(w, 48.0).into(),
            });
            eprintln!("[spike] step=main_top_bounds rect=(0,0 {w}x48): {}", fmt_res(&r));
        }
        None => eprintln!("[spike] step=main_top_bounds err: no webview 'main'"),
    }

    // 2) 第二 webview：注入脚本 + 导航拦截；位置与尺寸随 add_child 三参数直接传入
    let parsed: tauri::Url = url.parse().map_err(|e| format!("url parse: {e}"))?;
    // 注入脚本（六问 2）：改标题前缀 + 挂 window.__inapp + 往 DOM 插红条标记，
    // 标记条在页内可见，截图即可旁证注入是否执行（副 webview 无 devtools 打点通道）。
    let init_js = "document.title='[injected] '+document.title; window.__inapp=1;\
        document.addEventListener('DOMContentLoaded',function(){\
            var b=document.createElement('div');\
            b.textContent='[spike] injected OK __inapp='+window.__inapp;\
            b.style.cssText='position:fixed;top:0;left:0;z-index:999999;background:#c00;color:#fff;font:14px monospace;padding:4px 8px';\
            (document.body||document.documentElement).appendChild(b);\
        });";
    let wv = window
        .add_child(
            WebviewBuilder::new("app-browser", WebviewUrl::External(parsed))
                .initialization_script(init_js)
                // 导航拦截（六问 3）：返回 false 拦下 baidu.com；回调打点即旁证注册生效
                .on_navigation(|u| {
                    let allow = !u.as_str().contains("baidu.com");
                    eprintln!(
                        "[spike] on_navigation {} allowed={}",
                        u.as_str(),
                        allow
                    );
                    allow
                }),
            LogicalPosition::new(0.0, 48.0),
            LogicalSize::new(w, h - 48.0),
        )
        .map_err(|e| format!("add_child: {e}"))?;
    eprintln!(
        "[spike] step=add_child label=app-browser rect=(0,48 {w}x{}) url={url}: ok (init_script/on_navigation attached)",
        h - 48.0
    );

    // 3) 运行期 bounds 通道复验：add_child 传参之外再显式 set_bounds 一次（六问 5 相关）
    let r = wv.set_bounds(tauri::Rect {
        position: LogicalPosition::new(0.0, 48.0).into(),
        size: LogicalSize::new(w, h - 48.0).into(),
    });
    eprintln!("[spike] step=child_set_bounds rect=(0,48 {w}x{}): {}", h - 48.0, fmt_res(&r));
    Ok(())
}

/// spike 收尾核心逻辑（命令与 setup 定时任务共用）：close 副 webview +
/// 主 webview 复原为整窗物理 bounds。对应六问 4。
pub fn spike_close_core(app: &AppHandle) -> Result<(), String> {
    let window = app.get_window("main").ok_or("no main window")?;
    // 销毁副 webview：tauri 2 无 remove_webview，Webview::close 即销毁途径
    match app.get_webview("app-browser") {
        Some(wv) => {
            let r = wv.close();
            eprintln!("[spike] step=close_child: {}", fmt_res(&r));
        }
        None => eprintln!("[spike] step=close_child: skip (no app-browser)"),
    }
    // 主 webview 复原整窗（物理坐标，与 window.inner_size() 同源最稳）
    match app.get_webview("main") {
        Some(main_wv) => {
            let size = window.inner_size().map_err(|e| e.to_string())?;
            let r = main_wv.set_bounds(tauri::Rect {
                position: PhysicalPosition::new(0, 0).into(),
                size: size.into(),
            });
            eprintln!(
                "[spike] step=main_restore rect=(0,0 {}x{}): {}",
                size.width, size.height, fmt_res(&r)
            );
        }
        None => eprintln!("[spike] step=main_restore err: no webview 'main'"),
    }
    Ok(())
}

/// spike：主 webview 缩顶 48 逻辑px + 第二 webview 加载 url 占剩余区
/// + initialization_script 注入 + on_navigation 拦截 baidu.com。
/// 触发：前端 `invoke("spike_inapp_webview")`（定时任务会自动跑同逻辑）；
/// 可选参数 url：六问 6 需重开载 my.cwxu.edu.cn 验 profile 持久化，缺省 example.com。
#[tauri::command]
pub async fn spike_inapp_webview(app: AppHandle, url: Option<String>) -> Result<(), String> {
    let url = url.unwrap_or_else(|| "https://example.com".into());
    spike_run_with_url(&app, &url)
}

/// spike：移除第二 webview 并复原主 webview 为整窗。
#[tauri::command]
pub async fn spike_close_inapp(app: AppHandle) -> Result<(), String> {
    spike_close_core(&app)
}
