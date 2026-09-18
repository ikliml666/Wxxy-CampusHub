//! CAS 登录客户端：kaptcha → login → sso_follow → portal_probe。
//!
//! 协议事实全部来自 `docs/cas-recon/REPORT.md`（实测打通）：
//! - 无 Cookie 依赖即可完成 CAS 登录；成功响应顶层即 `{tgt, ticket}`
//! - `POST /v1/tickets`：x-www-form-urlencoded，头 `token` = RSA("lyasp"+毫秒)
//! - 错误码全集 16 个，映射见 [`map_error_code`]

use crate::error::CampusAuthError;
use crate::jar::RecordingJar;
use crate::rsa::cas_token_header;
use md5::{Digest, Md5};
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

/// 门户网关 csrf 密钥（2026-09-18 实机取证：门户 app bundle 内常量 `GATEWAY_KEY:"lianyi2019"`）。
const GATEWAY_KEY: &str = "lianyi2019";

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

/// 门户用户资料（`POST /tryLoginUserInfo`，2026-09-18 实测：GET 返回 405，
/// 带门户会话 Cookie + JSON body `{}`，响应 `data.userName` 为真实姓名）。
#[derive(Debug, Clone)]
pub struct PortalProfile {
    /// 真实姓名（`data.userName`）。
    pub name: String,
    /// 院系/专业（`data.departmentName`），缺失/空为 None。
    pub department: Option<String>,
    /// 学号（`data.userId`），缺失为 None（portraitChange 鉴权头 loginUserId/loginUserName 用）。
    pub user_id: Option<String>,
    /// 组织 id（`data.orgId`），缺失为 None（学生实测为 `"-1"`）。
    pub org_id: Option<String>,
    /// 网关 JWT（`data.tokenId`，**无 Bearer 前缀**），缺失为 None（portraitChange 鉴权头 Authorization 用）。
    pub token_id: Option<String>,
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
        let token = cas_token_header(now_ms).map_err(|e| CasLoginError::Network(e.to_string()))?;
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
            && final_url
                .host_str()
                .is_some_and(|h| h.ends_with(".cwxu.edu.cn"))
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

    /// GET 门户 `/api/upp/userControl/getLoginInfo` → `data.headPortrait` 裸 base64。
    ///
    /// 复用 [`Self::http`]（与登录/探测同 jar、同 UA），门户 base 由 [`PORTAL_PROBE`]
    /// 派生（不另设常量）；解析见 [`extract_head_portrait`]（纯函数，离线单测覆盖）。
    /// 会话过期时服务端返回非预期结构 → Parse 错误，上层据此提示重新登录。
    pub async fn portal_login_info(&self) -> Result<String, CampusAuthError> {
        let base = PORTAL_PROBE.trim_end_matches('/');
        let body = self
            .http
            .get(format!("{base}/api/upp/userControl/getLoginInfo"))
            .send()
            .await?
            .text()
            .await?;
        extract_head_portrait(&body)
    }

    /// POST 门户 `/tryLoginUserInfo` → 真实姓名与院系（[`PortalProfile`]）。
    ///
    /// 复用 [`Self::http`]（与登录/探测同 jar、同 UA），门户 base 由 [`PORTAL_PROBE`]
    /// 派生（不另设常量）；解析见 [`extract_user_profile`]（纯函数，离线单测覆盖）。
    /// 2026-09-18 实测：GET 返回 405，须 POST JSON `{}`；会话失效时响应缺 userName
    /// → Parse 错误，上层尽力而为降级，不影响登录主流程。
    pub async fn portal_user_profile(&self) -> Result<PortalProfile, CampusAuthError> {
        let base = PORTAL_PROBE.trim_end_matches('/');
        let body = self
            .http
            .post(format!("{base}/tryLoginUserInfo"))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await?
            .text()
            .await?;
        extract_user_profile(&body)
    }

    /// POST 门户 `/api/authc/users/portraitChange` 上传头像（2026-09-18 实机取证）。
    ///
    /// - `data_url`：完整 data URL（`data:image/jpeg;base64,…`）。服务端**原样存储**、
    ///   不做压缩，体积守卫须由调用方完成（本方法只校验前缀形态）。
    /// - 请求头（除会话 Cookie 外全部必需）：`Authorization` = tryLoginUserInfo 的
    ///   `data.tokenId`（JWT，无 Bearer 前缀）、`loginUserId`/`loginUserName` = `data.userId`
    ///   （学号，门户前端同款）、`loginUserOrgId` = `data.orgId`（学生为 `"-1"`，缺失同值兜底）、
    ///   `csrfTimestamp` = 当前毫秒、`csrfToken` = [`csrf_token`]，另有 Content-Type /
    ///   X-Requested-With / Accept 与门户前端一致。
    /// - 每次调用现取一次 [`Self::portal_user_profile`]（JWT 随取随用最新）并现算 csrf；
    ///   复用 [`Self::http`] 与 jar（不新建 client）。
    pub async fn portal_change_portrait(&self, data_url: &str) -> Result<(), CampusAuthError> {
        if !data_url.starts_with("data:image/") {
            return Err(CampusAuthError::Parse(
                "头像数据 URL 非法（须以 data:image/ 开头）".to_string(),
            ));
        }
        let base = PORTAL_PROBE.trim_end_matches('/');
        let profile = self.portal_user_profile().await?;
        // ponytail: meta.success=false 属业务失败而非解析失败，CampusAuthError 暂无
        // Unknown 变体，以 Parse 透传服务端原文；error.rs 增加 Unknown 后在此分流。
        let token_id = profile.token_id.clone().ok_or_else(|| {
            CampusAuthError::Parse(
                "tryLoginUserInfo 未返回 tokenId，无法上传头像（请重新登录）".to_string(),
            )
        })?;
        let user_id = profile.user_id.clone().ok_or_else(|| {
            CampusAuthError::Parse(
                "tryLoginUserInfo 未返回 userId，无法上传头像（请重新登录）".to_string(),
            )
        })?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| CampusAuthError::Parse(format!("系统时间异常: {e}")))?
            .as_millis();
        let resp = self
            .http
            .post(format!("{base}/api/authc/users/portraitChange"))
            .header("Authorization", token_id)
            .header("loginUserId", &user_id)
            .header("loginUserName", &user_id)
            .header(
                "loginUserOrgId",
                profile.org_id.as_deref().unwrap_or("-1"),
            )
            .header("csrfTimestamp", now_ms.to_string())
            .header("csrfToken", csrf_token(now_ms))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
            .body(serde_json::json!({ "displayPhoto": data_url }).to_string())
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(CampusAuthError::Parse(format!("portraitChange HTTP {status}")));
        }
        parse_portrait_change_response(&text)
    }
}

/// 从 getLoginInfo 响应提取 `data.headPortrait` 裸 base64（纯函数供离线单测）。
///
/// 实测形态为完整 data URL（`data:image/png;base64,iVBOR…`），同时容忍裸 base64：
/// 以 `data:` 开头则剥掉首个 `,` 之前的前缀（不限于 png，jpeg/webp 同理），否则原样返回。
/// data 缺失 / headPortrait 为 null / 非法 JSON / 剥离后为空 → [`CampusAuthError::Parse`]。
/// 若将来 headPortrait 改为 CDN URL，在 `data:` 分支处加「http 开头 → 原样透传」分支即可。
pub fn extract_head_portrait(body: &str) -> Result<String, CampusAuthError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| CampusAuthError::Parse(format!("getLoginInfo 响应解析失败: {e}")))?;
    let raw = v
        .get("data")
        .and_then(|d| d.get("headPortrait"))
        .and_then(|h| h.as_str())
        .ok_or_else(|| CampusAuthError::Parse("getLoginInfo 缺少 headPortrait".to_string()))?;
    let bare = match raw.strip_prefix("data:") {
        Some(rest) => rest.split_once(',').map(|(_, b)| b).ok_or_else(|| {
            CampusAuthError::Parse("headPortrait data URL 缺少 , 分隔".to_string())
        })?,
        None => raw,
    };
    if bare.is_empty() {
        return Err(CampusAuthError::Parse("headPortrait 为空".to_string()));
    }
    Ok(bare.to_string())
}

/// 从 tryLoginUserInfo 响应提取用户资料（纯函数供离线单测）。
///
/// 实测形态：`{"meta":...,"data":{"userId":"...","userName":"张三","departmentName":"…",
/// "email":"…","orgId":"-1","tokenId":"<JWT>"}}`。
/// `userName` 缺失 / null / 空串（trim 后）→ [`CampusAuthError::Parse`]（上层据此回退学号）；
/// `departmentName` / `userId` / `orgId` / `tokenId` 缺失 / null / 空串 → `None`。
/// 姓名与院系不进日志（敏感纪律）。
pub fn extract_user_profile(body: &str) -> Result<PortalProfile, CampusAuthError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| CampusAuthError::Parse(format!("tryLoginUserInfo 响应解析失败: {e}")))?;
    let data = v
        .get("data")
        .ok_or_else(|| CampusAuthError::Parse("tryLoginUserInfo 缺少 data".to_string()))?;
    let name = data
        .get("userName")
        .and_then(|n| n.as_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| CampusAuthError::Parse("tryLoginUserInfo 缺少 userName".to_string()))?
        .to_string();
    // 与 userName 同构的可选字符串字段：缺失 / null / 空串一律 None
    let opt_str = |key: &str| {
        data.get(key)
            .and_then(|n| n.as_str())
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
    };
    Ok(PortalProfile {
        name,
        department: opt_str("departmentName"),
        user_id: opt_str("userId"),
        org_id: opt_str("orgId"),
        token_id: opt_str("tokenId"),
    })
}

/// 门户网关 csrfToken（纯函数供离线单测，含金标向量）。
///
/// `md5("timestamp=<ts_ms>,key=<GATEWAY_KEY>")` 小写 hex——2026-09-18 实机取证：
/// 密钥来自门户 app bundle 常量 `GATEWAY_KEY:"lianyi2019"`，门户前端每次请求现算。
pub fn csrf_token(ts_ms: u128) -> String {
    let digest = Md5::digest(format!("timestamp={ts_ms},key={GATEWAY_KEY}").as_bytes());
    to_hex(digest.as_slice())
}

/// 字节序列 → 小写 hex（md5 摘要 16 字节 → 32 字符；3 行够用，不为此引 hex crate）。
fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// 解析 portraitChange 响应（纯函数供离线单测）。
///
/// 实测成功形态：`{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":true}`。
/// `meta.success` 非 true → [`CampusAuthError::Parse`]（message 服务端原文，缺失给固定文案）；
/// 非法 JSON → Parse。HTTP 非 2xx 由调用方先行拦截，不经此函数。
pub fn parse_portrait_change_response(body: &str) -> Result<(), CampusAuthError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| CampusAuthError::Parse(format!("portraitChange 响应解析失败: {e}")))?;
    if v.get("meta")
        .and_then(|m| m.get("success"))
        .and_then(|s| s.as_bool())
        == Some(true)
    {
        return Ok(());
    }
    let msg = v
        .get("meta")
        .and_then(|m| m.get("message"))
        .and_then(|m| m.as_str())
        .filter(|m| !m.trim().is_empty())
        .unwrap_or("上传头像失败（服务端未返回原因）");
    Err(CampusAuthError::Parse(msg.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 实测响应形态：headPortrait 为完整 data URL（2026-09-18 门户实机取证）。
    #[test]
    fn head_portrait_with_data_url_prefix() {
        let body = r#"{"meta":{"code":0},"data":{"firLogin":"0","guideUsed":"0","headPortrait":"data:image/png;base64,iVBORw0KGgo=","strategy":"0","userId":"2023001"}}"#;
        assert_eq!(extract_head_portrait(body).unwrap(), "iVBORw0KGgo=");
    }

    /// 容忍裸 base64（无 data: 前缀）。
    #[test]
    fn head_portrait_bare_base64() {
        let body = r#"{"data":{"headPortrait":"iVBORw0KGgo="}}"#;
        assert_eq!(extract_head_portrait(body).unwrap(), "iVBORw0KGgo=");
    }

    /// 非 png 的 data URL 前缀同样按 `data:` → `,` 通用剥离。
    #[test]
    fn head_portrait_other_mime_prefix() {
        let body = r#"{"data":{"headPortrait":"data:image/jpeg;base64,/9j/4AAQ"}}"#;
        assert_eq!(extract_head_portrait(body).unwrap(), "/9j/4AAQ");
    }

    /// data 缺失 / headPortrait 为 null → Parse 错误。
    #[test]
    fn head_portrait_missing_or_null() {
        assert!(extract_head_portrait(r#"{"meta":{}}"#).is_err());
        assert!(extract_head_portrait(r#"{"data":{}}"#).is_err());
        assert!(extract_head_portrait(r#"{"data":{"headPortrait":null}}"#).is_err());
    }

    /// 非法 JSON / 空 base64 / 残缺 data URL → Parse 错误。
    #[test]
    fn head_portrait_invalid_inputs() {
        assert!(extract_head_portrait("not json").is_err());
        assert!(extract_head_portrait(r#"{"data":{"headPortrait":""}}"#).is_err());
        assert!(
            extract_head_portrait(r#"{"data":{"headPortrait":"data:image/png;base64"}}"#).is_err()
        );
    }

    // ---------- extract_user_profile ----------

    /// 实测响应形态：userName（真实姓名）+ departmentName（院系/专业）+ userId/orgId/tokenId
    /// 齐全（上传鉴权头来源；占位值非真实数据）。
    #[test]
    fn user_profile_full() {
        let body = r#"{"meta":{"code":0},"data":{"userId":"2023001","userName":"张三","departmentName":"示例学院示例专业","orgId":"-1","tokenId":"jwt-placeholder-token","email":"x@cwxu.edu.cn","userType":"student"}}"#;
        let p = extract_user_profile(body).unwrap();
        assert_eq!(p.name, "张三");
        assert_eq!(p.department.as_deref(), Some("示例学院示例专业"));
        assert_eq!(p.user_id.as_deref(), Some("2023001"));
        assert_eq!(p.org_id.as_deref(), Some("-1"));
        assert_eq!(p.token_id.as_deref(), Some("jwt-placeholder-token"));
    }

    /// departmentName 缺失 / null / 空串 → None，不影响 name；orgId/tokenId 缺失 /
    /// null / 空串同样 → None（上传时由调用方按缺 tokenId 拦截）。
    #[test]
    fn user_profile_department_optional() {
        let p = extract_user_profile(r#"{"data":{"userName":"张三"}}"#).unwrap();
        assert_eq!(p.name, "张三");
        assert!(p.department.is_none());
        assert!(p.user_id.is_none());
        assert!(p.org_id.is_none());
        assert!(p.token_id.is_none());
        let p = extract_user_profile(r#"{"data":{"userName":"张三","departmentName":null,"orgId":null,"tokenId":null}}"#).unwrap();
        assert!(p.department.is_none());
        assert!(p.org_id.is_none());
        assert!(p.token_id.is_none());
        let p = extract_user_profile(r#"{"data":{"userName":"张三","departmentName":"","orgId":"","tokenId":""}}"#).unwrap();
        assert!(p.department.is_none());
        assert!(p.org_id.is_none());
        assert!(p.token_id.is_none());
    }

    /// userName 缺失 / null / 空串 → Parse 错误（上层回退学号）。
    #[test]
    fn user_profile_name_missing_or_null() {
        assert!(extract_user_profile(r#"{"meta":{}}"#).is_err());
        assert!(extract_user_profile(r#"{"data":{}}"#).is_err());
        assert!(extract_user_profile(r#"{"data":{"userName":null}}"#).is_err());
        assert!(extract_user_profile(r#"{"data":{"userName":""}}"#).is_err());
        assert!(extract_user_profile(r#"{"data":{"userName":"  "}}"#).is_err());
    }

    /// 非法 JSON → Parse 错误。
    #[test]
    fn user_profile_invalid_json() {
        assert!(extract_user_profile("not json").is_err());
        assert!(extract_user_profile("").is_err());
    }

    // ---------- csrf_token ----------

    /// 金标向量（2026-09-18 实机取证）：md5("timestamp=1789705884524,key=lianyi2019")
    /// == "f523769fc014de2a561b4a81a0cf4c7d"。
    #[test]
    fn csrf_token_golden_vector() {
        assert_eq!(csrf_token(1789705884524), "f523769fc014de2a561b4a81a0cf4c7d");
    }

    /// 不同 ts 输出均为 32 位小写 hex，且互不相同（防呆）。
    #[test]
    fn csrf_token_lowercase_hex_varies() {
        for ts in [0u128, 1, 2, 1234567890, u128::MAX] {
            let t = csrf_token(ts);
            assert_eq!(t.len(), 32);
            assert!(
                t.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "非小写 hex: {t}"
            );
        }
        assert_ne!(csrf_token(1), csrf_token(2));
    }

    // ---------- parse_portrait_change_response ----------

    /// 实测成功形态：meta.success=true + data=true → Ok。
    #[test]
    fn portrait_change_success() {
        assert!(parse_portrait_change_response(
            r#"{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":true}"#
        )
        .is_ok());
    }

    /// meta.success=false → Parse 错误，message 保留服务端原文（上层直接展示）。
    #[test]
    fn portrait_change_failure_keeps_server_message() {
        let err = parse_portrait_change_response(
            r#"{"meta":{"success":false,"statusCode":500,"message":"token 已过期"},"data":null}"#,
        )
        .unwrap_err();
        match err {
            CampusAuthError::Parse(m) => assert_eq!(m, "token 已过期"),
            other => panic!("应为 Parse 错误: {other:?}"),
        }
    }

    /// 非法 JSON / 缺 meta / message 缺失 → Parse 错误（后者给固定文案）。
    #[test]
    fn portrait_change_invalid_or_missing_meta() {
        assert!(parse_portrait_change_response("not json").is_err());
        assert!(parse_portrait_change_response(r#"{"data":true}"#).is_err());
        let err = parse_portrait_change_response(r#"{"meta":{"success":false}}"#).unwrap_err();
        match err {
            CampusAuthError::Parse(m) => {
                assert!(m.contains("服务端未返回原因"));
            }
            other => panic!("应为 Parse 错误: {other:?}"),
        }
    }
}
