//! 一卡通「全功能复刻」live 探针（一次性、**严格只读**）。
//!
//! # 本测试只回答一个问题
//!
//! 学校系统里有没有可以直接拉到的「历史用电 / 缴费记录 / 账单」数据？
//!
//! # 候选端点不是猜的（来源：2026-09-19 抓官方前端 bundle 反查）
//!
//! - `/charge-pc/js/app.*.js`（PC 账单页；请求拦截器带 `synjones-auth`，来源写死 `pc`）
//! - `/charge-app/static/js/app.*.js`（App 版 H5，`systemType="pay"`，`baseApi=origin+"/charge"`；
//!   拦截器对**每个**请求加 `Authorization: Basic charge:charge_secret` + `synjones-auth: bearer <token>`）
//!
//! 从这两份 bundle 里逐字提取的真实端点（本测试的候选清单即由它们构成）：
//! `/charge/turnover/app_account`、`/charge/turnover/app_totalAccount`、
//! `/charge/turnover/appAccountDetail`、`/charge/turnover/mouthAccount`、`/charge/turnover/pie_account`、
//! `/charge/turnover/personal_data`、`/charge/feeitem/getRechargeRecord`、`/charge/order/oldOrdersData`、
//! `/charge/order/threeExpen_account`、`/charge/receivable/personal_data`、`/charge/billapply/personal_data`、
//! `/charge/orderdetail/personal_data`、`/charge/order/personal_data`。
//! 另有派单里给出的「合理变体」（`/charge/order/list` 等）一并在表里探，用实测把它们从「可能存在」
//! 降级为「确定不存在」。
//!
//! # 红线（本测试**只读**）
//!
//! - 只发 **GET** 与 `/charge/feeitem/getThirdData`（只读级联查询，零副作用；建单/支付/删单一律不碰）。
//! - **禁调**：`/blade-pay/pay`、`/charge/order/deleteOrder`、`/charge/order/thirdOrder`、
//!   `/charge/order/addRefundOrder`、`/charge/sceneBind/add`、`/charge/receivable/updateReceivable`、
//!   `/charge/billapply/update` —— 这些会建单/改学校数据，一个都不探。
//! - 不打印 token/账号/户号/姓名/订单号；样本写仓库外 `%TEMP%/campushub-m4-probe/`，写盘前脱敏。
//!
//! 复跑：`cargo test -p campus-synjones -- --ignored m4_history_probe_live --nocapture`

use campus_auth::captcha::{solve, KaptchaTemplates};
use campus_auth::cas::CasClient;
use campus_auth::rsa::rsa_encrypt_hex;
use campus_synjones::sso::{default_target_url, sso_token};
use campus_synjones::{BERSERKER_BASE, SYN_ACCESS_SOURCE};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// App 版（`/charge-app/`）拦截器逐字出现的 Basic 头（= `charge:charge_secret`，公开硬编码常量）。

// ==================== recon 目录 / 脱敏（照抄 recharge_probe_live.rs 的约定） ====================

fn recon_dir() -> PathBuf {
    std::env::var("CAMPUS_HUB_RECON_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("campushub-m4-probe"))
}

/// 连续 ≥6 位数字打码（orderid/turnoverid 类兜底）。
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

/// **精确键名**打码表（只打叶子 PII 键，**不打容器键**）。
///
/// 教训（第一轮实测）：早先按子串匹配，`accountList` / `accountTotal` 这类容器键含 `account`
/// 被判敏感，整个数组被替换成 `"***"` —— 证据直接报废，只剩 `count` 可看。故改为精确匹配。
const SENSITIVE_KEYS: &[&str] = &[
    "account", "accountno", "account_no", "cardno", "card_num", "yktcard", "sno", "userno",
    "username", "loginname", "login_name", "realname", "real_name", "name", "phone", "mobile",
    "mobilephone", "idcard", "idno", "email", "openid", "nickname", "avatar", "password", "pwd",
    "token", "auth", "cookie", "ticket", "orderid", "order_id", "orderno", "out_trade_no",
    "turnoverid", "payid", "paycard", "transferoutid", "original_turnoverid", "original_orderid",
    "third_party", "third_orderid", "batchid", "batch_id", "userbean", "paybean", "operatorbean",
    "bankcard", "verifyfname", "csrq", "sfzh",
];

fn is_sensitive_key(k: &str) -> bool {
    SENSITIVE_KEYS.contains(&k.to_ascii_lowercase().as_str())
}

/// 字符串值里的**连续 ≥6 位数字串**一律打码（订单号/户号/房间 uid 兜底）。
/// 日期 `2026-09-19 16:40:11` 的数字段都 <6 位，不受影响；金额同样不受影响。
fn mask_digit_runs(s: &str) -> String {
    let (mut out, mut run) = (String::new(), String::new());
    for c in s.chars() {
        if c.is_ascii_digit() {
            run.push(c);
            continue;
        }
        if run.chars().count() >= 6 {
            out.push_str("***");
        } else {
            out.push_str(&run);
        }
        run.clear();
        out.push(c);
    }
    if run.chars().count() >= 6 {
        out.push_str("***");
    } else {
        out.push_str(&run);
    }
    out
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
        Value::String(s) => Value::String(mask_digit_runs(s)),
        other => other.clone(),
    }
}

fn dump(dir: &Path, name: &str, v: &Value) {
    let text = serde_json::to_string_pretty(&redact_json(v)).unwrap_or_default();
    std::fs::write(dir.join(name), text).ok();
}

// ==================== 凭据（只读、只在内存；三来源与 synjones_live.rs 同链） ====================

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
    dpapi_unprotect_b64(b64).ok().filter(|t| !t.trim().is_empty())
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

fn walk(prefix: &str, v: &Value, arrays: &mut Vec<String>, record_keys: &mut Vec<String>, depth: usize) {
    match v {
        Value::Object(o) => {
            for (k, val) in o {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                match val {
                    Value::Array(a) => {
                        arrays.push(format!("{p}={}", a.len()));
                        if !a.is_empty() && record_keys.is_empty() {
                            if let Some(Value::Object(first)) = a.first() {
                                *record_keys = first.keys().cloned().collect();
                            }
                        }
                        if depth < 3 {
                            for item in a.iter().take(1) {
                                walk(&p, item, arrays, record_keys, depth + 1);
                            }
                        }
                    }
                    Value::Object(_) if depth < 3 => walk(&p, val, arrays, record_keys, depth + 1),
                    _ => {}
                }
            }
        }
        Value::Array(a) => {
            arrays.push(format!("{prefix}={}", a.len()));
        }
        _ => {}
    }
}

/// 有载荷判据：任一数组非空，或 `count`/`total` 类计数 > 0，或顶层有业务键（非信封键）。
fn id_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

// ==================== 通用探测（带参数，只读 GET） ====================

/// 探测时允许打印**值**的字段白名单（只含金额/状态/枚举类；人名、账号、卡号一律不打）。
const NUM_KEYS: &[&str] = &[
    "balance",
    "elec_accamt",
    "accamt",
    "db_balance",
    "unsettle_amount",
    "daycostamt",
    "daycostlimit",
    "nonpwdlimit",
    "singlelimit",
    "autotrans_flag",
    "autotrans_amt",
    "autotrans_limite",
    "lostflag",
    "acc_status",
    "freezeflag",
    "status",
    "type",
    "payacc",
    "expenses",
    "income",
    "amount",
    "typeId",
    "tranamt",
    "typeFrom",
    "count",
    "total",
    "retcode",
];

/// 递归打印白名单字段的键与值（值超 24 字符截断，仍走敏感词打码）。
fn show_numbers(prefix: &str, v: &Value, out: &mut Vec<String>, depth: usize) {
    if depth > 4 {
        return;
    }
    match v {
        Value::Object(o) => {
            for (k, val) in o {
                let p = format!("{prefix}.{k}");
                if NUM_KEYS.contains(&k.as_str()) && !val.is_object() && !val.is_array() {
                    let s = match val {
                        Value::String(s) => mask_digits(&s.chars().take(24).collect::<String>()),
                        other => other.to_string(),
                    };
                    out.push(format!("{p}={s}"));
                }
                show_numbers(&p, val, out, depth + 1);
            }
        }
        Value::Array(a) => {
            for (i, val) in a.iter().take(3).enumerate() {
                show_numbers(&format!("{prefix}[{i}]"), val, out, depth + 1);
            }
        }
        _ => {}
    }
}

async fn probe(
    http: &reqwest::Client,
    auth: &str,
    label: &str,
    path: &str,
    params: &[(&str, String)],
) -> Value {
    let mut rb = http
        .get(format!("{BERSERKER_BASE}{path}"))
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
        .header("synAccessSource", SYN_ACCESS_SOURCE)
        .header("synjones-auth", auth)
        .query(&[("synAccessSource", SYN_ACCESS_SOURCE)]);
    for (k, v) in params {
        rb = rb.query(&[(*k, v.as_str())]);
    }
    let resp = rb.send().await.expect("探测请求失败（网络层）");
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);

    let mut arrays = Vec::new();
    let mut record_keys = Vec::new();
    walk("", &v, &mut arrays, &mut record_keys, 0);
    let mut nums = Vec::new();
    show_numbers("", &v, &mut nums, 0);

    println!(
        "\n[{}] HTTP {} code={} msg={:?} bodyLen={}",
        label,
        status,
        v.get("code")
            .and_then(Value::as_i64)
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".to_string()),
        v.get("msg")
            .or_else(|| v.get("message"))
            .and_then(Value::as_str)
            .map(mask_digits)
            .unwrap_or_default(),
        body.len()
    );
    println!(
        "  顶层键: {:?}",
        v.as_object()
            .map(|o| o.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    );
    println!("  数组: {:?}", arrays);
    println!("  首记录键: {:?}", record_keys);
    if !nums.is_empty() {
        println!("  白名单字段: {:?}", nums);
    }
    if v.is_null() && !body.is_empty() {
        println!("  (非 JSON) 开头: {}", mask_digit_runs(&body.chars().take(120).collect::<String>()));
    }
    v
}

// ==================== 主流程 ====================

#[tokio::test]
#[ignore = "live：需校园网与真实凭据（严格只读：只发 GET，写路径一律不碰）"]
async fn ecard_features_probe_live() {
    let t_start = Instant::now();
    let dir = recon_dir();
    std::fs::create_dir_all(&dir).expect("创建 recon 目录失败");
    println!("[recon] 样本目录: {}", dir.display());

    let cas = CasClient::new().expect("创建 CasClient 失败");
    let http = cas.http_client().clone();

    // ---------- 取 token（TGT 优先，回落账密） ----------
    let mut token = None;
    let mut tgt = None;
    if let Some(t) = tgt_from_app_session() {
        println!("[凭据] 来源=session.json(DPAPI TGT) 长度={}", t.len());
        match sso_token(&cas, &t, &default_target_url()).await {
            Ok(tok) => {
                token = Some(tok);
                tgt = Some(t);
            }
            Err(e) => println!("[凭据] 本机 TGT 换票失败（{e}）→ 回落账密登录"),
        }
    } else {
        println!("[凭据] session.json 无可用 TGT");
    }
    let token = match token {
        Some(t) => t,
        None => {
            let (username, password, src) = match creds_from_accounts_file() {
                Some((u, p)) => (u, p, "accounts.json(DPAPI 密码)"),
                None => {
                    let (u, p) = creds_from_env_file()
                        .expect("无可用凭据（accounts.json / CAMPUS_HUB_CREDS 都不可用）");
                    (u, p, "CAMPUS_HUB_CREDS")
                }
            };
            println!("[凭据] 来源={src} 用户长度={}", username.len());
            let t = cas_login(&cas, &username, &password).await;
            let tok = sso_token(&cas, &t, &default_target_url())
                .await
                .expect("账密登录取得的新 TGT 应能换到 token");
            tgt = Some(t);
            tok
        }
    };
    let auth = token.auth_value();
    println!(
        "[凭据] ✅ token 已取得：access_token 长度={} token_type={:?}（值不打印）",
        token.access_token.len(),
        token.token_type
    );

    let mut bodies = serde_json::Map::new();

    // ==================== A. 卡与账户（只读） ====================
    let cards = probe(&http, &auth, "A1 getCampusCards", "/berserker-app/ykt/tsm/getCampusCards", &[]).await;
    bodies.insert("A1_getCampusCards".into(), redact_json(&cards));
    let card0 = cards
        .get("data")
        .and_then(|d| d.get("card"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(Value::Null);
    let account0 = id_of(card0.get("account").unwrap_or(&Value::Null)).unwrap_or_default();
    println!("  └ 取到 {} 张卡，account 长度={}", cards.get("data").and_then(|d| d.get("card")).and_then(Value::as_array).map(|a| a.len()).unwrap_or(0), account0.len());
    if !account0.is_empty() {
        let d = probe(
            &http,
            &auth,
            "A2 queryCard?account=",
            "/berserker-app/ykt/tsm/queryCard",
            &[("account", account0.clone())],
        )
        .await;
        bodies.insert("A2_queryCard_by_account".into(), redact_json(&d));
    }
    let d = probe(
        &http,
        &auth,
        "A3 queryCard?scene=recharge",
        "/berserker-app/ykt/tsm/queryCard",
        &[("scene", "recharge".to_string())],
    )
    .await;
    bodies.insert("A3_queryCard_scene_recharge".into(), redact_json(&d));

    let d = probe(&http, &auth, "A4 queryCardByTransfer", "/berserker-app/ykt/tsm/queryCardByTransfer", &[]).await;
    bodies.insert("A4_queryCardByTransfer".into(), redact_json(&d));

    let d = probe(&http, &auth, "A5 queryCurrentCard（可绑卡）", "/berserker-app/ykt/tsm/queryCurrentCard", &[]).await;
    bodies.insert("A5_queryCurrentCard".into(), redact_json(&d));

    // ==================== B. 账单统计三件套（参数是否生效，对照实验） ====================
    let cur_year = "2026";
    let d = probe(
        &http,
        &auth,
        "B1 count 无参数",
        "/berserker-search/statistics/turnover/count",
        &[],
    )
    .await;
    bodies.insert("B1_count_no_params".into(), redact_json(&d));

    let d = probe(
        &http,
        &auth,
        "B2 count timeFrom=2026-01-01..2026-12-31",
        "/berserker-search/statistics/turnover/count",
        &[
            ("timeFrom", format!("{cur_year}-01-01")),
            ("timeTo", format!("{cur_year}-12-31")),
        ],
    )
    .await;
    bodies.insert("B2_count_2026".into(), redact_json(&d));

    let d = probe(
        &http,
        &auth,
        "B3 count timeFrom=2020-01-01..2020-01-31（对照：若与 B2 相同则参数不生效）",
        "/berserker-search/statistics/turnover/count",
        &[
            ("timeFrom", "2020-01-01".to_string()),
            ("timeTo", "2020-01-31".to_string()),
        ],
    )
    .await;
    bodies.insert("B3_count_2020_01".into(), redact_json(&d));

    for (label, date_type, stat_date, ty) in [
        ("B4 sum/user 月-按日 支出", "month", "day", "2"),
        ("B5 sum/user 月-按日 收入", "month", "day", "1"),
        ("B6 sum/user 年-按月 支出", "year", "month", "2"),
    ] {
        let d = probe(
            &http,
            &auth,
            label,
            "/berserker-search/statistics/turnover/sum/user",
            &[
                ("dateStr", if date_type == "month" { "2026-09".to_string() } else { "2026".to_string() }),
                ("dateType", date_type.to_string()),
                ("statisticsDateStr", stat_date.to_string()),
                ("type", ty.to_string()),
            ],
        )
        .await;
        bodies.insert(format!("B_{label}"), redact_json(&d));
    }

    let d = probe(
        &http,
        &auth,
        "B7 分类饼图 type=2",
        "/berserker-search/statistics/turnover",
        &[
            ("type", "2".to_string()),
            ("timeFrom", format!("{cur_year}-01-01")),
            ("timeTo", format!("{cur_year}-12-31")),
        ],
    )
    .await;
    bodies.insert("B7_assort_expense".into(), redact_json(&d));

    let d = probe(
        &http,
        &auth,
        "B8 turnoverType 分类字典",
        "/berserker-search/search/turnoverType",
        &[],
    )
    .await;
    bodies.insert("B8_turnoverType".into(), redact_json(&d));

    // ==================== C. 流水形态（筛选/搜索/按单查） ====================
    let d = probe(
        &http,
        &auth,
        "C1 turnover size=2 type=2（支出）",
        "/berserker-search/search/personal/turnover",
        &[("size", "2".into()), ("current", "1".into()), ("type", "2".into())],
    )
    .await;
    bodies.insert("C1_turnover_expense".into(), redact_json(&d));
    let t0 = d
        .get("data")
        .and_then(|x| x.get("records"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(Value::Null);
    let order_id = id_of(t0.get("orderId").unwrap_or(&Value::Null))
        .or_else(|| id_of(t0.get("orderid").unwrap_or(&Value::Null)))
        .unwrap_or_default();
    println!("  └ 流水首条 orderId 是否可取: {}", !order_id.is_empty());

    if !order_id.is_empty() {
        let d = probe(
            &http,
            &auth,
            "C2 turnover?orderId=（账单详情）",
            "/berserker-search/search/personal/turnover",
            &[("orderId", order_id.clone())],
        )
        .await;
        bodies.insert("C2_turnover_by_orderid".into(), redact_json(&d));
    }

    let d = probe(
        &http,
        &auth,
        "C3 turnover 带 account + type=2（卡包消费记录口径）",
        "/berserker-search/search/personal/turnover",
        &[
            ("size", "2".into()),
            ("current", "1".into()),
            ("type", "2".into()),
            ("account", account0.clone()),
        ],
    )
    .await;
    bodies.insert("C3_turnover_by_account".into(), redact_json(&d));

    let d = probe(
        &http,
        &auth,
        "C4 turnover info=食堂（搜索）+ highlightFieldsClass",
        "/berserker-search/search/personal/turnover",
        &[
            ("size", "2".into()),
            ("current", "1".into()),
            ("info", "食堂".into()),
            ("highlightFieldsClass", "text-primary".into()),
        ],
    )
    .await;
    bodies.insert("C4_turnover_search_info".into(), redact_json(&d));

    let d = probe(
        &http,
        &auth,
        "C5 turnover typeId 分类筛选（id 取自 B8 首项由人工填；此处传 1 探形态）",
        "/berserker-search/search/personal/turnover",
        &[
            ("size", "2".into()),
            ("current", "1".into()),
            ("typeId", "1".into()),
        ],
    )
    .await;
    bodies.insert("C5_turnover_by_typeId".into(), redact_json(&d));

    let d = probe(
        &http,
        &auth,
        "C6 turnover sortFields=tranamt&sortType=desc（统计页口径）",
        "/berserker-search/search/personal/turnover",
        &[
            ("size", "2".into()),
            ("current", "1".into()),
            ("sortFields", "tranamt".into()),
            ("sortType", "desc".into()),
        ],
    )
    .await;
    bodies.insert("C6_turnover_sorted".into(), redact_json(&d));

    // ==================== D. 配置与安全键盘 ====================
    let d = probe(
        &http,
        &auth,
        "D1 frontInfo?type=pc（聚合配置：功能开关来源）",
        "/berserker-app/frontInfo",
        &[("type", "pc".to_string())],
    )
    .await;
    bodies.insert("D1_frontInfo_pc".into(), redact_json(&d));

    let d = probe(
        &http,
        &auth,
        "D2 frontInfo?type=app（本客户端 app 口径是否同样可用）",
        "/berserker-app/frontInfo",
        &[("type", "app".to_string())],
    )
    .await;
    bodies.insert("D2_frontInfo_app".into(), redact_json(&d));

    let d = probe(
        &http,
        &auth,
        "D3 安全键盘 type=Number（只验结构，不打印映射内容）",
        "/berserker-secure/keyboard",
        &[("type", "Number".to_string())],
    )
    .await;
    bodies.insert("D3_keyboard_number".into(), redact_json(&d));

    let d = probe(
        &http,
        &auth,
        "D4 安全键盘 type=Standard",
        "/berserker-secure/keyboard",
        &[("type", "Standard".to_string())],
    )
    .await;
    bodies.insert("D4_keyboard_standard".into(), redact_json(&d));

    // ==================== E. 应用清单（宫格入口开关来源） ====================
    let d = probe(
        &http,
        &auth,
        "E1 getAllApps?platType=pc&userType=user&websiteRequired=false",
        "/berserker-app/app/getAllApps",
        &[
            ("platType", "pc".into()),
            ("userType", "user".into()),
            ("websiteRequired", "false".into()),
        ],
    )
    .await;
    bodies.insert("E1_getAllApps".into(), redact_json(&d));

    // ==================== F. 一卡通充值片区（frontConfig.recharge = 401） ====================
    // 401「慧新易校一卡通充值」实测 status=1、impl_interface 为空、flag=1100000000（无级联）。
    // 下面全是**只读**：片区配置 + 账单 + 级联查询（官方语义纯查询，无副作用）。
    let d = probe(
        &http,
        &auth,
        "F1 singleFeeitem?feeitemid=401（充值金额配置）",
        "/charge/feeitem/singleFeeitem",
        &[("feeitemid", "401".to_string())],
    )
    .await;
    bodies.insert("F1_singleFeeitem_401".into(), redact_json(&d));

    let d = probe(
        &http,
        &auth,
        "F2 turnover/app_account?feeitemid=401（一卡通充值账单）",
        "/charge/turnover/app_account",
        &[
            ("feeitemid", "401".to_string()),
            ("size", "2".to_string()),
            ("current", "1".to_string()),
        ],
    )
    .await;
    bodies.insert("F2_app_account_401".into(), redact_json(&d));

    {
        let resp = http
            .post(format!("{BERSERKER_BASE}/charge/feeitem/getThirdData"))
            .header("synAccessSource", SYN_ACCESS_SOURCE)
            .header("synjones-auth", &auth)
            .form(&[
                ("synAccessSource", SYN_ACCESS_SOURCE),
                ("feeitemid", "401"),
                ("type", "select"),
                ("level", "0"),
            ])
            .send()
            .await
            .expect("getThirdData(401) 请求失败");
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        let mut arrays = Vec::new();
        let mut record_keys = Vec::new();
        walk("", &v, &mut arrays, &mut record_keys, 0);
        println!(
            "\n[F3 getThirdData(401, select, level=0)] HTTP {} code={} bodyLen={}",
            status,
            v.get("code").and_then(Value::as_i64).unwrap_or(-1),
            body.len()
        );
        println!(
            "  顶层键: {:?}",
            v.as_object()
                .map(|o| o.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
        );
        println!("  数组: {:?}", arrays);
        println!("  首记录键: {:?}", record_keys);
        println!(
            "  map 键: {:?}",
            v.get("map")
                .and_then(Value::as_object)
                .map(|o| o.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
        );
        bodies.insert("F3_getThirdData_401".into(), redact_json(&v));
    }

    // ---------- 落盘（已脱敏） ----------
    dump(
        &dir,
        "ecard_features_probe.json",
        &Value::Object(bodies),
    );
    println!("\n[完成] 用时 {:?}；样本目录 {}", t_start.elapsed(), dir.display());
}
