//! CAS 登录命令面（IPC 契约见计划 Task 10 Interfaces；全部 DTO camelCase）。
//!
//! 登录内核 [`run_login`]：
//! - 自动模式：kaptcha → `captcha::solve` → login，仅 `WrongCaptcha` / 识别失败时重试，
//!   总提交 ≤[`MAX_LOGIN_ATTEMPTS`] 次；`WrongUserOrPwd` 绝不自动重试（防连续错误计数锁号）。
//! - 手动模式：前端传入验证码 uid + 用户答案，单次提交不重试。
//! - 成功后：`sso_follow(PORTAL_SERVICE)` 建门户会话 → `save_account`(DPAPI) →
//!   session.json 落盘 → AppState 写入。
//!
//! 敏感纪律：明文密码仅在内存中使用并立即 RSA 加密；日志用户名打码（[`mask_username`]）、
//! 密码绝不进入日志或错误消息。

use crate::account::store;
use crate::infra::state::{self, AppState, CasSession};
use base64::Engine as _;
use campus_auth::captcha::{self, KaptchaTemplates};
use campus_auth::cas::{
    CaptchaInfo, CasClient, CasLoginError, CasLoginOk, SessionState, CAS_BASE, PORTAL_SERVICE,
};
use campus_auth::rsa::rsa_encrypt_hex;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use tauri::State;

/// 自动识别重试上限（总提交次数；REPORT.md 连续错误计数防锁号）。
const MAX_LOGIN_ATTEMPTS: u32 = 3;
/// 三次自动识别穷尽时返回给前端的约定消息（前端据此切手动模式）。
const CAPTCHA_MANUAL: &str = "CAPTCHA_MANUAL";
/// CAS 登出请求超时（尽力而为，不阻塞本地清理）。
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(5);

/// 统一返回形态（计划 Global Constraints 冻结）：
/// `{ success, message?, data? }`，与前端 `types.ts::CommandResult<T>` 对齐
///（skip_serializing_if 使缺省字段与 TS `?:` 可选语义一致）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult<T> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

impl<T> CommandResult<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            message: None,
            data: Some(data),
        }
    }

    pub fn empty() -> Self {
        Self {
            success: true,
            message: None,
            data: None,
        }
    }

    pub fn err(msg: &str) -> Self {
        Self {
            success: false,
            message: Some(msg.to_string()),
            data: None,
        }
    }
}

// ---------- DTO（全部 camelCase，前端契约参数命名对齐） ----------

/// get_captcha → data（pngBase64 已是裸 base64，前端自行拼 data:image/png;base64,）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptchaData {
    pub uid: String,
    pub png_base64: String,
}

/// check_session → data。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckSessionData {
    pub logged_in: bool,
}

/// login / login_manual / login_saved 成功 → data。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginData {
    pub username: String,
    pub display_name: String,
}

/// login 系命令的 data 双形态（untagged：序列化时内联，前端拿到裸对象）：
/// 成功 = `{ username, displayName }`；三次识别穷尽 = `{ uid, pngBase64 }`（手动模式数据）。
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum LoginResultData {
    LoggedIn(LoginData),
    ManualNeeded(CaptchaData),
}

/// list_accounts → data。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountListData {
    pub accounts: Vec<AccountInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub username: String,
    pub last_login: String,
    /// 显示名（与 store::AccountRecord 对齐；无来源时缺省，前端回退 username）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// login / login_manual / login_saved 参数（前端 camelCase 字段经 serde 映射）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginArgs {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginManualArgs {
    pub username: String,
    pub password: String,
    pub captcha_uid: String,
    pub captcha_code: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginSavedArgs {
    pub username: String,
}

// ---------- 登录内核（错误映射 / 重试决策为纯函数，离线单测） ----------

/// 验证码提供方式。
enum CaptchaMode {
    /// 自动识别：kaptcha → solve → login，重试 ≤[`MAX_LOGIN_ATTEMPTS`]。
    Auto,
    /// 手动：前端传入验证码 uid + 用户答案，单次提交（错误由用户重输）。
    Manual { uid: String, code: String },
}

/// 用户名打码（日志脱敏）：保留前 2 位 + 末位，其余以 *** 掩盖。
fn mask_username(username: &str) -> String {
    let chars: Vec<char> = username.chars().collect();
    match chars.len() {
        0 => "***".to_string(),
        1..=2 => "***".to_string(),
        n => format!(
            "{}***{}",
            chars[..2].iter().collect::<String>(),
            chars[n - 1]
        ),
    }
}

/// 从服务端 data 原文解析「连续错误计数」提示。
/// REPORT.md：错误次数提示在 data（如 `"已连续错误N次,阈值M"`）。
/// 返回 Some((n, m))；n+1 >= m 表示再错一次即达阈值，必须停止自动重试。
fn parse_error_count_hint(data_raw: &str) -> Option<(u32, u32)> {
    let num_after = |marker: &str| -> Option<u32> {
        let pos = data_raw.find(marker)?;
        let digits: String = data_raw[pos + marker.len()..]
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        digits.parse().ok()
    };
    Some((num_after("已连续错误")?, num_after("阈值")?))
}

/// CasLoginError → 中文消息（错误码映射表，计划 Task 10 Step 1 冻结口径）。
/// 消息只含固定文案 / 错误码 / 计数提示，绝不含密码或完整用户名。
fn login_error_message(err: &CasLoginError) -> String {
    match err {
        CasLoginError::WrongUserOrPwd => "账号或密码错误".to_string(),
        CasLoginError::UserLocked => "账号已锁定".to_string(),
        CasLoginError::WrongCaptcha => "验证码错误".to_string(),
        CasLoginError::NeedTwoVerify(data) => match parse_error_count_hint(data) {
            Some((n, m)) if n + 1 >= m => format!(
                "登录错误次数即将达到阈值（已连续错误 {n} 次，阈值 {m} 次），已停止自动重试，请稍后再试"
            ),
            _ => "需要二次验证，请使用网页端完成验证后再试".to_string(),
        },
        CasLoginError::Unknown(code) => format!("登录失败（服务端返回码: {code}）"),
        CasLoginError::Network(e) => format!("网络错误: {e}"),
    }
}

/// 自动模式重试决策：仅 `WrongCaptcha` 重试且总提交 ≤[`MAX_LOGIN_ATTEMPTS`]；
/// 其余（含 WrongUserOrPwd / UserLocked / NeedTwoVerify / Unknown / Network）一律立即终止。
fn should_retry(err: &CasLoginError, attempt: u32) -> bool {
    matches!(err, CasLoginError::WrongCaptcha) && attempt < MAX_LOGIN_ATTEMPTS
}

/// 裸 base64 PNG → 算术验证码答案（None = 无法识别，上层刷新重试）。
fn solve_captcha(png_base64: &str, templates: &KaptchaTemplates) -> Option<String> {
    let png = base64::engine::general_purpose::STANDARD
        .decode(png_base64)
        .ok()?;
    captcha::solve(&png, templates)
}

/// 登录内核：三命令（login / login_manual / login_saved）共享。
/// 返回 Err<String> 仅限 IPC 框架层错误；业务失败一律 Ok(CommandResult::err)。
async fn run_login(
    state: &State<'_, AppState>,
    username: &str,
    password: &str,
    mode: CaptchaMode,
) -> Result<CommandResult<LoginResultData>, String> {
    log::info!("登录开始: user={}", mask_username(username));
    // 登录用全新 client：干净 jar，避免旧会话 cookie 干扰（前端 JS 登录前清 CASTGC 同语义）。
    let client = CasClient::new().map_err(|e| e.to_string())?;
    // 密码仅在内存中，立即 RSA 加密；此后全程只用密文。
    let password_rsa = rsa_encrypt_hex(password).map_err(|e| e.to_string())?;

    match mode {
        CaptchaMode::Manual { uid, code } => {
            // 手动模式：单次提交，验证码错误由用户刷新重输，不自动重试。
            match client.login(username, &password_rsa, &uid, &code).await {
                Ok(ok) => finish_login(state, client, username, password, ok).await,
                Err(e) => Ok(CommandResult::err(&login_error_message(&e))),
            }
        }
        CaptchaMode::Auto => {
            let templates = KaptchaTemplates::load();
            let mut last_captcha: Option<CaptchaInfo> = None;
            for attempt in 1..=MAX_LOGIN_ATTEMPTS {
                let info = match client.kaptcha().await {
                    Ok(i) => i,
                    Err(e) => {
                        return Ok(CommandResult::err(&format!("获取验证码失败: {e}")));
                    }
                };
                last_captcha = Some(info.clone());
                let answer = solve_captcha(&info.png_base64, &templates);
                let Some(answer) = answer else {
                    // 识别失败：未提交到 CAS，不消耗连续错误计数，刷新重试。
                    log::debug!(
                        "验证码识别失败（第 {attempt}/{MAX_LOGIN_ATTEMPTS} 次），刷新重试: user={}",
                        mask_username(username)
                    );
                    continue;
                };
                match client
                    .login(username, &password_rsa, &info.uid, &answer)
                    .await
                {
                    Ok(ok) => return finish_login(state, client, username, password, ok).await,
                    Err(e) if should_retry(&e, attempt) => {
                        log::debug!(
                            "提交被拒（验证码错误，第 {attempt}/{MAX_LOGIN_ATTEMPTS} 次），自动重试: user={}",
                            mask_username(username)
                        );
                        continue;
                    }
                    Err(e) => return Ok(CommandResult::err(&login_error_message(&e))),
                }
            }
            // 自动识别穷尽 → 转手动。穷尽前最后一次提交的 uid 可能已被 CAS 消耗，
            // 尽力刷新一张新验证码图供前端直接展示（刷新失败用最后一张兜底）。
            let manual = match client.kaptcha().await {
                Ok(i) => Some(i),
                Err(_) => last_captcha,
            };
            log::info!(
                "自动验证码识别穷尽，转手动模式: user={}",
                mask_username(username)
            );
            Ok(CommandResult {
                success: false,
                message: Some(CAPTCHA_MANUAL.to_string()),
                data: manual.map(|i| {
                    LoginResultData::ManualNeeded(CaptchaData {
                        uid: i.uid,
                        png_base64: i.png_base64,
                    })
                }),
            })
        }
    }
}

/// 登录成功收尾：sso_follow 建门户会话 → save_account(DPAPI) → session.json → AppState。
async fn finish_login(
    state: &State<'_, AppState>,
    client: CasClient,
    username: &str,
    password: &str,
    ok: CasLoginOk,
) -> Result<CommandResult<LoginResultData>, String> {
    // 门户会话建立（302 落点 cookie 种进本 client 的 jar；REPORT.md 三节实测）。
    if let Err(e) = client.sso_follow(PORTAL_SERVICE, &ok.ticket).await {
        return Ok(CommandResult::err(&format!("门户会话建立失败: {e}")));
    }
    let dir = state::data_dir()?;
    // 尽力而为取门户真实姓名（tryLoginUserInfo）：失败只警告，绝不影响登录成功。
    // 姓名不进日志（敏感纪律）；失败时保留上次已存的显示名（save_account 为整条
    // upsert，直接传 None 会把旧 displayName 抹掉）。
    let display_name: Option<String> = match client.portal_user_profile().await {
        Ok(profile) => Some(profile.name),
        Err(e) => {
            log::warn!(
                "门户资料获取失败，displayName 回退学号: {e}: user={}",
                mask_username(username)
            );
            store::load_accounts(&dir)
                .ok()
                .and_then(|accs| {
                    accs.into_iter()
                        .find(|r| r.username == username)
                        .and_then(|r| r.display_name)
                })
        }
    };
    // 账号存储（DPAPI 加密后落盘 accounts.json）。失败不阻断已建立的会话，降级为日志。
    if let Err(e) = store::save_account(&dir, username, password, display_name.as_deref()) {
        log::warn!("账号保存失败（会话已建立，本次登录不受影响）: {e}");
    }
    // 会话落盘（cookie 逐条 DPAPI 加密，P0-3「重启保持」）。
    let snapshot = client.jar().snapshot();
    if let Err(e) = state::persist_session(&dir, username, &snapshot) {
        log::warn!("会话持久化失败: {e}");
    }
    // 锁纪律：同步赋值，guard 在语句末 drop，此后无 await。
    let portal = campus_portal::PortalClient::new(client.clone());
    *state.session.lock().await = Some(CasSession {
        client,
        username: username.to_string(),
        portal,
    });
    log::info!("登录成功: user={}", mask_username(username));
    Ok(CommandResult::ok(LoginResultData::LoggedIn(LoginData {
        username: username.to_string(),
        // displayName 无来源时用 username（计划 Task 10 Interfaces）
        display_name: display_name.unwrap_or_else(|| username.to_string()),
    })))
}

// ---------- 内部辅助 ----------

/// 取当前会话 client。锁纪律：锁内只 clone（reqwest::Client 为 Arc 包装，廉价），
/// drop guard 后再 await。profile 模块的 sync_official_avatar 同样取用。
pub(crate) async fn session_client(state: &State<'_, AppState>) -> Option<CasClient> {
    state
        .session
        .lock()
        .await
        .as_ref()
        .map(|s| s.client.clone())
}

/// 已存密码取用（login_saved 前置；解密失败返回明确中文错误，路径有单测）。
fn saved_password(dir: &Path, username: &str) -> Result<String, String> {
    store::load_password(dir, username)
}

// ---------- 命令面（7 条，invoke 名与前端 tauriApi 契约逐字对齐） ----------

/// 获取验证码（手动模式用）。无会话时用临时 client（kaptcha 请求不依赖已登录态）。
#[tauri::command]
pub async fn get_captcha(state: State<'_, AppState>) -> Result<CommandResult<CaptchaData>, String> {
    let client = match session_client(&state).await {
        Some(c) => c,
        None => match CasClient::new() {
            Ok(c) => c,
            Err(e) => return Ok(CommandResult::err(&e.to_string())),
        },
    };
    match client.kaptcha().await {
        Ok(info) => Ok(CommandResult::ok(CaptchaData {
            uid: info.uid,
            png_base64: info.png_base64,
        })),
        Err(e) => Ok(CommandResult::err(&e.to_string())),
    }
}

/// 登录（自动验证码识别）。
#[tauri::command]
pub async fn login(
    state: State<'_, AppState>,
    account: LoginArgs,
) -> Result<CommandResult<LoginResultData>, String> {
    run_login(
        &state,
        &account.username,
        &account.password,
        CaptchaMode::Auto,
    )
    .await
}

/// 手动验证码登录（自动识别穷尽后前端切此模式）。
#[tauri::command]
pub async fn login_manual(
    state: State<'_, AppState>,
    account: LoginManualArgs,
) -> Result<CommandResult<LoginResultData>, String> {
    run_login(
        &state,
        &account.username,
        &account.password,
        CaptchaMode::Manual {
            uid: account.captcha_uid,
            code: account.captcha_code,
        },
    )
    .await
}

/// 已存账号一键重登：DPAPI 解密已存密码 → 复用 login 自动流程。
#[tauri::command]
pub async fn login_saved(
    state: State<'_, AppState>,
    account: LoginSavedArgs,
) -> Result<CommandResult<LoginResultData>, String> {
    let dir = state::data_dir()?;
    // 解密失败（坏密文/账号不存在）在此直接返回中文错误，不发起任何网络请求。
    let password = match saved_password(&dir, &account.username) {
        Ok(p) => p,
        Err(e) => return Ok(CommandResult::err(&e)),
    };
    run_login(&state, &account.username, &password, CaptchaMode::Auto).await
}

/// 会话状态确认：jar 有 customsid + 门户探测 Alive 双确认（portal_probe 内实现）。
/// false（无会话/已过期/探测失败）时清 AppState 会话与 session.json（派单：失败时清 session.json）。
#[tauri::command]
pub async fn check_session(
    state: State<'_, AppState>,
) -> Result<CommandResult<CheckSessionData>, String> {
    let logged_in = match session_client(&state).await {
        Some(client) => client.portal_probe().await == SessionState::Alive,
        None => false,
    };
    if !logged_in {
        state.session.lock().await.take();
        if let Ok(dir) = state::data_dir() {
            state::clear_session(&dir);
        }
    }
    Ok(CommandResult::ok(CheckSessionData { logged_in }))
}

/// 登出：CAS 全局登出（尽力而为）+ 清 AppState 会话 + 删 session.json。
#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> Result<CommandResult<()>, String> {
    if let Some(client) = session_client(&state).await {
        // CAS 标准登出端点 GET {CAS_BASE}/logout 销毁全局会话（REPORT.md 登出语义）。
        // 复用 sso_follow 发 GET（其仅追加 ticket 参数，服务端忽略）；REST 登录无 CASTGC，
        // 服务端可能本就无全局会话，失败不影响本地清理。
        let url = format!("{CAS_BASE}/logout");
        let _ = tokio::time::timeout(LOGOUT_TIMEOUT, client.sso_follow(&url, "")).await;
    }
    state.session.lock().await.take();
    if let Ok(dir) = state::data_dir() {
        state::clear_session(&dir);
    }
    log::info!("登出完成");
    Ok(CommandResult::empty())
}

/// 已保存账号列表（密码绝不出现在返回里）。
#[tauri::command]
pub async fn list_accounts() -> Result<CommandResult<AccountListData>, String> {
    let dir = state::data_dir()?;
    match store::load_accounts(&dir) {
        Ok(records) => Ok(CommandResult::ok(AccountListData {
            accounts: records
                .into_iter()
                .map(|r| AccountInfo {
                    username: r.username,
                    last_login: r.last_login,
                    display_name: r.display_name,
                })
                .collect(),
        })),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

/// 删除已保存账号（复用 store::remove_account，错误原文为中文，透传）。
#[tauri::command]
pub async fn remove_account(username: String) -> Result<CommandResult<()>, String> {
    let dir = state::data_dir()?;
    match store::remove_account(&dir, &username) {
        Ok(()) => Ok(CommandResult::empty()),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

// ---------- 单测（离线：重试状态机 / 错误映射 / 计数提示解析 / 解密失败路径 / 打码） ----------

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("campushub-auth-{tag}-{n}"))
    }

    /// 错误码 → 中文消息映射（计划 Task 10 Step 1 冻结口径）。
    #[test]
    fn login_error_message_maps_codes() {
        assert_eq!(
            login_error_message(&CasLoginError::WrongUserOrPwd),
            "账号或密码错误"
        );
        assert_eq!(
            login_error_message(&CasLoginError::UserLocked),
            "账号已锁定"
        );
        assert_eq!(
            login_error_message(&CasLoginError::WrongCaptcha),
            "验证码错误"
        );
        assert!(
            login_error_message(&CasLoginError::Unknown("ISMODIFYPASS".into()))
                .contains("ISMODIFYPASS")
        );
        assert!(
            login_error_message(&CasLoginError::Network("timeout".into())).contains("网络错误")
        );
        // 消息不泄露原文 data（敏感纪律）
        let msg = login_error_message(&CasLoginError::NeedTwoVerify("anything".into()));
        assert!(!msg.contains("anything"));
        assert!(msg.contains("二次验证"));
    }

    /// 重试状态机：仅 WrongCaptcha 重试且总提交 ≤3；WrongUserOrPwd 绝不重试。
    #[test]
    fn retry_policy_only_wrong_captcha_within_three_attempts() {
        for attempt in 1..MAX_LOGIN_ATTEMPTS {
            assert!(should_retry(&CasLoginError::WrongCaptcha, attempt));
        }
        // 第 3 次失败后穷尽，不再重试
        assert!(!should_retry(
            &CasLoginError::WrongCaptcha,
            MAX_LOGIN_ATTEMPTS
        ));
        // 其余错误一律立即终止
        assert!(!should_retry(&CasLoginError::WrongUserOrPwd, 1));
        assert!(!should_retry(&CasLoginError::UserLocked, 1));
        assert!(!should_retry(&CasLoginError::NeedTwoVerify("y".into()), 1));
        assert!(!should_retry(
            &CasLoginError::Unknown("NOAUTHORIZATION".into()),
            1
        ));
        assert!(!should_retry(&CasLoginError::Network("x".into()), 1));
    }

    /// 连续错误计数提示解析（REPORT.md：data 如 "已连续错误N次,阈值M"）与接近阈值文案。
    #[test]
    fn error_count_hint_parsing_and_threshold_message() {
        assert_eq!(
            parse_error_count_hint("\"已连续错误2次,阈值5\""),
            Some((2, 5))
        );
        assert_eq!(
            parse_error_count_hint("已连续错误10次,阈值10"),
            Some((10, 10))
        );
        assert_eq!(parse_error_count_hint("无提示"), None);
        assert_eq!(parse_error_count_hint("已连续错误2次,缺阈值"), None);
        // 接近阈值（n+1 >= m）→ 明确提示停止自动重试
        let near = login_error_message(&CasLoginError::NeedTwoVerify(
            "\"已连续错误4次,阈值5\"".into(),
        ));
        assert!(near.contains("阈值"));
        let far = login_error_message(&CasLoginError::NeedTwoVerify(
            "\"已连续错误1次,阈值5\"".into(),
        ));
        assert!(far.contains("二次验证"));
    }

    /// login_saved 解密失败路径：坏密文 / 账号不存在 → 明确中文错误（不发起网络请求）。
    #[test]
    fn login_saved_decrypt_failure_paths() {
        let dir = temp_dir("decrypt-fail");
        fs::create_dir_all(&dir).unwrap();
        let bad_b64 = r#"{"accounts":[{"username":"2023999","passwordB64":"!!!非法base64!!!","lastLogin":"0"}]}"#;
        fs::write(dir.join("accounts.json"), bad_b64).unwrap();

        // 坏密文 → 解密报错（DPAPI/Base64 层）
        let err = saved_password(&dir, "2023999").unwrap_err();
        assert!(
            err.contains("Base64") || err.contains("解密"),
            "实际错误: {err}"
        );
        // 账号不存在 → 明确错误
        assert!(saved_password(&dir, "nobody")
            .unwrap_err()
            .contains("不存在"));

        fs::remove_dir_all(&dir).ok();
    }

    /// 用户名打码（日志脱敏）。
    #[test]
    fn username_masking() {
        assert_eq!(mask_username("2023001"), "20***1");
        assert_eq!(mask_username("ab"), "***");
        assert_eq!(mask_username(""), "***");
        assert_eq!(mask_username("a1b2c3d4e5"), "a1***5");
    }
}
