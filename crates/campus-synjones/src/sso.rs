//! lyCas SSO 桥：用已有 CAS TGT 换慧新E校 token。
//!
//! # 桥结构（2026-09-19 live 实测，`tests/synjones_live.rs` 逐跳取证）
//!
//! **浏览器入口**（人用，客户端换票不用它）：
//!
//! ```text
//! GET {BASE}/berserker-auth/cas/redirect/lyCas?targetUrl=<目标>
//!   → 302 https://wxcas.cwxu.edu.cn/lyuapServer/login?service=<编码后的>
//!     {BASE}/berserker-auth/cas/login/lyCas?targetUrl=<二次编码的目标>
//! ```
//!
//! **客户端换票要走的链**（2 跳，`service` 必须是 [`LY_CAS_SERVICE_PREFIX`] 那条）：
//!
//! ```text
//! POST {CAS_BASE}/v1/tickets/{TGT}  (form service=<lyCas service>)  → 纯文本 ST
//! GET  <lyCas service>&ticket=ST-…
//!   → 302 {BASE}/<targetUrl 对应的子应用路径>?synjones-auth=<token>     ← token 在 URL query
//!   → 200（子 SPA 壳；其 created() 读该 query 写 sessionStorage.access_token）
//! ```
//!
//! # targetUrl 的硬性要求（四组对照实测）
//!
//! 落点是否带 `?synjones-auth=<token>` **取决于 targetUrl 指向哪个子应用**：
//! 子应用页（`/campus-card-pc/`、`/charge-pc/pays/450`）带 token 且 token 可调业务接口；
//! `/plat/shouyeUser` 与「无 targetUrl」落点都是 `/plat/?name=…&ticket=…`，**不带 token**。
//! 故默认取 [`DEFAULT_TARGET_PATH`]（一卡通子应用）。
//!
//! # 会话纪律（实测）
//!
//! - **token 是「单活」的**：同一账号同一时刻只有**最新一次 SSO 签发**的 token 有效；
//!   早签发的 token 在后续 SSO 之后调业务接口返回 401。故产品必须**单 token 缓存**、
//!   不并发多次 SSO（[`crate::client::SynjonesClient`] 即此形态）。
//! - CAS TGT 会过期（实测：隔夜的 `tgtB64` 换票得到 `HTTP 500` + 正文以 `TGT` 开头的错误），
//!   此时只能重新账密登录；故换票失败要给出「请重新登录」的可操作文案。
//! - 302 链的跟随复用 [`CasClient::sso_follow`]（内含 body 中断容忍与 http→https 升级；
//!   升级只对 `.cwxu.edu.cn` 生效，内网明文 IP `10.3.100.110` 不会被误升级，正合本场景）。

use crate::{
    CampusSynjonesError, SynjonesToken, BERSERKER_BASE, LY_CAS_REDIRECT_PATH, LY_CAS_SERVICE_PREFIX,
};
use campus_auth::cas::CasClient;
use campus_webvpn::{wrap_url, WebVpnSession, GATEWAY};

/// 默认 targetUrl 路径：**一卡通子应用首页**。
///
/// 实测（2026-09-19 live 探针，四组对照）：落点是否带 `?synjones-auth=<token>` 取决于
/// targetUrl 指向哪个子应用——
/// - `/campus-card-pc/`（一卡通）→ 落点 `/campus-card-pc/?synjones-auth=…`，token 可用 ✅
/// - `/charge-pc/pays/450`（电费）→ 落点 `/charge-pc/pays/450?synjones-auth=…`，token 可用 ✅
/// - `/plat/shouyeUser`（移动端首页）→ 落点 `/plat/?name=…&ticket=…`，**不带 token** ❌
/// - 无 targetUrl → 落点 `/plat/?name=…&ticket=…`，**不带 token** ❌
///
/// 选一卡通页作默认：token 的目标消费者就是它（子 SPA 自身也从该 query 读 token），
/// 且路径稳定；电费页 URL 带 feeitemid，会随片区调整失效。
pub const DEFAULT_TARGET_PATH: &str = "/campus-card-pc/";

/// 默认 targetUrl 全值。
pub fn default_target_url() -> String {
    format!("{BERSERKER_BASE}{DEFAULT_TARGET_PATH}")
}

/// 带 `?targetUrl=<percent-encoded>` 的完整 URL（`Url::query_pairs_mut` 负责编码，
/// 与实测 Location 的编码层级一致：service 内部一次、CAS 再整体编码一次）。
fn with_target(prefix: &str, target_url: &str) -> String {
    let mut u = reqwest::Url::parse(prefix).expect("常量前缀恒为合法 URL");
    if !target_url.is_empty() {
        u.query_pairs_mut().append_pair("targetUrl", target_url);
    }
    u.to_string()
}

/// 换票要用的 service 全值（[`LY_CAS_LOGIN_PATH`] + `targetUrl`）。
pub fn ly_cas_service_url(target_url: &str) -> String {
    with_target(LY_CAS_SERVICE_PREFIX, target_url)
}

/// 浏览器入口 URL（[`LY_CAS_REDIRECT_PATH`] + `targetUrl`），供人工对照/诊断用。
pub fn ly_cas_redirect_url(target_url: &str) -> String {
    with_target(&format!("{BERSERKER_BASE}{LY_CAS_REDIRECT_PATH}"), target_url)
}

/// 从落点 URL 提取 token（纯函数，单测覆盖）。
///
/// 实测形态：落点 URL 带 `?synjones-auth=<raw token>`（**无 `bearer ` 前缀**，
/// 子 SPA `created()` 读该 query 后原样写 sessionStorage 的 `access_token`）。
/// 另容忍值自带 `bearer ` 前缀（部分链路以 `synjones-auth=bearer <token>` 传递）。
fn extract_token_from_url(url: &reqwest::Url) -> Option<SynjonesToken> {
    let mut raw = None;
    let mut token_type = None;
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "synjones-auth" => raw = Some(v.to_string()),
            // 少数链路以标准 OAuth 字段名回传（探针实测为准，未命中时本分支不触发）
            "access_token" => raw = Some(v.to_string()),
            "token_type" => token_type = Some(v.to_string()),
            _ => {}
        }
    }
    let raw = raw?;
    let trimmed = raw.trim();
    // 容忍 "bearer <token>" / "Bearer <token>" 形态
    let (tt, tok) = match trimmed.split_once(' ') {
        Some((a, b)) if a.eq_ignore_ascii_case("bearer") => (Some("bearer".to_string()), b.trim()),
        _ => (token_type, trimmed),
    };
    if tok.is_empty() {
        return None;
    }
    let mut t = SynjonesToken::bearer(tok);
    if let Some(tt) = tt {
        if !tt.trim().is_empty() {
            t.token_type = tt;
        }
    }
    Some(t)
}

/// query 的**键名**（供错误消息与探针诊断：值一律不外泄）。
pub fn query_keys(url: &reqwest::Url) -> Vec<String> {
    url.query_pairs().map(|(k, _)| k.to_string()).collect()
}

/// CAS TGT → 慧新E校 token（SSO 桥）。
///
/// 流程：换票（绑定 lyCas service 的 ST）→ 手动跟随 302 链 → 落点 URL 提取 token。
/// 落点无 token 时返回 [`CampusSynjonesError::SsoFailed`]，消息含 host/path/query 键名
/// （**不含任何值**）便于定位，并给出可操作文案。
pub async fn sso_token(
    cas: &CasClient,
    tgt: &str,
    target_url: &str,
) -> Result<SynjonesToken, CampusSynjonesError> {
    let service = ly_cas_service_url(target_url);
    let st = cas
        .sso_ticket(tgt, &service)
        .await
        .map_err(|e| CampusSynjonesError::SsoFailed(crate::redact_secrets(&format!(
            "CAS 换票失败（TGT 可能已过期）：{e}"
        ))))?;
    let landing = cas.sso_follow(&service, &st).await.map_err(|e| {
        CampusSynjonesError::SsoFailed(crate::redact_secrets(&format!(
            "跟随 lyCas 回跳链失败：{e}"
        )))
    })?;
    extract_token_from_url(&landing).ok_or_else(|| {
        CampusSynjonesError::SsoFailed(format!(
            "落点未携带 token（host={:?} path={} query 键={:?}），请重新登录后重试",
            landing.host_str(),
            landing.path(),
            query_keys(&landing)
        ))
    })
}

/// WebVPN 回跳链的入口 URL（纯函数供单测）：`wrap_url(service & ticket=ST)`。
///
/// - **service 本身不参与换票包装**：换票 `POST {CAS_BASE}/v1/tickets/{TGT}` 走公网
///   CAS（`wxcas.cwxu.edu.cn` 校外可达），service 必须保持 CAS 注册的内网原值；
/// - **回跳入口才包装**：`GET service&ticket=ST` 这一步要真正打到 lyCas（内网），
///   校外经网关代理；`WebVpnSession::follow_wrap` 对链中每一跳 Location 同样先判后包
///   （幂等：Location 可能已是网关包装形态）。
/// - ticket 用 `&` 追加（lyCas service 自带 query）+ form_urlencoded 编码，与
///   `campus_webvpn::session::login_redirect_url` 同款。
pub fn vpn_entry_url(service: &str, ticket: &str) -> Result<String, campus_webvpn::WebVpnError> {
    let mut u = reqwest::Url::parse(service)
        .map_err(campus_webvpn::WebVpnError::InvalidUrl)?;
    u.query_pairs_mut().append_pair("ticket", ticket);
    wrap_url(u.as_str(), GATEWAY)
}

/// CAS TGT → 慧新E校 token（SSO 桥，**校外 WebVPN 链路**）。
///
/// 与 [`sso_token`] 的差异只在回跳链的走向：换票同款（CAS 公网可达、service 原值），
/// 回跳链入口经网关包装、由 [`WebVpnSession::follow_wrap`] 手动跟随且**每一跳 Location
/// 都先判后包**。token 仍从落点 URL query 提取（`wrap_url` 原样保留 query，提取逻辑
/// 与校内链路完全同源）。`vpn` 用网关侧 cookie jar（WebVPN 登录态），与 CAS jar 互不污染。
pub async fn sso_token_via(
    cas: &CasClient,
    tgt: &str,
    target_url: &str,
    vpn: &WebVpnSession,
) -> Result<SynjonesToken, CampusSynjonesError> {
    let service = ly_cas_service_url(target_url);
    let st = cas
        .sso_ticket(tgt, &service)
        .await
        .map_err(|e| CampusSynjonesError::SsoFailed(crate::redact_secrets(&format!(
            "CAS 换票失败（TGT 可能已过期）：{e}"
        ))))?;
    let entry = vpn_entry_url(&service, &st).map_err(|e| {
        CampusSynjonesError::SsoFailed(crate::redact_secrets(&format!(
            "回跳入口包装失败：{e}"
        )))
    })?;
    let landing = vpn.follow_wrap(&entry).await.map_err(|e| {
        CampusSynjonesError::SsoFailed(crate::redact_secrets(&format!(
            "跟随 lyCas 回跳链失败（WebVPN）：{e}"
        )))
    })?;
    extract_token_from_url(&landing).ok_or_else(|| {
        CampusSynjonesError::SsoFailed(format!(
            "落点未携带 token（host={:?} path={} query 键={:?}），请重新登录后重试",
            landing.host_str(),
            landing.path(),
            query_keys(&landing)
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> reqwest::Url {
        reqwest::Url::parse(s).expect("测试 URL 恒合法")
    }

    /// service URL：前缀正确 + targetUrl 被 percent-encode（与实测 Location 形态一致）。
    #[test]
    fn service_url_encodes_target() {
        let s = ly_cas_service_url("http://10.3.100.110/charge-pc/pays/450");
        assert_eq!(
            s,
            "http://10.3.100.110/berserker-auth/cas/login/lyCas?targetUrl=http%3A%2F%2F10.3.100.110%2Fcharge-pc%2Fpays%2F450"
        );
    }

    /// 无 targetUrl 时只留裸 service（探针对照组用）。
    #[test]
    fn service_url_without_target_is_bare_prefix() {
        assert_eq!(ly_cas_service_url(""), LY_CAS_SERVICE_PREFIX);
        assert_eq!(ly_cas_redirect_url(""), format!("{BERSERKER_BASE}{LY_CAS_REDIRECT_PATH}"));
    }

    /// 落点提取：`?synjones-auth=<raw>` → bearer。
    #[test]
    fn extract_raw_token_from_query() {
        let t = extract_token_from_url(&url("http://10.3.100.110/plat/shouyeUser?synjones-auth=ABC123"))
            .expect("应提取到 token");
        assert_eq!(t.access_token, "ABC123");
        assert_eq!(t.token_type, "bearer");
        assert_eq!(t.auth_value(), "bearer ABC123");
    }

    /// 落点提取：值自带 bearer 前缀时按空格切分，不把前缀当 token。
    #[test]
    fn extract_token_strips_bearer_prefix() {
        let t = extract_token_from_url(&url("http://10.3.100.110/plat/?synjones-auth=bearer%20ABC123"))
            .unwrap();
        assert_eq!(t.access_token, "ABC123");
    }

    /// 无 token 的落点（如 CAS 登录页）返回 None，而不是伪造空 token。
    #[test]
    fn extract_none_when_absent() {
        assert!(extract_token_from_url(&url("https://wxcas.cwxu.edu.cn/lyuapServer/login?service=x")).is_none());
        assert!(extract_token_from_url(&url("http://10.3.100.110/plat/?synjones-auth=")).is_none());
    }

    /// 默认 targetUrl 必须是实测带 token 的子应用页（`/plat/shouyeUser` 实测不带 token，禁用）。
    #[test]
    fn default_target_is_subapp_page() {
        assert_eq!(DEFAULT_TARGET_PATH, "/campus-card-pc/");
        assert_eq!(
            default_target_url(),
            format!("{BERSERKER_BASE}{DEFAULT_TARGET_PATH}")
        );
        assert!(
            !default_target_url().contains("/plat/"),
            "移动端 SPA 路径落点不带 token（实测），不得作为默认"
        );
    }

    /// query 键名列表不含值（错误消息与探针日志只暴露键名）。
    #[test]
    fn query_keys_exposes_only_names() {
        let u = url("http://h/p?a=1&synjones-auth=SECRET");
        let keys = query_keys(&u);
        assert_eq!(keys, vec!["a".to_string(), "synjones-auth".to_string()]);
        assert!(!format!("{keys:?}").contains("SECRET"));
    }

    /// WebVPN 回跳入口：service 保持内网原值参与 targetUrl，仅整条回跳 URL 被网关包装，
    /// ticket 以 `&` 追加且编码；幂等性（已包装形态原样通过）同样成立。
    #[test]
    fn vpn_entry_url_wraps_service_and_appends_ticket() {
        let service = ly_cas_service_url("http://10.3.100.110/charge-pc/pays/450");
        let entry = vpn_entry_url(&service, "ST-abc123").expect("应包装成功");
        let hex = campus_webvpn::encrypt_host("10.3.100.110");
        assert_eq!(
            entry,
            format!(
                "{GATEWAY}/http/{hex}/berserker-auth/cas/login/lyCas?targetUrl=http%3A%2F%2F10.3.100.110%2Fcharge-pc%2Fpays%2F450&ticket=ST-abc123"
            )
        );
        // ticket 特殊字符被编码（防御性，与 login_redirect_url 同款）；
        // 裸前缀无 query → 用 `?` 追加（正式链路 service 自带 targetUrl → `&`，上面已验证）
        let entry2 = vpn_entry_url(&LY_CAS_SERVICE_PREFIX, "ST a+b").expect("应包装成功");
        assert!(entry2.ends_with("?ticket=ST+a%2Bb"), "实际 {entry2}");
        // 幂等：已是网关形态的入口原样通过（follow_wrap 逐跳同理）
        assert_eq!(vpn_entry_url(&entry, "ST-x").unwrap(), format!("{entry}&ticket=ST-x"));
    }
}
