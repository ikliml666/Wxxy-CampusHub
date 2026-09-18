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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
