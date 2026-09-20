//! 托盘常驻（M5 批 4）：系统托盘图标 + 「显示主窗口 / 退出」菜单 + 左键唤起窗口。
//!
//! # 关闭行为语义
//!
//! 主窗口关闭按钮 = **隐藏到托盘**（`lib.rs` 的 `on_window_event` 拦截
//! `CloseRequested` 后 hide），应用本体常驻托盘继续跑通知轮询；真退出只走
//! 托盘菜单「退出」（[`quit`] → `app.exit`）。图标加载三级回落：编译期嵌入的
//! `default_window_icon()` → `include_bytes!` 的 `icons/icon.ico` → 空图标
//! （托盘创建失败只 log，绝不阻塞应用启动）。托盘缺席时关窗语义降级：`lib.rs`
//! 检查 [`TRAY_READY`]，未建成则不拦截关窗（走默认关闭退出应用）——窗口藏
//! 起来却没有托盘可唤回，比退出更糟。
//!
//! 参照 Wxxy-CampusLogin `app/tray.rs` 精简：无动态菜单刷新 / 子菜单 /
//! spawn_blocking（本应用菜单静态，事件处理均为瞬时操作）。

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

/// 托盘图标 id（预留：后续动态刷新菜单时经 `app.tray_by_id` 定位）。
const TRAY_ID: &str = "main-tray";

/// 托盘是否成功建成：`lib.rs` 的关窗拦截以此为准——托盘缺席时主窗口关闭
/// **不拦截**（走默认关闭退出应用），否则窗口藏起来就再也唤不回了（P1）。
pub(crate) static TRAY_READY: AtomicBool = AtomicBool::new(false);

/// 显示并聚焦主窗口（已最小化则还原；窗口不存在时静默跳过）。
fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// 真退出（托盘菜单「退出」专用；主窗口关闭不经过这里）。
fn quit(app: &AppHandle) {
    app.exit(0);
}

/// 构建并注册托盘图标与菜单（`lib.rs` setup 调用，一次）。
pub fn build_tray(app: &AppHandle) {
    let menu = match build_menu(app) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("[tray] 构建托盘菜单失败: {e}");
            return;
        }
    };

    let icon = app
        .default_window_icon()
        .cloned()
        .or_else(|| {
            tauri::image::Image::from_bytes(include_bytes!("../icons/icon.ico")).ok()
        })
        .unwrap_or_else(|| {
            log::warn!("[tray] 托盘图标加载失败，使用空图标");
            tauri::image::Image::new(&[], 0, 0)
        });

    // 托盘图标创建失败不影响应用运行（缺托盘只是少了常驻入口），但要记下
    // TRAY_READY = false：关窗拦截放行，主窗口关闭走默认退出（否则藏起来的
    // 窗口没有托盘可唤回）。
    match TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("锡院助手")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // 左键点击 = 唤起主窗口；右键由 show_menu_on_left_click(false) 弹菜单
            if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
                show_main(tray.app_handle());
            }
        })
        .build(app)
    {
        Ok(_) => TRAY_READY.store(true, Ordering::Relaxed),
        Err(e) => log::warn!("[tray] 托盘创建失败（主窗口关闭将直接退出）: {e}"),
    }
}

/// 两项静态菜单：显示主窗口 / 退出。
fn build_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    let show = MenuItemBuilder::with_id("show", "显示主窗口").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "退出").build(app)?;
    MenuBuilder::new(app).item(&show).separator().item(&quit_item).build()
}
