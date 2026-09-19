//! 门户鉴权头 live 诊断探针（**只读**：只发请求、只打印结构，不改任何产品代码）。
//!
//! 目标故障：客户端资讯栏目等门户数据获取失败，文案
//! 「获取登录信息失败: 响应解析失败: tryLoginUserInfo 缺少 userName」。
//!
//! 本探针判别三种响应形态（都不打印任何真实值）：
//! 1. 返回登录页 HTML（会话已失效）；
//! 2. 是 JSON 但 `data` / `data.userName` 缺失或为 null（响应结构变化或账号角色差异）；
//! 3. `meta.code` / `meta.message` 说了什么。
//!
//! 对照组（同一进程内，判别「会话失效」还是「结构变化」）：
//! - `portal_probe()`（`customsid` cookie + 首页是否被弹回 CAS）；
//! - `GET /api/upp/userControl/getLoginInfo`（另一条依赖门户会话的接口）；
//! - 匿名 `POST /tryLoginUserInfo`（干净 jar，未登录响应形态的基线）。
//!
//! 凭据来源（按序）：① 环境变量 `CAMPUS_HUB_CREDS` 指向的凭据文件
//! （`账号:xxx` / `密码:yyy` 逐行，见 `crates/campus-auth/tests/jwglxt_live.rs`）；
//! ② `%APPDATA%/campushub/accounts.json`（`passwordB64` 为 DPAPI 密文，本文件内
//! 裸 FFI 调 `CryptUnprotectData`，与 `tauri-app/.../account/crypto.rs` 同款）。
//!
//! 敏感纪律：账号/密码/token/JWT/cookie 只在内存使用，绝不打印、绝不落盘；
//! 打码后的**结构**样本写 `%TEMP%/campushub-m3-recon/`（仓库外）。
//!
//! 运行：`cargo test -p campus-portal --test portal_diag_live -- --ignored --nocapture`

#![cfg(windows)]

use base64::Engine as _;
use campus_auth::captcha::{solve, KaptchaTemplates};
use campus_auth::cas::{CasClient, PORTAL_SERVICE, PORTAL_PROBE};
use campus_auth::rsa::rsa_encrypt_hex;
use serde_json::Value;
use std::fmt::Write as _;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(15);

// ---------------- 凭据来源 ----------------

/// `CAMPUS_HUB_CREDS` 凭据文件解析（与 cas_live.rs / jwglxt_live.rs 同款口径）。
fn creds_from_env() -> Option<(String, String)> {
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

/// `%APPDATA%/campushub/accounts.json` 首条账号的 DPAPI 解密密码。
fn creds_from_accounts() -> Option<(String, String)> {
    let dir = std::env::var("APPDATA").ok()?;
    let raw = std::fs::read_to_string(std::path::Path::new(&dir).join("campushub/accounts.json"))
        .ok()?;
    let file: Value = serde_json::from_str(&raw).ok()?;
    let acc = file.get("accounts")?.as_array()?.first()?.clone();
    let username = acc.get("username")?.as_str()?.to_string();
    let password = dpapi::unprotect(acc.get("passwordB64")?.as_str()?)?;
    Some((username, password))
}

/// 凭据：环境变量优先，其次账号库；都不通返回 None（调用方如实报告无法取证）。
fn read_creds() -> Option<(String, String)> {
    let creds = creds_from_env().or_else(creds_from_accounts);
    match &creds {
        Some((u, _)) => println!(
            "[凭据] 来源={} user={}",
            if std::env::var("CAMPUS_HUB_CREDS").is_ok() {
                "CAMPUS_HUB_CREDS"
            } else {
                "accounts.json(DPAPI)"
            },
            mask_user(u)
        ),
        None => println!("[凭据] 无来源：CAMPUS_HUB_CREDS 未设置且 accounts.json 不可解密"),
    }
    creds
}

/// 用户名打码（保留首 2 位 + 末位；与 commands/auth.rs::mask_username 同口径）。
fn mask_user(u: &str) -> String {
    let c: Vec<char> = u.chars().collect();
    match c.len() {
        0..=2 => "***".to_string(),
        n => format!("{}***{}", c[..2].iter().collect::<String>(), c[n - 1]),
    }
}

/// DPAPI 裸 FFI（CurrentUser 作用域；与 tauri-app 的 account/crypto.rs 等价）。
mod dpapi {
    use base64::Engine as _;
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

    pub fn unprotect(encrypted_base64: &str) -> Option<String> {
        let mut data = base64::engine::general_purpose::STANDARD
            .decode(encrypted_base64)
            .ok()?;
        let mut input = DataBlob {
            cb_data: data.len() as u32,
            pb_data: data.as_mut_ptr(),
        };
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: ptr::null_mut(),
        };
        let rc = unsafe {
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
        if rc == 0 || output.pb_data.is_null() || output.cb_data == 0 {
            return None;
        }
        let plain = unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize) }
            .to_vec();
        unsafe { LocalFree(output.pb_data as *mut std::ffi::c_void) };
        String::from_utf8(plain).ok()
    }
}

// ---------------- 打码的结构描述 ----------------

/// 字符串叶子只给「长度 + 形态标签」，绝不输出内容。
fn describe_str(s: &str) -> String {
    let tag = if s.is_empty() {
        "empty"
    } else if !s.is_ascii() {
        "非 ASCII 文本 ***"
    } else if s.matches('.').count() == 2 && s.len() > 60 {
        "JWT-like ***"
    } else if s.chars().all(|c| c.is_ascii_digit()) {
        "纯数字（学号类）***"
    } else if s.contains('@') {
        "email-like ***"
    } else {
        "文本 ***"
    };
    format!("string len={} {tag}", s.len())
}

/// JSON 结构树（键名 + 类型 + 长度；值一律打码）。数组只展开前 3 项。
fn describe(v: &Value, key: &str, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    match v {
        Value::Null => {
            let _ = writeln!(out, "{pad}{key}: null");
        }
        Value::Bool(b) => {
            let _ = writeln!(out, "{pad}{key}: bool={b}");
        }
        Value::Number(n) => {
            let _ = writeln!(out, "{pad}{key}: number={n}");
        }
        Value::String(s) => {
            let _ = writeln!(out, "{pad}{key}: {}", describe_str(s));
        }
        Value::Array(a) => {
            let _ = writeln!(out, "{pad}{key}: array[{}]", a.len());
            for (i, it) in a.iter().take(3).enumerate() {
                describe(it, &format!("[{i}]"), depth + 1, out);
            }
        }
        Value::Object(o) => {
            let _ = writeln!(out, "{pad}{key}: object{{{}}}", o.len());
            for (k, val) in o {
                describe(val, k, depth + 1, out);
            }
        }
    }
}

/// `meta.message` 这类服务端文案：保留中文（用户可见的提示），ASCII 字母数字串打码。
fn mask_msg(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() >= 4 {
            out.push_str("***");
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out.chars().take(200).collect()
}

/// 响应体判别结果（用于断言各假设）。
fn classify_body(status: reqwest::StatusCode, ct: &str, body: &str) -> String {
    let trimmed = body.trim_start();
    let kind = if ct.contains("text/html") || trimmed.starts_with('<') || trimmed.starts_with("<!") {
        "① 登录页 / HTML（非 JSON）"
    } else if serde_json::from_str::<Value>(body).is_ok() {
        "② JSON"
    } else if trimmed.is_empty() {
        "空 body"
    } else {
        "非 JSON 非 HTML 文本"
    };
    format!("HTTP {status} ct={ct:?} len={} → {kind}", body.len())
}

/// HTML 只报标签名与长度（不打印正文，避免页面内嵌 PII 泄漏）。
fn html_shape(body: &str) -> String {
    let mut tags: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '/')
            .collect();
        if !name.is_empty() && !tags.contains(&name) {
            tags.push(name);
        }
        if let Some(j) = rest.find('>') {
            rest = &rest[j + 1..];
        } else {
            break;
        }
        if tags.len() >= 12 {
            break;
        }
    }
    format!("标签名（前 12 个）: {tags:?}")
}

// ---------------- 探针主体 ----------------

/// CAS 登录（模板照抄 `crates/campus-auth/tests/jwglxt_live.rs::cas_login`）：
/// kaptcha → solve → login，识别不出换一张，≤3 张。
async fn cas_login(client: &CasClient, username: &str, password: &str) -> campus_auth::cas::CasLoginOk {
    let templates = KaptchaTemplates::load();
    let password_rsa = rsa_encrypt_hex(password).expect("RSA 加密密码失败");
    let mut ok = None;
    for _ in 1..=3 {
        let captcha = client.kaptcha().await.expect("获取验证码失败");
        let png = base64::engine::general_purpose::STANDARD
            .decode(&captcha.png_base64)
            .expect("验证码 base64 解码失败");
        let Some(answer) = solve(&png, &templates) else {
            continue; // 识别不出换一张，不消耗登录错误计数
        };
        ok = Some(
            client
                .login(username, &password_rsa, &captcha.uid, &answer)
                .await
                .expect("CAS 登录请求失败（网络层）"),
        );
        break;
    }
    ok.expect("3 张验证码均未能识别（或登录均失败）")
}

/// `POST {portal}/tryLoginUserInfo`（门户前端同款：Content-Type JSON + body `{}`）。
/// 返回 (status, content-type, body)。
async fn post_user_info(client: &CasClient) -> (reqwest::StatusCode, String, String) {
    let url = format!("{}/tryLoginUserInfo", PORTAL_PROBE.trim_end_matches('/'));
    let resp = client
        .http_client()
        .post(url)
        .timeout(TIMEOUT)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body("{}")
        .send()
        .await
        .expect("tryLoginUserInfo 请求失败（网络层）");
    let status = resp.status();
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.text().await.unwrap_or_default();
    (status, ct, body)
}

/// `GET {portal}/api/upp/userControl/getLoginInfo`（对照组：同样依赖门户会话）。
async fn get_login_info(client: &CasClient) -> (reqwest::StatusCode, String, String) {
    let url = format!(
        "{}/api/upp/userControl/getLoginInfo",
        PORTAL_PROBE.trim_end_matches('/')
    );
    let resp = client
        .http_client()
        .get(url)
        .timeout(TIMEOUT)
        .send()
        .await
        .expect("getLoginInfo 请求失败（网络层）");
    let status = resp.status();
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.text().await.unwrap_or_default();
    (status, ct, body)
}

/// 打印一次响应（结构摘要 + 判别结论），并把打码样本写 recon 目录。
fn report(tag: &str, status: reqwest::StatusCode, ct: &str, body: &str, dir: &std::path::Path) {
    println!("\n=== {tag} ===");
    println!("[判别] {}", classify_body(status, ct, body));
    match serde_json::from_str::<Value>(body) {
        Ok(v) => {
            let mut s = String::new();
            describe(&v, "<root>", 0, &mut s);
            print!("[结构] {s}");
            let meta = v.get("meta");
            println!(
                "[meta] code={:?} message={:?}",
                meta.and_then(|m| m.get("code")),
                meta.and_then(|m| m.get("message"))
                    .and_then(|m| m.as_str())
                    .map(mask_msg)
            );
            // 关键判别：data 是否存在、userName 是否存在/为空
            let data = v.get("data");
            println!(
                "[判别关键] data 存在={} data 是 null={} userName 存在={} userName 为空/非字符串={}",
                data.is_some(),
                data.is_some_and(|d| d.is_null()),
                data.and_then(|d| d.get("userName")).is_some(),
                data.and_then(|d| d.get("userName"))
                    .map(|n| !n.is_string() || n.as_str().is_some_and(|s| s.trim().is_empty()))
                    .unwrap_or(true),
            );
            let _ = std::fs::write(dir.join(format!("{tag}_structure.txt")), s);
        }
        Err(_) => {
            println!("[结构] 非 JSON；{}", html_shape(body));
            let _ = std::fs::write(dir.join(format!("{tag}_raw_redacted.txt")), html_shape(body));
        }
    }
}

/// 用 `session.json`（DPAPI 加密的 cookie 快照）复原会话 jar —— 等价于客户端重启后
/// 走的 `restore_session()`（`tauri-app/src-tauri/src/infra/state.rs`），即用户现场状态。
fn restore_stale_session() -> Option<(CasClient, Vec<String>)> {
    let dir = std::env::var("APPDATA").ok()?;
    let raw =
        std::fs::read_to_string(std::path::Path::new(&dir).join("campushub/session.json")).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let cookies: Vec<(String, String)> = v
        .get("cookies")?
        .as_array()?
        .iter()
        .filter_map(|c| {
            let name = c.get("name")?.as_str()?.to_string();
            let value = dpapi::unprotect(c.get("valueB64")?.as_str()?)?;
            Some((name, value))
        })
        .collect();
    if cookies.is_empty() {
        return None;
    }
    let client = CasClient::new().ok()?;
    client.jar().restore(&cookies);
    let names = cookies.iter().map(|(n, _)| n.clone()).collect();
    Some((client, names))
}

/// 门户鉴权头全链诊断：CAS 登录 → tryLoginUserInfo 结构 → 对照组 → 复原会话。
#[tokio::test]
#[ignore = "live：需校园网与真实凭据"]
async fn portal_diag_live() {
    let dir = std::env::var("CAMPUS_HUB_RECON_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("campushub-m3-recon"));
    std::fs::create_dir_all(&dir).expect("创建 recon 目录失败");
    println!("[recon] 打码样本目录: {}", dir.display());

    // 0. 未登录基线（干净 jar）：未登录时 tryLoginUserInfo 长什么样
    let anon = CasClient::new().expect("创建干净 CasClient 失败");
    let (s, ct, b) = post_user_info(&anon).await;
    report("anon_tryLoginUserInfo", s, &ct, &b, &dir);

    // 0b. 现场复现：从 session.json 复原的旧会话（cookie 停在用户上次登录时刻）
    match restore_stale_session() {
        Some((stale, names)) => {
            println!("\n--- 现场复现：session.json 复原的旧会话 ---");
            println!(
                "[旧会话] cookie 名={names:?} has(customsid)={}",
                stale.jar().has("customsid")
            );
            println!("[旧会话] portal_probe={:?}", stale.portal_probe().await);
            let (s, ct, b) = post_user_info(&stale).await;
            report("stale_tryLoginUserInfo", s, &ct, &b, &dir);
            let portal = campus_portal::PortalClient::new(stale.clone());
            match portal.query_info_columns().await {
                Ok(c) => println!("[旧会话][产品路径] query_info_columns 成功，栏目数={}", c.len()),
                Err(e) => println!("[旧会话][产品路径] query_info_columns 失败: {e}"),
            }
        }
        None => println!("\n[旧会话] session.json 缺失或无法解密 → 跳过现场复现"),
    }

    // 1. 凭据（env 优先，其次 accounts.json）
    let Some((username, password)) = read_creds() else {
        println!("\n[结论] 无可用凭据 → 无法完成登录链路取证，仅以上匿名/旧会话基线可用");
        return;
    };

    // 2. CAS 登录 + 门户 SSO（建会话）
    let client = CasClient::new().expect("创建 CasClient 失败");
    let ok = cas_login(&client, &username, &password).await;
    println!(
        "[登录] tgt/ticket 已取得（tgt 前缀={} ticket 前缀={}）",
        ok.tgt.split('-').next().unwrap_or("***"),
        ok.ticket.split('-').next().unwrap_or("***")
    );
    client
        .sso_follow(PORTAL_SERVICE, &ok.ticket)
        .await
        .expect("SSO 回跳门户失败");

    // 3. 门户会话探针（customsid + 首页是否被弹回 CAS）
    let cookie_names: Vec<String> = client.jar().snapshot().iter().map(|(n, _)| n.clone()).collect();
    println!(
        "[jar] cookie 名={cookie_names:?} has(customsid)={}",
        client.jar().has("customsid")
    );
    let probe = client.portal_probe().await;
    println!("[portal_probe] {probe:?}");

    // 4. tryLoginUserInfo（故障点）+ getLoginInfo 对照
    let (s, ct, b) = post_user_info(&client).await;
    report("logged_tryLoginUserInfo", s, &ct, &b, &dir);
    let (s2, ct2, b2) = get_login_info(&client).await;
    report("logged_getLoginInfo", s2, &ct2, &b2, &dir);

    // 5. 产品路径复现：PortalClient::query_info_columns（用户看到的报错就在这条链上）
    let portal = campus_portal::PortalClient::new(client.clone());
    match portal.query_info_columns().await {
        Ok(cols) => println!("[产品路径] query_info_columns 成功，栏目数={}", cols.len()),
        Err(e) => println!("[产品路径] query_info_columns 失败: {e}"),
    }

    // 6. 产品解析路径的真实结果（与静态分析对照）
    match client.portal_user_profile().await {
        Ok(p) => println!(
            "[产品路径] extract_user_profile 成功: name 长度={} department 有={} userId 有={} orgId 有={} tokenId 有={}",
            p.name.chars().count(),
            p.department.is_some(),
            p.user_id.is_some(),
            p.org_id.is_some(),
            p.token_id.is_some()
        ),
        Err(e) => println!("[产品路径] extract_user_profile 失败: {e}"),
    }
}
