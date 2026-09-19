//! M3.1 批 C live 测试：电费充值链路**跑到「密码键盘就绪」为止，绝不提交支付**。
//!
//! # 本测试做什么 / 不做什么
//!
//! 做：① 片区 450 级联到真实房间 → ② 用末级 `map.data` 构造电费 `third_party` → ③ `create_order`
//! 建 **1 元** 订单 → ④ `fetch_pay_methods` 取支付方式（**固化 code 集合与 `nopassword` 真实值**，
//! 这是批 D 前端分支的关键输入）→ ⑤ 需密码时 `query_account` 取安全键盘 → ⑥ **立即 `cancel_order`
//! 清理并复查状态**。
//!
//! **不做**：绝不调 `submit_pay`（下一次真实的 `submit_pay` 就是扣款）。唯一碰 `submit_pay` 的一次
//! 是清理之后的**无效参数**校验（订单号/支付方式 id 都是假的），只验证错误分支——见
//! [`submit_pay_invalid_params_errors`] 那段的内联说明。
//!
//! # 清理保证
//!
//! 建单是**副作用请求**，故本测试把「断言」与「清理」严格分开：所有检查只把结论记进 `failures`，
//! 清理（`cancel_order` + 复查）跑完才 `assert`。任何一步失败都不会留下未清理订单。
//!
//! # 凭据 / 打码（照抄批 B 探针 `recharge_probe_live.rs`）
//!
//! 凭据三来源：`%APPDATA%/campushub/session.json` 的 DPAPI TGT → `accounts.json` 账号库 →
//! `CAMPUS_HUB_CREDS` 文件。token/账号/密码一律不打印；`third_party` 含户号（PII），只打印长度与键名；
//! 落盘样本经 `redact_json` 脱敏写到仓库外 `%TEMP%/campushub-m3-recon/`。
//!
//! ⚠️ token 单活：本测试的 SSO 桥换票会**顶掉**本机此前的 token（跑前确认没有别的实例在用同一账号）。
//!
//! 复跑：`cargo test -p campus-synjones -- --ignored recharge_live --nocapture`

use campus_auth::captcha::{solve, KaptchaTemplates};
use campus_auth::cas::CasClient;
use campus_auth::rsa::rsa_encrypt_hex;
use campus_synjones::charge::{query_cascade, RoomStep};
use campus_synjones::recharge::{self, PayMethod, ACCOUNT_CODES};
use campus_synjones::sso::{default_target_url, sso_token};
use campus_synjones::{SynjonesClient, SynjonesToken, BERSERKER_BASE};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// 被测片区：450「梅园1号-梅园3号」（实测启用的三个电费片区之一）。
const FEEITEM: &str = "450";
/// 实测可用的房间号（批 3 live 固化：末级是输入级，`101` 命中）。
const ROOM: &str = "101";
/// 建单金额：**最小金额 1 元**（`retain_money` 实测 "1"）——只为拿 orderid 与 payList，不提交支付。
const TRANAMT: &str = "1";
/// 支付动作端点（与大写常量同源；测试里做原始形状核对时直接用字符串）。
const EP_BLADE_PAY_RAW: &str = "/blade-pay/pay";

// ==================== recon 目录 / 脱敏（与批 B 探针一致） ====================

fn recon_dir() -> PathBuf {
    std::env::var("CAMPUS_HUB_RECON_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("campushub-m3-recon"))
}

/// 连续 ≥6 位数字打码（orderid / 长 id 兜底），其余原样（错误文案要可读）。
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
        "orderid", "order_id", "uuid", "keys",
    ]
    .iter()
    .any(|s| k.contains(s))
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
        Value::String(s) => {
            let digits = s.chars().filter(|c| c.is_ascii_digit()).count();
            if digits >= 8 && digits == s.trim().len() {
                Value::String("***".to_string())
            } else {
                Value::String(s.clone())
            }
        }
        other => other.clone(),
    }
}

fn dump(dir: &Path, name: &str, v: &Value) {
    let text = serde_json::to_string_pretty(&redact_json(v)).unwrap_or_default();
    std::fs::write(dir.join(name), text).ok();
}

/// 响应形状摘要（只回键名，不回值）。
fn keys_of(v: &Value) -> String {
    v.as_object()
        .map(|o| o.keys().cloned().collect::<Vec<_>>().join(","))
        .unwrap_or_else(|| format!("<{}>", if v.is_null() { "null" } else { "非对象" }))
}

/// JSON 形状摘要（类型 + 长度，**绝不含值**；用于核对 `passwordMap`/`accountno` 的真实形态）。
fn shape_of(v: &Value) -> String {
    match v {
        Value::Object(o) => format!("object(keys={})", o.keys().count()),
        Value::Array(a) => format!("array(len={})", a.len()),
        Value::String(s) => format!("string(len={})", s.chars().count()),
        Value::Number(n) => format!("number({n})"),
        Value::Bool(b) => format!("bool({b})"),
        Value::Null => "null".to_string(),
    }
}

// ==================== 凭据（照抄批 B 探针；只读、只在内存） ====================

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
            .trim();
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

/// 取 token：本机 TGT 换票优先，失败回落账密登录（与批 B 探针同链）。
async fn token_or_login(cas: &CasClient) -> SynjonesToken {
    if let Some(tgt) = tgt_from_app_session() {
        println!("[凭据] 来源=session.json(DPAPI TGT) 长度={}", tgt.len());
        match sso_token(cas, &tgt, &default_target_url()).await {
            Ok(tok) => return tok,
            Err(e) => println!("[凭据] 本机 TGT 换票失败（{e}）→ 回落账密登录"),
        }
    } else {
        println!("[凭据] session.json 无可用 TGT");
    }
    let (username, password) = match creds_from_accounts_file() {
        Some(up) => (up.0, up.1),
        None => creds_from_env_file()
            .expect("无可用凭据：session.json 无 tgtB64、accounts.json 不可解密、CAMPUS_HUB_CREDS 未设置"),
    };
    let ok = cas_login(cas, &username, &password).await;
    sso_token(cas, &ok.tgt, &default_target_url())
        .await
        .expect("账密登录取得的新 TGT 应能换到 token")
}

// ==================== 级联 + third_party ====================

/// 逐级走完 450 的级联，返回三级已选（校区/楼栋/房间）。
async fn cascade_to_room(client: &SynjonesClient) -> Vec<RoomStep> {
    let q0 = query_cascade(client, FEEITEM, &[])
        .await
        .expect("第 1 级（校区）失败");
    println!(
        "[级联] levels={:?} 第1级选项={}",
        q0.levels
            .iter()
            .map(|l| format!("{}:{}", l.level, l.name))
            .collect::<Vec<_>>(),
        q0.options.len()
    );
    assert!(!q0.options.is_empty(), "第 1 级（校区）应有下拉选项");
    let step1 = RoomStep {
        level: q0.options[0].level,
        code: q0.options[0].code.clone(),
        value: q0.options[0].value.clone(),
        name: q0.options[0].label.clone(),
    };
    println!("[级联] 校区 = {}", step1.name);

    let q1 = query_cascade(client, FEEITEM, std::slice::from_ref(&step1))
        .await
        .expect("第 2 级（楼栋）失败");
    assert!(!q1.options.is_empty(), "第 2 级应有下拉选项");
    let step2 = RoomStep {
        level: q1.options[0].level,
        code: q1.options[0].code.clone(),
        value: q1.options[0].value.clone(),
        name: q1.options[0].label.clone(),
    };
    println!("[级联] 楼栋 = {}", step2.name);

    let q2 = query_cascade(client, FEEITEM, &[step1.clone(), step2.clone()])
        .await
        .expect("第 3 级失败");
    assert!(q2.options.is_empty(), "末级（房间）是输入级，实测无下拉");
    let step3 = RoomStep {
        level: q2.levels.last().map(|l| l.level).unwrap_or(3),
        code: q2
            .levels
            .last()
            .map(|l| l.code.clone())
            .unwrap_or_else(|| "room".to_string()),
        value: ROOM.to_string(),
        name: ROOM.to_string(),
    };
    let q3 = query_cascade(
        client,
        FEEITEM,
        &[step1.clone(), step2.clone(), step3.clone()],
    )
    .await
    .expect("末级（房间）失败");
    assert!(q3.is_final, "房间号 {ROOM} 应命中末级");
    let view = q3.view.as_ref().expect("末级应有 view");
    assert!(view.tip.is_none(), "房间号非法：{:?}", view.tip);
    println!(
        "[级联] 房间 {ROOM} 末级展示 = {:?}",
        view.fields
            .iter()
            .map(|f| format!("{}: {}", f.label, f.value))
            .collect::<Vec<_>>()
    );
    vec![step1, step2, step3]
}

/// 单独发一次 `type=IEC` 的 `getThirdData` 的**形状核对**：产品 API 合成的 `third_party`
/// 必须是末级 `map.data` 的 JSON（含 `myCustomInfo`），键名逐个对上。
///
/// 返回 `(third_party 串, 键名摘要)`；串含户号，调用方**只准打印长度与键名**。
async fn product_third_party(client: &SynjonesClient, steps: &[RoomStep]) -> (String, String) {
    let tp = recharge::third_party_for_room(client, FEEITEM, steps)
        .await
        .expect("产品 API 合成 third_party 失败");
    let v: Value = serde_json::from_str(&tp).expect("third_party 应是 JSON 串");
    let keys = keys_of(&v);
    assert!(
        v.get("myCustomInfo").is_some(),
        "各级都有名字时应有 myCustomInfo（官方口径）"
    );
    (tp, keys)
}

// ==================== 主流程 ====================

#[tokio::test]
#[ignore = "live：需校园网 + 真实凭据；会建 1 元订单（不提交支付）并立即清理"]
async fn recharge_live() {
    let dir = recon_dir();
    std::fs::create_dir_all(&dir).expect("创建 recon 目录失败");
    println!("[recon] 样本目录: {}", dir.display());

    let cas = CasClient::new().expect("创建 CasClient 失败");
    let http = reqwest::Client::new();
    let token = token_or_login(&cas).await;
    let auth = token.auth_value();
    println!(
        "[凭据] ✅ SSO 桥 token 已取得：长度={} scheme={:?}（值不打印）",
        token.access_token.len(),
        token.token_type
    );
    let client = SynjonesClient::new(
        CasClient::new().expect("新建 client 失败"),
        None,
        Some(token.clone()),
    );

    // ---------- ① 级联到房间 ----------
    let steps = cascade_to_room(&client).await;

    // ---------- ② third_party（**产品 API 后端合成**，前端不参与） ----------
    let (third_party, tp_keys) = product_third_party(&client, &steps).await;
    println!(
        "[third_party] 长度={} 键=[{}]（含户号，值不打印）",
        third_party.len(),
        tp_keys
    );
    dump(&dir, "recharge_third_party.json", &json!({"keys": tp_keys}));

    // ---------- ③ 建单（1 元；副作用请求，不重试） ----------
    let mut failures: Vec<String> = Vec::new();
    let mut created: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    let (order_id, pay_info) =
        match recharge::create_order(&client, FEEITEM, TRANAMT, &steps).await {
            Ok(id) => {
                println!("[建单] ✅ 成功：orderid 长度={}（third_party 由后端合成）", id.len());
                created.push(id.clone());
                (id, Some("带后端合成的 third_party"))
            }
            Err(e) => {
                let msg = mask_digits(&e.to_string());
                println!("[建单] ❌ 失败：{msg}");
                dump(
                    &dir,
                    "recharge_create_failed.json",
                    &json!({"error": msg, "thirdPartyLen": third_party.len()}),
                );
                failures.push(format!("建单失败：{msg}"));
                (String::new(), None)
            }
        };
    if let Some(how) = pay_info {
        notes.push(format!("建单成功方式：{how}"));
    }

    // ---------- ④ 支付方式（批 D 的关键输入：真实 code 集合与 nopassword 值） ----------
    let mut account_probe: Option<PayMethod> = None;
    if !order_id.is_empty() {
        match recharge::fetch_pay_methods(&client, &order_id).await {
            Ok((order, methods)) => {
                println!(
                    "[支付方式] order.status={} tranamt={:?} payexpdate_present={} payList 条数={}",
                    order.status,
                    order.tranamt,
                    order.pay_exp_date.is_some(),
                    methods.len()
                );
                dump(
                    &dir,
                    "recharge_pay_methods.json",
                    &json!({
                        "order": {"status": order.status, "tranamt": order.tranamt,
                                  "payexpdatePresent": order.pay_exp_date.is_some()},
                        "methods": methods.iter().map(|m| json!({
                            "code": m.code, "payid": m.payid, "name": m.name,
                            "nopassword": m.nopassword, "remark": m.remark,
                        })).collect::<Vec<_>>(),
                    }),
                );
                for m in &methods {
                    println!(
                        "  └ code={} payid={} name={:?} nopassword={} remark={:?}",
                        m.code, m.payid, m.name, m.nopassword, m.remark
                    );
                }
                if methods.is_empty() {
                    failures.push("payList 里没有 ACCOUNT/ACCOUNTTSM 支付方式".to_string());
                }
                if let Some(bad) = methods
                    .iter()
                    .find(|m| !ACCOUNT_CODES.contains(&m.code.as_str()))
                {
                    failures.push(format!("过滤失效：出现非账户类支付方式 {}", bad.code));
                }
                // ★批 D 的关键输入（**2026-09-19 live 实测固化**）：450 的账户类渠道**只有一条**
                // `ACCOUNTTSM`（payid 64，展示名「电子账户」），且**需密码**（`nopassword` 是 JSON
                // 布尔 `false`，官方「免密」分支在此片区不成立）。
                if methods.len() != 1 || methods[0].code != "ACCOUNTTSM" {
                    failures.push(format!(
                        "支付方式集合与实测固化不符：{:?}（实测应为唯一的 ACCOUNTTSM）",
                        methods.iter().map(|m| m.code.as_str()).collect::<Vec<_>>()
                    ));
                }
                if methods.first().is_some_and(|m| m.nopassword) {
                    failures.push("450 的 ACCOUNTTSM 实测为需密码（nopassword=false），断言不符".to_string());
                }
                if methods.first().is_some_and(|m| m.payid != "64") {
                    failures.push(format!(
                        "payid 实测为 64，实际 {}",
                        methods[0].payid
                    ));
                }
                if methods.first().is_some_and(|m| m.name != "电子账户") {
                    failures.push(format!("展示名实测为「电子账户」，实际 {:?}", methods[0].name));
                }
                if let Some(m) = methods.iter().find(|m| m.code == "ACCOUNT") {
                    account_probe = Some(m.clone());
                } else {
                    account_probe = methods.first().cloned();
                }
            }
            Err(e) => {
                failures.push(format!("查支付方式失败：{}", mask_digits(&e.to_string())));
            }
        }
    }

    // ---------- ⑤ 账户 / 安全键盘（绝不打印 keys） ----------
    if let Some(pay) = account_probe.clone().filter(|_| !order_id.is_empty()) {
        match recharge::query_account(&client, &order_id, &pay, None).await {
            Ok((accounts, ccctypes, pad)) => {
                println!(
                    "[查账户] code={} 账号数={}（值不打印）ccctype={:?} passwordMap={}",
                    pay.code,
                    accounts.len(),
                    ccctypes,
                    pad.is_some()
                );
                if pay.nopassword {
                    notes.push(format!("{} 免密（nopassword=true）", pay.code));
                } else {
                    notes.push(format!("{} 需密码（nopassword=false）", pay.code));
                }
                // 需密码的支付方式：带账号再查一次，拿账户类型与键盘（官方 getAccounttype）
                let accountno = accounts.first().cloned();
                // 原始响应形状核对（只报类型/长度，绝不打值）：`passwordMap` 只在**带 accountno** 时下发
                if let Some(acc) = accountno.as_deref() {
                    let (_r, raw) = raw_probe(
                        "paystep:2 带 accountno（形状核对）",
                        http.post(format!("{BERSERKER_BASE}{EP_BLADE_PAY_RAW}"))
                            .header("synjones-auth", auth.clone())
                            .header("synAccessSource", "app")
                            .header("Content-Type", "application/x-www-form-urlencoded")
                            .form(&[
                                ("orderid", order_id.as_str()),
                                ("paystep", "2"),
                                ("paytype", pay.code.as_str()),
                                ("paytypeid", pay.payid.as_str()),
                                ("accountno", acc),
                            ]),
                    )
                    .await;
                    println!(
                        "    └ 原始 data 形状：accountno={} ccctype={} passwordMap={}",
                        shape_of(&raw["data"]["accountno"]),
                        shape_of(&raw["data"]["ccctype"]),
                        shape_of(&raw["data"]["passwordMap"])
                    );
                }
                let pad_seen = match recharge::query_account(
                    &client,
                    &order_id,
                    &pay,
                    accountno.as_deref(),
                )
                .await
                {
                    Ok((_, ccctypes2, pad2)) => {
                        println!(
                            "[查账户·带账号] ccctype={:?} passwordMap={}",
                            ccctypes2,
                            pad2.is_some()
                        );
                        pad2.or(pad)
                    }
                    Err(e) => {
                        println!("[查账户·带账号] 失败：{}", mask_digits(&e.to_string()));
                        pad
                    }
                };
                match &pad_seen {
                    Some(p) => {
                        println!(
                            "[安全键盘] ✅ 就绪：keys 个数={}（内容绝不打印）uuid 长度={}",
                            p.keys.len(),
                            p.uuid.len()
                        );
                        if p.keys.len() != 10 {
                            failures.push(format!("passwordMap 键位数 {} ≠ 10", p.keys.len()));
                        }
                        if p.uuid.is_empty() {
                            failures.push("passwordMap 缺 uuid".to_string());
                        }
                        // Debug 必须打码（红线：keys 不进日志）
                        if !format!("{p:?}").contains("已打码") {
                            failures.push("PasswordPad 的 Debug 未打码".to_string());
                        }
                    }
                    None => {
                        let msg = if pay.nopassword {
                            "免密支付方式不下发键盘（符合预期）".to_string()
                        } else {
                            "需密码但没拿到 passwordMap（到不了「密码键盘就绪」）".to_string()
                        };
                        println!("[安全键盘] ⚠️ {msg}");
                        if !pay.nopassword {
                            failures.push(msg);
                        }
                    }
                }
            }
            Err(e) => {
                let msg = mask_digits(&e.to_string());
                println!("[查账户] ❌ 失败：{msg}");
                failures.push(format!("查账户失败：{msg}"));
            }
        }
    }

    // ---------- ⑥ 清理：取消所有本次创建的订单，并复查状态 ----------
    println!("[清理] 本次创建订单 {} 笔", created.len());
    for id in &created {
        let r = recharge::cancel_order(&client, id).await;
        println!(
            "[清理] cancel_order orderid(长度 {}) → {}",
            id.len(),
            match &r {
                Ok(()) => "✅ 已取消".to_string(),
                Err(e) => format!("❌ {}", mask_digits(&e.to_string())),
            }
        );
        if let Err(e) = r {
            failures.push(format!("取消失败：{}", mask_digits(&e.to_string())));
        }
        // 复查：取消后再查状态（build 期 order.status 应为 0，取消后服务端不再返回该单/状态变化）
        match recharge::fetch_order_status(&client, id).await {
            Ok((order, _)) => {
                println!("[清理复查] order.status={}（0=待支付/1=已完成）", order.status);
                if order.status == 0 {
                    failures.push(format!(
                        "订单（长度 {}）取消后仍为待支付状态——请人工在官方页面核对",
                        id.len()
                    ));
                }
            }
            Err(e) => println!(
                "[清理复查] 该订单已不可查（服务端删除成功）：{}",
                mask_digits(&e.to_string())
            ),
        }
    }

    // ---------- ⑦ 提交支付的**无效参数**错误分支（绝不产生扣款） ----------
    // 说明：订单号用 "0"、payid 用不存在的值——服务端不可能据此扣任何人的钱；
    // 这一步只验证「submit_pay 会如实报错、且不会把跳转分支当成功」。
    if let Some(pay) = account_probe.clone() {
        let bogus = PayMethod {
            code: pay.code.clone(),
            payid: "0".to_string(),
            name: pay.name.clone(),
            nopassword: pay.nopassword,
            remark: None,
        };
        let r = recharge::submit_pay(&client, "0", &bogus, "0", "0", None, None).await;
        match r {
            Err(e) => println!(
                "[提交·无效参数] ✅ 如预期报错（无扣款可能）：{}",
                mask_digits(&e.to_string())
            ),
            Ok(()) => failures.push(
                "⚠️ submit_pay 用无效订单号居然返回成功——立即人工核查是否有异常订单".to_string(),
            ),
        }
    }

    // ---------- ⑧ 断言（清理已完成才到这里） ----------
    let summary = json!({
        "feeitem": FEEITEM, "room": ROOM, "tranamt": TRANAMT,
        "thirdPartyLen": third_party.len(),
        "thirdPartyKeys": tp_keys,
        "ordersCreated": created.len(),
        "notes": notes,
        "failures": failures,
    });
    dump(&dir, "recharge_live_summary.json", &summary);
    println!("[★小结] {}", serde_json::to_string_pretty(&summary).unwrap_or_default());

    assert!(failures.is_empty(), "live 检查未通过：{failures:?}");
    assert_eq!(created.len(), 1, "应恰好建单 1 笔（多出来说明有对照尝试成功了）");
    assert!(
        created.iter().all(|id| !id.is_empty()),
        "orderid 不应为空"
    );
}

/// 建单常量与金额下限的关系（离线断言，作为 live 参数的自检）：1 元必须落在 `retain_money..=maxmoney`。
/// 实测 450：`retain_money="1"`、`maxmoney="200"`、`daymaxmoney="500"`（见计划 §1.3 与 live 取证）。
#[test]
fn live_amount_is_within_feeitem_limits() {
    let amount: f64 = TRANAMT.parse().unwrap();
    assert_eq!(amount, 1.0);
    assert!((1.0..=200.0).contains(&amount), "1 元在 450 的下限/上限之间");
}

// ==================== 诊断探针（live，只读/不扣款） ====================

/// 一次原始请求的结论（只留状态与 code/msg/键名，值经打码）。
struct Raw {
    status: u16,
    code: Option<i64>,
    msg: String,
    data_keys: String,
}

async fn raw_probe(label: &str, req: reqwest::RequestBuilder) -> (Raw, Value) {
    let resp = req.send().await.expect("诊断请求失败（网络层）");
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let r = Raw {
        status,
        code: v.get("code").and_then(Value::as_i64),
        msg: v
            .get("msg")
            .or_else(|| v.get("message"))
            .and_then(Value::as_str)
            .map(mask_digits)
            .unwrap_or_default(),
        data_keys: keys_of(&v["data"]),
    };
    println!(
        "[诊断 {}] HTTP {} code={:?} msg={:?} data键=[{}]",
        label, r.status, r.code, r.msg, r.data_keys
    );
    (r, v)
}

/// **live 诊断**：查清两件实测未通的事——
/// ① 需密码的 `ACCOUNTTSM` 为何在 `paystep:2` 不下发 `passwordMap`（参数组合是否是原因）；
/// ② `POST /charge/order/deleteOrder` 恒 500 的原因（参数/方法/形态）。
///
/// 全程**只读**（`paystep:2` 是查询动作，不扣款、不建单）；仅当需要一笔订单做实验对象时建 **1 元** 单，
/// 并在末尾用所有可用形态尝试清理。跑法：
/// `cargo test -p campus-synjones -- --ignored recharge_diag_live --nocapture`
#[tokio::test]
#[ignore = "live：需校园网 + 真实凭据；诊断用（可能建 1 元订单并尽力清理）"]
async fn recharge_diag_live() {
    let dir = recon_dir();
    std::fs::create_dir_all(&dir).expect("创建 recon 目录失败");
    let cas = CasClient::new().expect("创建 CasClient 失败");
    let http = reqwest::Client::new();
    let token = token_or_login(&cas).await;
    let auth = token.auth_value();
    let client = SynjonesClient::new(
        CasClient::new().expect("新建 client 失败"),
        None,
        Some(token.clone()),
    );
    let pay_url = format!("{BERSERKER_BASE}/blade-pay/pay");
    let info_url = format!("{BERSERKER_BASE}/charge/pay/getpayinfo");

    // ---------- ① 遗留未支付订单能否列出（清理前的现场盘点） ----------
    for (label, q) in [
        ("personal_data 无参", ""),
        ("personal_data status=0", "?status=0"),
        ("personal_data status=1", "?status=1"),
        ("personal_data status=2", "?status=2"),
        ("personal_data 分页", "?status=0&current=1&size=10"),
        ("personal_data status=0&payid=64", "?status=0&payid=64"),
    ] {
        let (_r, v) = raw_probe(
            label,
            http.get(format!("{BERSERKER_BASE}/charge/order/personal_data{q}"))
                .header("synjones-auth", auth.clone())
                .header("synAccessSource", "app"),
        )
        .await;
        dump(&dir, "diag_personal_data.json", &v);
    }
    // POST / JSON 形态也试（GET 形态对 status=0 恒 500 → 留下「遗留未支付订单」这条红线的取证）
    for (label, req) in [
        (
            "personal_data POST form status=0",
            http.post(format!("{BERSERKER_BASE}/charge/order/personal_data"))
                .header("Content-Type", "application/x-www-form-urlencoded")
                .form(&[("status", "0")]),
        ),
        (
            "personal_data POST json status=0",
            http.post(format!("{BERSERKER_BASE}/charge/order/personal_data"))
                .header("Content-Type", "application/json")
                .body(r#"{"status":0}"#),
        ),
    ] {
        let (_r, v) = raw_probe(
            label,
            req.header("synjones-auth", auth.clone())
                .header("synAccessSource", "app"),
        )
        .await;
        println!("    └ data 形状={}", shape_of(&v["data"]));
    }

    // ---------- ② 建一笔 1 元订单做实验对象（只建一笔；末尾尽力清理） ----------
    let steps = cascade_to_room(&client).await;
    let (third_party, tp_keys) = product_third_party(&client, &steps).await;
    println!("[诊断] third_party 长度={} 键=[{tp_keys}]", third_party.len());
    let order_id = recharge::create_order(&client, FEEITEM, TRANAMT, &steps)
        .await
        .expect("诊断需要一笔订单作为实验对象（建单失败）");
    println!("[诊断] 实验订单 orderid 长度={}（值不打印）", order_id.len());

    // 支付方式（拿到 paytype/paytypeid）
    let (_order, methods) = recharge::fetch_pay_methods(&client, &order_id)
        .await
        .expect("取支付方式失败");
    let pay = methods.first().cloned().expect("payList 为空");
    println!(
        "[诊断] 用 code={} payid={} nopassword={} 做参数矩阵",
        pay.code, pay.payid, pay.nopassword
    );

    // ---------- ③ paystep:2 参数矩阵：passwordMap 什么时候才下发 ----------
    // 先取账号列表（不带 accountno）
    let (accounts, ccctypes, pad0) = recharge::query_account(&client, &order_id, &pay, None)
        .await
        .expect("查账户（不带 accountno）失败");
    println!(
        "[诊断] 不带 accountno：账号数={} ccctype={:?} passwordMap={}",
        accounts.len(),
        ccctypes,
        pad0.is_some()
    );
    let accountno = accounts.first().cloned().unwrap_or_default();
    let ccctype = ccctypes.first().cloned().unwrap_or_default();

    // 各种组合（全部只读）
    let variants: Vec<(&str, Vec<(&str, String)>)> = vec![
        ("基线（+accountno）", vec![("accountno", accountno.clone())]),
        (
            "+accountno +ccctype",
            vec![
                ("accountno", accountno.clone()),
                ("ccctype", ccctype.clone()),
            ],
        ),
        (
            "+accountno +ccctype +isWX=0",
            vec![
                ("accountno", accountno.clone()),
                ("ccctype", ccctype.clone()),
                ("isWX", "0".to_string()),
            ],
        ),
        (
            "+accountno +userAgent",
            vec![
                ("accountno", accountno.clone()),
                ("userAgent", "wechat-mp".to_string()),
            ],
        ),
        ("+accountno +ccctype +uuid", {
            vec![
                ("accountno", accountno.clone()),
                ("ccctype", ccctype.clone()),
                ("uuid", String::new()),
            ]
        }),
        // 判定点：`SynjonesClient::post_form` 会强制往 body 塞 `synAccessSource=app`，
        // 这几档用来判定「body 里多这个字段是否会让服务端不下发 passwordMap」。
        (
            "+accountno +body synAccessSource",
            vec![
                ("accountno", accountno.clone()),
                ("synAccessSource", "app".to_string()),
            ],
        ),
    ];
    for (label, extra) in variants {
        let mut form: Vec<(&str, String)> = vec![
            ("orderid", order_id.clone()),
            ("paystep", "2".to_string()),
            ("paytype", pay.code.clone()),
            ("paytypeid", pay.payid.clone()),
        ];
        form.extend(extra);
        let (_r, v) = raw_probe(
            label,
            http.post(&pay_url)
                .header("synjones-auth", auth.clone())
                .header("synAccessSource", "app")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .form(&form),
        )
        .await;
        // 只报 data 的键名与 passwordMap 是否存在，绝不打印键位内容
        println!(
            "    └ data=[{}] 有 passwordMap={} 有 accountno={} 有 ccctype={}",
            keys_of(&v["data"]),
            v["data"].get("passwordMap").is_some(),
            v["data"].get("accountno").is_some(),
            v["data"].get("ccctype").is_some()
        );
        // passwordMap 的**内层形状**（只到类型/长度，绝不打字符）：用来判定是「服务端没下发」还是
        // 「解析太严」（例如值是字符串而非数组、或键位数不是 10）
        let inner = v["data"]["passwordMap"]
            .as_object()
            .map(|o| {
                o.values()
                    .map(|keys| {
                        let arr = keys
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .map(|k| match k {
                                        Value::String(s) => format!("str{}", s.chars().count()),
                                        other => shape_of(other),
                                    })
                                    .collect::<Vec<_>>()
                                    .join(",")
                            })
                            .unwrap_or_else(|| shape_of(keys));
                        format!("[{}]", arr)
                    })
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .unwrap_or_default();
        println!(
            "    └ passwordMap 存在={} 内层={}",
            v["data"].get("passwordMap").is_some(),
            inner
        );
        dump(&dir, "diag_paystep2.json", &v);
    }

    // ---------- ④ deleteOrder 形态矩阵 ----------
    let del_url = format!("{BERSERKER_BASE}/charge/order/deleteOrder");
    let del_variants: Vec<(&str, reqwest::RequestBuilder)> = vec![
        (
            "POST form orderid",
            http.post(&del_url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .form(&[("orderid", order_id.as_str())]),
        ),
        (
            "POST query ?orderid=",
            http.post(format!("{del_url}?orderid={order_id}")),
        ),
        (
            "GET query ?orderid=",
            http.get(format!("{del_url}?orderid={order_id}")),
        ),
        (
            "POST json body",
            http.post(&del_url)
                .header("Content-Type", "application/json")
                .body(format!(r#"{{"orderid":"{order_id}"}}"#)),
        ),
        (
            "POST form orderid+payid",
            http.post(&del_url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .form(&[("orderid", order_id.as_str()), ("payid", pay.payid.as_str())]),
        ),
        (
            "POST form orderid（数字形态）",
            http.post(&del_url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .form(&[("orderid", order_id.as_str())]),
        ),
    ];
    let mut del_results: Vec<String> = Vec::new();
    for (label, req) in del_variants {
        let (r, _v) = raw_probe(
            label,
            req.header("synjones-auth", auth.clone())
                .header("synAccessSource", "app"),
        )
        .await;
        del_results.push(format!("{label}: code={:?} msg={:?}", r.code, r.msg));
    }

    // ---------- ⑤ 清理复查 ----------
    let after = recharge::fetch_order_status(&client, &order_id).await;
    match after {
        Ok((o, _)) => println!("[诊断复查] 实验订单 order.status={}", o.status),
        Err(e) => println!("[诊断复查] 实验订单已不可查：{}", mask_digits(&e.to_string())),
    }
    let info = raw_probe(
        "getpayinfo 实验订单",
        http.get(format!("{info_url}?orderid={order_id}"))
            .header("synjones-auth", auth.clone())
            .header("synAccessSource", "app"),
    )
    .await;
    dump(
        &dir,
        "recharge_diag_summary.json",
        &json!({
            "orderIdLen": order_id.len(),
            "payCode": pay.code, "payId": pay.payid,
            "accountsCount": accounts.len(), "ccctypes": ccctypes,
            "deleteOrderResults": del_results,
            "finalOrderQuery": {"code": info.0.code, "msg": info.0.msg},
        }),
    );
    println!("[诊断] deleteOrder 各形态：{del_results:?}");
}
