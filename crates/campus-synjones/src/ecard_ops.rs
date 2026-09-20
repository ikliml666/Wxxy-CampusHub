//! 一卡通安全键盘（批 1：只做**取键盘 + 内存缓存**；写操作在批 4 接入）。
//!
//! # 协议事实（2026-09-19 live，探针 `tests/ecard_features_probe_live.rs` D3/D4）
//!
//! `GET /berserker-secure/keyboard?type=Number|Standard`（信封 [`Envelope::Berserker`]，
//! 响应 `{code, data, msg, success}`）：
//!
//! - `type=Number`：`data.numberKeyboard`（10 字符乱序串）+ `numberKeyboardImage`（10 张图）+ `uuid`；
//! - `type=Standard`：`numberKeyboard` / `upperLetterKeyboard` / `lowerLetterKeyboard` /
//!   `symbolKeyboard` 四组，各配同名 `<组>Image` 图数组，共享同一个 `uuid`。
//!
//! 键位语义：`<组>Keyboard` 串的第 i 个字符就是第 i 张图上的字符——用户点击位置 i，
//! 明文即 `keys[i]`（批 4 拼 `pwd = "1$1$" + 明文 + "$1$" + uuid` 用）。
//!
//! # 缓存红线（契约 §2.4，与电费 `passwordMap` 同构）
//!
//! 1. 真实 `uuid → 键位串` 映射**只在内存**：TTL 300 秒、至多 8 把、不落盘、不打日志、
//!    不进错误文案；`SecurePad`/`PadSecret` 的 `Debug` 手写打码。
//! 2. 前端只拿 [`SecurePad::pad_id`]（**本进程随机生成**，真实 uuid 绝不外泄）+ `keys`（渲染）+ `images`。
//! 3. [`take_pad`] 取走即删（消耗语义），批 4 拼 `pwd` 时从这里拿真实 uuid 与键位串。
//!
//! 键位组顺序固定为 **数字 → 大写 → 小写 → 符号**（前端按下标渲染，两边必须同序）。

use crate::client::{Envelope, SynjonesClient};
use crate::CampusSynjonesError;
use serde::Serialize;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 安全键盘端点。
pub const EP_KEYBOARD: &str = "/berserker-secure/keyboard";

/// 键盘映射的存活时长（契约 §2.4：5 分钟）。
pub const PAD_TTL: Duration = Duration::from_secs(300);
/// 最多同时缓存 8 把（超出淘汰最旧）。
pub const MAX_PADS: usize = 8;

/// 键盘种类（命令参数 `"number"` / `"standard"`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardKind {
    /// 数字键盘（10 键，密码/验证码输入）。
    Number,
    /// 全键盘（数字/大小写/符号四组）。
    Standard,
}

impl KeyboardKind {
    /// 服务端 `type` 参数值（实测大小写敏感）。
    fn type_param(self) -> &'static str {
        match self {
            KeyboardKind::Number => "Number",
            KeyboardKind::Standard => "Standard",
        }
    }

    /// 从命令参数解析（其余值一律拒绝，不猜）。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "number" => Some(KeyboardKind::Number),
            "standard" => Some(KeyboardKind::Standard),
            _ => None,
        }
    }
}

/// 交给前端的键盘：`pad_id` 是本进程随机 id（真实 uuid 在缓存里），`keys`/`images`
/// 供渲染（下标一一对应）。
#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurePad {
    /// 本进程随机 id（前端提交操作时原样带回）。
    pub pad_id: String,
    /// 键位显示字符（数字键盘 10 个；全键盘四组拼接）。
    pub keys: Vec<String>,
    /// 键位图片（base64 PNG，与 `keys` 同长；服务端未下发时为空表）。
    pub images: Vec<String>,
}

/// `Debug` 打码：键位字符与 pad_id 都不进日志/panic 消息。
impl std::fmt::Debug for SecurePad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecurePad {{ pad_id: ***, keys: <{} 键，已打码> }}", self.keys.len())
    }
}

/// 批 4 拼 `pwd` 用的秘密材料：真实服务端 uuid + 键位串（**绝不**回前端、不落盘、不打日志）。
#[derive(Clone, PartialEq)]
pub struct PadSecret {
    /// 服务端下发的键盘 uuid。
    pub uuid: String,
    /// 键位字符（下标即用户点击位置）。
    pub keys: Vec<String>,
}

impl std::fmt::Debug for PadSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PadSecret {{ uuid: ***, keys: <{} 键，已打码> }}", self.keys.len())
    }
}

struct PadEntry {
    pad_id: String,
    uuid: String,
    keys: Vec<String>,
    created: Instant,
}

static PADS: OnceLock<Mutex<Vec<PadEntry>>> = OnceLock::new();

fn pads() -> &'static Mutex<Vec<PadEntry>> {
    PADS.get_or_init(|| Mutex::new(Vec::new()))
}

/// pad_id 计数器（与纳秒时钟混合，进程内唯一且不可猜）。
fn next_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

fn new_pad_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // ponytail: 时间+计数器混排即可满足「进程内唯一、不可枚举」；不引 rand 依赖
    format!("pad{:016x}{:04x}", nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15), next_seq())
}

/// 入缓存：清过期 → 超量淘汰最旧 → 追加。返回分配的 pad_id。
fn store_pad(uuid: String, keys: Vec<String>) -> String {
    let pad_id = new_pad_id();
    let now = Instant::now();
    if let Ok(mut list) = pads().lock() {
        list.retain(|e| now.duration_since(e.created) < PAD_TTL);
        while list.len() >= MAX_PADS {
            list.remove(0);
        }
        list.push(PadEntry { pad_id: pad_id.clone(), uuid, keys, created: now });
    }
    pad_id
}

/// 取走即删（消耗语义）。过期 / 不存在 → None。
pub fn take_pad(pad_id: &str) -> Option<PadSecret> {
    let mut list = pads().lock().ok()?;
    let now = Instant::now();
    let pos = list.iter().position(|e| e.pad_id == pad_id)?;
    let e = list.remove(pos);
    if now.duration_since(e.created) >= PAD_TTL {
        return None;
    }
    Some(PadSecret { uuid: e.uuid, keys: e.keys })
}

// ---------------- 解析与取数 ----------------

/// 键位组的字段名（顺序即对外键位顺序：数字 → 大写 → 小写 → 符号）。
const KEY_GROUPS: [(&str, &str); 4] = [
    ("numberKeyboard", "numberKeyboardImage"),
    ("upperLetterKeyboard", "upperLetterKeyboardImage"),
    ("lowerLetterKeyboard", "lowerLetterKeyboardImage"),
    ("symbolKeyboard", "symbolKeyboardImage"),
];

/// 解析键盘 `data`：`uuid` + 全部键位组的字符串与图片（按 [`KEY_GROUPS`] 顺序拼接；
/// Number 型只有 numberKeyboard 一组，自然兼容）。键位串为空或无 uuid → None。
pub fn parse_keyboard(data: &Value) -> Option<(String, Vec<String>, Vec<String>)> {
    let uuid = crate::ecard::text_of(Some(data.get("uuid")?));
    if uuid.is_empty() {
        return None;
    }
    let mut keys: Vec<String> = Vec::new();
    let mut images: Vec<String> = Vec::new();
    for (k_field, img_field) in KEY_GROUPS {
        let Some(raw) = data.get(k_field) else { continue };
        // 实测形态：键位是「N 个字符的字符串」，图片是与键数等长的数组；数组形态一并容忍（类型漂移先例）
        let group_keys: Vec<String> = match raw {
            Value::String(s) => s.chars().map(|c| c.to_string()).collect(),
            Value::Array(a) => a
                .iter()
                .map(|it| crate::ecard::text_of(Some(it)))
                .filter(|s| !s.is_empty())
                .collect(),
            _ => continue,
        };
        if group_keys.is_empty() {
            continue;
        }
        keys.extend(group_keys);
        if let Some(imgs) = data.get(img_field).and_then(Value::as_array) {
            images.extend(imgs.iter().map(|it| crate::ecard::text_of(Some(it))));
        }
    }
    (!keys.is_empty()).then_some((uuid, keys, images))
}

/// 取一把安全键盘并写入进程级缓存（返回给前端的只有 pad_id + 键位 + 图片）。
pub async fn fetch_secure_keyboard(
    client: &SynjonesClient,
    kind: KeyboardKind,
) -> Result<SecurePad, CampusSynjonesError> {
    // 官方恒带 `order=1`（bundle 全部调用点 `keyboardOptions:{types,order:1}`），
    // 与官方完全对齐（不带该参数曾出现间歇性失败的用户报告）。
    let v = client
        .get(
            EP_KEYBOARD,
            &[("type", kind.type_param()), ("order", "1")],
            Envelope::Berserker,
        )
        .await?;
    let (uuid, keys, images) = parse_keyboard(&v["data"]).ok_or_else(|| {
        CampusSynjonesError::Parse("安全键盘响应缺少 uuid 或键位数据".to_string())
    })?;
    let pad_id = store_pad(uuid, keys.clone());
    Ok(SecurePad { pad_id, keys, images })
}

// ---------------- 写操作（批 3；协议 §1.8：bundle 反查，**未 live 验证**） ----------------
//
// 写路径红线：本节只按 bundle 反查出的协议实现，**绝不发真实请求验证**——挂失/改密/转账
// 一旦真发出去就是不可逆的真实状态变更。所有端点/参数名以契约 §1.8 表格为准，若实测
// 结构与本实现不符，按现场报错修正，不猜第二个形态。

/// 一卡通写操作端点（`/berserker-app/ykt/tsm/*`）。
pub const EP_LOST: &str = "/berserker-app/ykt/tsm/lostCard";
/// ⚠️ 路径是**小写 `unlostCard`**（官方 bundle 原文）——该校路径大小写敏感，
/// 旧实现的 `unLostCard` 在解挂提交时得到 `HTTP 404`（2026-09-20 官方动态抓包对照确认）。
pub const EP_UNLOST: &str = "/berserker-app/ykt/tsm/unlostCard";
pub const EP_CHECK_PWD: &str = "/berserker-app/ykt/tsm/checkPwd";
pub const EP_MODIFY_PWD: &str = "/berserker-app/ykt/tsm/modifyPwd";
pub const EP_SEND_FIND_PWD_VER: &str = "/berserker-app/ykt/tsm/sendfindPwdVer";
pub const EP_FIND_PWD: &str = "/berserker-app/ykt/tsm/findPwd";
pub const EP_PAY_LIMITE_MODIFY: &str = "/berserker-app/ykt/tsm/payLimiteModify";
pub const EP_MODIFY_ACC: &str = "/berserker-app/ykt/tsm/modifyAcc";
pub const EP_SEND_BIND_BANK_VER: &str = "/berserker-app/ykt/tsm/sendBindBankVer";
pub const EP_BUILD_BANK_RELATION: &str = "/berserker-app/ykt/tsm/buildBankCardRelation";
pub const EP_CANCEL_BANK: &str = "/berserker-app/ykt/tsm/cancelBankCardRelation";

/// 绑/解绑校园卡走 `/berserker-base/*`（其余写操作都在 `/berserker-app/ykt/tsm/*`）。
pub const EP_SEND_BIND_USER_VER: &str = "/berserker-base/accountuser/sendBindUserVerCode";
pub const EP_BIND_USER: &str = "/berserker-base/accountuser/bindUser";
pub const EP_UNBIND_USER: &str = "/berserker-base/accountuser/unBind";

/// `pwdType` 参数值（bundle 反查恒为 `1`）。
pub const PWD_TYPE: &str = "1";

/// 前端提交形态的密码输入：只有一次性 [`SecurePad::pad_id`] + 用户点击的**位置下标序列**。
///
/// 明文密码**永不出现在本结构里**——拼装在 [`assemble_pwd`] 内完成后立即丢弃；`Debug` 打码。
#[derive(Clone, PartialEq)]
pub struct PasswordInput {
    /// [`SecurePad::pad_id`] 原样带回（取走即删，一把键盘只能提交一次）。
    pub pad_id: String,
    /// 用户点击的键位下标（按 [`SecurePad::keys`] 的下标）。
    pub positions: Vec<usize>,
}

impl std::fmt::Debug for PasswordInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PasswordInput {{ pad_id: ***, positions: <{} 项> }}", self.positions.len())
    }
}

/// 用一把已取出的键盘秘密把位置序列翻译成 `pwd` 串（纯函数，单测钉住）。
///
/// 位置越界 → 可读错误（**不含任何键位/明文内容**）；位置序列为空 → 报「请输入密码」。
/// 产物形态：`"1$1$" + 明文 + "$1$" + keyboardUuid`（bundle 反查，未 live 验证）。
fn build_pwd(secret: &PadSecret, positions: &[usize]) -> Result<String, CampusSynjonesError> {
    if positions.is_empty() {
        return Err(CampusSynjonesError::Parse("请输入密码".to_string()));
    }
    let mut plain = String::new();
    for (i, &pos) in positions.iter().enumerate() {
        match secret.keys.get(pos) {
            Some(k) => plain.push_str(k),
            None => {
                return Err(CampusSynjonesError::Parse(format!(
                    "密码输入第 {} 位的位置无效，请重新获取键盘后再输入",
                    i + 1
                )))
            }
        }
    }
    Ok(format!("1$1${plain}$1${}", secret.uuid))
}

/// 从缓存取走键盘（消耗语义）并拼出 `pwd` 串；拼完明文只在本次调用的返回值里存在，
/// 调用方把返回值直接塞进 form 后即丢弃，**不得**留存 / 打日志 / 写进错误文案。
pub fn assemble_pwd(input: PasswordInput) -> Result<String, CampusSynjonesError> {
    let secret = take_pad(&input.pad_id).ok_or_else(|| {
        CampusSynjonesError::Parse("密码键盘已过期或不存在，请重新获取键盘后再输入".to_string())
    })?;
    build_pwd(&secret, &input.positions)
}

/// 写操作响应的**双层判定**第二层（第一层 `code==200` 已由 `client` 判过）：
/// 业务层要求 `data.retcode == "0"`，失败文案取 `data.errmsg`（缺失回落顶层 `msg`）。
///
/// ⚠️ 未 live 验证：`retcode` 缺失的响应按成功放行（免得误杀 `checkPwd` 等可能不带
/// retcode 的形态）；`retcode` 存在且非 `"0"` 一律失败。学校侧的可读原因（errmsg）
/// 原样透出，不吞掉、不二次包装。
fn require_retcode_ok(v: &Value) -> Result<(), CampusSynjonesError> {
    let Some(retcode) = v.get("data").and_then(|d| d.get("retcode")) else {
        return Ok(());
    };
    if crate::ecard::text_of(Some(retcode)) == "0" {
        return Ok(());
    }
    let errmsg = v
        .get("data")
        .and_then(|d| d.get("errmsg"))
        .and_then(Value::as_str)
        .or_else(|| v.get("msg").and_then(Value::as_str))
        .unwrap_or("操作失败");
    Err(CampusSynjonesError::Api {
        code: retcode.as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
        msg: errmsg.trim().to_string(),
    })
}

/// 取 `data.<key>` 的字符串值（发码类接口用 `data.account` 作后续 `id`/`uuid`）。
fn data_text(v: &Value, key: &str, what: &str) -> Result<String, CampusSynjonesError> {
    let s = crate::ecard::text_of(v.get("data").and_then(|d| d.get(key)));
    if s.is_empty() {
        return Err(CampusSynjonesError::Parse(format!("响应缺少 {what}（{key}）")));
    }
    Ok(s)
}

/// 挂失（免密）。⚠️ 真实调用会立即冻结卡片——**只由用户显式触发，绝不重试**。
pub async fn lost_card(
    client: &SynjonesClient,
    account: &str,
) -> Result<(), CampusSynjonesError> {
    let v = client
        .post_json(EP_LOST, &[("account", account.to_string())], Envelope::Berserker)
        .await?;
    require_retcode_ok(&v)
}

/// 解挂（本校 `frontConfig` 无 `unlockFlag` → 官方默认**需密码**；`pad` 为 None 时不带
/// `pwd`/`pwdType` 字段，交给服务端按缺省校验）。
pub async fn unlost_card(
    client: &SynjonesClient,
    account: &str,
    pad: Option<PasswordInput>,
) -> Result<(), CampusSynjonesError> {
    let mut form = vec![("account", account.to_string())];
    if let Some(pad) = pad {
        form.push(("pwd", assemble_pwd(pad)?));
        form.push(("pwdType", PWD_TYPE.to_string()));
    }
    let v = client.post_json(EP_UNLOST, &form, Envelope::Berserker).await?;
    require_retcode_ok(&v)
}

/// 校验查询密码（GET，query 携带 `pwd`）。仅校验，通过即 `Ok(())`。
pub async fn check_pwd(
    client: &SynjonesClient,
    account: &str,
    pad: PasswordInput,
) -> Result<(), CampusSynjonesError> {
    let pwd = assemble_pwd(pad)?;
    let v = client
        .get(
            EP_CHECK_PWD,
            &[("account", account), ("pwd", pwd.as_str()), ("pwdType", PWD_TYPE)],
            Envelope::Berserker,
        )
        .await?;
    require_retcode_ok(&v)
}

/// 修改查询密码（三处密码各自带自己的键盘 uuid，由三次 [`assemble_pwd`] 分别消耗）。
pub async fn modify_pwd(
    client: &SynjonesClient,
    account: &str,
    old: PasswordInput,
    new: PasswordInput,
    renew: PasswordInput,
) -> Result<(), CampusSynjonesError> {
    let form = [
        ("account", account.to_string()),
        ("oldpw", assemble_pwd(old)?),
        ("newpw", assemble_pwd(new)?),
        ("renewpw", assemble_pwd(renew)?),
    ];
    let v = client.post_json(EP_MODIFY_PWD, &form, Envelope::Berserker).await?;
    require_retcode_ok(&v)
}

/// 找回密码-发验证码。返回 `data.account` 作为后续 [`find_pwd`] 的 `id`。
pub async fn send_find_pwd_code(
    client: &SynjonesClient,
    account: &str,
) -> Result<String, CampusSynjonesError> {
    let v = client
        .post_json(EP_SEND_FIND_PWD_VER, &[("account", account.to_string())], Envelope::Berserker)
        .await?;
    require_retcode_ok(&v)?;
    data_text(&v, "account", "找回密码会话 id")
}

/// 找回密码-提交新密码（免旧密，凭短信验证码 `vercode` + [`send_find_pwd_code`] 的 `id`）。
pub async fn find_pwd(
    client: &SynjonesClient,
    account: &str,
    new: PasswordInput,
    renew: PasswordInput,
    vercode: &str,
    id: &str,
) -> Result<(), CampusSynjonesError> {
    let form = [
        ("account", account.to_string()),
        ("newpw", assemble_pwd(new)?),
        ("renewpw", assemble_pwd(renew)?),
        ("vercode", vercode.to_string()),
        ("id", id.to_string()),
    ];
    let v = client.post_json(EP_FIND_PWD, &form, Envelope::Berserker).await?;
    require_retcode_ok(&v)
}

/// 限额设置（`daycostlimit`/`nonpwdlimit`/`singlelimit` 一律**分**；前端传元，此处 ×100）。
///
/// `acc_type` 来自卡上 `accinfos[].type`，形态是 `<卡号>-<子账户码>`（实测 `42940-000`）。
/// **必须拆开**：官方前端是 `account: accType.split("-")[0]` + `acctype: accType.split("-")[1]`
/// （bundle 反查 offset 221394）。整串塞给 `acctype` 会得到 `code=60006 电子账户信息不存在`
/// （2026-09-19 真机实测踩过）。
pub async fn set_limits(
    client: &SynjonesClient,
    account: &str,
    acc_type: &str,
    day_yuan: f64,
    nonpwd_yuan: f64,
    single_yuan: f64,
) -> Result<(), CampusSynjonesError> {
    let (acc, acct) = match acc_type.split_once('-') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        // 无 `-` 的形态（服务端若改口径）：account 回落到调用方给的卡号，acctype 原样
        None => (account.to_string(), acc_type.to_string()),
    };
    // 官方报文 2026-09-20 抓包：`{"account":"42940","acctype":"000","daycostlimit":50000,
    // "nonpwdlimit":0,"singlelimit":0}`——金额是 **JSON number 分**。
    let fen = |y: f64| serde_json::Value::Number(serde_json::Number::from((y * 100.0).round() as i64));
    let form = [
        ("account", serde_json::json!(acc)),
        ("acctype", serde_json::json!(acct)),
        ("daycostlimit", fen(day_yuan)),
        ("nonpwdlimit", fen(nonpwd_yuan)),
        ("singlelimit", fen(single_yuan)),
    ];
    let v = client
        .post_json_vals(EP_PAY_LIMITE_MODIFY, &form, Envelope::Berserker)
        .await?;
    require_retcode_ok(&v)
}

/// 转账标识（圈存）。
///
/// 官方报文 2026-09-20 抓包（真实提交、`retcode=0`）：
/// - flag=1：`{"account","autotransFlag":"1","autotransAmt":2000}`（**不带 limite**）
/// - flag=2：`{"account","autotransFlag":"2","autotransAmt":2000,"autotransLimite":2000}`
///
/// 要点：`autotransFlag` 是**字符串**；金额是 **JSON number 分**。
pub async fn set_autotrans(
    client: &SynjonesClient,
    account: &str,
    flag: i64,
    amt_yuan: f64,
    limite_yuan: Option<f64>,
) -> Result<(), CampusSynjonesError> {
    let fen = |y: f64| serde_json::Value::Number(serde_json::Number::from((y * 100.0).round() as i64));
    let mut form = vec![
        ("account", serde_json::json!(account)),
        ("autotransFlag", serde_json::json!(flag.to_string())),
        ("autotransAmt", fen(amt_yuan)),
    ];
    if let Some(l) = limite_yuan {
        form.push(("autotransLimite", fen(l)));
    }
    let v = client
        .post_json_vals(EP_MODIFY_ACC, &form, Envelope::Berserker)
        .await?;
    require_retcode_ok(&v)
}

/// 绑定银行卡-发验证码（`specialversion=="1"` 的学校才带 `phone`/`bankacc`；本校 =0，
/// 前端不传即不带）。返回 `data.account` 作为后续 [`bind_bank`] 的 `id`。
pub async fn send_bind_bank_code(
    client: &SynjonesClient,
    account: &str,
    phone: Option<&str>,
    bankacc: Option<&str>,
) -> Result<String, CampusSynjonesError> {
    let mut form = vec![("account", account.to_string())];
    if let Some(p) = phone {
        form.push(("phone", p.to_string()));
    }
    if let Some(b) = bankacc {
        form.push(("bankacc", b.to_string()));
    }
    let v = client.post_json(EP_SEND_BIND_BANK_VER, &form, Envelope::Berserker).await?;
    require_retcode_ok(&v)?;
    data_text(&v, "account", "绑卡会话 id")
}

/// 绑定银行卡-提交。
pub async fn bind_bank(
    client: &SynjonesClient,
    account: &str,
    bankacc: &str,
    vercode: &str,
    id: &str,
    pad: PasswordInput,
) -> Result<(), CampusSynjonesError> {
    let form = [
        ("account", account.to_string()),
        ("bankacc", bankacc.to_string()),
        ("vercode", vercode.to_string()),
        ("id", id.to_string()),
        ("pwd", assemble_pwd(pad)?),
        ("pwdType", PWD_TYPE.to_string()),
    ];
    let v = client.post_json(EP_BUILD_BANK_RELATION, &form, Envelope::Berserker).await?;
    require_retcode_ok(&v)
}

/// 解绑银行卡（免密）。
pub async fn cancel_bank(
    client: &SynjonesClient,
    account: &str,
) -> Result<(), CampusSynjonesError> {
    let v = client
        .post_json(EP_CANCEL_BANK, &[("account", account.to_string())], Envelope::Berserker)
        .await?;
    require_retcode_ok(&v)
}

/// 绑定校园卡（电子账户）-发验证码。返回 `data.account` 作为后续 [`bind_user`] 的 `uuid`。
pub async fn send_bind_user_code(
    client: &SynjonesClient,
    account: &str,
) -> Result<String, CampusSynjonesError> {
    let v = client
        .post_json(EP_SEND_BIND_USER_VER, &[("account", account.to_string())], Envelope::Berserker)
        .await?;
    require_retcode_ok(&v)?;
    data_text(&v, "account", "绑校园卡会话 uuid")
}

/// 绑定校园卡-提交（`bindType:"2"`、`verCode` 大小写照抄 bundle）。
pub async fn bind_user(
    client: &SynjonesClient,
    account: &str,
    ver_code: &str,
    id: &str,
    pad: PasswordInput,
) -> Result<(), CampusSynjonesError> {
    let form = [
        ("account", account.to_string()),
        ("bindType", "2".to_string()),
        ("uuid", id.to_string()),
        ("verCode", ver_code.to_string()),
        ("pwd", assemble_pwd(pad)?),
        ("pwdType", PWD_TYPE.to_string()),
    ];
    let v = client.post_json(EP_BIND_USER, &form, Envelope::Berserker).await?;
    require_retcode_ok(&v)
}

/// 解绑校园卡（`bindType:"2"`；`remark` 缺省传空串，bundle 形态里该键必带）。
pub async fn unbind_user(
    client: &SynjonesClient,
    account: &str,
    remark: Option<&str>,
    pad: PasswordInput,
) -> Result<(), CampusSynjonesError> {
    let form = [
        ("account", account.to_string()),
        ("bindType", "2".to_string()),
        ("remark", remark.unwrap_or("").to_string()),
        ("pwd", assemble_pwd(pad)?),
        ("pwdType", PWD_TYPE.to_string()),
    ];
    let v = client.post_json(EP_UNBIND_USER, &form, Envelope::Berserker).await?;
    require_retcode_ok(&v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// **实测形态**（D3/D4）：键位是字符串、图片数组等长；Number 只有一组。
    #[test]
    fn parse_keyboard_reads_live_shapes() {
        let number = json!({
            "numberKeyboard": "X!-/n%~_Bv",
            "numberKeyboardImage": ["i0", "i1", "i2", "i3", "i4", "i5", "i6", "i7", "i8", "i9"],
            "password": "x",
            "uuid": "0123456789abcdef0123456789abcdef"
        });
        let (uuid, keys, images) = parse_keyboard(&number).expect("Number 键盘应可解析");
        assert_eq!(uuid, "0123456789abcdef0123456789abcdef");
        assert_eq!(keys.len(), 10, "实测恒 10 键");
        assert_eq!(keys[0], "X");
        assert_eq!(keys[9], "v");
        assert_eq!(images.len(), 10, "图片与键位一一对应");

        // Standard：四组按固定顺序拼接（数字→大写→小写→符号）
        let standard = json!({
            "numberKeyboard": "._:U=uZlMV",
            "upperLetterKeyboard": "xF]8<0hARe6",
            "lowerLetterKeyboard": "'$m9Q\\{@;p/7Ldr",
            "symbolKeyboard": "O&g(aYc%qo!>3wb`s^2Df",
            "uuid": "6fe409cf-7fa4-44a1-888d-67a10f2eda49"
        });
        let (_, keys, images) = parse_keyboard(&standard).expect("Standard 键盘应可解析");
        assert_eq!(keys.len(), 10 + 11 + 15 + 21);
        assert_eq!(keys[0], ".", "第一组必须是数字组（前端按下标渲染，顺序不可变）");
        assert_eq!(keys[10], "x");
        assert!(images.is_empty(), "未下发图片 → 空表（渲染层自行降级）");
    }

    /// 容错：缺 uuid / 键位空 / 键位非字符串数组 → None（宁可取不到键盘，不给残缺键盘）。
    #[test]
    fn parse_keyboard_rejects_broken_payloads() {
        assert!(parse_keyboard(&json!({})).is_none());
        assert!(parse_keyboard(&json!({"uuid": ""})).is_none());
        assert!(parse_keyboard(&json!({"uuid": "u", "numberKeyboard": ""})).is_none());
        assert!(parse_keyboard(&json!({"uuid": "u", "numberKeyboard": 123})).is_none());
        // 数组形态（类型漂移先例）也吃得下
        let arr = parse_keyboard(&json!({"uuid": "u", "numberKeyboard": ["1", "2"]}));
        assert_eq!(arr.unwrap().1, vec!["1", "2"]);
    }

    /// 缓存生命周期（**合并为单测**：共享 static，拆开并行跑会互相干扰容量断言）：
    /// take 取走即删 → 过期作废 → 超量淘汰最旧。
    #[test]
    fn pad_cache_lifecycle() {
        // 1) 取走即删（消耗语义）；pad_id 与真实 uuid 不相同
        let pad_id = store_pad("uuid-real-1".to_string(), vec!["1".into(), "2".into()]);
        assert!(pad_id.starts_with("pad"));
        assert_ne!(pad_id, "uuid-real-1", "真实 uuid 绝不当 pad_id 外泄");
        let secret = take_pad(&pad_id).expect("首次 take 应取到");
        assert_eq!(secret.uuid, "uuid-real-1");
        assert_eq!(secret.keys, vec!["1".to_string(), "2".to_string()]);
        assert!(take_pad(&pad_id).is_none(), "取走即删（重复 take 拿不到）");
        assert!(take_pad("不存在的id").is_none());

        // 2) 过期：TTL 之后的条目取不到（回拨 created，不真等 5 分钟）
        let expired_id = store_pad("uuid-exp".to_string(), vec!["1".into()]);
        if let Ok(mut list) = pads().lock() {
            if let Some(e) = list.iter_mut().find(|e| e.pad_id == expired_id) {
                e.created = Instant::now()
                    .checked_sub(PAD_TTL + Duration::from_secs(1))
                    .expect("回拨时长应合法");
            }
        }
        assert!(take_pad(&expired_id).is_none(), "过期的键盘映射必须作废");

        // 3) 容量上限：第 9 把进来时最旧的一把被淘汰
        let ids: Vec<String> = (0..MAX_PADS)
            .map(|i| store_pad(format!("u{i}"), vec!["k".into()]))
            .collect();
        let newest = store_pad("u-new".to_string(), vec!["k".into()]);
        assert!(take_pad(&newest).is_some(), "新键盘必须可用");
        assert!(take_pad(&ids[0]).is_none(), "超出容量最旧被淘汰");
        assert!(take_pad(&ids[1]).is_some(), "次旧的仍在");
    }

    /// 红线：`SecurePad` / `PadSecret` 的 Debug 必须打码。
    #[test]
    fn debug_masks_secret_material() {
        let pad = SecurePad { pad_id: "pad-secret".into(), keys: vec!["9".into()], images: vec![] };
        let dbg = format!("{pad:?}");
        assert!(!dbg.contains("pad-secret"));
        assert!(!dbg.contains('9'));
        assert!(dbg.contains("1 键"));

        let secret = PadSecret { uuid: "uuid-secret".into(), keys: vec!["8".into()] };
        let dbg = format!("{secret:?}");
        assert!(!dbg.contains("uuid-secret"));
        assert!(!dbg.contains('8'));
    }

    /// KeyboardKind：参数解析与服务端 type 值（大小写敏感）。
    #[test]
    fn keyboard_kind_parses_strictly() {
        assert_eq!(KeyboardKind::parse("number"), Some(KeyboardKind::Number));
        assert_eq!(KeyboardKind::parse(" standard "), Some(KeyboardKind::Standard));
        assert_eq!(KeyboardKind::parse("Number"), None, "命令参数只认小写");
        assert_eq!(KeyboardKind::parse(""), None);
        assert_eq!(KeyboardKind::Number.type_param(), "Number");
        assert_eq!(KeyboardKind::Standard.type_param(), "Standard");
    }

    // ---------------- 写操作（批 3） ----------------

    /// `pwd` 拼装：位置序列 → `"1$1$" + 明文 + "$1$" + uuid`（§1.8 bundle 反查形态）。
    /// 纯函数打在 [`build_pwd`] 上（不碰共享 pad 缓存，避免与并行用例互相淘汰干扰）。
    #[test]
    fn build_pwd_translates_positions() {
        let secret = PadSecret {
            uuid: "uuid-1234".into(),
            keys: ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        // 用户点的是 9,5,2,0 → 明文按键位表翻译（不是数字本身，是 keys[pos]）
        let pwd = build_pwd(&secret, &[9, 5, 2, 0]).expect("合法位置应拼装成功");
        assert_eq!(pwd, "1$1$0631$1$uuid-1234");
        // 全键盘（四组拼接）同规则
        let std = PadSecret { uuid: "u".into(), keys: vec!["a".into(), "B".into(), "c".into()] };
        assert_eq!(build_pwd(&std, &[1, 0, 2]).unwrap(), "1$1$Bac$1$u");
    }

    /// `pwd` 拼装失败：位置越界 / 空序列 → 可读错误，**错误文案不含任何键位内容**。
    #[test]
    fn build_pwd_rejects_bad_positions_without_leaking() {
        let secret = PadSecret {
            uuid: "uuid-x".into(),
            keys: vec!["7".into(), "8".into(), "9".into()],
        };
        let e = build_pwd(&secret, &[0, 3]).expect_err("下标 3 越界应报错");
        assert!(e.to_string().contains("位置无效"));
        assert!(!e.to_string().contains('7'), "不得泄漏键位：{e}");
        let e = build_pwd(&secret, &[]).expect_err("空序列应报错");
        assert!(e.to_string().contains("请输入密码"));
    }

    /// [`assemble_pwd`] 的失败分支：padId 过期 / 不存在 → 可读错误。
    /// 成功分支 = take_pad（已有 `pad_cache_lifecycle` 钉死）+ [`build_pwd`]（上两条）的组合。
    #[test]
    fn assemble_pwd_rejects_missing_pad() {
        let e = assemble_pwd(PasswordInput {
            pad_id: "不存在的pad".into(),
            positions: vec![0],
        })
        .expect_err("padId 不存在应报错");
        assert!(e.to_string().contains("密码键盘已过期"));
        // Debug 打码：PasswordInput 不外泄 pad_id 与点击序列内容
        let dbg = format!(
            "{:?}",
            PasswordInput { pad_id: "pad-secret-xyz".into(), positions: vec![1, 2] }
        );
        assert!(!dbg.contains("pad-secret-xyz"), "实际 {dbg}");
    }

    /// 双层判定（§1.8）：`code=200`（client 已判）之外还要求 `data.retcode=="0"`。
    #[test]
    fn retcode_double_layer_judgement() {
        // 成功：retcode="0"
        require_retcode_ok(&json!({"code": 200, "data": {"retcode": "0"}, "msg": "ok"}))
            .expect("retcode=0 应成功");
        // 失败：retcode="1" + data.errmsg → errmsg 原样透出
        let e = require_retcode_ok(&json!({
            "code": 200, "data": {"retcode": "1", "errmsg": "密码错误，请重新输入"}, "msg": "顶层msg"
        }))
        .expect_err("retcode=1 应失败");
        match &e {
            CampusSynjonesError::Api { code, msg } => {
                assert_eq!(*code, 1);
                assert_eq!(msg, "密码错误，请重新输入", "errmsg 优先于顶层 msg");
            }
            other => panic!("实际 {other:?}"),
        }
        // errmsg 缺失 → 回落顶层 msg
        let e = require_retcode_ok(&json!({
            "code": 200, "data": {"retcode": "9"}, "msg": "系统繁忙"
        }))
        .expect_err("应失败");
        assert!(e.to_string().contains("系统繁忙"), "实际 {e}");
        // 两者都缺 → 通用文案
        let e = require_retcode_ok(&json!({"code": 200, "data": {"retcode": "2"}}))
            .expect_err("应失败");
        assert!(e.to_string().contains("操作失败"), "实际 {e}");
        // retcode 缺失 → 放行（未 live 验证形态不误杀）
        require_retcode_ok(&json!({"code": 200, "data": {"foo": 1}})).expect("无 retcode 应放行");
        // retcode 数字形态（类型漂移先例）也吃得下
        require_retcode_ok(&json!({"code": 200, "data": {"retcode": 0}}))
            .expect("数字 0 应成功");
    }

    /// 元 → 分换算的浮点陷阱（`0.1` 元必须得 10 分；`0.29`×100=28.999… round 后 29）。
    /// 现在金额统一走 JSON number（`set_limits`/`set_autotrans` 内联 `(y*100).round()`），
    /// 该口径用同样的算式钉在这里。
    #[test]
    fn yuan_to_fen_handles_float_traps() {
        let fen = |y: f64| (y * 100.0).round() as i64;
        assert_eq!(fen(0.1), 10);
        assert_eq!(fen(0.29), 29, "0.29×100=28.999…，round 后必须 29");
        assert_eq!(fen(79.96), 7996);
        assert_eq!(fen(100.0), 10000);
        assert_eq!(fen(0.0), 0);
    }

    /// PasswordInput 的 PartialEq/Clone 形态（命令层要按参数重组它）。
    #[test]
    fn password_input_is_plain_data() {
        let a = PasswordInput { pad_id: "p".into(), positions: vec![1, 2] };
        let b = a.clone();
        assert_eq!(a, b);
    }
}
