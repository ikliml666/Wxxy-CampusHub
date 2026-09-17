//! 应用状态与会话持久化（%APPDATA%/campushub/session.json）。
//!
//! AppState 的会话锁用 `tokio::sync::Mutex`（std Mutex 守卫非 Send，跨 await 编译不过，
//! 参考项目 commands/login.rs:87 同款教训）。锁纪律：锁内只 clone 出 client
//!（reqwest::Client 为 Arc 包装，clone 廉价），drop guard 后再 await。

use crate::account::crypto;
use campus_auth::cas::CasClient;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

/// 一次已建立的 CAS 会话（client 内含挂载 RecordingJar 的 reqwest Client）。
pub struct CasSession {
    pub client: CasClient,
    pub username: String,
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
}

/// 登录成功后持久化：jar 快照逐条 DPAPI 加密落盘（session.json）。
pub fn persist_session(
    dir: &Path,
    username: &str,
    cookies: &[(String, String)],
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
    };
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let json = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?;
    fs::write(session_path(dir), json).map_err(|e| format!("写 session.json 失败: {e}"))
}

/// 读取并解密 session.json。文件缺失/损坏 → None；单个 cookie 解密失败跳过。
pub fn load_session(dir: &Path) -> Option<(String, Vec<(String, String)>)> {
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
    Some((record.username, cookies))
}

/// 清除会话文件（登出 / 会话过期时）。
pub fn clear_session(dir: &Path) {
    let _ = fs::remove_file(session_path(dir));
}

/// 启动回填（P0-3「重启保持」）：session.json → 新 CasClient → jar.restore。
/// 全部 cookie 解密失败视为无可恢复会话。
pub fn restore_session() -> Option<CasSession> {
    let dir = data_dir().ok()?;
    let (username, cookies) = load_session(&dir)?;
    if cookies.is_empty() {
        return None;
    }
    let client = CasClient::new().ok()?;
    client.jar().restore(&cookies);
    Some(CasSession { client, username })
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
        persist_session(&dir, "2023001", &cookies).unwrap();

        // 落盘内容不含 cookie 明文
        let raw = fs::read_to_string(session_path(&dir)).unwrap();
        assert!(!raw.contains("S3cretSessionValue"));

        let (username, restored) = load_session(&dir).unwrap();
        assert_eq!(username, "2023001");
        assert_eq!(restored, cookies);

        clear_session(&dir);
        assert!(load_session(&dir).is_none());
    }
}
