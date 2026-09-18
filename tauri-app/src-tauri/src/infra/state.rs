//! 应用状态与会话持久化（%APPDATA%/campushub/session.json）。
//!
//! AppState 的会话锁用 `tokio::sync::Mutex`（std Mutex 守卫非 Send，跨 await 编译不过，
//! 参考项目 commands/login.rs:87 同款教训）。锁纪律：锁内只 clone 出 client
//!（reqwest::Client 为 Arc 包装，clone 廉价），drop guard 后再 await。

use crate::account::crypto;
use campus_auth::cas::CasClient;
use campus_portal::PortalClient;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

/// 一次已建立的 CAS 会话（client 内含挂载 RecordingJar 的 reqwest Client）。
pub struct CasSession {
    pub client: CasClient,
    pub username: String,
    /// 门户业务客户端（复用 client 的会话 Cookie jar；含 JWT 内存缓存，
    /// 缓存生命周期跟随本会话，登录/登出/重启恢复时随会话重建或丢弃）。
    pub portal: PortalClient,
    /// CAS TGT（`CasLoginOk.tgt`，仅内存明文）。CAS 服务端不种任何登录 cookie，
    /// TGT 是教务会话静默续期（`jwglxt_sso` 换票）的唯一凭据，与 cookie 同级敏感：
    /// 只经 DPAPI 加密落盘，绝不入日志/错误消息。None = 旧会话文件无 TGT，
    /// 教务 901 时上层直接引导重新登录。
    pub tgt: Option<String>,
}

pub struct AppState {
    /// 会话（None = 未登录）。锁内只 clone，drop guard 后再 await。
    pub session: Mutex<Option<CasSession>>,
}

impl AppState {
    pub fn new(initial: Option<CasSession>) -> Self {
        Self {
            session: Mutex::new(initial),
        }
    }
}

/// 应用数据目录：%APPDATA%/campushub。
pub fn data_dir() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|d| d.join("campushub"))
        .ok_or_else(|| "无法定位应用数据目录（%APPDATA%）".to_string())
}

fn session_path(dir: &Path) -> PathBuf {
    dir.join("session.json")
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CookieRecord {
    name: String,
    /// DPAPI 密文 base64。
    value_b64: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionRecord {
    username: String,
    cookies: Vec<CookieRecord>,
    /// CAS TGT（DPAPI 密文 base64）。与 cookie 同级敏感，绝不落明文。
    /// `default` 兼容旧版 session.json（无该字段）；为 None 时序列化省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tgt_b64: Option<String>,
}

/// session.json 解密结果（[`restore_session`] 与单测消费）。
pub struct StoredSession {
    pub username: String,
    pub cookies: Vec<(String, String)>,
    /// TGT 明文（仅内存）。单条解密失败静默为 None——TGT 丢失只影响教务
    /// 会话静默续期（下次课表请求 901 后引导重新登录），不影响门户会话恢复。
    pub tgt: Option<String>,
}

/// 登录成功后持久化：jar 快照 + TGT 逐项 DPAPI 加密落盘（session.json）。
pub fn persist_session(
    dir: &Path,
    username: &str,
    cookies: &[(String, String)],
    tgt: Option<&str>,
) -> Result<(), String> {
    let record = SessionRecord {
        username: username.to_string(),
        cookies: cookies
            .iter()
            .map(|(name, value)| {
                Ok(CookieRecord {
                    name: name.clone(),
                    value_b64: crypto::dpapi_protect(value)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        tgt_b64: tgt.map(crypto::dpapi_protect).transpose()?,
    };
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let json = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?;
    fs::write(session_path(dir), json).map_err(|e| format!("写 session.json 失败: {e}"))
}

/// 读取并解密 session.json。文件缺失/损坏 → None；单个 cookie / TGT 解密失败跳过。
pub fn load_session(dir: &Path) -> Option<StoredSession> {
    let raw = fs::read_to_string(session_path(dir)).ok()?;
    let record: SessionRecord = serde_json::from_str(&raw).ok()?;
    let cookies = record
        .cookies
        .iter()
        .filter_map(|c| {
            crypto::dpapi_unprotect(&c.value_b64)
                .ok()
                .map(|v| (c.name.clone(), v))
        })
        .collect();
    let tgt = record
        .tgt_b64
        .and_then(|b64| crypto::dpapi_unprotect(&b64).ok());
    Some(StoredSession {
        username: record.username,
        cookies,
        tgt,
    })
}

/// 清除会话文件（登出 / 会话过期时）。
pub fn clear_session(dir: &Path) {
    let _ = fs::remove_file(session_path(dir));
}

/// 启动回填（P0-3「重启保持」）：session.json → 新 CasClient → jar.restore，
/// TGT 一并回填（教务会话静默续期凭据）。
/// 全部 cookie 解密失败视为无可恢复会话。
pub fn restore_session() -> Option<CasSession> {
    let dir = data_dir().ok()?;
    let stored = load_session(&dir)?;
    if stored.cookies.is_empty() {
        return None;
    }
    let client = CasClient::new().ok()?;
    client.jar().restore(&stored.cookies);
    let portal = PortalClient::new(client.clone());
    Some(CasSession {
        client,
        username: stored.username,
        portal,
        tgt: stored.tgt,
    })
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use std::{fs, time::{SystemTime, UNIX_EPOCH}};

    fn temp_dir(tag: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("campushub-sess-{tag}-{n}"))
    }

    #[test]
    fn session_roundtrip_and_missing_file() {
        let dir = temp_dir("rt");
        // 缺失 → None
        assert!(load_session(&dir).is_none());

        let cookies = vec![
            ("customsid".to_string(), "S3cretSessionValue".to_string()),
            ("rememberMe".to_string(), "abc=/+==".to_string()),
        ];
        let tgt = "TGT-12345-SecretGrant";
        persist_session(&dir, "2023001", &cookies, Some(tgt)).unwrap();

        // 落盘内容不含 cookie 与 TGT 明文（TGT 与 cookie 同级敏感，均 DPAPI 密文）
        let raw = fs::read_to_string(session_path(&dir)).unwrap();
        assert!(!raw.contains("S3cretSessionValue"));
        assert!(!raw.contains(tgt));
        assert!(raw.contains("tgtB64"), "应存在 TGT 密文字段");

        let stored = load_session(&dir).unwrap();
        assert_eq!(stored.username, "2023001");
        assert_eq!(stored.cookies, cookies);
        assert_eq!(stored.tgt.as_deref(), Some(tgt));

        clear_session(&dir);
        assert!(load_session(&dir).is_none());
    }

    /// TGT 缺省与旧格式兼容：无 TGT 落盘 → tgt=None；
    /// 旧版 session.json（无 tgtB64 字段）仍可读取。
    #[test]
    fn session_without_tgt_and_legacy_file_compat() {
        let dir = temp_dir("notgt");
        let cookies = vec![("customsid".to_string(), "V".to_string())];
        persist_session(&dir, "2023002", &cookies, None).unwrap();
        let raw = fs::read_to_string(session_path(&dir)).unwrap();
        assert!(!raw.contains("tgtB64"), "无 TGT 时不应写出空密文字段");
        let stored = load_session(&dir).unwrap();
        assert_eq!(stored.tgt, None);

        // 旧版文件（仅 username + cookies，无 tgtB64）→ 兼容读取，tgt=None
        let legacy_dir = temp_dir("legacy");
        fs::create_dir_all(&legacy_dir).unwrap();
        let legacy_json = format!(
            r#"{{"username":"2023003","cookies":[{{"name":"customsid","valueB64":"{}"}}]}}"#,
            crypto::dpapi_protect("LegacyValue").unwrap()
        );
        fs::write(session_path(&legacy_dir), legacy_json).unwrap();
        let legacy = load_session(&legacy_dir).unwrap();
        assert_eq!(legacy.username, "2023003");
        assert_eq!(legacy.cookies, vec![("customsid".to_string(), "LegacyValue".to_string())]);
        assert_eq!(legacy.tgt, None);

        fs::remove_dir_all(&legacy_dir).ok();
    }
}
