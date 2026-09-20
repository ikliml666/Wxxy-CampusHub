//! 深澜（Srun）WebVPN URL 包装纯函数库（无 IO、无 Tauri 依赖）。
//!
//! # 算法（逆向自 webvpn.cwxu.edu.cn 网关，活样本见 docs/cas-recon/REPORT.md）
//!
//! 包装式：`{gateway}/{scheme}/hex(IV)+hex(AES-128-CFB128(host))` + 原 path?query
//!
//! - key = IV = 16 字节 ASCII 常量（[`crypto`] 模块内单一来源）；
//! - AES-128-CFB128 为流式模式，密文长度恒等于明文字节长度（无 padding）；
//! - 输出前缀 hex(IV) 由 key **派生计算**得出，调用方禁止硬编码该 hex 字符串；
//! - scheme 段跟随目标 scheme（https → `/https/`、http → `/http/`），非默认端口写作
//!   `{scheme}-{port}`；仅支持 http/https；
//! - 输入 host 已是网关域时幂等原样返回（见 [`wrap::wrap_url`]）。
//!
//! CFB 加密与解密同为 keystream 异或（对称），故 [`crypto::decrypt_host`] 可直接
//! 还原网关 URL 中抠出的 `hex(IV)+hex(密文)` 整段，供测试与后续批次解析复用。
//!
//! M4 批 2 起增加 [`session`]（WebVPN 会话层：CAS TGT 静默登录 / 探活 / 业务句柄），
//! 本 crate 由纯函数库扩展为「纯函数 + 会话」双能力（协议核心仍无 Tauri 依赖）。

pub mod crypto;
pub mod route;
pub mod session;
pub mod wrap;

pub use crypto::{decrypt_host, encrypt_host};
pub use route::{route, NetZone, RouteDecision};
pub use session::WebVpnSession;
pub use wrap::{wrap_url, GATEWAY};

/// 统一错误类型。
#[derive(Debug, thiserror::Error)]
pub enum WebVpnError {
    #[error("invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("unsupported scheme `{0}`: WebVPN only wraps http/https targets")]
    UnsupportedScheme(String),
    #[error("URL has no host component")]
    MissingHost,
    #[error("invalid hex ciphertext: {0}")]
    InvalidHex(#[from] hex::FromHexError),
    #[error("IV segment missing or does not equal the fixed gateway IV")]
    InvalidIv,
    #[error("decrypted host is not valid UTF-8: {0}")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),
    /// 会话层网络错误（登录 302 链 / 探活请求失败）。
    #[error("WebVPN HTTP request failed: {0}")]
    Network(#[from] reqwest::Error),
    /// CAS 换票失败（TGT 失效 / 网络 / 解析，原样透传 campus-auth 错误链）。
    #[error("WebVPN SSO failed: {0}")]
    Sso(#[from] campus_auth::CampusAuthError),
}
