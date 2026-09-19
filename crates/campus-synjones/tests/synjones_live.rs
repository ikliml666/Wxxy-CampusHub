//! 慧新E校 lyCas SSO 桥 live 探针 + 取证固化（M3 批 1）。
//!
//! # 本测试回答三个问题（`docs/superpowers/plans/2026-09-19-m3-ecard-electricity.md` §1.6）
//!
//! 1. lyCas 桥落点的 token **怎么传**：URL query？Set-Cookie？都没有？
//! 2. 换票 service 用哪个 targetUrl 才拿得到 token（默认 `/plat/shouyeUser` /
//!    电费页 `/charge-pc/pays/450` / 裸 service 三组对照）？
//! 3. 拿到的 token 调 `GET /berserker-app/ykt/tsm/queryCurrentCard` 是否 200 且 `code==200`；
//!    顺带对照 `synAccessSource=pc` 是否复现 4030（验证 M3「必须 app」的前提）。
//!
//! # 凭据来源（按序尝试，**只读、只在内存、绝不打印**）
//!
//! 1. **本机 app 会话文件** `%APPDATA%/campushub/session.json` 的 `tgtB64`
//!    （DPAPI 密文，Tauri 层 `infra/state.rs:66-69` 写入）——最省：只用 TGT 换票，
//!    不碰密码、不走验证码、不消耗 CAS 连续错误计数。
//! 2. **本机账号库** `%APPDATA%/campushub/accounts.json` 的 `passwordB64`
//!    （`account/store.rs:13-23`）→ 完整 CAS 账密登录（验证码自动识别 ≤3 张）。
//! 3. 环境变量 `CAMPUS_HUB_CREDS` 指向的凭据文件（格式见 `campus-auth/tests/jwglxt_live.rs:22-52`）。
//!
//! DPAPI 解密：测试内自带裸 FFI（`CryptUnprotectData`，拷自 `account/crypto.rs:99-116` 的思路），零新依赖。
//!
//! # 纪律
//!
//! - token / ticket / cookie 值 / 卡号一律**打码或不打印**；只打印 host、path、query **键名**、cookie **名**。
//! - 样本写仓库外 recon 目录（`CAMPUS_HUB_RECON_DIR`，默认 `%TEMP%/campushub-m3-recon`），
//!   且写盘前经 [`redact_json`] 脱敏（account/name/token 类键与长数字串一律 `***`）。
//!
//! 复跑：`cargo test -p campus-synjones -- --ignored synjones_sso_live --nocapture`

use campus_auth::captcha::{solve, KaptchaTemplates};
use campus_auth::cas::CasClient;
use campus_auth::rsa::rsa_encrypt_hex;
use campus_synjones::client::Envelope;
use campus_synjones::ecard::{fetch_current_card, fetch_transactions, EP_TURNOVER};
use campus_synjones::sso::{default_target_url, ly_cas_service_url, ly_cas_redirect_url, sso_token};
use campus_synjones::{SynjonesClient, SynjonesToken, BERSERKER_BASE};
use serde_json::Value;
use std::path::{Path, PathBuf};

// ==================== recon 目录 ====================

fn recon_dir() -> PathBuf {
    std::env::var("CAMPUS_HUB_RECON_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("campushub-m3-recon"))
}

// ==================== 凭据（只读、只在内存） ====================

fn appdata_campushub() -> Option<PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .map(|d| PathBuf::from(d).join("campushub"))
}

/// DPAPI 解密（Windows CurrentUser 作用域）。非 Windows 恒 Err。
#[cfg(target_os = "windows")]
fn dpapi_unprotect(cipher: &[u8]) -> Result<Vec<u8>, String> {
    use std::ptr;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptUnprotectData(
            data_in: *mut DataBlob,
            data_descr: *mut *mut u16,
            optional_entropy: *mut DataBlob,
            reserved: *mut std::ffi::c_void,
            prompt_struct: *mut std::ffi::c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(h_mem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    let mut input_owned = cipher.to_vec();
    let mut input = DataBlob {
        cb_data: input_owned.len() as u32,
        pb_data: input_owned.as_mut_ptr(),
    };
    let mut output = DataBlob {
        cb_data: 0,
        pb_data: ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &mut input,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            0,
            &mut output,
        )
    };
    if ok == 0 {
        return Err("DPAPI 解密失败".to_string());
    }
    if output.pb_data.is_null() || output.cb_data == 0 {
        unsafe { LocalFree(output.pb_data as *mut std::ffi::c_void) };
        return Err("DPAPI 解密返回空数据".to_string());
    }
    let data = unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize).to_vec() };
    unsafe { LocalFree(output.pb_data as *mut std::ffi::c_void) };
    Ok(data)
}

#[cfg(not(target_os = "windows"))]
fn dpapi_unprotect(_cipher: &[u8]) -> Result<Vec<u8>, String> {
    Err("DPAPI 仅 Windows 支持".to_string())
}

fn dpapi_unprotect_b64(b64: &str) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("base64 解码失败: {e}"))?;
    let plain = dpapi_unprotect(&bytes)?;
    String::from_utf8(plain).map_err(|e| format!("UTF-8 失败: {e}"))
}

/// 来源 1：本机 app 会话文件里的 CAS TGT（DPAPI 密文）。
fn tgt_from_app_session() -> Option<String> {
    let path = appdata_campushub()?.join("session.json");
    let text = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let b64 = v.get("tgtB64")?.as_str()?;
    match dpapi_unprotect_b64(b64) {
        Ok(t) if !t.trim().is_empty() => Some(t),
        Ok(_) => {
            println!("[凭据] session.json 的 tgtB64 解密为空");
            None
        }
        Err(e) => {
            println!("[凭据] session.json 的 tgtB64 {e}");
            None
        }
    }
}

/// 来源 2：本机账号库（username + DPAPI 密码）。
fn creds_from_accounts_file() -> Option<(String, String)> {
    let path = appdata_campushub()?.join("accounts.json");
    let text = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let first = v.get("accounts")?.as_array()?.first()?.clone();
    let username = first.get("username")?.as_str()?.to_string();
    let password = dpapi_unprotect_b64(first.get("passwordB64")?.as_str()?).ok()?;
    Some((username, password))
}

/// 来源 3：`CAMPUS_HUB_CREDS` 凭据文件（解析逻辑与 jwglxt_live 同款）。
fn creds_from_env_file() -> Option<(String, String)> {
    let path = std::env::var("CAMPUS_HUB_CREDS").ok()?;
    let text = std::fs::read_to_string(&path).ok()?;

    fn field(line: &str, key: &str) -> Option<String> {
        let i = line.find(key)?;
        let rest = line[i + key.len()..]
            .trim_start_matches([':', '：'])
            .trim_start();
        let v: String = rest
            .chars()
            .take_while(|c| !matches!(c, '，' | ',' | '。' | ' ' | '\t' | '\r'))
            .collect();
        (!v.trim().is_empty()).then(|| v.trim().to_string())
    }

    let (mut user, mut pass) = (None, None);
    for line in text.lines() {
        if user.is_none() {
            user = field(line, "账号");
        }
        if pass.is_none() {
            pass = field(line, "密码");
        }
    }
    Some((user?, pass?))
}

/// CAS 账密登录（验证码自动识别 ≤3 张；识别不出换图，不消耗错误计数）。
async fn cas_login(client: &CasClient, username: &str, password: &str) -> campus_auth::cas::CasLoginOk {
    let templates = KaptchaTemplates::load();
    let password_rsa = rsa_encrypt_hex(password).expect("RSA 加密密码失败");
    for _ in 1..=3 {
        let captcha = client.kaptcha().await.expect("获取验证码失败");
        let png = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &captcha.png_base64,
        )
        .expect("验证码 base64 解码失败");
        let Some(answer) = solve(&png, &templates) else {
            continue;
        };
        return client
            .login(username, &password_rsa, &captcha.uid, &answer)
            .await
            .expect("CAS 登录请求失败（网络层）");
    }
    panic!("3 张验证码均未能识别（或登录均失败）");
}

/// 账密登录取新鲜 TGT（来源 2/3：账号库 DPAPI 密码，或 `CAMPUS_HUB_CREDS` 文件）。
/// **打印不含任何凭据值。**
async fn login_tgt_via_password() -> (String, &'static str) {
    let (creds, src) = match creds_from_accounts_file() {
        Some(c) => (c, "accounts.json(DPAPI 密码)"),
        None => (
            creds_from_env_file().expect(
                "无可用凭据：session.json 无 tgtB64、accounts.json 不可解密、CAMPUS_HUB_CREDS 未设置",
            ),
            "CAMPUS_HUB_CREDS",
        ),
    };
    let (username, password) = creds;
    println!("[凭据] 来源={src} 用户长度={}", username.len());
    let client = CasClient::new().expect("创建 CasClient 失败");
    let ok = cas_login(&client, &username, &password).await;
    println!("[凭据] 账密登录成功，TGT 长度={}", ok.tgt.len());
    (ok.tgt, src)
}

/// 换票失败的原因分类：只回**类别与长度**。
///
/// ⚠️ CAS 的错误正文会回显 TGT，故**绝不打印正文**，只做特征判定。
async fn classify_ticket_failure(cas: &CasClient, tgt: &str, service: &str) -> String {
    use campus_auth::cas::CAS_BASE;
    let resp = cas
        .http_client()
        .post(format!("{CAS_BASE}/v1/tickets/{tgt}"))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .form(&[("service", service)])
        .send()
        .await;
    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.text().await.unwrap_or_default();
            let low = body.to_ascii_lowercase();
            if low.contains("expire") {
                format!("HTTP {status}，正文含 expire（→ TGT 已过期，长度 {}）", body.len())
            } else if low.starts_with("tgt") {
                format!("HTTP {status}，正文以 TGT 开头（疑似回显票据的错误，长度 {}）", body.len())
            } else {
                format!("HTTP {status}，正文长度 {}（未识别类别）", body.len())
            }
        }
        Err(e) => format!("诊断请求失败：{e}"),
    }
}

// ==================== 逐跳追踪（Policy::none + 打码打印） ====================

/// 一跳的可打印事实（**只留 host/path/query 键名/cookie 名，绝不含值**）。
struct Hop {
    status: u16,
    host_path: String,
    query_keys: Vec<String>,
    set_cookie_names: Vec<String>,
    location_host_path: Option<String>,
}

fn no_redirect_client() -> (reqwest::Client, std::sync::Arc<reqwest::cookie::Jar>) {
    let jar = std::sync::Arc::new(reqwest::cookie::Jar::default());
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .cookie_provider(jar.clone())
        .build()
        .expect("构建探针 client 失败");
    (client, jar)
}

/// 手动跟随（≤10 跳），返回逐跳事实 + 落点完整 URL（完整 URL 只在内存用于提 token）。
async fn trace_hops(
    http: &reqwest::Client,
    start: &str,
) -> (Vec<Hop>, reqwest::Url) {
    let mut url = reqwest::Url::parse(start).expect("起始 URL 非法");
    let mut hops = Vec::new();
    let mut final_url = url.clone();
    for _ in 0..10 {
        let resp = http.get(url.clone()).send().await.expect("探针请求失败");
        let status = resp.status().as_u16();
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let set_cookie_names: Vec<String> = resp
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter_map(|s| s.split(';').next())
            .filter_map(|kv| kv.split('=').next())
            .map(|n| n.trim().to_string())
            .collect();
        let here = resp.url().clone();
        let location_url = location.as_ref().and_then(|l| here.join(l).ok());
        hops.push(Hop {
            status,
            host_path: format!(
                "{}://{}{}",
                here.scheme(),
                here.host_str().unwrap_or("?"),
                here.path()
            ),
            query_keys: here.query_pairs().map(|(k, _)| k.to_string()).collect(),
            set_cookie_names,
            location_host_path: location_url.map(|u| {
                format!(
                    "{}://{}{}",
                    u.scheme(),
                    u.host_str().unwrap_or("?"),
                    u.path()
                )
            }),
        });
        final_url = here;
        let _ = resp.bytes().await; // body 中断容忍（hyper IncompleteMessage）
        if (300..400).contains(&status) {
            if let Some(l) = location {
                match url.join(&l) {
                    Ok(u) => {
                        url = u;
                        continue;
                    }
                    Err(_) => break,
                }
            }
        }
        break;
    }
    (hops, final_url)
}

fn print_hops(label: &str, hops: &[Hop]) {
    println!("  [{label}] 共 {} 跳", hops.len());
    for (i, h) in hops.iter().enumerate() {
        println!(
            "    #{i} {} → status={} query键={:?} set-cookie名={:?} location={:?}",
            h.host_path, h.status, h.query_keys, h.set_cookie_names, h.location_host_path
        );
    }
}

/// 从落点 URL 的 query 里找 token 载体（返回 (键名, token)）——探针穷举所有候选。
fn token_from_query(url: &reqwest::Url) -> Option<(String, String)> {
    for key in ["synjones-auth", "access_token", "token"] {
        if let Some((_, v)) = url.query_pairs().find(|(k, _)| k == key) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                let tok = v
                    .split_once(' ')
                    .filter(|(a, _)| a.eq_ignore_ascii_case("bearer"))
                    .map(|(_, b)| b.trim().to_string())
                    .unwrap_or(v);
                return Some((key.to_string(), tok));
            }
        }
    }
    None
}

/// 从 cookie jar 里找 token 载体（返回 (cookie 名, 值)）——探针穷举，落点不带 query 时的可能载体。
fn token_from_jar(jar: &reqwest::cookie::Jar, url: &reqwest::Url) -> Option<(String, String)> {
    use reqwest::cookie::CookieStore as _;
    let header = jar.cookies(url)?;
    let text = header.to_str().ok()?;
    for pair in text.split(';') {
        let (name, value) = pair.trim().split_once('=')?;
        if matches!(name, "access_token" | "synjones-auth" | "token") && !value.is_empty() {
            return Some((name.to_string(), value.to_string()));
        }
    }
    None
}

// ==================== 探针主流程 ====================

/// 候选 targetUrl（§1.6 未知 #1 的对照实验）。
///
/// 实测结论：**落点是否带 `?synjones-auth=<token>` 取决于 targetUrl 指向哪个子应用**——
/// `/plat/shouyeUser`（移动端 SPA）落点是 `/plat/?name=…&ticket=…`，**不带 token**；
/// 子应用页面（campus-card-pc / charge-pc）落点才带 token，且该 token 可直接调业务接口。
fn candidates() -> Vec<(&'static str, String)> {
    vec![
        ("默认（一卡通 /campus-card-pc/）", default_target_url()),
        ("移动端首页 /plat/shouyeUser", format!("{BERSERKER_BASE}/plat/shouyeUser")),
        ("电费页 /charge-pc/pays/450", format!("{BERSERKER_BASE}/charge-pc/pays/450")),
        ("无 targetUrl（裸 service）", String::new()),
    ]
}

/// 一次桥探针：对三个候选 targetUrl 各换一次票并逐跳追踪，返回 (可用 token 列表, 取证报告)。
async fn probe_bridge(
    cas: &CasClient,
    http: &reqwest::Client,
    jar: &reqwest::cookie::Jar,
    tgt: &str,
) -> (Vec<CandidateToken>, Value) {
    let mut found = Vec::new();
    let mut report = serde_json::Map::new();

    for (label, target) in candidates() {
        println!("\n===== 候选：{label} =====");
        let service = ly_cas_service_url(&target);
        // 换票：ST 绑定 service（用 CasClient 的公开 API，与产品实现同一路径）
        let st = match cas.sso_ticket(tgt, &service).await {
            Ok(st) => st,
            Err(e) => {
                // 失败时补一次分类诊断（只回类别，不回正文——CAS 错误正文会回显 TGT）
                let diag = classify_ticket_failure(cas, tgt, &service).await;
                println!("  [换票] 失败：{e} → 诊断：{diag}");
                report.insert(
                    label.to_string(),
                    serde_json::json!({"service": service, "ticket": "failed", "diag": diag}),
                );
                continue;
            }
        };
        println!("  [换票] 成功（ST 长度 {}）", st.len());
        let mut start = reqwest::Url::parse(&service).expect("service 恒合法");
        start.query_pairs_mut().append_pair("ticket", &st);
        let (hops, landing) = trace_hops(http, start.as_str()).await;
        print_hops(label, &hops);
        let carrier_query = token_from_query(&landing);
        let carrier_jar = token_from_jar(jar, &landing);
        println!(
            "  [落点] host={:?} path={} query键={:?}",
            landing.host_str(),
            landing.path(),
            landing.query_pairs().map(|(k, _)| k.to_string()).collect::<Vec<_>>()
        );
        println!(
            "  [token 载体] query={:?} cookie={:?}",
            carrier_query.as_ref().map(|(k, _)| k.as_str()),
            carrier_jar.as_ref().map(|(k, _)| k.as_str())
        );
        // 关键：拿到 token **立即**验证，避免「多候选先后签发导致旧 token 失效」的顺序混淆
        let mut entry = serde_json::json!({
            "service": service,
            "hops": hops.iter().map(|h| serde_json::json!({
                "status": h.status, "hostPath": h.host_path,
                "queryKeys": h.query_keys, "setCookieNames": h.set_cookie_names,
                "location": h.location_host_path,
            })).collect::<Vec<_>>(),
            "landing": {
                "host": landing.host_str(),
                "path": landing.path(),
                "queryKeys": landing.query_pairs().map(|(k, _)| k.to_string()).collect::<Vec<_>>(),
            },
            "tokenCarrier": {
                "queryKey": carrier_query.as_ref().map(|(k, _)| k.clone()),
                "cookieName": carrier_jar.as_ref().map(|(k, _)| k.clone()),
            },
        });
        if let Some((key, tok)) = carrier_query.or(carrier_jar) {
            let token = SynjonesToken::bearer(tok);
            let (ok, detail) = verify_token(&token).await;
            println!("  [验证 queryCurrentCard] ok={ok} {detail}");
            entry["verify"] = serde_json::json!({"ok": ok, "detail": detail});
            found.push(CandidateToken {
                label: label.to_string(),
                carrier: key,
                verified: ok,
            });
        }
        report.insert(label.to_string(), entry);
    }

    (found, Value::Object(report))
}

/// 用候选 token 打一次业务接口（`SynjonesClient` 不带 TGT → 不做静默重进，纯测 token 本身）。
async fn verify_token(token: &SynjonesToken) -> (bool, String) {
    let client = SynjonesClient::new(
        CasClient::new().expect("新建 CasClient 失败"),
        None,
        Some(token.clone()),
    );
    match client
        .get("/berserker-app/ykt/tsm/queryCurrentCard", &[], Envelope::Berserker)
        .await
    {
        Ok(v) => (true, format!("code={} msg={}", v["code"], v["msg"])),
        Err(e) => (false, e.to_string()),
    }
}

/// 一个候选 targetUrl 的换票结果（含该 token 是否通过业务接口验证）。
///
/// 不保留 token 本体（验证在签发后立即完成）——避免「后签发顶掉先签发」的误用，
/// 也避免 token 在探针里被长期持有。
struct CandidateToken {
    label: String,
    carrier: String,
    verified: bool,
}

#[tokio::test]
#[ignore = "live：需校园网 + 本机会话/凭据，仅主智能体验收时 -- --ignored 运行"]
async fn synjones_sso_live() {
    let dir = recon_dir();
    std::fs::create_dir_all(&dir).expect("创建 recon 目录失败");
    println!("[recon] 样本目录: {}", dir.display());

    // 浏览器入口（人用的那个）也看一眼：确认它 302 到 CAS 且 service 指向 lyCas login 端点
    let (probe_http, probe_jar) = no_redirect_client();
    let (entry_hops, _) = trace_hops(&probe_http, &ly_cas_redirect_url(&default_target_url())).await;
    print_hops("浏览器入口 cas/redirect/lyCas", &entry_hops);

    let cas = CasClient::new().expect("创建 CasClient 失败");
    let mut report = serde_json::Map::new();
    let mut found: Vec<CandidateToken> = Vec::new();
    let mut used_tgt: Option<(String, &'static str)> = None;

    // 来源 1（最省）：本机 app 会话文件里的 TGT —— 免密码、免验证码、不消耗 CAS 错误计数
    if let Some(tgt) = tgt_from_app_session() {
        println!("\n[凭据] 尝试来源=session.json(DPAPI TGT) 长度={}", tgt.len());
        let (f, r) = probe_bridge(&cas, &probe_http, &probe_jar, &tgt).await;
        report.insert("source=session.json".to_string(), r);
        if f.iter().any(|c| c.verified) {
            used_tgt = Some((tgt.clone(), "session.json"));
        }
        found = f;
    } else {
        println!("\n[凭据] session.json 无可用 TGT");
    }

    // 来源 2/3（回落）：账密登录取新鲜 TGT
    if used_tgt.is_none() {
        println!("\n[凭据] 本机 TGT 未换来可用 token → 回落账密登录");
        let (tgt, src) = login_tgt_via_password().await;
        let (f, r) = probe_bridge(&cas, &probe_http, &probe_jar, &tgt).await;
        report.insert(format!("source={src}"), r);
        if f.iter().any(|c| c.verified) {
            used_tgt = Some((tgt.clone(), src));
        }
        found = f;
    }

    std::fs::write(
        dir.join("sso_bridge_trace.json"),
        serde_json::to_string_pretty(&Value::Object(report)).unwrap_or_default(),
    )
    .ok();

    assert!(
        !found.is_empty(),
        "全部来源 × 全部候选 targetUrl 均未从落点取到 token——桥结构与 bundle 推断不符，需人工裁决（见 recon 目录 sso_bridge_trace.json）"
    );

    // 候选对照结果（每个 token 在签发后**立即**验证，无「后签发顶掉先签发」的顺序混淆）
    for c in &found {
        println!(
            "[候选结果] {} 载体={} 验证通过={}",
            c.label, c.carrier, c.verified
        );
    }
    assert!(
        found.iter().any(|c| c.verified),
        "落点 token 全部无法通过业务接口验证（未返回 code==200）——见 recon 目录 sso_bridge_trace.json"
    );
    let (tgt_used, tgt_src) = used_tgt.expect("有可用 token 却未记录来源 TGT（探针逻辑错误）");

    // ===== 产品路径端到端 =====
    // 用**产品实现**的 sso_token + 产品默认 targetUrl 再走一次桥；后续取证全用这个 token。
    // 实测：同一账号同一时刻只有**最新签发**的 token 有效（旧 token 被后续 SSO 顶掉），
    // 故此处先取产品 token，再做全部下游调用（中途不再 SSO）。
    println!(
        "\n[产品路径] sso_token(tgt 来源={tgt_src}, targetUrl={})",
        default_target_url()
    );
    let product_token = sso_token(&cas, &tgt_used, &default_target_url())
        .await
        .expect("产品 sso_token 应能从默认 targetUrl 换到 token");
    let (ok, detail) = verify_token(&product_token).await;
    println!("[产品路径] queryCurrentCard ok={ok} {detail}");
    assert!(ok, "产品 sso_token 得到的 token 必须能调通业务接口");
    let token = product_token;
    println!("✅ 可用组合：产品 sso_token + 默认 targetUrl「{}」", default_target_url());

    // 对照：synAccessSource=pc 是否复现 4030（验证「必须 app」的既有结论）
    let probe = reqwest::Client::new();
    for source in ["app", "pc"] {
        let url = format!(
            "{BERSERKER_BASE}/berserker-app/ykt/tsm/queryCurrentCard?synAccessSource={source}"
        );
        let resp = probe
            .get(&url)
            .header("synjones-auth", token.auth_value())
            .header("synAccessSource", source)
            .send()
            .await;
        match resp {
            Ok(r) => {
                let status = r.status().as_u16();
                let body = r.text().await.unwrap_or_default();
                let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                println!(
                    "[对照 synAccessSource={source}] HTTP {status} code={} msg={}",
                    v.get("code").unwrap_or(&Value::Null),
                    v.get("msg").and_then(Value::as_str).unwrap_or("")
                );
                if source == "app" {
                    assert_eq!(status, 200, "synAccessSource=app 应 HTTP 200");
                    assert_eq!(v.get("code").and_then(Value::as_i64), Some(200), "code 应为 200");
                }
            }
            Err(e) => println!("[对照 synAccessSource={source}] 请求失败：{e}"),
        }
    }

    // 固化断言：拿到的卡列表可解析出余额字段（值为分，具体数字不写进断言）
    let client = SynjonesClient::new(CasClient::new().expect("新建 client 失败"), None, Some(token));
    let v = client
        .get("/berserker-app/ykt/tsm/queryCurrentCard", &[], Envelope::Berserker)
        .await
        .expect("queryCurrentCard 应成功");
    let cards = v["data"]["card"].as_array().cloned().unwrap_or_default();
    println!("[断言] data.card 条数={}", cards.len());
    assert!(!cards.is_empty(), "data.card 应非空（拿到真实卡信息）");
    let c0 = &cards[0];
    println!(
        "[断言] 首卡字段: {:?}",
        c0.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default()
    );
    for key in ["db_balance", "unsettle_amount", "elec_accamt", "acc_status"] {
        println!("[断言] 字段 {key} 存在={}", c0.get(key).is_some());
    }
    assert!(
        c0.get("db_balance").is_some(),
        "首卡应含 db_balance（余额字段，单位分）"
    );
    dump(&dir, "query_current_card.json", &v);

    // ---------- 追加取证（计划 §1.4/§1.6 的未确认项，为批 2/3 固化字段名） ----------
    let account = c0
        .get("account")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // 卡明细：官方充值页的调用形态（`queryCard?scene=recharge`，无需卡号）
    match client
        .get(
            "/berserker-app/ykt/tsm/queryCard",
            &[("scene", "recharge")],
            Envelope::Berserker,
        )
        .await
    {
        Ok(v) => dump(&dir, "query_card_recharge.json", &v),
        Err(e) => println!("[queryCard?scene=recharge] 失败：{e}"),
    }

    // 学校账户（计划标注「待实测字段」）
    match client
        .get(
            "/berserker-app/ykt/tsm/getSchoolAccountinfo",
            &[],
            Envelope::Berserker,
        )
        .await
    {
        Ok(v) => dump(&dir, "school_account.json", &v),
        Err(e) => println!("[getSchoolAccountinfo] 失败：{e}"),
    }

    // 一卡通流水：`type` 语义对照（1/2/3），并把记录字段名固化
    let mut turnover_ok = false;
    if !account.is_empty() {
        for t in ["1", "2", "3"] {
            let params = [
                ("size", "3"),
                ("current", "1"),
                ("account", account.as_str()),
                ("type", t),
            ];
            match client
                .get(
                    "/berserker-search/search/personal/turnover",
                    &params,
                    Envelope::Search,
                )
                .await
            {
                Ok(v) => {
                    turnover_ok = true;
                    let recs = v["data"]["records"].as_array().cloned().unwrap_or_default();
                    println!(
                        "[流水 type={t}] total={} 返回条数={} 首条字段={:?}",
                        v["data"]["total"],
                        recs.len(),
                        recs.first()
                            .and_then(|r| r.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()))
                            .unwrap_or_default()
                    );
                    dump(&dir, &format!("turnover_type{t}.json"), &v);
                }
                Err(e) => println!("[流水 type={t}] 失败：{e}"),
            }
        }
    } else {
        println!("[流水] 跳过：响应未给 account（无法构造查询参数）");
    }
    assert!(
        turnover_ok,
        "一卡通流水端点应至少一种 type 返回 code==200（批 2 的取数前提）"
    );
}

/// 脱敏后写 recon 样本 + 打印（样本目录在仓库外，且已过 [`redact_json`]）。
fn dump(dir: &std::path::Path, name: &str, v: &Value) {
    let redacted = redact_json(v);
    let text = serde_json::to_string_pretty(&redacted).unwrap_or_default();
    println!("----- {name} -----\n{text}\n----- /{name} -----");
    std::fs::write(dir.join(name), text).ok();
}

// ==================== 对照实验（默认不跑，需用户在场） ====================

/// oauth 账密兜底对照（**产品不接入**，见计划 §2.2）。
///
/// 需用户在慧新E校的密码（可能 ≠ CAS 密码），从环境变量读取，不落盘不打印：
/// `SYNJONES_USERNAME` / `SYNJONES_PASSWORD`。
/// 跑法：`SYNJONES_USERNAME=… SYNJONES_PASSWORD=… cargo test -p campus-synjones -- --ignored synjones_oauth_password_experiment --nocapture`
#[tokio::test]
#[ignore = "对照实验：需用户在慧新E校的密码，用户在场时才跑"]
async fn synjones_oauth_password_experiment() {
    let (Ok(username), Ok(password)) = (
        std::env::var("SYNJONES_USERNAME"),
        std::env::var("SYNJONES_PASSWORD"),
    ) else {
        println!("[oauth 对照] 跳过：未设置 SYNJONES_USERNAME / SYNJONES_PASSWORD");
        return;
    };
    // Basic = mobile_service_platform:mobile_service_platform_secret 的 base64（bundle 逐字实测）
    const BASIC: &str = "Basic bW9iaWxlX3NlcnZpY2VfcGxhdGZvcm06bW9iaWxlX3NlcnZpY2VfcGxhdGZvcm1fc2VjcmV0";
    let form = [
        ("username", username),
        ("password", password),
        ("grant_type", "password".to_string()),
        ("scope", "all".to_string()),
        ("loginFrom", "pc".to_string()),
        ("logintype", "username".to_string()),
        ("synAccessSource", "app".to_string()),
    ];
    let resp = reqwest::Client::new()
        .post(format!("{BERSERKER_BASE}/berserker-auth/oauth/token"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Authorization", BASIC)
        .form(&form)
        .send()
        .await
        .expect("oauth/token 请求失败");
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    println!(
        "[oauth 对照] HTTP {status} code={} 有 access_token={} token_type={:?}",
        v.get("code").unwrap_or(&Value::Null),
        v.get("access_token").is_some(),
        v.get("token_type")
    );
}

// ==================== M3 批 2：一卡通取数 live 探针 ====================

/// 批 2 live 探针：余额口径复核 + 流水解析 + 「今日/本月消费」参数实测（计划 §1.7 未知 3）。
///
/// 复跑：`cargo test -p campus-synjones -- --ignored synjones_ecard_live --nocapture`
#[tokio::test]
#[ignore = "live：需校园网 + 本机会话/凭据，仅主智能体验收时 -- --ignored 运行"]
async fn synjones_ecard_live() {
    let dir = recon_dir();
    std::fs::create_dir_all(&dir).expect("创建 recon 目录失败");
    println!("[recon] 样本目录: {}", dir.display());

    let cas = CasClient::new().expect("创建 CasClient 失败");
    // 凭据来源 1（最省）：本机 app 会话的 TGT；换票失败（TGT 隔夜过期）→ 回落账密登录
    let mut token = None;
    if let Some(t) = tgt_from_app_session() {
        println!("[凭据] 来源=session.json(DPAPI TGT) 长度={}", t.len());
        match sso_token(&cas, &t, &default_target_url()).await {
            Ok(tok) => token = Some(tok),
            Err(e) => println!("[凭据] 本机 TGT 换票失败（{e}）→ 回落账密登录"),
        }
    } else {
        println!("[凭据] session.json 无可用 TGT");
    }
    let token = match token {
        Some(t) => t,
        None => {
            let (tgt, _src) = login_tgt_via_password().await;
            sso_token(&cas, &tgt, &default_target_url())
                .await
                .expect("账密登录取得的新 TGT 应能换到 token")
        }
    };
    // 业务请求只认 token 头（无 TGT → 不触发静默重进，纯测本次取数）
    let client = SynjonesClient::new(CasClient::new().expect("新建 client 失败"), None, Some(token));

    // ---------- 1) 当前卡 ----------
    let card = fetch_current_card(&client)
        .await
        .expect("queryCurrentCard 应可解析出首卡");
    println!(
        "[卡] cardname 长度={} account 长度={} 状态={} 卡账户={} 元 电子账户={} 元",
        card.cardname.chars().count(),
        card.account.chars().count(),
        card.status_label,
        card.balance_yuan,
        card.elec_accamt_yuan
    );
    assert!(
        card.account.chars().count() > 3,
        "卡号应非空（后续流水查询要用它）"
    );

    // ---------- 2) 流水（支出方向，批 1 实测 total=1006） ----------
    let page = fetch_transactions(&client, &card.account, 1, 5, "2")
        .await
        .expect("turnover?type=2 应成功");
    println!("[流水 type=2] total={} 本页条数={}", page.total, page.records.len());
    assert!(!page.records.is_empty(), "支出方向流水应有记录");
    if let Some(first) = page.records.first() {
        println!(
            "[流水 首条] time={:?} 摘要长度={} 金额={} 元 收入={} 交易方长度={} 地点长度={}",
            first.time,
            first.summary.chars().count(),
            first.amount_yuan,
            first.is_income,
            first.pay_name.chars().count(),
            first.location_name.chars().count()
        );
        assert!(!first.is_income, "type=2 方向下首条应为支出（负号）");
        assert!(first.amount_yuan < 0.0, "支出金额应为负：{}", first.amount_yuan);
    }

    // 收支两个方向都取一页（收入方向 total 批 1 实测 36）
    if let Ok(income) = fetch_transactions(&client, &card.account, 1, 3, "1").await {
        println!("[流水 type=1] total={} 本页条数={}", income.total, income.records.len());
    }
    // 不带 type = 全部？决定钱包页流水列表要不要过滤方向
    match client
        .get(
            EP_TURNOVER,
            &[("account", &card.account), ("current", "1"), ("size", "3")],
            Envelope::Search,
        )
        .await
    {
        Ok(v) => println!(
            "[流水 不带 type] code=200 total={} 条数={}（> 支出 total 即为全量）",
            v["data"]["total"],
            v["data"]["records"].as_array().map(Vec::len).unwrap_or(0)
        ),
        Err(e) => println!("[流水 不带 type] 失败：{e}"),
    }

    // ---------- 3) 口径复核：最新一条流水的 cardBalance == elec_accamt ----------
    if let Ok(v) = client
        .get(
            EP_TURNOVER,
            &[
                ("account", &card.account),
                ("current", "1"),
                ("size", "1"),
                ("type", "2"),
            ],
            Envelope::Search,
        )
        .await
    {
        let raw = &v["data"]["records"][0]["cardBalance"];
        println!("[口径复核] 最新流水 cardBalance 原值={raw:?}（类型 {:?}）", raw);
        let snap = raw
            .as_str()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .or_else(|| raw.as_f64());
        if let Some(fen) = snap {
            println!("  → 换算 {} 元 ↔ 电子账户 {} 元（应相等）", fen / 100.0, card.elec_accamt_yuan);
            assert_eq!(
                fen / 100.0,
                card.elec_accamt_yuan,
                "电子账户余额应等于最新流水的交易后余额快照（批 1 口径）"
            );
        } else {
            println!("  ⚠️ cardBalance 缺失或非数值——口径复核未做（见流水样本字段）");
        }
    }

    // ---------- 4) 「今日/本月消费」端点参数实测 ----------
    // 日期从最新流水的 jndatetimeStr 取（不引 chrono，零新依赖）
    let stamp = page
        .records
        .first()
        .map(|r| r.time.clone())
        .unwrap_or_default();
    let today = stamp.chars().take(10).collect::<String>();
    let month_start = if today.len() >= 7 {
        format!("{}-01", &today[..7])
    } else {
        String::new()
    };
    println!("[统计探针] 以最新流水日期为基准：today={today:?} month_start={month_start:?}");

    let acct = card.account.as_str();
    let sum_variants: Vec<(&str, Vec<(&str, String)>)> = vec![
        ("无参", vec![]),
        ("account", vec![("account", acct.to_string())]),
        (
            "account+type2",
            vec![("account", acct.to_string()), ("type", "2".to_string())],
        ),
        (
            "account+beginTime/endTime=今天",
            vec![
                ("account", acct.to_string()),
                ("beginTime", format!("{today} 00:00:00")),
                ("endTime", format!("{today} 23:59:59")),
            ],
        ),
        (
            "account+beginTime/endTime=本月",
            vec![
                ("account", acct.to_string()),
                ("beginTime", format!("{month_start} 00:00:00")),
                ("endTime", format!("{today} 23:59:59")),
            ],
        ),
        (
            "account+startTime/endTime=今天",
            vec![
                ("account", acct.to_string()),
                ("startTime", format!("{today} 00:00:00")),
                ("endTime", format!("{today} 23:59:59")),
            ],
        ),
        (
            "account+startDate/endDate=今天",
            vec![
                ("account", acct.to_string()),
                ("startDate", today.clone()),
                ("endDate", today.clone()),
            ],
        ),
    ];
    let sum_hit = probe_matrix(
        &dir,
        &client,
        "sum_user",
        "/berserker-search/statistics/turnover/sum/user",
        sum_variants,
    )
    .await;

    let count_variants: Vec<(&str, Vec<(&str, String)>)> = vec![
        ("account", vec![("account", acct.to_string())]),
        (
            "account+type2",
            vec![("account", acct.to_string()), ("type", "2".to_string())],
        ),
        (
            "account+三日期参",
            vec![
                ("account", acct.to_string()),
                ("dateType", "1".to_string()),
                ("dateStr", today.clone()),
                ("statisticsDateStr", today.clone()),
            ],
        ),
        (
            "account+三日期参 dateType=2",
            vec![
                ("account", acct.to_string()),
                ("dateType", "2".to_string()),
                ("dateStr", today.clone()),
                ("statisticsDateStr", today.clone()),
            ],
        ),
    ];
    let count_hit = probe_matrix(
        &dir,
        &client,
        "turnover_count",
        "/berserker-search/statistics/turnover/count",
        count_variants,
    )
    .await;

    // ---------- 5) sum/user 必填三参（第一轮实测：缺任一 → code=400 点名三字段）----------
    // 服务端索赔：`statisticsDateStr` / `dateType` / `dateStr` 均不能为空 → 组合试探语义
    let month = month_start.chars().take(7).collect::<String>();
    let daily = |dt: &str| -> Vec<(&str, String)> {
        vec![
            ("account", acct.to_string()),
            ("dateType", dt.to_string()),
            ("dateStr", today.clone()),
            ("statisticsDateStr", today.clone()),
        ]
    };
    let sum_variants2: Vec<(&str, Vec<(&str, String)>)> = vec![
        ("dateType=1 今天", daily("1")),
        ("dateType=2 今天", daily("2")),
        ("dateType=3 今天", daily("3")),
        ("dateType=0 今天", daily("0")),
        ("dateType=day 今天", daily("day")),
        ("dateType=month 今天", daily("month")),
        (
            "dateType=2 dateStr=本月首日 stat=今天",
            vec![
                ("account", acct.to_string()),
                ("dateType", "2".to_string()),
                ("dateStr", month_start.clone()),
                ("statisticsDateStr", today.clone()),
            ],
        ),
        (
            "dateType=2 dateStr=今天 stat=本月首日",
            vec![
                ("account", acct.to_string()),
                ("dateType", "2".to_string()),
                ("dateStr", today.clone()),
                ("statisticsDateStr", month_start.clone()),
            ],
        ),
        (
            "dateType=2 dateStr=月份 stat=月份",
            vec![
                ("account", acct.to_string()),
                ("dateType", "2".to_string()),
                ("dateStr", month.clone()),
                ("statisticsDateStr", month.clone()),
            ],
        ),
    ];
    let sum_hit2 = probe_matrix(
        &dir,
        &client,
        "sum_user2",
        "/berserker-search/statistics/turnover/sum/user",
        sum_variants2,
    )
    .await;

    // ---------- 6) sum/user 第三轮：日期**格式**试探（第二轮三参齐备仍 400「业务异常」）----------
    let sum_variants3: Vec<(&str, Vec<(&str, String)>)> = vec![
        (
            "dateType=1 yyyyMMdd",
            vec![
                ("account", acct.to_string()),
                ("dateType", "1".to_string()),
                ("dateStr", "20260918".to_string()),
                ("statisticsDateStr", "20260918".to_string()),
            ],
        ),
        (
            "dateType=2 yyyyMMdd",
            vec![
                ("account", acct.to_string()),
                ("dateType", "2".to_string()),
                ("dateStr", "20260918".to_string()),
                ("statisticsDateStr", "20260918".to_string()),
            ],
        ),
        (
            "dateType=1 ISO+0000",
            vec![
                ("account", acct.to_string()),
                ("dateType", "1".to_string()),
                ("dateStr", "2026-09-18T00:00:00.000+0000".to_string()),
                ("statisticsDateStr", "2026-09-18T00:00:00.000+0000".to_string()),
            ],
        ),
        (
            "dateType=2 yyyyMM",
            vec![
                ("account", acct.to_string()),
                ("dateType", "2".to_string()),
                ("dateStr", "202609".to_string()),
                ("statisticsDateStr", "202609".to_string()),
            ],
        ),
        (
            "dateType=1 三参 + size",
            vec![
                ("account", acct.to_string()),
                ("dateType", "1".to_string()),
                ("dateStr", today.clone()),
                ("statisticsDateStr", today.clone()),
                ("size", "10".to_string()),
            ],
        ),
        (
            "dateType=1 三参 + type=2",
            vec![
                ("account", acct.to_string()),
                ("dateType", "1".to_string()),
                ("dateStr", today.clone()),
                ("statisticsDateStr", today.clone()),
                ("type", "2".to_string()),
            ],
        ),
    ];
    let sum_hit3 = probe_matrix(
        &dir,
        &client,
        "sum_user3",
        "/berserker-search/statistics/turnover/sum/user",
        sum_variants3,
    )
    .await;

    // ---------- 7) sum/user 第四轮：第三轮实测「+ type 才不再 400」，固化为日/月两种取值 ----------
    let daily2 = |dt: &str, dstr: &str, stat: &str, ty: &str| -> Vec<(&str, String)> {
        vec![
            ("account", acct.to_string()),
            ("dateType", dt.to_string()),
            ("dateStr", dstr.to_string()),
            ("statisticsDateStr", stat.to_string()),
            ("type", ty.to_string()),
        ]
    };
    let sum_variants4: Vec<(&str, Vec<(&str, String)>)> = vec![
        ("日 type=2（有流水的那天）", daily2("1", &today, &today, "2")),
        ("日 type=1（有流水的那天）", daily2("1", &today, &today, "1")),
        ("月 type=2 首日→今天", daily2("2", &month_start, &today, "2")),
        ("月 type=2 首日→首日", daily2("2", &month_start, &month_start, "2")),
        ("dateType=3 type=2 首日→今天", daily2("3", &month_start, &today, "2")),
        ("dateType=0 type=2 首日→今天", daily2("0", &month_start, &today, "2")),
    ];
    let sum_hit4 = probe_matrix(
        &dir,
        &client,
        "sum_user4",
        "/berserker-search/statistics/turnover/sum/user",
        sum_variants4,
    )
    .await;

    println!(
        "[统计探针结论] sum/user 第一轮={sum_hit} 第二轮={sum_hit2} 第三轮={sum_hit3} 第四轮={sum_hit4} count 可用={count_hit}"
    );

    // 交易类型字典（计划 §1.4 待实测项，顺手固化）
    probe_matrix(
        &dir,
        &client,
        "turnover_type",
        "/berserker-search/search/turnoverType",
        vec![("无参", vec![])],
    )
    .await;

    println!("[统计探针结论] sum/user 有可用变体={sum_hit} count 有可用变体={count_hit}");
    println!("✅ 批 2 取数路径（当前卡 + 流水）live 通过");
}

/// 未知端点的参数矩阵探针：逐个变体打印响应形状（已脱敏），返回是否有任一变体成功。
async fn probe_matrix(
    dir: &Path,
    client: &SynjonesClient,
    label: &str,
    path: &str,
    variants: Vec<(&str, Vec<(&str, String)>)>,
) -> bool {
    let mut hit = false;
    for (name, owned) in variants {
        let refs: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
        match client.get(path, &refs, Envelope::Search).await {
            Ok(v) => {
                println!(
                    "[{label} / {name}] OK data 形状: {}",
                    shape_of(&v["data"])
                );
                let safe = name.replace(['/', ' ', '=', '+'], "_");
                dump_short(dir, &format!("{label}_{safe}.json"), &v, 1200);
                hit = true;
            }
            Err(e) => println!("[{label} / {name}] 失败：{e}"),
        }
    }
    hit
}

/// 响应形状摘要：对象给键名、数组给长度 + 首元素形状（未固化端点的字段发现用）。
fn shape_of(v: &Value) -> String {
    match v {
        Value::Object(o) => format!(
            "object keys=[{}]",
            o.keys().cloned().collect::<Vec<_>>().join(",")
        ),
        Value::Array(a) => format!(
            "array(len={}) 首元素: {}",
            a.len(),
            a.first().map(shape_of).unwrap_or_else(|| "无".to_string())
        ),
        Value::String(s) => format!("string(len={})", s.len()),
        Value::Number(n) => format!("number({n})"),
        Value::Bool(b) => format!("bool({b})"),
        Value::Null => "null".to_string(),
    }
}

/// 脱敏后写样本 + 打印（截断按**字符**，不按字节——防切出非法 UTF-8）。
fn dump_short(dir: &Path, name: &str, v: &Value, max_chars: usize) {
    let text = serde_json::to_string_pretty(&redact_json(v)).unwrap_or_default();
    let head: String = text.chars().take(max_chars).collect();
    println!("----- {name} -----\n{head}\n----- /{name} -----");
    std::fs::write(dir.join(name), text).ok();
}

// ==================== 脱敏（写盘/打印样本前一律过一遍） ====================

/// 键名命中即整值打码（账号/卡号/姓名/token 类）。
fn is_sensitive_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    [
        "account", "cardno", "card_num", "yktcard", "sno", "username", "loginname", "realname",
        "idcard", "phone", "mobile", "bankcard", "openid", "nickname", "avatar", "email", "token",
        "name", "password", "secret", "mobilephone", "idno", "auth", "synjones", "cookie", "ticket",
    ]
    .iter()
    .any(|s| k.contains(s))
}

/// 长数字串（≥8 位）一律打码（卡号/学号兜底规则，防键名漏网）。
fn looks_like_id(s: &str) -> bool {
    let digits = s.chars().filter(|c| c.is_ascii_digit()).count();
    digits >= 8 && digits == s.trim().len()
}

/// 递归脱敏：敏感键 → `***`；长数字串 → `***`；其余保留（余额、状态等需要看真值）。
fn redact_json(v: &Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, val)| {
                    if is_sensitive_key(k) {
                        (k.clone(), Value::String("***".to_string()))
                    } else {
                        (k.clone(), redact_json(val))
                    }
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(redact_json).collect()),
        Value::String(s) if looks_like_id(s) => Value::String("***".to_string()),
        other => other.clone(),
    }
}

/// 单测：脱敏规则本身（离线可跑，保护「样本不含 PII」这条纪律）。
#[test]
fn redact_masks_identifiers_but_keeps_balances() {
    let raw: Value = serde_json::json!({
        "code": 200,
        "data": {
            "retcode": "0",
            "card": [{
                "account": "20230001",
                "cardname": "校园卡",
                "db_balance": "1751",
                "unsettle_amount": "0",
                "elec_accamt": "2500",
                "acc_status": 0,
                "lostflag": 0,
                "accinfo": [{"name": "电子账户", "type": "1", "balance": "1234"}],
            }],
            "synjones-auth": "SECRET",
            "unknownId": "12345678",
        }
    });
    let out = redact_json(&raw);
    let text = serde_json::to_string(&out).unwrap();
    assert!(!text.contains("20230001"), "卡号必须打码：{text}");
    assert!(!text.contains("SECRET"), "token 必须打码：{text}");
    assert!(!text.contains("12345678"), "长数字串必须打码：{text}");
    assert!(!text.contains("电子账户"), "accinfo.name 打码（键名含 name）：{text}");
    // 余额/状态等数值保留（取证需要真值，且它们不是身份标识）
    assert!(text.contains("1751"), "db_balance 应保留：{text}");
    assert!(text.contains("2500"), "elec_accamt 应保留：{text}");
    assert!(text.contains("1234"), "accinfo.balance 应保留：{text}");
    assert_eq!(out["data"]["card"][0]["acc_status"], Value::from(0));
}

/// 单测：候选列表（离线）——默认必须是实测可用的子应用页，而非 `/plat/shouyeUser`。
#[test]
fn candidates_cover_variants() {
    let c = candidates();
    assert_eq!(c.len(), 4);
    assert_eq!(c[0].1, default_target_url());
    assert!(
        c[0].1.ends_with("/campus-card-pc/"),
        "默认 targetUrl 必须是实测带 token 的一卡通子应用页，实际 {}",
        c[0].1
    );
    assert!(c[1].1.ends_with("/plat/shouyeUser"), "保留负对照（实测不带 token）");
    assert!(c[2].1.ends_with("/charge-pc/pays/450"));
    assert!(c[3].1.is_empty(), "最后一个候选是裸 service（无 targetUrl）");
}
