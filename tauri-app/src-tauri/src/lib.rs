//! 锡院助手 src-tauri 接线层：状态管理 + Tauri 命令注册。
//! 协议核心全部在 campus-auth crate，本 crate 只做 IPC 接线与本地持久化。

pub mod account;
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
        ])
        // M4 批 2 启动补采：今天还没采过 + 有内存会话时，后台补一次日余额快照。
        // 不弹窗、不阻塞启动（spawn 后立刻返回）、失败只记日志（不出现 token/账号/户号）。
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                commands::electricity_history::startup_snapshot(handle).await;
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
