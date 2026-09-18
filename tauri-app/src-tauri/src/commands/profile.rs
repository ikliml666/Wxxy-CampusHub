//! 头像存取与官方头像同步/上传命令（IPC 契约见计划 §四 冻结契约）。
//!
//! - 五命令统一返回 [`AvatarData`]：`{ imageBase64: string|null, source: "local"|"official"|null }`；
//!   裸 base64（不带 `data:` 前缀），前端自行拼 data URI（与 `CaptchaData.pngBase64` 同风格）。
//! - 落盘 `%APPDATA%/campushub/profile.json`：`{ localBase64?, officialBase64?, officialFetchedAt? }`。
//!   头像非凭据，明文 base64 落盘即可，不走 DPAPI。
//! - 生效优先级：本地 > 官方 > 无；`clear_avatar` 只清本地，官方保留。
//! - 官方头像经 [`CasClient::portal_login_info`] 拉取（会话 jar 内请求）；
//!   无会话 → 约定错误「请先登录」。
//! - `upload_official_avatar` 经 [`CasClient::portal_change_portrait`] 上传回学校：
//!   服务端原样存储不压缩，200KB 体积守卫在本端做（官方客户端同款口径）。

use super::auth::{session_client, CommandResult};
use crate::infra::state::{self, AppState};
use campus_auth::cas::CasClient;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

/// 本机单图 base64 上限 2MB（超限 err「本机头像过大（上限 2MB）」）。
const AVATAR_MAX_B64: usize = 2 * 1024 * 1024;
/// 学校官方头像上限：剥离 data URL 前缀后的裸 base64 长度 200KB
/// （官方客户端 size/1024<=200 同款口径；服务端原样存储，守卫必须由本端做）。
const OFFICIAL_AVATAR_MAX_B64: usize = 200 * 1024;
/// 无会话时同步/上传官方头像的约定文案（前端据此引导登录）。
const ERR_NO_SESSION: &str = "请先登录";

/// get/set/clear/sync 四命令统一返回形状（键恒在、值可 null，与冻结形状一致）。
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AvatarData {
    pub image_base64: Option<String>,
    pub source: Option<String>,
}

/// profile.json 落盘结构（camelCase，字段缺省即不存在）。
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ProfileFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    official_base64: Option<String>,
    /// epoch 毫秒字符串（风格对齐 account::store::now_ms）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    official_fetched_at: Option<String>,
}

fn profile_path(dir: &Path) -> std::path::PathBuf {
    dir.join("profile.json")
}

fn now_ms() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default()
}

fn read_profile(dir: &Path) -> ProfileFile {
    fs::read_to_string(profile_path(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_profile(dir: &Path, file: &ProfileFile) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let json = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
    fs::write(profile_path(dir), json).map_err(|e| format!("写 profile.json 失败: {e}"))
}

// ---------- 存储核心（同步纯逻辑，离线单测） ----------

/// 当前生效头像：本地 > 官方 > 无（纯函数供单测）。
fn current_avatar(file: &ProfileFile) -> AvatarData {
    if let Some(b64) = &file.local_base64 {
        AvatarData {
            image_base64: Some(b64.clone()),
            source: Some("local".to_string()),
        }
    } else if let Some(b64) = &file.official_base64 {
        AvatarData {
            image_base64: Some(b64.clone()),
            source: Some("official".to_string()),
        }
    } else {
        AvatarData {
            image_base64: None,
            source: None,
        }
    }
}

/// 头像 base64 校验（纯函数供单测）：空串 / 超限拒绝。
fn validate_avatar_b64(b64: &str) -> Result<(), String> {
    if b64.trim().is_empty() {
        return Err("头像数据为空".to_string());
    }
    if b64.len() > AVATAR_MAX_B64 {
        return Err(format!("本机头像过大（上限 {}MB）", AVATAR_MAX_B64 / 1024 / 1024));
    }
    Ok(())
}

/// 上传学校的 data URL 校验（纯函数供单测）：须以 `data:image/` 开头、
/// 剥离首个 `,` 之前的前缀后裸 base64 非空且 ≤ [`OFFICIAL_AVATAR_MAX_B64`]。
fn validate_official_data_url(data_url: &str) -> Result<(), String> {
    if !data_url.starts_with("data:image/") {
        return Err("头像数据 URL 非法".to_string());
    }
    let Some((_, bare)) = data_url.split_once(',') else {
        return Err("头像数据 URL 非法".to_string());
    };
    if bare.is_empty() {
        return Err("头像数据为空".to_string());
    }
    if bare.len() > OFFICIAL_AVATAR_MAX_B64 {
        return Err("学校头像上限 200KB，请调小尺寸或质量".to_string());
    }
    Ok(())
}

/// 写入本地头像（覆盖 localBase64），返回当前生效头像。
fn store_local_avatar(dir: &Path, b64: &str) -> Result<AvatarData, String> {
    let mut file = read_profile(dir);
    file.local_base64 = Some(b64.to_string());
    write_profile(dir, &file)?;
    Ok(current_avatar(&file))
}

/// 清本地头像（officialBase64 保留），返回当前生效头像。
fn clear_local_avatar(dir: &Path) -> Result<AvatarData, String> {
    let mut file = read_profile(dir);
    file.local_base64 = None;
    write_profile(dir, &file)?;
    Ok(current_avatar(&file))
}

/// 落盘官方头像（覆盖 officialBase64 + officialFetchedAt），返回当前生效头像。
fn store_official_avatar(dir: &Path, b64: &str) -> Result<AvatarData, String> {
    let mut file = read_profile(dir);
    file.official_base64 = Some(b64.to_string());
    file.official_fetched_at = Some(now_ms());
    write_profile(dir, &file)?;
    Ok(current_avatar(&file))
}

/// 无会话 → Err(约定文案「请先登录」)；有会话 → Ok(client)。
/// 同步函数便于离线单测覆盖文案契约（命令壳内锁纪律后先判会话再发网络）。
fn require_session_client(
    client: Option<CasClient>,
) -> Result<CasClient, CommandResult<AvatarData>> {
    client.ok_or_else(|| CommandResult::err(ERR_NO_SESSION))
}

// ---------- 命令面（5 条，invoke 名与计划 §四 冻结契约逐字对齐） ----------

/// 读取当前生效头像（本地 > 官方；都无则 imageBase64/source 为 null）。
#[tauri::command]
pub fn get_avatar() -> Result<CommandResult<AvatarData>, String> {
    let dir = state::data_dir()?;
    Ok(CommandResult::ok(current_avatar(&read_profile(&dir))))
}

/// 写入本地头像（覆盖）。空串 / 超过 2MB → err。
#[tauri::command]
pub fn set_avatar(image_base64: String) -> Result<CommandResult<AvatarData>, String> {
    if let Err(msg) = validate_avatar_b64(&image_base64) {
        return Ok(CommandResult::err(&msg));
    }
    let dir = state::data_dir()?;
    match store_local_avatar(&dir, &image_base64) {
        Ok(data) => Ok(CommandResult::ok(data)),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

/// 清本地头像（官方头像保留）。返回清完后当前生效头像。
#[tauri::command]
pub fn clear_avatar() -> Result<CommandResult<AvatarData>, String> {
    let dir = state::data_dir()?;
    match clear_local_avatar(&dir) {
        Ok(data) => Ok(CommandResult::ok(data)),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

/// 用当前会话拉官方头像（getLoginInfo.headPortrait）并落盘。
/// 无会话 → err「请先登录」；网络/解析失败 → err（不落盘、不清已有头像）。
#[tauri::command]
pub async fn sync_official_avatar(
    state: State<'_, AppState>,
) -> Result<CommandResult<AvatarData>, String> {
    let dir = state::data_dir()?;
    // 锁纪律：session_client 锁内 clone 出 client，drop guard 后才发网络请求。
    let client = match require_session_client(session_client(&state).await) {
        Ok(c) => c,
        Err(res) => return Ok(res),
    };
    match client.portal_login_info().await {
        Ok(b64) => match store_official_avatar(&dir, &b64) {
            Ok(data) => Ok(CommandResult::ok(data)),
            Err(e) => Ok(CommandResult::err(&e)),
        },
        Err(e) => Ok(CommandResult::err(&format!("获取官方头像失败: {e}"))),
    }
}

/// 上传头像回学校（portraitChange）并重拉官方头像落盘。
///
/// - 无会话 → err「请先登录」；data URL 非法 / 裸 base64 超 200KB → err（冻结文案）；
///   服务端 `meta.success=false` → err（message 原文）。
/// - 成功后重新 `portal_login_info` 拉官方头像落盘（`officialBase64`），返回最新
///   [`AvatarData`]——服务端原样存储，以服务端回读为准。
/// - 日志只记结果与打码用户名，**绝不记 data_url 内容**（体积可达数百 KB）。
#[tauri::command]
pub async fn upload_official_avatar(
    state: State<'_, AppState>,
    image_data_url: String,
) -> Result<CommandResult<AvatarData>, String> {
    let dir = state::data_dir()?;
    // 锁纪律：session_client 锁内 clone 出 client，drop guard 后才发网络请求。
    let client = match require_session_client(session_client(&state).await) {
        Ok(c) => c,
        Err(res) => return Ok(res),
    };
    if let Err(msg) = validate_official_data_url(&image_data_url) {
        return Ok(CommandResult::err(&msg));
    }
    // 打码用户名供日志（锁内 clone，纪律同上；与 auth.rs::mask_username 同款，跨模块
    // 最小复制、不为此扩可见性）。
    let masked_user = {
        let guard = state.session.lock().await;
        guard
            .as_ref()
            .map(|s| mask_username(&s.username))
            .unwrap_or_default()
    };
    if let Err(e) = client.portal_change_portrait(&image_data_url).await {
        log::warn!("上传学校头像失败: user={masked_user} err={e}");
        return Ok(CommandResult::err(&e.to_string()));
    }
    match client.portal_login_info().await {
        Ok(b64) => match store_official_avatar(&dir, &b64) {
            Ok(data) => {
                log::info!("上传学校头像成功: user={masked_user}");
                Ok(CommandResult::ok(data))
            }
            Err(e) => Ok(CommandResult::err(&e)),
        },
        Err(e) => Ok(CommandResult::err(&format!("上传成功但获取官方头像失败: {e}"))),
    }
}

/// 用户名打码（日志脱敏）：保留前 2 位 + 末位，其余以 *** 掩盖
/// （与 auth.rs::mask_username 同款实现）。
fn mask_username(username: &str) -> String {
    let chars: Vec<char> = username.chars().collect();
    match chars.len() {
        0 | 1 | 2 => "***".to_string(),
        n => format!(
            "{}***{}",
            chars[..2].iter().collect::<String>(),
            chars[n - 1]
        ),
    }
}

// ---------- 单测（离线：优先级轮转 / 超限拒绝 / 无会话文案契约） ----------

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 唯一临时目录（纳秒时间戳，测试间不冲突；不清理，系统临时目录自净）。
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("campushub-profile-{tag}-{n}"))
    }

    /// 存取轮转 + 优先级：官方 → 本地覆盖展示 → 清本地回落官方 → 全空为 null。
    #[test]
    fn avatar_roundtrip_and_priority() {
        let dir = temp_dir("priority");

        // 空文件：无头像
        assert_eq!(
            current_avatar(&read_profile(&dir)),
            AvatarData {
                image_base64: None,
                source: None,
            }
        );

        // 只落官方 → source=official
        let data = store_official_avatar(&dir, "OFFICIAL_B64").unwrap();
        assert_eq!(data.source.as_deref(), Some("official"));
        assert_eq!(data.image_base64.as_deref(), Some("OFFICIAL_B64"));
        // 落盘结构含 officialFetchedAt
        let file = read_profile(&dir);
        assert_eq!(file.official_base64.as_deref(), Some("OFFICIAL_B64"));
        assert!(file.official_fetched_at.is_some());

        // 再落本地 → 优先级切到 local
        let data = store_local_avatar(&dir, "LOCAL_B64").unwrap();
        assert_eq!(data.source.as_deref(), Some("local"));
        assert_eq!(data.image_base64.as_deref(), Some("LOCAL_B64"));
        // 官方字段仍在（clear 只清本地）
        assert_eq!(
            read_profile(&dir).official_base64.as_deref(),
            Some("OFFICIAL_B64")
        );

        // 清本地 → 回落官方
        let data = clear_local_avatar(&dir).unwrap();
        assert_eq!(data.source.as_deref(), Some("official"));
        assert_eq!(data.image_base64.as_deref(), Some("OFFICIAL_B64"));
        assert!(read_profile(&dir).local_base64.is_none());

        // 官方也清掉 → 双 null（模拟全部清空后的兜底形态）
        let mut file = read_profile(&dir);
        file.official_base64 = None;
        file.official_fetched_at = None;
        assert_eq!(current_avatar(&file).source, None);
    }

    /// set_avatar 校验：空串拒绝、超 2MB 拒绝（冻结文案）、恰好 2MB 通过。
    #[test]
    fn avatar_validation_rejects_empty_and_oversize() {
        assert_eq!(validate_avatar_b64("   ").unwrap_err(), "头像数据为空");
        let oversized = "A".repeat(2 * 1024 * 1024 + 1);
        assert_eq!(
            validate_avatar_b64(&oversized).unwrap_err(),
            "本机头像过大（上限 2MB）"
        );
        // 恰好 2MB 通过
        assert_eq!(validate_avatar_b64(&"A".repeat(2 * 1024 * 1024)), Ok(()));
        assert_eq!(validate_avatar_b64("aGVsbG8="), Ok(()));
    }

    /// 上传学校 data URL 校验：非法前缀 / 残缺 URL / 空 base64 拒绝；
    /// 恰好 200KB 通过（守卫按剥离前缀后的裸长度）、超 200KB 冻结文案拒绝。
    #[test]
    fn official_data_url_validation() {
        assert_eq!(
            validate_official_data_url("iVBORw0KGgo=").unwrap_err(),
            "头像数据 URL 非法"
        );
        assert_eq!(
            validate_official_data_url("data:text/plain;base64,AAAA").unwrap_err(),
            "头像数据 URL 非法"
        );
        assert_eq!(
            validate_official_data_url("data:image/png;base64").unwrap_err(),
            "头像数据 URL 非法"
        );
        assert_eq!(
            validate_official_data_url("data:image/png;base64,").unwrap_err(),
            "头像数据为空"
        );
        let ok_url = format!("data:image/jpeg;base64,{}", "A".repeat(200 * 1024));
        assert_eq!(validate_official_data_url(&ok_url), Ok(()));
        let oversized = format!("data:image/jpeg;base64,{}", "A".repeat(200 * 1024 + 1));
        assert_eq!(
            validate_official_data_url(&oversized).unwrap_err(),
            "学校头像上限 200KB，请调小尺寸或质量"
        );
    }

    /// 无会话同步的文案契约：success=false + message=「请先登录」（前端据此引导登录）。
    #[test]
    fn no_session_error_contract() {
        // CasClient 无 Debug，unwrap_err 不可用，用 match 取 Err 侧。
        let res = match require_session_client(None) {
            Ok(_) => panic!("无会话应返回 Err"),
            Err(res) => res,
        };
        assert!(!res.success);
        assert_eq!(res.message.as_deref(), Some("请先登录"));
        assert!(res.data.is_none());
    }
}
