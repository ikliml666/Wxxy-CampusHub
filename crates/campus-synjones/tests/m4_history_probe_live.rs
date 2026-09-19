//! M4 **历史数据可得性** live 探针（一次性、**严格只读**）。
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
use campus_synjones::charge::{list_feeitems, EP_FEEITEM, EP_GET_THIRD_DATA};
use campus_synjones::client::Envelope;
use campus_synjones::sso::{default_target_url, sso_token};
use campus_synjones::{SynjonesClient, BERSERKER_BASE, SYN_ACCESS_SOURCE};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// App 版（`/charge-app/`）拦截器逐字出现的 Basic 头（= `charge:charge_secret`，公开硬编码常量）。
const BASIC_CHARGE: &str = "Basic Y2hhcmdlOmNoYXJnZV9zZWNyZXQ=";

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

// ==================== 探针结果 ====================

/// 一次 GET 探测的可打印事实（值一律不打，只留键名/条数/文案）。
struct Hit {
    label: String,
    path: String,
    status: u16,
    code: Option<i64>,
    msg: String,
    /// 顶层键名（只键名）。
    keys: Vec<String>,
    /// 遍历到的数组：`键路径=条数`（深度 ≤3）。
    arrays: Vec<String>,
    /// 首个非空数组的首条记录**键名**（判断「能否给出时间序列」的依据）。
    record_keys: Vec<String>,
    /// 是否含非空有效载荷。
    has_payload: bool,
    /// 原始响应体字节数（判定「空体 / HTML 壳 / JSON」用）。
    body_len: usize,
    /// 非 JSON 响应的开头 80 字符（已打码；JSON 时为空）。
    body_head: String,
}

impl Hit {
    fn verdict(&self) -> String {
        if self.status == 404 {
            return "不可用（HTTP 404 路径不存在）".to_string();
        }
        if self.status != 200 {
            return format!("不可用（HTTP {}）", self.status);
        }
        match self.code {
            Some(200) => {
                if self.has_payload {
                    let n = self
                        .arrays
                        .iter()
                        .find(|s| !s.ends_with("=0"))
                        .cloned()
                        .unwrap_or_default();
                    format!("可用（{n}）")
                } else {
                    "空 data".to_string()
                }
            }
            Some(c) => format!("不可用（code={c} msg={:?}）", self.msg),
            None => {
                if self.has_payload {
                    format!("可用（无 code 字段，载荷 {}", self.arrays.join(","))
                } else if self.body_len == 0 {
                    "不可用（HTTP 200 空体）".to_string()
                } else {
                    format!(
                        "不可用（HTTP 200 非 JSON，{} 字节，开头 {:?}）",
                        self.body_len, self.body_head
                    )
                }
            }
        }
    }

    fn line(&self) -> String {
        format!(
            "{:<58} → HTTP {} → {} | 顶层键=[{}] | 数组=[{}] | 首条字段=[{}]",
            self.label,
            self.status,
            self.verdict(),
            self.keys.join(","),
            self.arrays.join(","),
            self.record_keys.join(","),
        )
    }
}

/// 递归收集数组的「键路径=条数」（深度 ≤3），并取首个非空数组首条记录的键名。
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
fn payload_of(v: &Value) -> bool {
    let mut arrays = Vec::new();
    let mut record_keys = Vec::new();
    walk("", v, &mut arrays, &mut record_keys, 0);
    let has_nonempty_array = arrays.iter().any(|s| !s.ends_with("=0"));
    let envelope = ["code", "message", "msg", "success"];
    let has_business_key = v
        .as_object()
        .map(|o| o.keys().any(|k| !envelope.contains(&k.as_str())))
        .unwrap_or(false);
    let counter = ["count", "total", "accountTotal"]
        .iter()
        .filter_map(|k| {
            let x = v.get(*k)?;
            x.as_i64()
                .or_else(|| x.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
        })
        .any(|n| n > 0);
    has_nonempty_array || counter || (has_business_key && arrays.is_empty())
}

async fn get_probe(
    http: &reqwest::Client,
    label: &str,
    path: &str,
    auth: Option<&str>,
    basic: bool,
    src_query: bool,
    src_header: bool,
) -> (Hit, Value) {
    let mut rb = http
        .get(format!("{BERSERKER_BASE}{path}"))
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*");
    if src_query {
        rb = rb.query(&[("synAccessSource", SYN_ACCESS_SOURCE)]);
    }
    if src_header {
        rb = rb.header("synAccessSource", SYN_ACCESS_SOURCE);
    }
    if basic {
        rb = rb.header("Authorization", BASIC_CHARGE);
    }
    if let Some(a) = auth {
        rb = rb.header("synjones-auth", a);
    }
    let resp = rb.send().await.expect("探测请求失败（网络层）");
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let mut arrays = Vec::new();
    let mut record_keys = Vec::new();
    walk("", &v, &mut arrays, &mut record_keys, 0);
    let hit = Hit {
        label: label.to_string(),
        path: path.to_string(),
        status,
        code: v.get("code").and_then(Value::as_i64),
        msg: v
            .get("msg")
            .or_else(|| v.get("message"))
            .and_then(Value::as_str)
            .map(mask_digits)
            .unwrap_or_default(),
        keys: v
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default(),
        arrays,
        record_keys,
        has_payload: payload_of(&v),
        body_len: body.len(),
        body_head: if serde_json::from_str::<Value>(&body).is_ok() {
            String::new()
        } else {
            mask_digit_runs(&body.chars().take(80).collect::<String>())
        },
    };
    println!("[{}] HTTP {} code={:?} msg={:?}", hit.label, status, hit.code, hit.msg);
    (hit, v)
}

/// 从 JSON 值里取一个 id（`as_str` 或数字都吃得下；订单/流水号在实测里两种形态都出现过）。
fn id_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// 批量跑一组 `(标签, 路径, 带 Basic, 来源入 query, 来源加头)` 的 GET 探测，落盘 + 收进 `hits`。
async fn run_batch(
    http: &reqwest::Client,
    dir: &Path,
    bodies: &mut serde_json::Map<String, Value>,
    hits: &mut Vec<Hit>,
    list: &[(String, String, bool, bool, bool)],
    auth: &str,
) {
    for (label, path, basic, sq, sh) in list {
        let tag = if *sh {
            label.clone()
        } else {
            format!("{label} [无 synAccessSource]")
        };
        let (hit, v) = get_probe(http, &tag, path, Some(auth), *basic, *sq, *sh).await;
        bodies.insert(
            tag.clone(),
            serde_json::json!({
                "path": path, "status": hit.status, "code": hit.code, "msg": hit.msg,
                "topLevelKeys": hit.keys, "arrays": hit.arrays, "firstRecordKeys": hit.record_keys,
                "verdict": hit.verdict(), "bodyLen": hit.body_len,
                "headers": {"basic": basic, "sourceQuery": sq, "sourceHeader": sh},
            }),
        );
        let safe_name = tag
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>();
        dump(dir, &format!("body_{safe_name}.json"), &v);
        hits.push(hit);
    }
}

// ==================== 主流程 ====================

#[tokio::test]
#[ignore = "live：需校园网与真实凭据（严格只读，候选端点来自官方 bundle 反查）"]
async fn m4_history_probe_live() {
    let t_start = Instant::now();
    let dir = recon_dir();
    std::fs::create_dir_all(&dir).expect("创建 recon 目录失败");
    println!("[recon] 样本目录: {}", dir.display());

    let cas = CasClient::new().expect("创建 CasClient 失败");
    let http = cas.http_client().clone();

    // ---------- 0) 匿名片区目录：拿到真实电费片区 id（448/449/450 的运行时真值） ----------
    let feeitems = list_feeitems(&cas).await.expect("匿名片区目录失败");
    let ids: Vec<String> = feeitems.iter().map(|f| f.id.clone()).collect();
    println!("[目录] 电费片區（status=1 且 impl_interface 非空）共 {} 条: {:?}", ids.len(), ids);
    for f in &feeitems {
        println!("  └ id={} name={:?} last_level_is_input={}", f.id, f.name, f.last_level_is_input);
    }
    let id0 = ids.first().cloned().unwrap_or_else(|| "448".to_string());
    let id1 = ids.get(1).cloned().unwrap_or_else(|| id0.clone());
    let id2 = ids.get(2).cloned().unwrap_or_else(|| id0.clone());

    // ---------- 1) 取 token（TGT 优先，回落账密；与既有 live 测试同链） ----------
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
                    let (u, p) = creds_from_env_file().expect("无可用凭据（accounts.json / CAMPUS_HUB_CREDS 都不可用）");
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

    let client = SynjonesClient::new(cas.clone(), tgt.clone(), Some(token.clone()));

    // ---------- 2) 三个片区各自第 1 级（校区）选项个数 ----------
    let mut cascade_level1: Vec<Value> = Vec::new();
    for fid in &ids {
        let v = client
            .post_form(
                EP_GET_THIRD_DATA,
                &[
                    ("feeitemid", fid.clone()),
                    ("type", "select".to_string()),
                    ("level", "0".to_string()),
                ],
                Envelope::Charge,
            )
            .await
            .expect("getThirdData level=0 失败");
        let map = &v["map"];
        let levels: Vec<String> = map["total"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|t| {
                        format!(
                            "{}:{}",
                            t["level"].as_u64().unwrap_or(0),
                            t["name"].as_str().unwrap_or("")
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let opts = map["data"].as_array().cloned().unwrap_or_default();
        let names: Vec<String> = opts
            .iter()
            .map(|o| mask_digits(o["name"].as_str().unwrap_or("")))
            .collect();
        println!(
            "[级联] 片区 {fid}：层级=[{}]，第 1 级选项个数 = {}，选项名（数字打码）={:?}",
            levels.join(" < "),
            opts.len(),
            names
        );
        cascade_level1.push(serde_json::json!({
            "feeitemid": fid,
            "levels": levels,
            "level1OptionCount": opts.len(),
            "level1OptionNames": names,
        }));
    }
    dump(&dir, "cascade_level1.json", &Value::Array(cascade_level1.clone()));

    // ---------- 3) 候选端点矩阵（全 GET；只读） ----------
    // (标签, 路径含 query)。`{id}` 会被替换成运行时真实片区 id。
    let candidates: Vec<(String, String)> = vec![
        // —— 官方 bundle 反查出的真实端点（账单/流水/充值记录）——
        ("turnover/app_account 无参".into(), "/charge/turnover/app_account".into()),
        (
            "turnover/app_account app_page(无 feeitemid)".into(),
            "/charge/turnover/app_account?app_page=1&app_row=15".into(),
        ),
        (
            "turnover/app_account app_page+feeitemid".into(),
            format!("/charge/turnover/app_account?app_page=1&app_row=15&feeitemid={id0}"),
        ),
        (
            "turnover/app_account pc风格 current/size".into(),
            format!("/charge/turnover/app_account?current=1&size=10&feeitemid={id0}"),
        ),
        (
            "turnover/app_account 带 createdate".into(),
            "/charge/turnover/app_account?createdate=2026-09&app_page=1&app_row=15".into(),
        ),
        (
            "turnover/app_totalAccount 带 feeitemid".into(),
            format!("/charge/turnover/app_totalAccount?feeitemid={id0}"),
        ),
        (
            "turnover/app_totalAccount 带 createdate".into(),
            "/charge/turnover/app_totalAccount?createdate=2026-09".into(),
        ),
        (
            "turnover/mouthAccount createdate(pc)".into(),
            "/charge/turnover/mouthAccount?createdate=2026-09".into(),
        ),
        (
            "turnover/mouthAccount startdate/enddate(app)".into(),
            "/charge/turnover/mouthAccount?startdate=2026-08-01&enddate=2026-09-19".into(),
        ),
        (
            "turnover/pie_account createdate".into(),
            "/charge/turnover/pie_account?createdate=2026-09".into(),
        ),
        (
            "turnover/personal_data id+flag=3".into(),
            format!("/charge/turnover/personal_data?id={id0}&flag=3"),
        ),
        (
            "turnover/personal_data 仅 id".into(),
            format!("/charge/turnover/personal_data?id={id0}"),
        ),
        (
            "turnover/showBillapply".into(),
            "/charge/turnover/showBillapply".into(),
        ),
        (
            "feeitem/getRechargeRecord 仅 feeitemid".into(),
            format!("/charge/feeitem/getRechargeRecord?feeitemid={id0}"),
        ),
        (
            "feeitem/getRechargeRecord feeitemid+rtype".into(),
            format!("/charge/feeitem/getRechargeRecord?feeitemid={id0}&rtype=1"),
        ),
        (
            "feeitem/showFeeitem feeitemid".into(),
            format!("/charge/feeitem/showFeeitem?feeitemid={id0}"),
        ),
        ("order/oldOrdersData status=1".into(), "/charge/order/oldOrdersData?status=1".into()),
        ("order/personal_data status=0".into(), "/charge/order/personal_data?status=0".into()),
        (
            "order/threeExpen_account successdate".into(),
            "/charge/order/threeExpen_account?successdate=2026-09".into(),
        ),
        (
            "receivable/personal_data feeitemid".into(),
            format!("/charge/receivable/personal_data?feeitemid={id0}"),
        ),
        ("receivable/personal_data status=0".into(), "/charge/receivable/personal_data?status=0".into()),
        ("billapply/personal_data 无参".into(), "/charge/billapply/personal_data".into()),
        (
            "billapply/personal_data feeitemid".into(),
            format!("/charge/billapply/personal_data?feeitemid={id0}"),
        ),
        (
            "billapply/billapplyPage".into(),
            "/charge/billapply/billapplyPage".into(),
        ),
        ("billType/combox_list".into(), "/charge/billType/combox_list".into()),
        ("billcontentType/combox_list".into(), "/charge/billcontentType/combox_list".into()),
        ("orderdetail/personal_data id=1".into(), "/charge/orderdetail/personal_data?id=1".into()),
        // —— 派单给的路径变体（实测把它们从「可能存在」钉成「确定不存在」）——
        ("order/list".into(), "/charge/order/list".into()),
        ("order/page".into(), "/charge/order/page".into()),
        ("order/query".into(), "/charge/order/query".into()),
        ("order/getList".into(), "/charge/order/getList".into()),
        ("order/listOrder".into(), "/charge/order/listOrder".into()),
        ("order/list?status=1".into(), "/charge/order/list?status=1".into()),
        ("order/page?pageNum=1&pageSize=10".into(), "/charge/order/page?pageNum=1&pageSize=10".into()),
        ("pay/orderList".into(), "/charge/pay/orderList".into()),
        ("pay/getOrderList".into(), "/charge/pay/getOrderList".into()),
        ("record".into(), "/charge/record".into()),
        ("records".into(), "/charge/records".into()),
        ("feedetail".into(), "/charge/feedetail".into()),
        ("feedetails".into(), "/charge/feedetails".into()),
        ("consume/list".into(), "/charge/consume/list".into()),
        ("electric/list".into(), "/charge/electric/list".into()),
        ("history".into(), "/charge/history".into()),
        ("queryHistory".into(), "/charge/queryHistory".into()),
        ("blade-pay/order/list".into(), "/blade-pay/order/list".into()),
        ("blade-pay/order/page".into(), "/blade-pay/order/page".into()),
        ("blade-pay/order/query".into(), "/blade-pay/order/query".into()),
        // —— 对照：已知可用的两条（证明探针本身能出绿）——
        ("对照 /charge/feeitem（匿名）".into(), EP_FEEITEM.into()),
        (
            "对照 /charge/turnover/appAccountDetail（先看无 id 的报错）".into(),
            "/charge/turnover/appAccountDetail".into(),
        ),
    ];

    let mut hits: Vec<Hit> = Vec::new();
    let mut bodies: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut turnoverid: Option<String> = None;
    let mut order_id: Option<String> = None;
    let mut userno: Option<String> = None;
    for (label, path) in &candidates {
        let (hit, v) = get_probe(&http, label, path, Some(&auth), true, true, true).await;
        // 无 Basic 的对照：仅对「带 Basic 失败」的路径做一次，判定 Basic 是否强制
        let mut retry_note = Value::Null;
        if hit.status != 200 || hit.code != Some(200) {
            let (hit2, _) = get_probe(
                &http,
                &format!("{label}（不带 Basic 对照）"),
                path,
                Some(&auth),
                false,
                true,
                true,
            )
            .await;
            retry_note = serde_json::json!({
                "noBasic": {"status": hit2.status, "code": hit2.code, "msg": hit2.msg, "verdict": hit2.verdict()}
            });
        }
        // 从账单列表里取一个真实 turnoverid，供详情端点用（只在内存，不落盘）
        if turnoverid.is_none() {
            turnoverid = v
                .get("accountList")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|r| r.get("TURNOVERID"))
                .and_then(id_of);
        }
        // 从订单列表里取一个真实订单 id（`orderDetailList[].id`，官方订单详情页用的就是它）
        if order_id.is_none() {
            order_id = v
                .get("orderList")
                .or_else(|| v.get("oldList"))
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|o| {
                    o.get("id")
                        .and_then(id_of)
                        .or_else(|| {
                            o.get("orderDetailList")
                                .and_then(Value::as_array)
                                .and_then(|d| d.first())
                                .and_then(|x| x.get("id"))
                                .and_then(id_of)
                        })
                });
        }
        // 取一个真实 userno（`threeExpen_account` 的官方入参之一），只在内存
        if userno.is_none() {
            userno = v
                .get("list")
                .or_else(|| v.get("orderList"))
                .or_else(|| v.get("oldList"))
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|r| r.get("userno"))
                .and_then(id_of);
        }
        bodies.insert(
            label.clone(),
            serde_json::json!({
                "path": path, "status": hit.status, "code": hit.code, "msg": hit.msg,
                "topLevelKeys": hit.keys, "arrays": hit.arrays, "firstRecordKeys": hit.record_keys,
                "verdict": hit.verdict(), "retry": retry_note,
            }),
        );
        // 原始（脱敏后）响应体样本另存，便于人工核对字段
        let safe_name = label
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>();
        dump(&dir, &format!("body_{safe_name}.json"), &v);
        hits.push(hit);
    }

    // ---------- 4) 第二轮：链式详情 + 参数语义（能否给「时间序列」） ----------
    // (标签, 路径, 是否带 synAccessSource)
    let mut phase2: Vec<(String, String, bool)> = Vec::new();

    if let Some(tid) = turnoverid.clone() {
        println!("[链式] accountList 取到 turnoverid（长度 {}，值不打印）", tid.len());
        phase2.push((
            "appAccountDetail?turnoverid=<真实>".into(),
            format!("/charge/turnover/appAccountDetail?turnoverid={tid}"),
            true,
        ));
        phase2.push((
            "billapply/personal_data?turnoverid=<真实>".into(),
            format!("/charge/billapply/personal_data?turnoverid={tid}"),
            true,
        ));
    } else {
        println!("[链式] 未从 accountList 取到 turnoverid → 详情端点只能以无 id 形态探测（见上表）");
    }
    if let Some(oid) = order_id.clone() {
        println!("[链式] orderList 取到订单 id（长度 {}，值不打印）", oid.len());
        phase2.push((
            "orderdetail/personal_data?id=<真实>".into(),
            format!("/charge/orderdetail/personal_data?id={oid}"),
            true,
        ));
    } else {
        println!("[链式] 未取到订单 id（orderList/oldList 都没有 id/orderDetailList[].id）");
    }

    // 账单主线：分页上限 / 日期区间 / 逐片区 / 逐月（月序列 = 能否攒出历史曲线）
    phase2.push((
        "app_account size=200（探分页上限）".into(),
        "/charge/turnover/app_account?current=1&size=200".into(),
        true,
    ));
    phase2.push((
        format!("app_account size=200 feeitemid={id1}"),
        format!("/charge/turnover/app_account?current=1&size=200&feeitemid={id1}"),
        true,
    ));
    phase2.push((
        format!("app_account size=200 feeitemid={id2}"),
        format!("/charge/turnover/app_account?current=1&size=200&feeitemid={id2}"),
        true,
    ));
    phase2.push((
        "app_account 区间 startdate/enddate".into(),
        "/charge/turnover/app_account?startdate=2026-01-01&enddate=2026-09-19".into(),
        true,
    ));
    phase2.push((
        format!("app_totalAccount feeitemid={id1}"),
        format!("/charge/turnover/app_totalAccount?feeitemid={id1}"),
        true,
    ));
    phase2.push((
        format!("app_totalAccount feeitemid={id2}"),
        format!("/charge/turnover/app_totalAccount?feeitemid={id2}"),
        true,
    ));
    phase2.push((
        "app_totalAccount 无参".into(),
        "/charge/turnover/app_totalAccount".into(),
        true,
    ));
    for m in ["2026-03", "2026-06", "2026-07", "2026-08"] {
        phase2.push((
            format!("mouthAccount createdate={m}"),
            format!("/charge/turnover/mouthAccount?createdate={m}"),
            true,
        ));
        phase2.push((
            format!("pie_account createdate={m}"),
            format!("/charge/turnover/pie_account?createdate={m}"),
            true,
        ));
    }
    phase2.push((
        format!("mouthAccount createdate=2026-09&feeitemid={id1}"),
        format!("/charge/turnover/mouthAccount?createdate=2026-09&feeitemid={id1}"),
        true,
    ));

    // turnover/personal_data 参数面（id 是片区还是流水？flag 是什么？）
    phase2.push((
        format!("turnover/personal_data id={id1}"),
        format!("/charge/turnover/personal_data?id={id1}"),
        true,
    ));
    phase2.push((
        format!("turnover/personal_data id={id2}"),
        format!("/charge/turnover/personal_data?id={id2}"),
        true,
    ));
    phase2.push((
        format!("turnover/personal_data id={id0}&flag=1"),
        format!("/charge/turnover/personal_data?id={id0}&flag=1"),
        true,
    ));
    phase2.push((
        format!("turnover/personal_data id={id0}&page=1&size=50"),
        format!("/charge/turnover/personal_data?id={id0}&page=1&size=50"),
        true,
    ));
    phase2.push((
        "turnover/personal_data 无参".into(),
        "/charge/turnover/personal_data".into(),
        true,
    ));

    // 订单主线：status 矩阵 + 关键对照（既有项目结论说 personal_data 恒 500）
    for s in ["0", "1", "2"] {
        phase2.push((
            format!("order/personal_data status={s}"),
            format!("/charge/order/personal_data?status={s}"),
            true,
        ));
        phase2.push((
            format!("order/oldOrdersData status={s}"),
            format!("/charge/order/oldOrdersData?status={s}"),
            true,
        ));
    }
    phase2.push((
        "order/personal_data status=0 **去掉 synAccessSource**（复现既有「恒 500」结论）".into(),
        "/charge/order/personal_data?status=0".into(),
        false,
    ));
    phase2.push((
        "order/personal_data 无参".into(),
        "/charge/order/personal_data".into(),
        true,
    ));

    // 应收 / 账单申请：逐片区 + 无参
    for fid in [id1.clone(), id2.clone()] {
        phase2.push((
            format!("receivable/personal_data feeitemid={fid}"),
            format!("/charge/receivable/personal_data?feeitemid={fid}"),
            true,
        ));
        phase2.push((
            format!("billapply/personal_data feeitemid={fid}"),
            format!("/charge/billapply/personal_data?feeitemid={fid}"),
            true,
        ));
    }
    phase2.push((
        "receivable/personal_data id=<片区>".into(),
        format!("/charge/receivable/personal_data?id={id0}"),
        true,
    ));

    let phase2_all: Vec<(String, String, bool, bool, bool)> = phase2
        .iter()
        .map(|(l, p, s)| (l.clone(), p.clone(), true, *s, *s))
        .collect();
    run_batch(&http, &dir, &mut bodies, &mut hits, &phase2_all, &auth).await;

    // ---------- 5) 头组矩阵：把既有结论「personal_data 恒 500」钉死在实测上 ----------
    // 既有结论来自 `tests/recharge_live.rs::recharge_diag_live`（该处对 GET 只加了 synAccessSource **头**、
    // 没进 query，且不带 App 拦截器的 Basic；本轮同参数复跑 + 逐项加头定位真实门槛）。
    println!("\n======== 头组矩阵：/charge/order/personal_data ========");
    let mut phase3: Vec<(String, String, bool, bool, bool)> = vec![];
    for (label, basic, sq, sh) in [
        ("personal_data 仅 token（无来源、无 Basic）", false, false, false),
        ("personal_data +来源头（既有 diag 形态）", false, false, true),
        ("personal_data +来源头+来源query", false, true, true),
        ("personal_data +Basic+来源query+来源头（App 拦截器形态）", true, true, true),
        ("personal_data +Basic 但无来源query", true, false, true),
    ] {
        phase3.push((
            label.to_string(),
            "/charge/order/personal_data?status=0".to_string(),
            basic,
            sq,
            sh,
        ));
    }
    for q in [
        "",
        "?status=0",
        "?status=1",
        "?status=2",
        "?status=0&current=1&size=10",
        "?status=0&payid=64",
    ] {
        phase3.push((
            format!("personal_data 参数形态{q:?}（App 拦截器头组）"),
            format!("/charge/order/personal_data{q}"),
            true,
            true,
            true,
        ));
    }
    run_batch(&http, &dir, &mut bodies, &mut hits, &phase3, &auth).await;

    // ---------- 6) 时序可得性：`threeExpen_account`（官方「支出统计」）+ 更早月份 + 单片区过滤 ----------
    let mut phase4: Vec<(String, String, bool, bool, bool)> = vec![];
    match userno.clone() {
        Some(u) => {
            println!("[时序] 取到 userno（长度 {}，值不打印）", u.len());
            for d in ["2026-09", "2026-08", "2025-09", "2026"] {
                phase4.push((
                    format!("threeExpen_account userno+successdate={d}"),
                    format!("/charge/order/threeExpen_account?userno={u}&successdate={d}"),
                    true,
                    true,
                    true,
                ));
            }
        }
        None => println!("[时序] 未取到 userno → threeExpen_account 只能以无 userno 形态探测（见上表）"),
    }
    // 更早月份：看学校侧保留多久（若全空则为「无历史」的直接证据）
    for m in ["2025-09", "2026-01", "2026-04", "2026-05"] {
        phase4.push((
            format!("pie_account createdate={m}"),
            format!("/charge/turnover/pie_account?createdate={m}"),
            true,
            true,
            true,
        ));
    }
    // 逐年过滤 + 单片区过滤：确认参数是否真的生效
    phase4.push((
        "app_totalAccount createdate=2025-09".to_string(),
        "/charge/turnover/app_totalAccount?createdate=2025-09".to_string(),
        true,
        true,
        true,
    ));
    phase4.push((
        "app_account feeitemid=401（一卡通充值片区）".to_string(),
        "/charge/turnover/app_account?current=1&size=100&feeitemid=401".to_string(),
        true,
        true,
        true,
    ));
    run_batch(&http, &dir, &mut bodies, &mut hits, &phase4, &auth).await;

    // ---------- 7) 一卡通流水（search 系）：每笔记录带 `cardBalance` 交易后余额快照 ----------
    // 电费充值的付款渠道是电子账户（`ACCOUNTTSM`，见 recharge.rs），故电费支出**可能**出现在一卡通流水里。
    // 若出现 ⇒ 「日期 + 金额 + 交易后余额」的事件级余额序列可得（非每日，但可替代自采的一部分）。
    println!("\n======== 一卡通流水（/berserker-search/search/personal/turnover）========");
    for (label, query, size) in [
        ("全部（current=1&size=30）", "current=1&size=30", 30u32),
        ("仅支出（type=2）", "current=1&size=30&type=2", 30u32),
    ] {
        let (hit, v) = get_probe(
            &http,
            &format!("turnover {label}"),
            &format!("/berserker-search/search/personal/turnover?{query}"),
            Some(&auth),
            true,
            true,
            true,
        )
        .await;
        let recs = v["data"]["records"].as_array().cloned().unwrap_or_default();
        let total = v["data"]["total"].as_u64().unwrap_or(0);
        let with_balance = recs
            .iter()
            .filter(|r| r.get("cardBalance").map(|b| !b.is_null()).unwrap_or(false))
            .count();
        println!(
            "  └ total={total} 本页={} 条，其中带 cardBalance 的 {} 条（前 {size} 条样本）",
            recs.len(),
            with_balance
        );
        for r in recs.iter().take(12) {
            let fen = |k: &str| r.get(k).and_then(id_of).and_then(|s| s.parse::<i64>().ok());
            println!(
                "    · {} | {} 分（typeFrom={}）| 交易后余额 {:?} 分 | 摘要={:?} | 交易方={:?} | 地点={:?}",
                mask_digit_runs(r.get("jndatetimeStr").and_then(Value::as_str).unwrap_or("")),
                fen("tranamt").unwrap_or(0),
                r.get("typeFrom").and_then(Value::as_str).unwrap_or(""),
                fen("cardBalance"),
                mask_digit_runs(r.get("resume").and_then(Value::as_str).unwrap_or("")),
                mask_digit_runs(r.get("payName").and_then(Value::as_str).unwrap_or("")),
                mask_digit_runs(r.get("locationName").and_then(Value::as_str).unwrap_or("")),
            );
        }
        dump(
            &dir,
            &format!("body_turnover_search_{}.json", if size == 30 && query.contains("type=2") { "expense" } else { "all" }),
            &v,
        );
        hits.push(hit);
    }

    // ---------- 5) 汇总 ----------
    println!("\n================ 汇总（每行：路径 → 状态 → 判定 → 形态） ================");
    for h in &hits {
        println!("{}", h.line());
    }

    let usable: Vec<&Hit> = hits
        .iter()
        .filter(|h| h.status == 200 && h.code == Some(200) && h.has_payload)
        .collect();
    println!("\n================ 判定为「可用（HTTP 200 且 data 非空）」的端点 ================");
    for h in &usable {
        println!(
            "{} | 路径 {} | 数组 {} | 首条字段 {}",
            h.label,
            h.path,
            h.arrays.join(","),
            h.record_keys.join(",")
        );
    }
    if usable.is_empty() {
        println!("（无）");
    }

    let report = serde_json::json!({
        "cascadeLevel1": cascade_level1,
        "electricityFeeitemIds": ids,
        "probes": bodies,
        "usableCount": usable.len(),
        "elapsedSecs": t_start.elapsed().as_secs_f64(),
    });
    dump(&dir, "m4_history_probe.json", &report);

    println!(
        "\n[耗时] 总 {} 秒；请求数 = 候选 {}（含失败重试对照）+ 参数语义 {} + 头组矩阵 {} + 时序 {} + 级联 {}（POST 只读查询）",
        t_start.elapsed().as_secs_f64(),
        candidates.len(),
        phase2.len(),
        phase3.len(),
        phase4.len(),
        ids.len()
    );
    println!("[样本] {}", dir.display());
    println!("[命令] cargo test -p campus-synjones -- --ignored m4_history_probe_live --nocapture");
}
