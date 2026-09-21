//! 锡院助手 src-tauri 接线层：状态管理 + Tauri 命令注册。
//! 协议核心全部在 campus-auth crate，本 crate 只做 IPC 接线与本地持久化。

pub mod account;
pub mod app_tray;
pub mod commands;
pub mod infra;

use infra::state::AppState;

pub fn run() {
    // 启动时回填会话：读 session.json → DPAPI 解密 cookie → restore 进新 client
    //（P0-3「重启保持」；manage 之前算好，避免 setup 内碰 tokio Mutex）
    let restored = infra::state::restore_session();
    tauri::Builder::default()
        .manage(AppState::new(restored))
        // 系统浏览器/文件打开（open_in_browser 命令在 Rust 侧调用其 API，
        // 不开放前端直接 invoke 插件命令，故无需额外 capability）
        .plugin(tauri_plugin_opener::init())
        // 系统通知（M5 通知中心）：只在 Rust 侧 poll_tick 经 NotificationExt 发送，
        // 同样不开放前端 invoke，无需额外 capability
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            commands::auth::get_captcha,
            commands::auth::login,
            commands::auth::login_manual,
            commands::auth::login_saved,
            commands::auth::check_session,
            commands::auth::logout,
            commands::auth::list_accounts,
            commands::auth::remove_account,
            commands::profile::get_avatar,
            commands::profile::set_avatar,
            commands::profile::clear_avatar,
            commands::profile::sync_official_avatar,
            commands::profile::upload_official_avatar,
            commands::portal::get_portal_overview,
            commands::portal::get_info_columns,
            commands::portal::get_info_list,
            commands::portal::get_info_detail,
            commands::portal::get_todo_tabs,
            commands::portal::get_todo_list,
            commands::portal::open_in_browser,
            commands::portal::get_app_catalog,
            commands::portal::get_schedule_classify,
            commands::portal::get_schedule_month,
            commands::portal::get_schedule_day_counts,
            commands::portal::open_app,
            commands::portal::download_attachment,
            commands::timetable::get_timetable,
            commands::timetable::import_timetable,
            commands::timetable::add_course_manual,
            commands::timetable::update_course,
            commands::timetable::delete_course,
            commands::timetable::export_ics,
            commands::timetable::parse_notice,
            commands::timetable::apply_override,
            commands::timetable::revoke_notice,
            commands::timetable::save_time_slots,
            commands::timetable::save_semester_config,
            commands::timetable::save_skipped_dates,
            commands::timetable::save_slot_rules,
            commands::timetable::export_timetable_json,
            commands::timetable::import_timetable_json,
            commands::timetable::move_day_courses,
            commands::timetable::quick_delete,
            commands::timetable::get_today_courses,
            commands::timetable::list_schedule_notices,
            commands::timetable::auto_parse_notices,
            commands::timetable::apply_swap_day,
            commands::timetable::save_swap_days,
            commands::timetable::fetch_holidays,
            commands::timetable::parse_notice_from_url,
            commands::synjones::get_ecard,
            commands::synjones::get_ecard_transactions,
            commands::synjones::get_wallet_cards,
            commands::electricity::list_feeitems,
            commands::electricity::query_electricity,
            commands::electricity::get_electricity_rooms,
            commands::electricity::save_electricity_room,
            commands::electricity::delete_electricity_room,
            commands::electricity::open_recharge_in_browser,
            commands::electricity::recharge_create,
            commands::electricity::recharge_pay_methods,
            commands::electricity::recharge_query_account,
            commands::electricity::recharge_submit,
            commands::electricity::recharge_status,
            commands::electricity::recharge_cancel,
            commands::electricity_history::get_electricity_bills,
            commands::electricity_history::get_electricity_monthly,
            commands::electricity_history::get_electricity_orders,
            commands::electricity_history::get_electricity_history,
            commands::electricity_history::bind_electricity_room,
            commands::electricity_history::run_electricity_snapshot,
            commands::ecard::get_ecard_overview,
            commands::ecard::get_ecard_types,
            commands::ecard::get_ecard_stats_summary,
            commands::ecard::get_ecard_stats_series,
            commands::ecard::get_ecard_stats_assort,
            commands::ecard::get_ecard_secure_keyboard,
            commands::ecard::ecard_lost,
            commands::ecard::ecard_unlost,
            commands::ecard::ecard_check_pwd,
            commands::ecard::ecard_modify_pwd,
            commands::ecard::ecard_send_find_pwd_code,
            commands::ecard::ecard_find_pwd,
            commands::ecard::ecard_set_limits,
            commands::ecard::ecard_set_autotrans,
            commands::ecard::ecard_face_detail,
            commands::ecard::ecard_face_upload,
            commands::ecard::get_plat_profile,
            commands::ecard::get_plat_equipment,
            commands::ecard::get_plat_login_logs,
            commands::ecard::get_plat_offline_switch,
            commands::ecard::plat_offline_device,
            commands::ecard::plat_remove_device,
            commands::ecard::get_ecard_paycode,
            commands::ecard::get_ecard_paycode_settings,
            commands::ecard::ecard_send_bind_bank_code,
            commands::ecard::ecard_bind_bank,
            commands::ecard::ecard_cancel_bank,
            commands::ecard::ecard_send_bind_user_code,
            commands::ecard::ecard_bind_user,
            commands::ecard::ecard_unbind_user,
            commands::notification::get_notifications,
            commands::notification::mark_notifications_read,
            commands::notification::get_notification_settings,
            commands::notification::save_notification_settings,
        ])
        // M4 批 2 启动补采：今天还没采过 + 有内存会话时，后台补一次日余额快照。
        // 不弹窗、不阻塞启动（spawn 后立刻返回）、失败只记日志（不出现 token/账号/户号）。
        .setup(|app| {
            // 日志初始化：log::warn/error 的输出端（默认 info 级、RUST_LOG 可覆盖）。
            // 此前无 logger——后台轮询（poll_tick/auto_sync_tick）的失败 warn 全部
            // 被静默丢弃，排障盲区（M2 [meeting-diag] 教训重演）。
            let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                .try_init();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                commands::electricity_history::startup_snapshot(handle).await;
            });
            // 每日自动同步（2026-09-20）：启动即检查一次，此后每小时检查（覆盖
            // 长开跨天）。教务课表为准 + 法定节假日刷新，闸与护栏见
            // commands::timetable::auto_sync_tick。不弹窗、失败静默重试。
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    commands::timetable::auto_sync_tick(handle.clone()).await;
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                }
            });
            // 通知中心轮询（M5 批 2）：门户资讯 / 待办 / 电费低余额三类检查合一，
            // 每 tick 热读 settings 拿间隔与开关；无会话静默跳过、单源失败退避
            // （×2 封顶 2h）、新通知发系统通知并进通知中心。见
            // commands::notification::poll_tick。不弹窗、失败只记日志。
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                commands::notification::poll_tick(handle).await;
            });
            // 托盘常驻（M5 批 4）：图标 + 「显示主窗口 / 退出」菜单，左键唤起
            // 窗口。创建失败只 log（缺托盘不影响应用）。
            app_tray::build_tray(app.handle());
            Ok(())
        })
        // 主窗口关闭 = 隐藏到托盘（托盘常驻语义，M5 批 4）：拦截 CloseRequested
        // 后仅 hide，应用继续跑后台轮询；真退出只走托盘菜单「退出」→ app.exit。
        // 托盘未建成（app_tray::TRAY_READY = false）时不拦截：走默认关闭退出，
        // 否则窗口藏起来没有托盘可唤回（P1）。
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" && app_tray::TRAY_READY.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
