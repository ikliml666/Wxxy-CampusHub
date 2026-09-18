//! 头像存取与官方头像同步命令（IPC 契约见计划 §四 冻结契约）。
//!
//! - 四命令统一返回 [`AvatarData`]：`{ imageBase64: string|null, source: "local"|"official"|null }`；
//!   裸 base64（不带 `data:` 前缀），前端自行拼 data URI（与 `CaptchaData.pngBase64` 同风格）。
//! - 落盘 `%APPDATA%/campushub/profile.json`：`{ localBase64?, officialBase64?, officialFetchedAt? }`。
//!   头像非凭据，明文 base64 落盘即可，不走 DPAPI。
//! - 生效优先级：本地 > 官方 > 无；`clear_avatar` 只清本地，官方保留。
//! - 官方头像经 [`CasClient::portal_login_info`] 拉取（会话 jar 内请求）；
//!   无会话 → 约定错误「请先登录」。

use super::auth::{session_client, CommandResult};
use crate::infra::state::{self, AppState};
use campus_auth::cas::CasClient;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

/// 单图 base64 上限 512KB（冻结契约：超限 err「头像文件过大（上限 512KB）」）。
const AVATAR_MAX_B64: usize = 512 * 1024;
/// 无会话时同步官方头像的约定文案（前端据此引导登录）。
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
        return Err(format!("头像文件过大（上限 {}KB）", AVATAR_MAX_B64 / 1024));
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

// ---------- 命令面（4 条，invoke 名与计划 §四 冻结契约逐字对齐） ----------

/// 读取当前生效头像（本地 > 官方；都无则 imageBase64/source 为 null）。
#[tauri::command]
pub fn get_avatar() -> Result<CommandResult<AvatarData>, String> {
    let dir = state::data_dir()?;
    Ok(CommandResult::ok(current_avatar(&read_profile(&dir))))
}

/// 写入本地头像（覆盖）。空串 / 超过 512KB → err。
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

    /// set_avatar 校验：空串拒绝、超 512KB 拒绝（冻结文案）、正常通过。
    #[test]
    fn avatar_validation_rejects_empty_and_oversize() {
        assert_eq!(validate_avatar_b64("   ").unwrap_err(), "头像数据为空");
        let oversized = "A".repeat(512 * 1024 + 1);
        assert_eq!(
            validate_avatar_b64(&oversized).unwrap_err(),
            "头像文件过大（上限 512KB）"
        );
        // 恰好 512KB 通过
        assert_eq!(validate_avatar_b64(&"A".repeat(512 * 1024)), Ok(()));
        assert_eq!(validate_avatar_b64("aGVsbG8="), Ok(()));
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
