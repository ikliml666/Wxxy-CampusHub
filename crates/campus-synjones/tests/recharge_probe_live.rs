//! M3.1 批 B **只读**探针：现有 SSO token 能否直接调 blade 系支付接口。
//!
//! # 本测试回答一个问题（`docs/superpowers/plans/2026-09-19-m3.1-native-recharge.md` §5）
//!
//! 我们手里的 token 来自 **CAS SSO 桥**（`sso.rs::sso_token`，落点 URL query `synjones-auth`），
//! 而 `/blade-pay/pay` 这类 App 版支付接口的「原生」token 来自 `POST /blade-auth/oauth/token`
//! （账密换取）。**两者是否通用，只能实测**——本探针就是那个实验。
//!
//! 判据（先匿名取基线，再带 token 对照）：
//! - 匿名 `POST /blade-pay/pay`（假 feeitemid）与带 token 的响应**同为** `code=401 请求未授权`
//!   ⇒ token 不被接受（blade 系要自己那套 token）；
//! - 带 token 时错误变成「feeitemid 不存在」之类**非未授权**的业务错误 ⇒ token 被接受，链路可通。
//! - `GET /charge/order/personal_data` 匿名即 `401 请求未授权` ⇒ 它是**必须鉴权**的端点，
//!   带 token 返回 200 即为「token 被 charge 系接受」的**决定性证据**。
//!   （注意：`/charge/pay/getpayinfo?orderid=<假 id>` **匿名也返回「未查询到订单信息」**，
//!   它对鉴权不设卡，**不能**用作 token 兼容性判据——第一轮实测纠正了计划 §3 批 B 的假设。）
//!
//! # 红线（本测试**只读**）
//!
//! - 绝不发起会创建订单/扣款的请求：建单探针**只用不存在的 `feeitemid=999999`**，
//!   且断言响应里**没有** `data.orderid`（一旦出现即为意外产生了订单，测试直接失败并要求人工处置）。
//! - 不调 `deleteOrder`、不调提交支付、不用真实 feeitemid。
//! - 不打印 token/账号/密码/学号；只打印长度与「是否存在」。
//!
//! 复跑：`cargo test -p campus-synjones -- --ignored recharge_token_compat_live --nocapture`

use campus_auth::captcha::{solve, KaptchaTemplates};
use campus_auth::cas::CasClient;
use campus_auth::rsa::rsa_encrypt_hex;
use campus_synjones::sso::{default_target_url, sso_token};
use campus_synjones::BERSERKER_BASE;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// App 版（`/charge-app/`）bundle 中逐字出现的 Basic 头（= `charge:charge_secret`，公开硬编码常量）。
const BASIC_CHARGE: &str = "Basic Y2hhcmdlOmNoYXJnZV9zZWNyZXQ=";

/// 对照用 Basic（`synjones_live.rs::synjones_oauth_password_experiment` 用的那套）。
const BASIC_MOBILE: &str =
    "Basic bW9iaWxlX3NlcnZpY2VfcGxhdGZvcm06bW9iaWxlX3NlcnZpY2VfcGxhdGZvcm1fc2VjcmV0";

// ==================== recon 目录 / 脱敏 ====================

fn recon_dir() -> PathBuf {
    std::env::var("CAMPUS_HUB_RECON_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("campushub-m3-recon"))
}

/// 连续 ≥6 位数字打码（orderid 类兜底），其余原样（错误文案要可读）。
fn mask_digits(s: &str) -> String {
    fn flush(run: &mut String, out: &mut String) {
        if run.chars().count() >= 6 {
            out.push_str("***");
        } else {
            out.push_str(run);
        }
        run.clear();
    }
    let mut out = String::new();
    let mut run = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

fn is_sensitive_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    [
        "account", "cardno", "card_num", "yktcard", "sno", "username", "loginname", "realname",
        "idcard", "phone", "mobile", "bankcard", "openid", "nickname", "avatar", "email", "token",
        "name", "password", "secret", "mobilephone", "idno", "auth", "synjones", "cookie", "ticket",
        "orderid", "order_id",
    ]
    .iter()
    .any(|s| k.contains(s))
}

fn looks_like_id(s: &str) -> bool {
    let digits = s.chars().filter(|c| c.is_ascii_digit()).count();
    digits >= 8 && digits == s.trim().len()
}

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

fn dump(dir: &Path, name: &str, v: &Value) {
    let text = serde_json::to_string_pretty(&redact_json(v)).unwrap_or_default();
    println!("----- {name} -----\n{text}\n----- /{name} -----");
    std::fs::write(dir.join(name), text).ok();
}

/// 响应形状摘要（只回键名与长度，不回值）。
fn shape_of(v: &Value) -> String {
    match v {
        Value::Object(o) => format!(
            "object keys=[{}]",
            o.keys().cloned().collect::<Vec<_>>().join(",")
        ),
        Value::Array(a) => format!("array(len={})", a.len()),
        Value::String(s) => format!("string(len={})", s.chars().count()),
        Value::Number(n) => format!("number({n})"),
        Value::Bool(b) => format!("bool({b})"),
        Value::Null => "null".to_string(),
    }
}

// ==================== 凭据（只读、只在内存；写法与 synjones_live.rs 一致） ====================

fn appdata_campushub() -> Option<PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .map(|d| PathBuf::from(d).join("campushub"))
}

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
    let data =
        unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize).to_vec() };
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

fn creds_from_accounts_file() -> Option<(String, String)> {
    let path = appdata_campushub()?.join("accounts.json");
    let text = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let first = v.get("accounts")?.as_array()?.first()?.clone();
    let username = first.get("username")?.as_str()?.to_string();
    let password = dpapi_unprotect_b64(first.get("passwordB64")?.as_str()?).ok()?;
    Some((username, password))
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

// ==================== 响应事实 ====================

/// 一次探测的可打印事实（**只留状态与 code/msg**，msg 经数字打码）。
struct Probe {
    label: &'static str,
    status: u16,
    code: Option<i64>,
    msg: String,
}

impl Probe {
    /// 「未授权」判定：`code==401`（含 charge 系 4030）或 msg 含未授权字样。
    fn unauthorized(&self) -> bool {
        matches!(self.code, Some(401) | Some(4030)) || self.msg.contains("未授权")
    }

    fn report(&self) {
        println!(
            "[{}] HTTP {} code={:?} msg={:?}",
            self.label, self.status, self.code, self.msg
        );
    }
}

/// 账密来源（与 synjones_live.rs 同链）：账号库 DPAPI 密码 → `CAMPUS_HUB_CREDS` 文件。
fn load_creds() -> (String, String, &'static str) {
    match creds_from_accounts_file() {
        Some((u, p)) => (u, p, "accounts.json(DPAPI 密码)"),
        None => {
            let (u, p) = creds_from_env_file().expect(
                "无可用凭据：session.json 无 tgtB64、accounts.json 不可解密、CAMPUS_HUB_CREDS 未设置",
            );
            (u, p, "CAMPUS_HUB_CREDS")
        }
    }
}

async fn probe(
    label: &'static str,
    req: reqwest::RequestBuilder,
) -> (Probe, Value) {
    let resp = req.send().await.expect("探测请求失败（网络层）");
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let p = Probe {
        label,
        status,
        code: v.get("code").and_then(Value::as_i64),
        msg: v
            .get("msg")
            .or_else(|| v.get("message"))
            .and_then(Value::as_str)
            .map(mask_digits)
            .unwrap_or_default(),
    };
    p.report();
    (p, v)
}

// ==================== 探针主流程 ====================

#[tokio::test]
#[ignore = "live：需校园网与真实凭据（严格只读，不产生订单）"]
async fn recharge_token_compat_live() {
    let dir = recon_dir();
    std::fs::create_dir_all(&dir).expect("创建 recon 目录失败");
    println!("[recon] 样本目录: {}", dir.display());

    let cas = CasClient::new().expect("创建 CasClient 失败");
    let http = reqwest::Client::new();
    let mut report = serde_json::Map::new();

    // ---------- 0) 匿名基线（无任何 token）：定标「未授权」长什么样 ----------
    let (anon_time, time_v) = probe(
        "匿名 /charge/order/getCurrentTime",
        http.get(format!("{BERSERKER_BASE}/charge/order/getCurrentTime")),
    )
    .await;
    assert_eq!(anon_time.status, 200, "匿名 getCurrentTime 应 200");

    let (anon_payinfo, _) = probe(
        "匿名 /charge/pay/getpayinfo?orderid=0",
        http.get(format!(
            "{BERSERKER_BASE}/charge/pay/getpayinfo?orderid=0"
        )),
    )
    .await;

    let (anon_blade, _) = probe(
        "匿名 POST /blade-pay/pay（假 feeitemid）",
        http.post(format!("{BERSERKER_BASE}/blade-pay/pay"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body("feeitemid=999999&tranamt=1&flag=choose&source=app&paystep=0"),
    )
    .await;

    let (anon_personal, _) = probe(
        "匿名 /charge/order/personal_data?status=0",
        http.get(format!(
            "{BERSERKER_BASE}/charge/order/personal_data?status=0"
        )),
    )
    .await;

    // App 版账密登录的 logintype 取值（匿名可读即取，供 oauth 对照用）
    let (logintype_probe, logintype_v) = probe(
        "匿名 /charge/logintype",
        http.get(format!("{BERSERKER_BASE}/charge/logintype")),
    )
    .await;
    let logintype = logintype_v
        .get("logintype")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    println!("[logintype] 实测取值={logintype:?}（长度 {}）", logintype.len());

    report.insert(
        "anonymousBaseline".to_string(),
        serde_json::json!({
            "getCurrentTime": {"status": anon_time.status, "code": anon_time.code, "msg": anon_time.msg,
                               "dataShape": shape_of(&time_v["data"])},
            "getpayinfo_fakeOrder": {"status": anon_payinfo.status, "code": anon_payinfo.code, "msg": anon_payinfo.msg},
            "bladePay_bogusFeeitem": {"status": anon_blade.status, "code": anon_blade.code, "msg": anon_blade.msg},
            "personalData": {"status": anon_personal.status, "code": anon_personal.code, "msg": anon_personal.msg},
            "logintype": logintype,
        }),
    );

    // ---------- 1) 现有 SSO 桥拿 token ----------
    let mut token = None;
    if let Some(tgt) = tgt_from_app_session() {
        println!("[凭据] 来源=session.json(DPAPI TGT) 长度={}", tgt.len());
        match sso_token(&cas, &tgt, &default_target_url()).await {
            Ok(tok) => token = Some(tok),
            Err(e) => println!("[凭据] 本机 TGT 换票失败（{e}）→ 回落账密登录"),
        }
    } else {
        println!("[凭据] session.json 无可用 TGT");
    }
    let token = match token {
        Some(t) => t,
        None => {
            let (username, password, src) = load_creds();
            println!("[凭据] 来源={src} 用户长度={}", username.len());
            let ok = cas_login(&cas, &username, &password).await;
            println!("[凭据] 账密登录成功，TGT 长度={}", ok.tgt.len());
            sso_token(&cas, &ok.tgt, &default_target_url())
                .await
                .expect("账密登录取得的新 TGT 应能换到 token")
        }
    };
    let auth = token.auth_value();
    println!(
        "[凭据] ✅ SSO 桥 token 已取得：access_token 长度={} token_type={:?}（值不打印）",
        token.access_token.len(),
        token.token_type
    );

    // ---------- 2) 带 token 逐端点探测（全部假参数，零副作用） ----------
    // 2a) 假 orderid：先看鉴权卡不卡（实测匿名即「未查询到订单信息」→ 不作判据，仅记录）
    let payinfo_form = http
        .get(format!("{BERSERKER_BASE}/charge/pay/getpayinfo?orderid=0"))
        .header("Authorization", BASIC_CHARGE)
        .header("synjones-auth", auth.clone());
    let (payinfo, _payinfo_v) = probe("带 token /charge/pay/getpayinfo?orderid=0", payinfo_form).await;

    // 2b) 建单端点 + 假 feeitemid：**这是主判据**
    let (blade, blade_v) = probe(
        "带 token POST /blade-pay/pay（feeitemid=999999，paystep=0）",
        http.post(format!("{BERSERKER_BASE}/blade-pay/pay"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Authorization", BASIC_CHARGE)
            .header("synjones-auth", auth.clone())
            .body("feeitemid=999999&tranamt=1&flag=choose&source=app&paystep=0"),
    )
    .await;

    // 红线核查：假 feeitemid 不应产生任何订单（响应里出现 orderid 即为意外副作用）
    let created_order = blade_v["data"]["orderid"]
        .as_str()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    assert!(
        !created_order,
        "⚠️ 建单探针意外产生了订单（响应 data.orderid 非空）——立即人工处置，勿依赖本测试"
    );

    // 2b-2) 只带 token、不带 Basic：实测 Basic 是否强制（计划 §5 未证实的点）
    let (blade_no_basic, _) = probe(
        "带 token 但不带 Basic POST /blade-pay/pay（假 feeitemid）",
        http.post(format!("{BERSERKER_BASE}/blade-pay/pay"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("synjones-auth", auth.clone())
            .body("feeitemid=999999&tranamt=1&flag=choose&source=app&paystep=0"),
    )
    .await;

    // 2c) personal_data：匿名 401、带 token 若 200 即为**决定性证据**
    let (personal, personal_v) = probe(
        "带 token /charge/order/personal_data?status=0",
        http.get(format!(
            "{BERSERKER_BASE}/charge/order/personal_data?status=0"
        ))
        .header("Authorization", BASIC_CHARGE)
        .header("synjones-auth", auth.clone()),
    )
    .await;

    // 2c-2) personal_data 参数矩阵（只读）：带 token 仍 500 ⇒ 该端点不给我们「遗留未支付订单」，
    //       实现批 C 需要另找判据——先把参数空间探一遍，省一轮返工。
    let mut personal_variants: Vec<Value> = Vec::new();
    for (name, query) in [
        ("无参", ""),
        ("status=0", "?status=0"),
        ("status=1", "?status=1"),
        ("status=0&current=1&size=10", "?status=0&current=1&size=10"),
        ("pageNum=1&pageSize=10", "?pageNum=1&pageSize=10"),
    ] {
        let (p, v) = probe(
            "personal_data 变体（带 token）",
            http.get(format!("{BERSERKER_BASE}/charge/order/personal_data{query}"))
                .header("Authorization", BASIC_CHARGE)
                .header("synjones-auth", auth.clone()),
        )
        .await;
        println!("  └ 变体 {name}: code={:?} data={}", p.code, shape_of(&v["data"]));
        personal_variants.push(serde_json::json!({
            "variant": name, "status": p.status, "code": p.code, "msg": p.msg,
            "dataShape": shape_of(&v["data"]),
        }));
    }

    // 待支付订单只报条数，绝不取消
    let pending = &personal_v["data"];
    let pending_desc = match pending {
        Value::Array(a) => format!("array 条数={}", a.len()),
        Value::Object(o) => {
            let n = ["records", "list", "rows", "data"]
                .iter()
                .find_map(|k| o.get(*k).and_then(Value::as_array).map(Vec::len));
            format!("object keys=[{}] 其中列表条数={:?}", o.keys().cloned().collect::<Vec<_>>().join(","), n)
        }
        other => shape_of(other),
    };
    println!(
        "[待支付订单] 匿名={} 带 token={} 形状：{pending_desc}",
        if anon_personal.unauthorized() { "401 未授权" } else { "非 401" },
        if personal.unauthorized() { "401 未授权" } else { "非 401" }
    );

    // ---------- 3) 判定 ----------
    let blade_accepts = !blade.unauthorized();
    let personal_accepts = !personal.unauthorized();
    let token_accepted = blade_accepts || personal_accepts;
    println!(
        "\n[★判定] token 被 blade-pay 接受={blade_accepts}（msg={:?}）｜被 charge/personal_data 接受={personal_accepts}（msg={:?}）",
        blade.msg, personal.msg
    );
    println!(
        "[★判定] getpayinfo(假 orderid) 带 token={} 匿名={} ⇒ 该端点对鉴权不设卡，不作判据",
        if payinfo.unauthorized() { "401" } else { "非 401" },
        if anon_payinfo.unauthorized() { "401" } else { "非 401" }
    );
    println!(
        "[Basic 头] 带 token 但不带 Basic 的建单探针：code={:?} msg={:?} ⇒ Basic {}强制（计划 §5 未证实项）",
        blade_no_basic.code,
        blade_no_basic.msg,
        if blade_no_basic.unauthorized() { "被" } else { "非" }
    );

    // ---------- 4) 若不兼容：只读对照 /blade-auth/oauth/token 账密路线 ----------
    let mut oauth_result = Value::Null;
    if !token_accepted {
        println!("\n[oauth 对照] SSO token 不被接受 → 试 App 版账密换 token（只取 token，不下单）");
        let (username, password, src) = load_creds();
        println!("[oauth 对照] 账密来源={src} 用户长度={}", username.len());
        // 账号库密码是学校统一身份密码（可能 ≠ 慧新E校密码）→ 失败即结论「该路线需用户另行输入」
        // 限制为 2 个 Basic 变体，避免多次错误账密尝试
        let login_type = if logintype.is_empty() {
            "student-sno".to_string()
        } else {
            logintype.clone()
        };
        let mut attempts = Vec::new();
        for (name, basic) in [("charge:charge_secret", BASIC_CHARGE), ("mobile_service_platform", BASIC_MOBILE)] {
            let (p, v) = probe(

                "oauth/token（App 账密）",
                http.post(format!("{BERSERKER_BASE}/blade-auth/oauth/token"))
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .header("Authorization", basic)
                    .form(&[
                        ("username", username.as_str()),
                        ("password", password.as_str()),
                        ("grant_type", "password"),
                        ("scope", "all"),
                        ("logintype", login_type.as_str()),
                    ]),
            )
            .await;
            let has_token = v
                .get("access_token")
                .and_then(Value::as_str)
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            let expires_in = v.get("expires_in").cloned().unwrap_or(Value::Null);
            println!(
                "[oauth 对照 {name}] 有 access_token={has_token} expires_in={expires_in} 用户长度={}",
                username.len()
            );
            attempts.push(serde_json::json!({
                "basic": name, "status": p.status, "code": p.code, "msg": p.msg,
                "hasCredential": has_token, "expiresIn": expires_in,
            }));
            if has_token {
                break;
            }
        }
        oauth_result = Value::Array(attempts);
        dump(&dir, "blade_oauth_token_attempts.json", &oauth_result);
    }

    // ---------- 5) 落盘取证 ----------
    // 注：键名刻意避开 token/auth 字样——`redact_json` 按**键名子串**打码，
    // 用 `withSsoToken` 这类键会把整棵证据子树变成 "***"（第一轮实测踩过）。
    report.insert(
        "withSsoHeader".to_string(),
        serde_json::json!({
            "credentialLen": token.access_token.len(),
            "credentialScheme": token.token_type,
            "getpayinfoFakeOrder": {"status": payinfo.status, "code": payinfo.code, "msg": payinfo.msg},
            "bladePayBogusFeeitem": {"status": blade.status, "code": blade.code, "msg": blade.msg,
                                     "dataShape": shape_of(&blade_v["data"])},
            "bladePayBogusFeeitemNoBasic": {"status": blade_no_basic.status, "code": blade_no_basic.code,
                                            "msg": blade_no_basic.msg},
            "personalDataStatus0": {"status": personal.status, "code": personal.code, "msg": personal.msg,
                                    "shape": pending_desc},
            "personalDataVariants": personal_variants,
            "verdict": {
                "acceptedByBladePay": blade_accepts,
                "acceptedByPersonalData": personal_accepts,
                "accepted": token_accepted,
            },
        }),
    );
    report.insert("passwordLoginFallback".to_string(), oauth_result);
    dump(&dir, "recharge_token_compat.json", &Value::Object(report));

    // ---------- 6) 固化断言 ----------
    assert_eq!(anon_blade.code, Some(401), "匿名建单应 401（基线：未授权长这样）");
    assert!(
        anon_personal.unauthorized(),
        "匿名 personal_data 应 401（它是必须鉴权的端点）"
    );
    assert!(
        anon_payinfo.code != Some(401),
        "getpayinfo(假 orderid) 匿名不应 401——该端点不卡鉴权，不能作判据"
    );
    assert!(
        logintype_probe.status == 200 && !logintype.is_empty(),
        "/charge/logintype 应匿名可读且给出非空 logintype"
    );
    if token_accepted {
        println!("\n✅ 结论：现有 SSO token 可调 blade/charge 支付系（无需新增 blade 登录）");
    } else {
        println!("\n⚠️ 结论：现有 SSO token 不被 blade/charge 支付系接受，需走 /blade-auth/oauth/token");
    }
    println!("[命令] cargo test -p campus-synjones -- --ignored recharge_token_compat_live --nocapture");
}
