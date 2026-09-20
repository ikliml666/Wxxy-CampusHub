//! 慧新E校业务客户端：统一头组注入 + 三套响应信封解析 + 会话失效静默重进。
//!
//! # 头组契约（照抄官方前端请求拦截器，2026-09-19 逆向实测）
//!
//! - **token**：`synjones-auth: <token_type> <token>`（token_type 缺省 `bearer`）。
//! - **来源参数**：`synAccessSource=app` **双份携带**——GET 走 query 后**再**加同名头；
//!   POST form 合并进 body 后**再**加同名头。少一份都可能触发 4030。
//! - 非 401 的 HTTP 异常与网络错误一律压成 [`CampusSynjonesError::Http`]，
//!   不把 reqwest 错误细节冒到前端。
//!
//! # 会话失效与静默重进
//!
//! `HTTP 401`（含 `code=4030` 来源授权被拒）与业务 `code ∈ {401,4030,4037,4038,4011}`
//! 统一归一为 [`CampusSynjonesError::NotLogin`]；首次命中时用 CAS TGT **静默重进桥一次**
//! 并重试一次，仍失败才向上报 `NotLogin`（与 `campus-auth::jwglxt` 的既有范式一致）。

use crate::sso::{default_target_url, sso_token, sso_token_via};
use crate::{CampusSynjonesError, SynjonesToken, BERSERKER_BASE, SYN_ACCESS_SOURCE};
use campus_auth::cas::CasClient;
use campus_webvpn::WebVpnSession;
use serde_json::Value;
use std::sync::Mutex;
use std::time::Duration;

/// 单请求超时（与 `campus-portal` 的 `REQUEST_TIMEOUT` 一致，避免命令悬挂）。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 会话失效的业务 code 白名单（官方前端 `frontConfig.unauthorizedCode` 缺省值）。
const UNAUTHORIZED_CODES: [i64; 4] = [4030, 4037, 4038, 4011];

/// 响应信封族——三套结构不可混用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Envelope {
    /// `/berserker-*`：`{code, success, data, msg}`。
    Berserker,
    /// `/charge/*`：`{code, message}`（401 时 `message` 可能是空串，甚至实测为 `","`）。
    Charge,
    /// `/berserker-search/*`：`{code, data, msg}`。
    Search,
}

impl Envelope {
    /// 三套信封的 code 字段名一致（`code`），差异只在错误文案字段（见 [`Envelope::message`]）。
    fn code(&self, v: &Value) -> Option<i64> {
        v.get("code").and_then(Value::as_i64)
    }

    /// 错误文案字段：charge 系用 `message`，其余用 `msg`。
    fn message(&self, v: &Value) -> String {
        let key = if *self == Envelope::Charge { "message" } else { "msg" };
        v.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    }
}

/// 解析响应信封（纯函数，单测覆盖）。
///
/// 成功（HTTP 2xx 且 `code==200`）返回**整个响应 JSON**——各业务端点取哪个字段不同
/// （berserker/search 取 `data`，charge 的 `feeitem` 目录在**顶层** `feeitemList`），
/// 故不在本层裁剪。
pub fn parse_envelope(
    kind: Envelope,
    status: u16,
    body: &str,
) -> Result<Value, CampusSynjonesError> {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let code = parsed.as_ref().and_then(|v| kind.code(v));

    // HTTP 401：鉴权失败（含 4030 来源授权被拒），归一为 NotLogin 以便上层静默重进。
    if status == 401 {
        if code == Some(4030) {
            eprintln!(
                "[synjones-diag] 服务端拒绝来源授权（HTTP 401 code=4030，已按 synAccessSource={SYN_ACCESS_SOURCE} 双份携带）"
            );
        }
        return Err(CampusSynjonesError::NotLogin);
    }
    if !(200..300).contains(&status) {
        return Err(CampusSynjonesError::Http(format!("HTTP {status}")));
    }
    let Some(v) = parsed else {
        return Err(CampusSynjonesError::Parse(format!(
            "响应非 JSON（长度 {}）",
            body.len()
        )));
    };
    let Some(code) = kind.code(&v) else {
        return Err(CampusSynjonesError::Parse(
            "响应缺少 code 字段（信封不符）".to_string(),
        ));
    };
    if code == 200 {
        return Ok(v);
    }
    if code == 401 || UNAUTHORIZED_CODES.contains(&code) {
        return Err(CampusSynjonesError::NotLogin);
    }
    Err(CampusSynjonesError::Api {
        code: code as i32,
        msg: kind.message(&v),
    })
}

/// 慧新E校业务客户端。Clone 廉价（`CasClient`、reqwest client 与 `WebVpnSession` 均为 Arc 包装，共享 Cookie jar）。
#[derive(Clone)]
pub struct SynjonesClient {
    /// 已登录的 CAS 会话（共享其 `http_client` 与 Cookie jar）。
    cas: CasClient,
    /// CAS TGT（会话失范时静默重进桥的唯一凭据；None = 无法静默重进）。
    tgt: Option<String>,
    /// 会话内 token 缓存（std Mutex：guard 不跨 await；缓存生命周期 = 客户端生命周期）。
    token: std::sync::Arc<Mutex<Option<SynjonesToken>>>,
    /// 业务根（默认 [`BERSERKER_BASE`]；单测指向本地桩服务）。
    base: String,
    /// SSO 桥的 targetUrl。
    target_url: String,
    /// 校外 WebVPN 路由覆盖（M4）：业务 base 覆盖（网关包装形态，去尾斜杠）。
    /// `None` = 校内直连现状（[`BERSERKER_BASE`]）；全部 base 拼接点收口在 [`Self::base_url`]。
    base_override: Option<String>,
    /// 校外 WebVPN 会话（与 `base_override` 同步经 [`Self::set_webvpn`] 设置）：
    /// 业务请求与 SSO 桥逐跳包装跟随共用它的网关 cookie jar。
    vpn: Option<WebVpnSession>,
}

impl SynjonesClient {
    /// 共享 HTTP 客户端（fapi 人脸采集等旁路请求复用连接池）。
    pub fn http_client(&self) -> &reqwest::Client {
        self.cas.http_client()
    }

    /// **业务请求应使用的 HTTP 句柄**（M4 路由收口点）：WebVPN 模式返回网关会话的
    /// 业务 client（网关 cookie jar），校内直连返回 CAS 的共享 client。
    /// 旁路请求（fapi 人脸采集等不走 `get`/`post_*` 的）一律经本方法取句柄。
    pub fn effective_http(&self) -> &reqwest::Client {
        match &self.vpn {
            Some(v) => v.wrapped_client(),
            None => self.cas.http_client(),
        }
    }

    /// 业务 base（M4 收口点）：WebVPN 模式返回网关包装形态，否则 [`BERSERKER_BASE`]。
    /// 所有 base 拼接点（`get`/`post_form`/`post_json_vals`/`recharge::json_post`/fapi）必须经它。
    pub fn base_url(&self) -> &str {
        self.base_override.as_deref().unwrap_or(self.base.as_str())
    }

    /// 是否处于校外 WebVPN 路由模式（命令层据此给写路径追加「走官网」提示等）。
    pub fn is_webvpn(&self) -> bool {
        self.vpn.is_some()
    }

    /// 设置校外 WebVPN 路由态（命令层路由决策的唯一注入口）。
    ///
    /// - `base`：`wrap_url(BERSERKER_BASE, GATEWAY)` 去尾斜杠后的网关包装形态；
    /// - `vpn`：与 base 同源的 WebVPN 会话（clone 与原实例共享网关 cookie jar，
    ///   锁外使用安全）；校内直连传 `(None, None)` 即回到现状。
    pub fn set_webvpn(&mut self, base: Option<String>, vpn: Option<WebVpnSession>) {
        self.base_override = base;
        self.vpn = vpn;
    }

    /// 当前 synjones token 快照（人脸采集要解 JWT 里的学号；None = 尚未登录）。
    pub fn token_snapshot(&self) -> Option<SynjonesToken> {
        self.token.lock().ok().and_then(|g| g.clone())
    }

    /// 构造：`token` 为 None 时首次业务请求会先走 SSO 桥取票。
    pub fn new(cas: CasClient, tgt: Option<String>, token: Option<SynjonesToken>) -> Self {
        Self {
            cas,
            tgt,
            token: std::sync::Arc::new(Mutex::new(token)),
            base: BERSERKER_BASE.to_string(),
            target_url: default_target_url(),
            base_override: None,
            vpn: None,
        }
    }

    /// 覆盖桥接 targetUrl（默认 `/plat/shouyeUser`）。
    pub fn with_target_url(mut self, target_url: impl Into<String>) -> Self {
        self.target_url = target_url.into();
        self
    }

    /// 当前 token（含 token_type），未登录为 None。
    pub fn token(&self) -> Option<SynjonesToken> {
        self.token.lock().ok().and_then(|g| g.clone())
    }

    /// 是否已有可用 token（前端据此决定「未登录时显示登录引导」而不发请求）。
    pub fn has_token(&self) -> bool {
        self.token().is_some_and(|t| !t.is_empty())
    }

    /// 取 token：命中缓存即返回，否则走 SSO 桥（无 TGT 时直接 `NotLogin`）。
    async fn ensure_token(&self) -> Result<SynjonesToken, CampusSynjonesError> {
        let cached = self.token.lock().ok().and_then(|g| g.clone());
        if let Some(t) = cached {
            if !t.is_empty() {
                return Ok(t);
            }
        }
        self.reenter().await
    }

    /// 静默重进：无视缓存重新走 SSO 桥，成功后刷新缓存。
    ///
    /// WebVPN 模式走 [`sso_token_via`]（逐跳包装跟随，复用网关 cookie jar），校内
    /// 直连走 [`sso_token`]（现状链路）——两条链的换票 service 都是 CAS 注册的内网
    /// 原值（CAS 公网可达，service 值不参与包装，见 `sso.rs`）。
    pub async fn reenter(&self) -> Result<SynjonesToken, CampusSynjonesError> {
        let Some(tgt) = self.tgt.as_deref() else {
            return Err(CampusSynjonesError::NotLogin);
        };
        let token = match &self.vpn {
            Some(v) => sso_token_via(&self.cas, tgt, &self.target_url, v).await?,
            None => sso_token(&self.cas, tgt, &self.target_url).await?,
        };
        if let Ok(mut g) = self.token.lock() {
            *g = Some(token.clone());
        }
        Ok(token)
    }

    /// 统一头组：`synjones-auth` + `synAccessSource` 同名头（query/body 侧由调用方另加）。
    fn with_headers(rb: reqwest::RequestBuilder, token: &SynjonesToken) -> reqwest::RequestBuilder {
        rb.header("synjones-auth", token.auth_value())
            .header("synAccessSource", SYN_ACCESS_SOURCE)
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/plain, */*",
            )
    }

    /// GET 业务接口：query 带 `synAccessSource`（**双份携带**之一）+ 统一头组。
    pub async fn get(
        &self,
        path: &str,
        params: &[(&str, &str)],
        kind: Envelope,
    ) -> Result<Value, CampusSynjonesError> {
        self.send(kind, |token| {
            let mut rb = Self::with_headers(
                self.effective_http()
                    .get(format!("{}{path}", self.base_url()))
                    .timeout(REQUEST_TIMEOUT),
                token,
            );
            for (k, v) in params {
                rb = rb.query(&[(k, v)]);
            }
            rb.query(&[("synAccessSource", SYN_ACCESS_SOURCE)])
        })
        .await
    }

    /// POST form 业务接口：body 合并 `synAccessSource` + 统一头组。
    pub async fn post_form(
        &self,
        path: &str,
        form: &[(&str, String)],
        kind: Envelope,
    ) -> Result<Value, CampusSynjonesError> {
        self.send(kind, |token| {
            let mut fields: Vec<(&str, String)> =
                vec![("synAccessSource", SYN_ACCESS_SOURCE.to_string())];
            fields.extend(form.iter().map(|(k, v)| (*k, v.clone())));
            Self::with_headers(
                self.effective_http()
                    .post(format!("{}{path}", self.base_url()))
                    .timeout(REQUEST_TIMEOUT)
                    .form(&fields),
                token,
            )
        })
        .await
    }

    /// POST **JSON** 业务接口（值全为字符串）：body 是对象（`synAccessSource` 合并进去）+ 统一头组。
    ///
    /// 与 [`Self::post_form`] 的区别只有 body 编码。**写操作（挂失/解挂/改密/限额/圈存/
    /// 转账/绑卡）实测只认 JSON**：官方 axios 实例默认 JSON body，用 form 提交同样参数会得到
    /// `code=400 业务异常`（2026-09-19 真机验证：限额与转账三个写端点全 400，改 JSON 后限额通过）。
    pub async fn post_json(
        &self,
        path: &str,
        form: &[(&str, String)],
        kind: Envelope,
    ) -> Result<Value, CampusSynjonesError> {
        let vals: Vec<(&str, Value)> = form
            .iter()
            .map(|(k, v)| (*k, Value::String(v.clone())))
            .collect();
        self.post_json_vals(path, &vals, kind).await
    }

    /// POST JSON 业务接口（值可为字符串/**数字**/布尔）。
    ///
    /// 金额类字段传 JSON **number** 而不是字符串——官方前端就是直接传 number
    /// （如转账的 `tranamt: this.amountValue.number`）；字符串形态在部分端点被拒
    /// （`code=400 操作失败`）。
    pub async fn post_json_vals(
        &self,
        path: &str,
        form: &[(&str, Value)],
        kind: Envelope,
    ) -> Result<Value, CampusSynjonesError> {
        let mut obj = serde_json::Map::with_capacity(form.len() + 1);
        obj.insert(
            "synAccessSource".to_string(),
            Value::String(SYN_ACCESS_SOURCE.to_string()),
        );
        for (k, v) in form {
            obj.insert((*k).to_string(), v.clone());
        }
        let body = Value::Object(obj);
        self.send(kind, |token| {
            Self::with_headers(
                self.effective_http()
                    .post(format!("{}{path}", self.base_url()))
                    .timeout(REQUEST_TIMEOUT)
                    .json(&body),
                token,
            )
        })
        .await
    }

    /// 发送 + 会话失效静默重进（首次 NotLogin → `reenter` → 重试一次 → 仍失败归一 NotLogin）。
    async fn send(
        &self,
        kind: Envelope,
        build: impl Fn(&SynjonesToken) -> reqwest::RequestBuilder,
    ) -> Result<Value, CampusSynjonesError> {
        let mut last = CampusSynjonesError::NotLogin;
        for attempt in 0..2 {
            let token = self.ensure_token().await?;
            let resp = build(&token)
                .send()
                .await
                .map_err(|e| CampusSynjonesError::Http(crate::redact_secrets(&e.to_string())))?;
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            match parse_envelope(kind, status, &body) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last = e;
                    let is_not_login = matches!(last, CampusSynjonesError::NotLogin);
                    if attempt == 0 && is_not_login && self.tgt.is_some() {
                        // 静默重进一次；重进失败按会话失效上抛（不暴露换票内部细节）
                        self.reenter().await.map_err(|_| CampusSynjonesError::NotLogin)?;
                        continue;
                    }
                    break;
                }
            }
        }
        Err(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc::{channel, Receiver};

    // ---------- 最小桩服务（std TcpListener，零新依赖） ----------

    /// 起一个只服务一条连接的桩：回 `response`，并把收到的原始请求文本回传。
    fn stub(response: String) -> (String, Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定本地端口失败");
        let addr = listener.local_addr().expect("取本地地址失败");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept 失败");
            let raw = read_request(&mut sock);
            tx.send(raw).ok();
            sock.write_all(response.as_bytes()).ok();
            sock.flush().ok();
        });
        (format!("http://{addr}"), rx)
    }

    /// 读到 header 结束 + 按 Content-Length 读满 body（reqwest 默认 keep-alive，不能只 read 一次）。
    fn read_request(sock: &mut std::net::TcpStream) -> String {
        let mut buf: Vec<u8> = Vec::new();
        let mut tmp = [0u8; 1024];
        let head_end = loop {
            let n = sock.read(&mut tmp).expect("读请求失败");
            if n == 0 {
                break buf.len();
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break p + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let len: usize = head
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                if k.trim().eq_ignore_ascii_case("content-length") {
                    v.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);
        while buf.len() < head_end + len {
            let n = sock.read(&mut tmp).expect("读 body 失败");
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    fn client_with_base(base: &str, token: &str) -> SynjonesClient {
        let mut c = SynjonesClient::new(
            CasClient::new().expect("创建 CasClient 失败"),
            None,
            Some(SynjonesToken::bearer(token)),
        );
        c.base = base.to_string();
        c
    }

    /// 桩响应（带 Content-Length，reqwest 才能判定读满）。
    fn http_json(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json;charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn http_status(code: u16, body: &str) -> String {
        format!(
            "HTTP/1.1 {code} Unauthorized\r\nContent-Type: application/json;charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    // ---------- 头组：双份携带 ----------

    #[tokio::test]
    async fn get_carries_source_in_query_and_header() {
        let (base, rx) = stub(http_json(r#"{"code":200,"success":true,"data":{}}"#));
        let c = client_with_base(&base, "T");
        let v = c
            .get("/berserker-app/ykt/tsm/queryCurrentCard", &[], Envelope::Berserker)
            .await
            .expect("桩应答 200 应成功");
        assert_eq!(v["code"].as_i64(), Some(200));
        let raw = rx.recv().expect("桩未收到请求");
        let head = raw.to_ascii_lowercase();
        assert!(
            raw.contains("GET /berserker-app/ykt/tsm/queryCurrentCard?synAccessSource=app"),
            "GET 的 query 必须带 synAccessSource=app，实际首行：{}",
            raw.lines().next().unwrap_or("")
        );
        assert!(head.contains("synjones-auth: bearer t"), "缺 synjones-auth 头：{raw}");
        assert!(
            head.contains("synaccesssource: app"),
            "GET 必须另带同名头（双份携带）：{raw}"
        );
    }

    #[tokio::test]
    async fn get_keeps_business_params_with_source() {
        let (base, rx) = stub(http_json(r#"{"code":200,"success":true,"data":{}}"#));
        let c = client_with_base(&base, "T");
        c.get(
            "/berserker-search/search/personal/turnover",
            &[("account", "123"), ("size", "20"), ("current", "1"), ("type", "2")],
            Envelope::Search,
        )
        .await
        .expect("应成功");
        let raw = rx.recv().expect("桩未收到请求");
        let first = raw.lines().next().unwrap_or("");
        assert!(first.contains("account=123"), "业务参数丢失：{first}");
        assert!(first.contains("type=2"), "业务参数丢失：{first}");
        assert!(first.contains("synAccessSource=app"), "来源参数丢失：{first}");
    }

    #[tokio::test]
    async fn post_form_carries_source_in_body_and_header() {
        let (base, rx) = stub(http_json(
            r#"{"code":200,"message":"ok","map":{"total":[]}}"#,
        ));
        let c = client_with_base(&base, "T");
        c.post_form(
            "/charge/feeitem/getThirdData",
            &[("feeitemid", "450".to_string()), ("type", "select".to_string()), ("level", "0".to_string())],
            Envelope::Charge,
        )
        .await
        .expect("应成功");
        let raw = rx.recv().expect("桩未收到请求");
        let (head, body) = raw.split_once("\r\n\r\n").expect("请求应有空行分隔");
        let head_lc = head.to_ascii_lowercase();
        assert!(
            head_lc.contains("content-type: application/x-www-form-urlencoded"),
            "form 请求应带 urlencoded Content-Type：{head}"
        );
        assert!(head_lc.contains("synaccesssource: app"), "POST 也要带同名头：{head}");
        assert!(head_lc.contains("synjones-auth: bearer t"), "缺 synjones-auth 头：{head}");
        assert!(body.contains("feeitemid=450"), "body 丢业务字段：{body}");
        assert!(body.contains("type=select"), "body 丢业务字段：{body}");
        assert!(body.contains("level=0"), "body 丢业务字段：{body}");
        assert!(body.contains("synAccessSource=app"), "body 必须合并来源参数：{body}");
    }

    #[tokio::test]
    async fn get_without_token_and_without_tgt_is_not_login() {
        // 无 token、无 TGT → 不能发请求，直接 NotLogin（前端据此显示登录引导）
        let mut c = SynjonesClient::new(CasClient::new().unwrap(), None, None);
        c.base = "http://127.0.0.1:1".to_string();
        let e = c
            .get("/berserker-app/ykt/tsm/queryCurrentCard", &[], Envelope::Berserker)
            .await
            .expect_err("应报 NotLogin");
        assert!(matches!(e, CampusSynjonesError::NotLogin), "实际 {e:?}");
    }

    // ---------- 三套信封 ----------

    #[test]
    fn berserker_envelope_success_and_errors() {
        let ok = parse_envelope(
            Envelope::Berserker,
            200,
            r#"{"code":200,"success":true,"data":{"schemeId":2},"msg":null}"#,
        )
        .expect("应成功");
        assert_eq!(ok["data"]["schemeId"].as_i64(), Some(2));
        // 业务错误（HTTP 200 但 code != 200）→ Api{code,msg}
        let e = parse_envelope(
            Envelope::Berserker,
            200,
            r#"{"code":500,"success":false,"data":null,"msg":"系统繁忙"}"#,
        )
        .expect_err("应报错");
        assert!(matches!(e, CampusSynjonesError::Api { code: 500, .. }), "实际 {e:?}");
        assert!(e.to_string().contains("系统繁忙"));
        // HTTP 401 + code=4030（来源授权被拒）→ NotLogin
        let e = parse_envelope(
            Envelope::Berserker,
            401,
            r#"{"code":4030,"success":false,"data":null,"msg":"未授权，请先授权"}"#,
        )
        .expect_err("应报 NotLogin");
        assert!(matches!(e, CampusSynjonesError::NotLogin), "实际 {e:?}");
    }

    #[test]
    fn charge_envelope_uses_message_field() {
        // charge 系错误文案在 message（实测 401 的 message 可能是空串或 ","）
        let ok = parse_envelope(
            Envelope::Charge,
            200,
            r#"{"msg":"success","code":200,"feeitemList":[{"feeitemid":450}]}"#,
        )
        .expect("应成功");
        assert_eq!(ok["feeitemList"][0]["feeitemid"].as_i64(), Some(450));
        let e = parse_envelope(Envelope::Charge, 200, r#"{"code":401,"message":","}"#)
            .expect_err("应报 NotLogin");
        assert!(matches!(e, CampusSynjonesError::NotLogin), "实际 {e:?}");
        let e = parse_envelope(Envelope::Charge, 200, r#"{"code":400,"message":"参数错误"}"#)
            .expect_err("应报 Api");
        match e {
            CampusSynjonesError::Api { code, msg } => {
                assert_eq!(code, 400);
                assert_eq!(msg, "参数错误");
            }
            other => panic!("实际 {other:?}"),
        }
    }

    #[test]
    fn search_envelope_and_unauthorized_codes() {
        let ok = parse_envelope(
            Envelope::Search,
            200,
            r#"{"code":200,"data":{"total":3,"records":[]},"msg":"success"}"#,
        )
        .expect("应成功");
        assert_eq!(ok["data"]["total"].as_i64(), Some(3));
        // 白名单 code（4011/4037/4038）一律 NotLogin，便于上层静默重进
        for code in [4011, 4037, 4038] {
            let body = format!(r#"{{"code":{code},"data":null,"msg":"未授权"}}"#);
            let e = parse_envelope(Envelope::Search, 200, &body).expect_err("应报错");
            assert!(matches!(e, CampusSynjonesError::NotLogin), "code={code} 实际 {e:?}");
        }
    }

    #[test]
    fn non_401_http_errors_and_bad_payload() {
        let e = parse_envelope(Envelope::Berserker, 502, "<html>bad gateway</html>")
            .expect_err("应报 Http");
        assert!(matches!(e, CampusSynjonesError::Http(_)), "实际 {e:?}");
        assert!(e.to_string().contains("502"));
        // 200 但非 JSON → Parse；缺 code → Parse
        let e = parse_envelope(Envelope::Berserker, 200, "not json").expect_err("应报 Parse");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");
        let e = parse_envelope(Envelope::Berserker, 200, r#"{"foo":1}"#).expect_err("应报 Parse");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");
    }

    /// HTTP 非 401 的 401 之外的 4xx 也算 Http（不只 401 走 NotLogin）。
    #[test]
    fn http_401_without_body_is_not_login() {
        let e = parse_envelope(Envelope::Berserker, 401, "").expect_err("应报 NotLogin");
        assert!(matches!(e, CampusSynjonesError::NotLogin), "实际 {e:?}");
    }

    /// 走真实请求路径的 401：桩回 401 → NotLogin；无 TGT 时不重进（不发第二条请求）。
    #[tokio::test]
    async fn stub_401_maps_to_not_login_without_reentry() {
        let (base, rx) = stub(http_status(401, r#"{"code":4030,"message":"未授权，请先授权"}"#));
        let c = client_with_base(&base, "T");
        let e = c
            .get("/berserker-app/ykt/tsm/queryCurrentCard", &[], Envelope::Berserker)
            .await
            .expect_err("401 应报 NotLogin");
        assert!(matches!(e, CampusSynjonesError::NotLogin), "实际 {e:?}");
        // 桩只服务一条连接：能收到请求即证明「无 TGT 时不重试」
        assert!(rx.recv().is_ok());
    }

    // ---------- M4：WebVPN 路由注入 ----------

    /// `set_webvpn` 注入的 base 覆盖对全部拼接路径生效：GET / POST form / POST JSON
    /// 三条业务路径都打到 override 指向的桩（校内默认 base 不变由既有桩测试覆盖）。
    #[tokio::test]
    async fn base_override_redirects_all_request_kinds() {
        let (base, rx) = stub(http_json(r#"{"code":200,"success":true,"data":{}}"#));
        let mut c = SynjonesClient::new(
            CasClient::new().expect("创建 CasClient 失败"),
            None,
            Some(SynjonesToken::bearer("T")),
        );
        c.set_webvpn(Some(base.clone()), None);
        assert_eq!(c.base_url(), base, "base_url 应返回 override");
        assert!(!c.is_webvpn(), "未给 vpn 会话时不算 WebVPN 模式（base-only 是测试形态）");

        c.get("/a", &[], Envelope::Berserker).await.expect("GET 应成功");
        assert!(rx.recv().expect("GET 未达桩").starts_with("GET /a"), "GET 应打 override");

        let (base2, rx2) = stub(http_json(r#"{"code":200,"message":"ok"}"#));
        c.set_webvpn(Some(base2.clone()), None);
        c.post_form("/b", &[("k", "v".to_string())], Envelope::Charge)
            .await
            .expect("POST form 应成功");
        assert!(rx2.recv().expect("POST form 未达桩").starts_with("POST /b"));

        let (base3, rx3) = stub(http_json(r#"{"code":200,"data":{}}"#));
        c.set_webvpn(Some(base3.clone()), None);
        c.post_json("/c", &[("k", "v".to_string())], Envelope::Berserker)
            .await
            .expect("POST JSON 应成功");
        assert!(rx3.recv().expect("POST JSON 未达桩").starts_with("POST /c"));
    }

    /// `set_webvpn(None, None)` 回到校内直连：base_url 回落默认值，is_webvpn 复位。
    #[test]
    fn clear_webvpn_restores_direct_base() {
        let mut c = SynjonesClient::new(
            CasClient::new().expect("创建 CasClient 失败"),
            None,
            Some(SynjonesToken::bearer("T")),
        );
        assert!(!c.is_webvpn(), "构造默认为直连");
        c.set_webvpn(Some("https://webvpn.cwxu.edu.cn/http/x".to_string()), None);
        assert_eq!(c.base_url(), "https://webvpn.cwxu.edu.cn/http/x", "override 生效");
        c.set_webvpn(None, None);
        assert!(!c.is_webvpn());
        assert_eq!(c.base_url(), BERSERKER_BASE, "清掉 override 应回落内网 base");
    }
}
