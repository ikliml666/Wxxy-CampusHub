//! plat（移动服务平台）SSO 探针——为「付款码 / plat 设置」功能做的鉴权链验证（ignored live）。
//!
//! 目标（2026-09-20 官方逆向的三问）：
//! 1. lyCas 桥 `targetUrl=/campus-card/`（mobile 壳）落点的 `synjones-auth` 是不是
//!    `client_id=mobile_service_platform` 的 plat JWT；
//! 2. 该 JWT 调 plat API（`synjones-auth: bearer <jwt>` 头）是否可用；
//! 3. 付款码三接口（`codebarPayinfo` / `batchGetBarCodeGet` / `getUserOfflienSwitch`）
//!    在该 JWT 下的真实响应结构。
//!
//! 红线：token / 验证码答案 / 卡号 / 手机号一律不打印——只打印形态（长度、键名、code）。
//! 运行：`CAMPUS_HUB_CREDS=<凭据文件> cargo test -p campus-synjones --test plat_sso_probe_live -- --ignored --nocapture`

use campus_auth::captcha::{solve, KaptchaTemplates};
use campus_auth::cas::CasClient;
use campus_auth::rsa::rsa_encrypt_hex;
use campus_synjones::sso::{default_target_url, sso_token};
use campus_synjones::{BERSERKER_BASE, SYN_ACCESS_SOURCE};
use serde_json::Value;
use std::path::PathBuf;

fn appdata_campushub() -> Option<PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .map(|d| PathBuf::from(d).join("campushub"))
}

fn dpapi_unprotect_b64(b64: &str) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| e.to_string())?;
    let plain = dpapi_unprotect(&bytes)?;
    String::from_utf8(plain).map_err(|e| e.to_string())
}

#[cfg(windows)]
fn dpapi_unprotect(cipher: &[u8]) -> Result<Vec<u8>, String> {
    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }
    #[link(name = "crypt32")]
    extern "system" {
        fn CryptUnprotectData(
            pDataIn: *const DataBlob,
            ppszDataDescr: *mut *mut u16,
            pOptionalEntropy: *const DataBlob,
            pvReserved: *mut core::ffi::c_void,
            pPromptStruct: *mut core::ffi::c_void,
            dwFlags: u32,
            pDataOut: *mut DataBlob,
        ) -> i32;
        fn LocalFree(h: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    }
    unsafe {
        let mut out = DataBlob { cb_data: 0, pb_data: std::ptr::null_mut() };
        let ok = CryptUnprotectData(
            &DataBlob { cb_data: cipher.len() as u32, pb_data: cipher.as_ptr() as *mut u8 },
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            &mut out,
        );
        if ok == 0 {
            return Err("CryptUnprotectData 失败".into());
        }
        let slice = std::slice::from_raw_parts(out.pb_data, out.cb_data as usize);
        let v = slice.to_vec();
        LocalFree(out.pb_data as *mut _);
        Ok(v)
    }
}

#[cfg(not(windows))]
fn dpapi_unprotect(_cipher: &[u8]) -> Result<Vec<u8>, String> {
    Err("非 Windows 平台不支持 DPAPI".into())
}

fn tgt_from_app_session() -> Option<String> {
    let path = appdata_campushub()?.join("session.json");
    let text = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let b64 = v.get("tgtB64")?.as_str()?;
    dpapi_unprotect_b64(b64).ok().filter(|t| !t.trim().is_empty())
}

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

async fn cas_login(client: &CasClient, username: &str, password: &str) -> String {
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
            println!("[captcha] 识别失败，换一张");
            continue;
        };
        return client
            .login(username, &password_rsa, &captcha.uid, &answer)
            .await
            .expect("CAS 登录请求失败（网络层）")
            .tgt;
    }
    panic!("3 张验证码均未能识别（或登录均失败）");
}

/// JWT payload 的 client_id / logintype（不打印 token 本体）。
fn jwt_shape(token: &str) -> String {
    let seg: Vec<&str> = token.split('.').collect();
    if seg.len() != 3 {
        return format!("非 JWT（段数={}）", seg.len());
    }
    use base64::Engine as _;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(seg[1])
        .or_else(|_| {
            base64::engine::general_purpose::STANDARD.decode(seg[1])
        })
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    match payload {
        Some(p) => format!(
            "client_id={:?} logintype={:?} exp={:?}",
            p.get("client_id").and_then(|v| v.as_str()),
            p.get("logintype").and_then(|v| v.as_str()),
            p.get("exp").and_then(|v| v.as_i64()),
        ),
        None => "payload 解码失败".into(),
    }
}

/// plat 鉴权头的 GET 请求 → 响应 code + data 键名（值不外泄）。
async fn plat_get(client: &reqwest::Client, path: &str, token: &str) -> Value {
    let r = client
        .get(format!("{BERSERKER_BASE}{path}"))
        .header("synjones-auth", format!("bearer {token}"))
        .header("synAccessSource", SYN_ACCESS_SOURCE)
        .send()
        .await
        .expect("plat 请求网络失败");
    let v: Value = r.json().await.expect("plat 响应非 JSON");
    v
}

fn summarize(v: &Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, val)| {
                    let s = val.to_string();
                    let shape = if s.len() > 40 {
                        format!("str({})", s.len())
                    } else {
                        s
                    };
                    (k.clone(), Value::String(shape))
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().take(1).map(summarize).collect()),
        other => other.clone(),
    }
}

#[tokio::test]
#[ignore = "live：需校园网与真实凭据，仅主智能体验收时 -- --ignored 运行"]
async fn plat_mobile_sso_probe() {
    let client = CasClient::new().expect("创建 CasClient 失败");
    let tgt = match tgt_from_app_session() {
        Some(t) => {
            println!("[auth] 复用应用 session.json 的 TGT");
            t
        }
        None => {
            let (u, p) = creds_from_env_file().expect("无应用 TGT 且无 CAMPUS_HUB_CREDS");
            println!("[auth] 凭据文件 CAS 登录（账号=****{}）", &u[u.len().saturating_sub(2)..]);
            cas_login(&client, &u, &p).await
        }
    };

    let http_client = reqwest::Client::new();
    // 0) 决定性实验：应用现有 PC 体系 token 直接配 synjones-auth 头调 plat API
    //    （服务端若不区分 token 体系，A/B 组无需 plat JWT）
    let pc_token = sso_token(&client, &tgt, &default_target_url())
        .await
        .expect("PC targetUrl 签名失败（TGT 过期？）");
    println!("[step0] PC token：len={} {}", pc_token.access_token.len(), jwt_shape(&pc_token.access_token));
    let user_pc = plat_get(&http_client, "/berserker-base/user", &pc_token.access_token).await;
    println!("[step0a] /berserker-base/user（PC token + synjones-auth 头）：code={:?}", user_pc.get("code").and_then(|v| v.as_i64()));
    let cbi_pc = plat_get(&http_client, "/berserker-app/ykt/tsm/codebarPayinfo", &pc_token.access_token).await;
    println!("[step0b] codebarPayinfo（PC token）：code={:?}", cbi_pc.get("code").and_then(|v| v.as_i64()));
    let bc_pc = plat_get(&http_client, "/berserker-app/ykt/tsm/batchGetBarCodeGet?account=42940&payacc=000&paytype=1", &pc_token.access_token).await;
    println!("[step0c] batchGetBarCodeGet（PC token）：code={:?} retcode={:?} barcode_len={:?}", bc_pc.get("code").and_then(|v| v.as_i64()), bc_pc.pointer("/data/retcode").and_then(|v| v.as_str()), bc_pc.pointer("/data/barcode").and_then(|v| v.as_array()).map(|a| a.len()));

    // 1) plat JWT 研究性尝试（结论：非必需——step0 已证 PC token 直接可用；
    //    若未来被服务端收紧，官方链是 CAS ST 喂 oauth/token，service 需与官方前端一致）
    let service = campus_synjones::sso::ly_cas_service_url(&format!("{BERSERKER_BASE}/campus-card/"));
    match client.sso_ticket(&tgt, &service).await {
        Ok(st) => {
            let resp = http_client
                .post(format!("{BERSERKER_BASE}/berserker-auth/oauth/token"))
                .header("Authorization", "Basic bW9iaWxlX3NlcnZpY2VfcGxhdGZvcm06bW9iaWxlX3NlcnZpY2VfcGxhdGZvcm1fc2VjcmV0")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .form(&[
                    ("username", st.as_str()),
                    ("password", st.as_str()),
                    ("grant_type", "password"),
                    ("scope", "all"),
                    ("loginFrom", "h5"),
                    ("logintype", "sso"),
                    ("device_token", "h5"),
                ])
                .send()
                .await;
            let got = match resp {
                Ok(r) => r.json::<Value>().await.ok().map(|v| v.get("access_token").is_some()),
                Err(_) => None,
            };
            println!("[step1] plat JWT 直换（研究性）：got_access_token={got:?}");
        }
        Err(e) => println!("[step1] 换票失败（跳过）：{}", &e.to_string()[..e.to_string().len().min(60)]),
    }

    let token = pc_token.access_token.clone();
    // 2) plat API 可用性（用户资料）——响应结构取证（值脱敏）
    let user = plat_get(&http_client, "/berserker-base/user", &token).await;
    println!(
        "[step2] /berserker-base/user：code={:?} data={}",
        user.get("code").and_then(|v| v.as_i64()),
        summarize(user.get("data").unwrap_or(&Value::Null))
    );

    // 3) 付款码三接口
    let cbi = plat_get(&http_client, "/berserker-app/ykt/tsm/codebarPayinfo", &token).await;
    println!(
        "[step3a] codebarPayinfo：code={:?} data={}",
        cbi.get("code").and_then(|v| v.as_i64()),
        summarize(cbi.get("data").unwrap_or(&Value::Null))
    );
    let bc = plat_get(
        &http_client,
        "/berserker-app/ykt/tsm/batchGetBarCodeGet?account=42940&payacc=000&paytype=1",
        &token,
    )
    .await;
    println!(
        "[step3b] batchGetBarCodeGet：code={:?} retcode={:?} expires={:?} barcode_len={:?}",
        bc.get("code").and_then(|v| v.as_i64()),
        bc.pointer("/data/retcode").and_then(|v| v.as_str()),
        bc.pointer("/data/expires").and_then(|v| v.as_i64()),
        bc.pointer("/data/barcode")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
    );
    let off = plat_get(&http_client, "/berserker-app/ykt/tsm/getUserOfflienSwitch", &token).await;
    println!(
        "[step3c] getUserOfflienSwitch：code={:?} data={}",
        off.get("code").and_then(|v| v.as_i64()),
        summarize(off.get("data").unwrap_or(&Value::Null))
    );


}
