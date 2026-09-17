//! CAS 登录客户端：kaptcha → login → sso_follow → portal_probe。
//!
//! 协议事实全部来自 `docs/cas-recon/REPORT.md`（实测打通）：
//! - 无 Cookie 依赖即可完成 CAS 登录；成功响应顶层即 `{tgt, ticket}`
//! - `POST /v1/tickets`：x-www-form-urlencoded，头 `token` = RSA("lyasp"+毫秒)
//! - 错误码全集 16 个，映射见 [`map_error_code`]

use crate::error::CampusAuthError;
use crate::jar::RecordingJar;
use crate::rsa::cas_token_header;
use serde::Deserialize;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub const CAS_BASE: &str = "https://wxcas.cwxu.edu.cn/lyuapServer";
pub const PORTAL_SERVICE: &str = "https://my.cwxu.edu.cn/shiro-cas";
pub const WEBVPN_SERVICE: &str = "https://webvpn.cwxu.edu.cn/login?cas_login=true";
pub const JWGL_SERVICE: &str = "https://jwgl.cwxu.edu.cn/sso/lyiotlogin";

/// 门户探测端点（首页）。
const PORTAL_PROBE: &str = "https://my.cwxu.edu.cn/";

/// 真实 Chrome UA（cas.js 同款）。
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// CAS 客户端（内部 `.cookie_provider(Arc<RecordingJar>)` 挂载记录型 Jar）。
/// Clone 供 AppState 锁内廉价 clone（reqwest::Client 为 Arc 包装），drop guard 后再 await。
#[derive(Clone)]
pub struct CasClient {
    /// 手动跟随重定向（sso_follow 用；Policy::none）
    http_manual: reqwest::Client,
    http: reqwest::Client,
    jar: Arc<RecordingJar>,
}

/// 验证码信息（`GET {CAS_BASE}/kaptcha` 响应）。
#[derive(Debug, Clone)]
pub struct CaptchaInfo {
    pub uid: String,
    /// 已剥掉 `data:image/png;base64,` 前缀的裸 base64（probe.js:52 同款处理）。
    pub png_base64: String,
    pub kaptcha_type: String,
}

/// CAS 登录成功结果。
#[derive(Debug, Clone)]
pub struct CasLoginOk {
    pub tgt: String,
    pub ticket: String,
}

/// CAS 登录失败（错误码全集与映射以 REPORT.md 为准）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CasLoginError {
    /// NOUSER：账号或密码错误
    WrongUserOrPwd,
    /// CODEFALSE：验证码错误
    WrongCaptcha,
    /// USERLOCK：账号锁定
    UserLocked,
    /// TWOVERIFY：需要二次验证（携带服务端 data 原文，含可能的错误次数提示）
    NeedTwoVerify(String),
    /// 其余错误码（USERDISABLED / PASSERROR / ISMODIFYPASS / … 原样上抛）
    Unknown(String),
    /// 网络层错误
    Network(String),
}

/// 门户会话状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Alive,
    Expired,
}

#[derive(Deserialize)]
struct KaptchaResp {
    #[serde(rename = "kaptchaType")]
    kaptcha_type: String,
    uid: String,
    content: String,
}

impl CasClient {
    pub fn new() -> Result<Self, CampusAuthError> {
        let jar = Arc::new(RecordingJar::new());
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .cookie_provider(jar.clone())
            .build()?;
        // 手动跟随重定向用：sso_follow 的 302 链里存在响应 body 中断的跳（hyper
        // IncompleteMessage），默认策略会因此整链失败；手动跟随只要拿到 Location 与
        // Set-Cookie 即可继续，body 异常可忽略。与 http 共享同一 cookie jar。
        let http_manual = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .cookie_provider(jar.clone())
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            http,
            http_manual,
            jar,
        })
    }

    /// 会话 Jar 句柄（check_session / 持久化用）。
    pub fn jar(&self) -> Arc<RecordingJar> {
        self.jar.clone()
    }

    /// GET {CAS_BASE}/kaptcha → {kaptchaType, uid, content}。
    pub async fn kaptcha(&self) -> Result<CaptchaInfo, CampusAuthError> {
        let body = self
            .http
            .get(format!("{CAS_BASE}/kaptcha"))
            .send()
            .await?
            .text()
            .await?;
        parse_kaptcha(&body)
    }

    /// POST {CAS_BASE}/v1/tickets：CAS 账密登录。
    ///
    /// `password_rsa_hex` 为 [`crate::rsa::rsa_encrypt_hex`] 的密文；头 `token` =
    /// [`cas_token_header`]（毫秒时间戳在方法内取当前时刻）。
    pub async fn login(
        &self,
        username: &str,
        password_rsa_hex: &str,
        captcha_uid: &str,
        captcha_code: &str,
    ) -> Result<CasLoginOk, CasLoginError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| CasLoginError::Network(e.to_string()))?
            .as_millis() as u64;
        let token =
            cas_token_header(now_ms).map_err(|e| CasLoginError::Network(e.to_string()))?;
        let body = build_login_body(
            username,
            password_rsa_hex,
            PORTAL_SERVICE,
            captcha_uid,
            captcha_code,
        );
        let text = self
            .http
            .post(format!("{CAS_BASE}/v1/tickets"))
            .header("token", token)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(
                reqwest::header::REFERER,
                format!("{CAS_BASE}/login?service={}", form_encode(PORTAL_SERVICE)),
            )
            .header(reqwest::header::ORIGIN, "https://wxcas.cwxu.edu.cn")
            .body(body)
            .send()
            .await
            .map_err(|e| CasLoginError::Network(e.to_string()))?
            .text()
            .await
            .map_err(|e| CasLoginError::Network(e.to_string()))?;
        parse_login_response(&text)
    }

    /// GET service?ticket= 手动跟随 302 链到落点。
    /// 不用默认重定向策略：该 302 链中存在响应体中断的跳（hyper `IncompleteMessage`），
    /// 默认策略会因读 body 失败使整链报错；手动跟随只要 Location 与 Set-Cookie 即可继续。
    pub async fn sso_follow(
        &self,
        service_url: &str,
        ticket: &str,
    ) -> Result<reqwest::Url, CampusAuthError> {
        let mut url = reqwest::Url::parse(service_url)
            .map_err(|e| CampusAuthError::Parse(format!("service URL 非法: {e}")))?;
        url.query_pairs_mut().append_pair("ticket", ticket);
        let mut final_url = url.clone();
        // 302 链中门户落点是 http:// 明文，而服务端对明文请求直接中断连接——
        // 每次请求前把 cwxu 域名的 http 升级为 https（只升不降，避免来回跳）。
        let upgrade = |u: &mut reqwest::Url| {
            if u.scheme() == "http" && u.host_str().is_some_and(|h| h.ends_with(".cwxu.edu.cn")) {
                let _ = u.set_scheme("https");
            }
        };
        for _ in 0..10 {
            upgrade(&mut final_url);
            let resp = self.http_manual.get(final_url.clone()).send().await?;
            let status = resp.status();
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            // 尽力读 body：某些跳 body 会中断（不影响 Set-Cookie 已入 jar 与下一跳）
            let _ = resp.bytes().await;
            if status.is_redirection() {
                if let Some(loc) = loc {
                    final_url = final_url
                        .join(&loc)
                        .map_err(|e| CampusAuthError::Parse(format!("重定向 URL 非法: {e}")))?;
                    continue;
                }
            }
            break;
        }
        // REPORT.md 三节实测：门户 302 落点是 http:// 明文，客户端应替换为 https
        if final_url.scheme() == "http"
            && final_url.host_str().is_some_and(|h| h.ends_with(".cwxu.edu.cn"))
        {
            final_url
                .set_scheme("https")
                .map_err(|_| CampusAuthError::Parse("URL scheme 替换失败".to_string()))?;
        }
        Ok(final_url)
    }

    /// 门户会话探测：jar 有 customsid 且访问门户首页未被弹回 CAS 域 = Alive。
    /// 判据取舍（均实测）：
    /// - 正文匹配不可用：门户首页 HTML 恒含 `lyuapServer/login` 常量（2 处），会把已登录判成 Expired；
    /// - `/shiro-cas` 端点不可用：无 ticket 访问会触发服务端断连（hyper IncompleteMessage）；
    /// - 未登录场景由 `customsid` 缺失挡住（首页未登录也返回 200 外壳，前端路由才跳登录）。
    ///
    /// ponytail: 过期精确检测待 M2 接入门户 API 后用鉴权接口（401/302 即过期）替代。
    pub async fn portal_probe(&self) -> SessionState {
        if !self.jar.has("customsid") {
            return SessionState::Expired;
        }
        let Ok(resp) = self.http.get(PORTAL_PROBE).send().await else {
            return SessionState::Expired;
        };
        let bounced = resp
            .url()
            .host_str()
            .is_some_and(|h| h.contains("wxcas.cwxu.edu.cn"));
        if bounced {
            SessionState::Expired
        } else {
            SessionState::Alive
        }
    }
}

/// 构造 CAS 登录请求体（application/x-www-form-urlencoded），纯函数供离线单测。
///
/// 字段与 cas.js 参照实现一致（REPORT.md 一节协议体，共 7 项）：
/// username / password(密文) / service / loginType(空) / id=验证码uid / code=验证码答案 /
/// otpcode(空)。
pub fn build_login_body(
    username: &str,
    password_rsa_hex: &str,
    service: &str,
    captcha_uid: &str,
    captcha_code: &str,
) -> String {
    let pairs = [
        ("username", username),
        ("password", password_rsa_hex),
        ("service", service),
        ("loginType", ""),
        ("id", captcha_uid),
        ("code", captcha_code),
        ("otpcode", ""),
    ];
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={}", form_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// application/x-www-form-urlencoded 值编码：字母数字与 `* - . _` 保留、空格→`+`、其余 %XX。
fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(*b as char);
            }
            b' ' => out.push('+'),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// 解析 kaptcha 响应（纯函数供离线单测）。
pub fn parse_kaptcha(body: &str) -> Result<CaptchaInfo, CampusAuthError> {
    let resp: KaptchaResp = serde_json::from_str(body)
        .map_err(|e| CampusAuthError::Parse(format!("kaptcha 响应解析失败: {e}")))?;
    // 剥掉 data URI 前缀（probe.js:52 同款处理）；无前缀则视为已是裸 base64
    let png_base64 = resp
        .content
        .strip_prefix("data:image/png;base64,")
        .unwrap_or(&resp.content)
        .to_string();
    Ok(CaptchaInfo {
        uid: resp.uid,
        png_base64,
        kaptcha_type: resp.kaptcha_type,
    })
}

/// 解析 /v1/tickets 响应（纯函数供离线单测）。
///
/// 成功：顶层 `{tgt, ticket}`（REPORT.md 三节实测形态；兼容 data 包裹与 data 为纯字符串）。
/// 失败：`{"meta":...,"data":{"code":"..."}}` → [`map_error_code`]。
pub fn parse_login_response(body: &str) -> Result<CasLoginOk, CasLoginError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| CasLoginError::Unknown(format!("响应非 JSON: {e}")))?;
    if let Some(code) = v
        .get("data")
        .and_then(|d| d.get("code"))
        .and_then(|c| c.as_str())
    {
        let data_raw = v
            .get("data")
            .map(|d| d.to_string())
            .unwrap_or_else(|| code.to_string());
        return Err(map_error_code(code, &data_raw));
    }
    let str_of = |obj: &serde_json::Value, key: &str| {
        obj.get(key).and_then(|x| x.as_str()).map(str::to_string)
    };
    let tgt = str_of(&v, "tgt").or_else(|| v.get("data").and_then(|d| str_of(d, "tgt")));
    // cas.js 兼容：data 为纯字符串时也把它当 ticket
    let ticket = str_of(&v, "ticket")
        .or_else(|| v.get("data").and_then(|d| str_of(d, "ticket")))
        .or_else(|| v.get("data").and_then(|d| d.as_str()).map(str::to_string));
    match (tgt, ticket) {
        (Some(tgt), Some(ticket)) if !tgt.is_empty() && !ticket.is_empty() => {
            Ok(CasLoginOk { tgt, ticket })
        }
        (tgt, ticket) => Err(CasLoginError::Unknown(format!(
            "响应缺少 tgt/ticket（tgt={tgt:?} ticket={ticket:?}）"
        ))),
    }
}

/// 错误码映射：REPORT.md 全集 16 码 —— NOUSER / CODEFALSE / USERLOCK / TWOVERIFY
/// 专属映射，其余 12 码归 Unknown(原样码)。
fn map_error_code(code: &str, data_raw: &str) -> CasLoginError {
    match code {
        "NOUSER" => CasLoginError::WrongUserOrPwd,
        "CODEFALSE" => CasLoginError::WrongCaptcha,
        "USERLOCK" => CasLoginError::UserLocked,
        "TWOVERIFY" => CasLoginError::NeedTwoVerify(data_raw.to_string()),
        other => CasLoginError::Unknown(other.to_string()),
    }
}
