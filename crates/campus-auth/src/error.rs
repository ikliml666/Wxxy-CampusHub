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
    /// 教务（jwglxt）会话无效：业务接口带 `X-Requested-With` 请求时服务端返回
    /// 自定义状态码 901（2026-09-18 实测）。上层按「未登录 / 需重新 SSO」降级
    /// （有 TGT 时可用 [`crate::cas::CasClient::jwglxt_sso`] 静默重进）。
    #[error("教务会话已失效，请重新登录")]
    JwglNotLogin,
}
