//! 多账号存储：%APPDATA%/campushub/accounts.json。
//!
//! `{ accounts: [{ username, passwordB64(DPAPI 密文), lastLogin, displayName? }] }`；
//! 密码永不落明文（DPAPI 加密后 base64）。

use super::crypto;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// 单条账号记录（camelCase 序列化，与计划冻结的 accounts.json 结构对齐）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecord {
    pub username: String,
    /// DPAPI 密文的 base64（dpapi_protect 输出）。
    pub password_b64: String,
    /// epoch 毫秒字符串（前端展示时格式化）。
    pub last_login: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AccountFile {
    #[serde(default)]
    accounts: Vec<AccountRecord>,
}

fn accounts_path(dir: &Path) -> std::path::PathBuf {
    dir.join("accounts.json")
}

fn now_ms() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default()
}

/// 保存/更新账号（同 username 覆盖密码与 last_login，upsert）。
pub fn save_account(
    dir: &Path,
    username: &str,
    password: &str,
    display_name: Option<&str>,
) -> Result<(), String> {
    let mut file = read_file(dir);
    let password_b64 = crypto::dpapi_protect(password)?;
    let record = AccountRecord {
        username: username.to_string(),
        password_b64,
        last_login: now_ms(),
        display_name: display_name.map(str::to_string),
    };
    match file.accounts.iter_mut().find(|r| r.username == username) {
        Some(existing) => *existing = record,
        None => file.accounts.push(record),
    }
    write_file(dir, &file)
}

/// 全部账号记录。
pub fn load_accounts(dir: &Path) -> Result<Vec<AccountRecord>, String> {
    Ok(read_file(dir).accounts)
}

/// 解密指定账号的密码（login_saved 一键重登用）。未找到返回 Err。
pub fn load_password(dir: &Path, username: &str) -> Result<String, String> {
    let file = read_file(dir);
    let record = file
        .accounts
        .iter()
        .find(|r| r.username == username)
        .ok_or_else(|| format!("账号 {username} 不存在"))?;
    crypto::dpapi_unprotect(&record.password_b64)
}

/// 删除账号。
pub fn remove_account(dir: &Path, username: &str) -> Result<(), String> {
    let mut file = read_file(dir);
    let before = file.accounts.len();
    file.accounts.retain(|r| r.username != username);
    if file.accounts.len() == before {
        return Err(format!("账号 {username} 不存在"));
    }
    write_file(dir, &file)
}

fn read_file(dir: &Path) -> AccountFile {
    fs::read_to_string(accounts_path(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_file(dir: &Path, file: &AccountFile) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let json = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
    fs::write(accounts_path(dir), json).map_err(|e| format!("写 accounts.json 失败: {e}"))
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    /// 唯一临时目录（纳秒时间戳，测试间不冲突；不清理，系统临时目录自净）。
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("campushub-test-{tag}-{n}"))
    }

    #[test]
    fn store_roundtrip_and_password_is_encrypted() {
        let dir = temp_dir("roundtrip");
        save_account(&dir, "2023001", "PlainPass!23", None).unwrap();

        let records = load_accounts(&dir).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].username, "2023001");
        // 密码字段为 base64 且非明文
        assert_ne!(records[0].password_b64, "PlainPass!23");
        assert!(!records[0].password_b64.contains("PlainPass"));
        assert!(crypto::dpapi_unprotect(&records[0].password_b64).is_ok());
        assert_eq!(load_password(&dir, "2023001").unwrap(), "PlainPass!23");
    }

    #[test]
    fn store_upsert_overwrites_same_username() {
        let dir = temp_dir("upsert");
        save_account(&dir, "2023001", "old", None).unwrap();
        save_account(&dir, "2023001", "new", Some("锡院学生")).unwrap();
        save_account(&dir, "2023002", "other", None).unwrap();

        let records = load_accounts(&dir).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(load_password(&dir, "2023001").unwrap(), "new");
        let rec = records.iter().find(|r| r.username == "2023001").unwrap();
        assert_eq!(rec.display_name.as_deref(), Some("锡院学生"));
    }

    #[test]
    fn store_remove_missing_errors() {
        let dir = temp_dir("remove");
        save_account(&dir, "2023001", "p", None).unwrap();
        assert!(remove_account(&dir, "2023001").is_ok());
        assert!(load_accounts(&dir).unwrap().is_empty());
        assert!(remove_account(&dir, "2023001").is_err());
        assert!(load_password(&dir, "2023001").is_err());
    }
}
