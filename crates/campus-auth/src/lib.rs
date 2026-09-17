//! 锡院助手 CAS 统一身份认证协议核心（无 Tauri 依赖，安卓可复用）。

pub mod cas;
pub mod captcha;
pub mod error;
pub mod jar;
pub mod rsa;

pub use error::CampusAuthError;
