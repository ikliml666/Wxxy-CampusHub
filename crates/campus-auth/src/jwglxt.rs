//! 正方教务（jwglxt）SSO：CAS REST 换票 → `/sso/lyiotlogin` 302 链。
//!
//! 协议事实全部来自 2026-09-18 实测（`tests/jwglxt_live.rs`，样本与逐跳日志）：
//! - 教务在 CAS 端注册的 service 即 [`JWGL_SERVICE`]（`/sso/lyiotlogin`，自引用回跳）；
//!   无 ticket 访问该端点会 302 到 `lyuapServer/login?service=<JWGL_SERVICE>`（可复核）
//! - **ST 与 service 绑定**：`login()` 签发的 ticket 绑定门户 service，直接拿去
//!   `/sso/lyiotlogin?ticket=` 会得到 404；必须先用 [`CasClient::sso_ticket`] 以
//!   已有 TGT 换发教务 service 的 ST
//! - 换票响应体为**纯文本 ST**（`ST-…`，非 JSON）
//! - 302 链（5 跳）：`/sso/lyiotlogin?ticket=ST` → `/sso/lyiotlogin` →
//!   `/jwglxt/ticketlogin?uid=…&verify=…`（种 `JSESSIONID`+`rememberMe`）→
//!   `/xtgl/login_slogin.html` → `/xtgl/index_initMenu.html?jsdm=xs` 落点（学生主界面）
//! - 教务会话 cookie：`route` + `JSESSIONID`（+`rememberMe`），与门户会话相互独立
//! - 课表接口会话无效时：带 `X-Requested-With: XMLHttpRequest` 返回**自定义状态码
//!   901**（空 body）；不带则返回 200 + 登录页 HTML —— [`CampusAuthError::JwglNotLogin`]
//!   据此向上层暴露「未登录 / 需重新 SSO」，[`CasClient::fetch_timetable_json`]
//!   封装「确保教务会话（TGT 静默重进）+ 拉取课表 JSON 原文」

use crate::cas::{CasClient, CAS_BASE, JWGL_SERVICE};
use crate::error::CampusAuthError;

/// 学生课表查询端点：`POST`，`gnmkdm=N2151` 固定在 **query**（2026-09-18 实测有效），
/// body 仅 `xnm=<学年起始年>&xqm=<学期代码>`（学期代码约定同
/// `campus_schedule::zhengfang::Semester::xqm`：3=第1学期 / 12=第2学期 / 16=暑期；
/// 本 crate 不依赖 campus-schedule，由调用方传数值）。
pub const KBCX_XSKBCX_URL: &str =
    "https://jwgl.cwxu.edu.cn/jwglxt/kbcx/xskbcx_cxXsKb.html?gnmkdm=N2151";

/// 课表接口响应判定（纯函数，离线单测）：
/// - `901`（自定义状态码，reqwest 不识别但 `as_u16()` 可取）→ [`CampusAuthError::JwglNotLogin`]
///   （带 `X-Requested-With` 时未登录的确定形态，实测空 body）
/// - `200` 且 body 以 `{` 开头 → 正常 JSON 原文（解析交上层 `campus_schedule::parse_kb_response`）
/// - `200` 但非 JSON（如登录页 HTML）与其他状态码 → [`CampusAuthError::Parse`]
///   （错误消息只含长度/状态码，不含响应原文——登录页 21KB 且含站点信息，不入错误链路）
fn interpret_kbcx_response(status: u16, body: &str) -> Result<String, CampusAuthError> {
    match status {
        901 => Err(CampusAuthError::JwglNotLogin),
        200 if body.trim_start().starts_with('{') => Ok(body.to_string()),
        200 => Err(CampusAuthError::Parse(format!(
            "课表接口返回非 JSON（长度 {}，疑似登录页）",
            body.len()
        ))),
        other => Err(CampusAuthError::Parse(format!(
            "课表接口异常 HTTP 状态 {other}"
        ))),
    }
}

impl CasClient {
    /// CAS REST v1 换票：`POST {CAS_BASE}/v1/tickets/{tgt}`（x-www-form-urlencoded，
    /// body `service=<service>`）→ 响应体纯文本即新 ST。
    ///
    /// 实测 200 + `ST-…`；非 `ST-` 开头（含错误页/JSON）→ [`CampusAuthError::Parse`]。
    /// TGT 来自 `CasLoginOk.tgt`，**必须随会话持久化**——CAS 服务端
    /// 不种任何登录 cookie（jar 实测登录后为空），会话复用全靠客户端保存 TGT。
    pub async fn sso_ticket(&self, tgt: &str, service: &str) -> Result<String, CampusAuthError> {
        let body = self
            .http_client()
            .post(format!("{CAS_BASE}/v1/tickets/{tgt}"))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .form(&[("service", service)])
            .send()
            .await?
            .text()
            .await?;
        let st = body.trim();
        if st.starts_with("ST-") {
            Ok(st.to_string())
        } else {
            Err(CampusAuthError::Parse(format!(
                "CAS 换票响应非 ST（前缀 {:?}，长度 {}）",
                st.chars().take(3).collect::<String>(),
                st.len()
            )))
        }
    }

    /// 一键 SSO 进正方教务：TGT 换教务 ST → [`CasClient::sso_follow`] 手动跟随
    /// [`JWGL_SERVICE`] 的 302 链，返回落点 URL（实测为
    /// `/jwglxt/xtgl/index_initMenu.html?jsdm=xs…` 学生主界面）。
    ///
    /// 成功后本 client 的 jar 已种教务域 `route`/`JSESSIONID`（+`rememberMe`），
    /// 同 jar 的 [`CasClient::http_client`] 即可直接调 `/jwglxt/` 业务接口。
    /// ST 一次性且绑定教务 service，本方法每次现换，不做缓存。
    pub async fn jwglxt_sso(&self, tgt: &str) -> Result<reqwest::Url, CampusAuthError> {
        let st = self.sso_ticket(tgt, JWGL_SERVICE).await?;
        self.sso_follow(JWGL_SERVICE, &st).await
    }

    /// 课表接口单次 POST（头组：`Content-Type: …charset=UTF-8` +
    /// `X-Requested-With: XMLHttpRequest`——XRW 必带，让会话失效表现为明确的
    /// 901 而非 200 登录页；Accept/Referer 实测非必需）。返回 (状态码, body 原文)。
    async fn post_kbcx(&self, xnm: u32, xqm: u32) -> Result<(u16, String), CampusAuthError> {
        let resp = self
            .http_client()
            .post(KBCX_XSKBCX_URL)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded;charset=UTF-8",
            )
            .header("X-Requested-With", "XMLHttpRequest")
            .body(format!("xnm={xnm}&xqm={xqm}"))
            .send()
            .await?;
        let status = resp.status().as_u16();
        // 901 为空 body，个别环境 text() 可能报错——与 live 测试同款兜底为空串，
        // 901 判定不依赖 body。
        let body = resp.text().await.unwrap_or_default();
        Ok((status, body))
    }

    /// 确保教务会话并拉取课表 JSON 原文（解析交上层，本 crate 不依赖
    /// campus-schedule 的运行时依赖）。
    ///
    /// 流程：先直接 POST 课表接口；若教务会话失效（901）：
    /// - `tgt = Some` → [`CasClient::jwglxt_sso`] 用 TGT 静默重进（换新 ST 走 5 跳链）
    ///   后重试**一次**；重进失败或重试仍 901 → [`CampusAuthError::JwglNotLogin`]
    ///   （TGT 失效 = 静默续期不可用，上层只能引导重新登录，故网络/解析细节归一为
    ///   该变体，不向用户暴露换票内部错误）
    /// - `tgt = None`（如旧版 session.json 无 TGT）→ 直接 [`CampusAuthError::JwglNotLogin`]
    pub async fn fetch_timetable_json(
        &self,
        tgt: Option<&str>,
        xnm: u32,
        xqm: u32,
    ) -> Result<String, CampusAuthError> {
        let (status, body) = self.post_kbcx(xnm, xqm).await?;
        match interpret_kbcx_response(status, &body) {
            Ok(json) => Ok(json),
            Err(CampusAuthError::JwglNotLogin) => {
                let Some(tgt) = tgt else {
                    return Err(CampusAuthError::JwglNotLogin);
                };
                if self.jwglxt_sso(tgt).await.is_err() {
                    return Err(CampusAuthError::JwglNotLogin);
                }
                let (status, body) = self.post_kbcx(xnm, xqm).await?;
                interpret_kbcx_response(status, &body)
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 901 → JwglNotLogin（未登录 + XRW 的实测形态）。
    #[test]
    fn interpret_kbcx_901_maps_to_not_login() {
        assert!(matches!(
            interpret_kbcx_response(901, ""),
            Err(CampusAuthError::JwglNotLogin)
        ));
    }

    /// 200 + JSON → 原文透传；200 + HTML（登录页形态）→ Parse。
    #[test]
    fn interpret_kbcx_200_json_ok_and_html_rejected() {
        let json = r#"{"kbList":[],"xsxx":{}}"#;
        assert_eq!(
            interpret_kbcx_response(200, json).unwrap(),
            json.to_string()
        );
        // 前导空白容忍
        assert!(interpret_kbcx_response(200, "  \n{\"kbList\":[]}").is_ok());
        // 200 登录页 HTML（不带 XRW 时才出现的形态，防御性拒绝）
        let html = "<!DOCTYPE html><html>教学管理信息服务平台</html>";
        let err = interpret_kbcx_response(200, html).unwrap_err();
        assert!(
            matches!(err, CampusAuthError::Parse(_)),
            "实际: {err:?}"
        );
        // 错误消息不含响应原文（21KB 登录页不入错误链路）
        let msg = err.to_string();
        assert!(!msg.contains("DOCTYPE"));
    }

    /// 其他状态码 → Parse（消息只含状态码）。
    #[test]
    fn interpret_kbcx_other_status_is_parse_error() {
        for s in [404, 500, 302] {
            let err = interpret_kbcx_response(s, "x").unwrap_err();
            assert!(matches!(err, CampusAuthError::Parse(_)));
            assert!(err.to_string().contains(&s.to_string()));
        }
    }

    /// 常量防漂移：课表端点必须与 JWGL_SERVICE 同源（教务域）且 gnmkdm 在 query。
    #[test]
    fn kbcx_url_same_origin_as_jwgl_service() {
        let kb = reqwest::Url::parse(KBCX_XSKBCX_URL).unwrap();
        let svc = reqwest::Url::parse(JWGL_SERVICE).unwrap();
        assert_eq!(kb.host_str(), svc.host_str());
        assert_eq!(kb.scheme(), svc.scheme());
        assert!(kb.path().ends_with("xskbcx_cxXsKb.html"));
        assert_eq!(kb.query(), Some("gnmkdm=N2151"));
    }
}
