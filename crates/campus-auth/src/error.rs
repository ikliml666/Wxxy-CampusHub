//! CAS 协议层错误类型。

/// campus-auth 协议核心错误。
#[derive(Debug, thiserror::Error)]
pub enum CampusAuthError {
    #[error("RSA 加密失败: {0}")]
    Rsa(String),
    #[error("HTTP 请求失败: {0}")]
    Http(#[from] reqwest::Error),
    #[error("响应解析失败: {0}")]
    Parse(String),
}
