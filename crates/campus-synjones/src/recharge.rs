//! 电费充值（App 口径：`/blade-pay/pay` + `/charge/pay/getpayinfo` + `/charge/order/deleteOrder`）。
//!
//! # 协议事实（2026-09-19 App bundle `/charge-app/static/js/app.*.js` 逐字核对 + 匿名实测）
//!
//! | 步 | 请求 | 关键参数 | 响应 |
//! |---|---|---|---|
//! | ① 建单 | `POST /blade-pay/pay` | `feeitemid, tranamt, flag=choose, source=app, paystep=0[, third_party]` | `data.orderid`（`data.orderflag=="results"` 时直接进结果） |
//! | ② 支付方式 | `GET /charge/pay/getpayinfo` | `orderid` | **顶层** `{order:{status,payexpdate,tranamt}, payList:[…]}` |
//! | ③ 查账户 | `POST /blade-pay/pay` | `orderid, paystep=2, paytype, paytypeid[, accountno]` | `data.accountno` / `data.ccctype` / `data.passwordMap{uuid:[10 键]}` |
//! | ④ 提交 | `POST /blade-pay/pay` | ③ + `accountno, ccctype, isWX=0`（+ 需密码时 `password`=**下标序列**、`uuid`） | 命中红线 5 即报错退出 |
//! | ⑤ 结果 | `GET /charge/pay/getpayinfo` | `orderid` | `order.status`：**0=待支付、1=已完成** |
//! | ⑥ 清理 | `POST /charge/order/deleteOrder` | `orderid` | 无 |
//!
//! - **`/blade-pay/pay` 是绝对路径**（App 的 `payApi = window.location.origin`），`/charge/*` 是 App 的
//!   `baseApi = <origin>/charge`——两者同源，故都作为 `BERSERKER_BASE` 之后的路径拼接。
//! - **`payList` 在 ② 的顶层**，不在 `paystep` 响应里（本项目最易误判处）。
//! - `paytype` ← `payList[i].code`；`paytypeid` ← `payList[i].payid`；默认取 `payList[0]`。
//! - **免密判据只有 `nopassword === 1`**（见 [`is_no_password`]）。实测 450 的真实值是
//!   `ACCOUNTTSM`/`payid=64`/`nopassword=false`（JSON **布尔** `false`，即需密码），
//!   `payList` 里只有这一条账户类渠道。
//! - `ACCOUNTTSM`/`CARD` 类的 `paystep:2` **再带 `ccctype` 就会被服务端当成提交**（实测回
//!   `code=400 密码为空`）——故查询账户绝不能带 `ccctype`，提交才带（见 [`query_account`]/[`submit_pay`]）。
//! - `passwordMap` **只在 query 带 `accountno` 时下发**（实测：不带 → 无 `passwordMap`；带 → 有）。
//! - 错误文案字段实测是 **`msg`**（匿名实测 `POST /blade-pay/pay` 回
//!   `{"code":401,"success":false,"data":null,"msg":"请求未授权"}`、`getpayinfo` 回
//!   `{"msg":"未查询到订单信息","code":500}`），故四个端点都用 [`Envelope::Berserker`] 解析：
//!   它只决定「读哪个键当错误文案」，`code==200` 即成功的判据两套信封一致
//!   （计划 §2.1 写的 `Envelope::Charge` 读 `message`，会把这些服务端原文丢掉）。
//! - ⚠️ **取消订单必须 JSON body**：`POST /charge/order/deleteOrder` 用 form / query / GET 一律
//!   `code=500 未知异常，请联系管理员`，只有 `Content-Type: application/json` + `{"orderid":"…"}`
//!   才回 `code=200 success`（实测 6 形态矩阵，见 [`cancel_order`]）。
//! - 官方 App 请求头另有 `Authorization: Basic Y2hhcmdlOmNoYXJnZV9zZWNyZXQ=`（= `charge:charge_secret`，
//!   bundle 硬编码**公开**常量），实测**非强制**（带/不带响应完全一致，匿名探针 `recharge_probe_live` 结论）；
//!   本项目统一头组由 [`SynjonesClient`] 注入，不额外加它（`client.rs` 不在本批改动范围）。
//! - 官方在请求头里**不发 `synAccessSource`**，而 [`SynjonesClient`] 会强制双份携带 `app`——
//!   实测被接受（见 live 测试 `recharge_live`），保留现状以免为省一个无害参数改 client 层。
//!
//! # 电费上下文串 `third_party`（**后端合成，前端拿不到、也不该拿**）
//!
//! App 口径：`third_party = JSON.stringify(末级 IEC 响应 map.data [+ myCustomInfo])`；多房间拆分缴费时
//! 再缀 `-<选中 id 列表>-<金额列表>`（App bundle：`this.third_party + o`，只在 `choosePaidVal` 非空时）。
//! 那段 `map.data` 含**户号（account）**等 PII，`charge.rs` 刻意不透出明细，故本模块**自己重调一次
//! `getThirdData`（`type=IEC`）取 `map.data`**，在 crate 内拼成串再随建单请求发出——PII 全程不出后端，
//! 前端也无从注入一个错误/伪造的房间上下文（见 [`third_party_for_room`]）。
//!
//! 单房间充值（本项目唯一形态）实测**只需 JSON 本体**、不缀 `-ids-金额`（那是官方「选应收项」形态），
//! 服务端接受并成功建单。
//!
//! # 红线（改本文件前先读）
//!
//! 1. **绝不接触真实卡密码**：见 [`PasswordPad`]——`keys` 只是键盘**显示字符**，只用于渲染；
//!    提交的是**键位下标序列**。keys 绝不还原、绝不落盘、绝不打日志、绝不回填输入框。
//! 2. **不碰金额语义**：`tranamt` 原样透传（只校验「非空且为正数」，不改写、不补零、不四舍五入）。
//! 3. **副作用请求不重试**：本模块不加重试/超时重发。[`SynjonesClient`] 内部仅在服务端**已明确拒绝**
//!    （401/403x）时静默重进并重发一次——那种情况不会产生副作用。
//! 4. **轮询有上限**：本模块只提供单次查询，次数/总时长上限由调用方（命令层/前端）把关。
//! 5. **拒绝跳转分支**：[`reject_redirect`] 命中 `webUrl`/`paysubmit`/`paymentcashierStr`/`qrCodeUrl`
//!    一律报错退出，不实现、不组装表单、不打开浏览器。

use crate::charge::{self, RoomStep};
use crate::client::{parse_envelope, Envelope, SynjonesClient};
use crate::CampusSynjonesError;
use serde::Serialize;
use serde_json::Value;

/// 支付动作端点（绝对路径；`/blade-pay/pay` 承载建单 / 查账户 / 提交三件事，靠 `paystep` 区分）。
pub const EP_BLADE_PAY: &str = "/blade-pay/pay";
/// 订单 + 支付方式查询（相对 `BERSERKER_BASE`；结果轮询复用同一端点，官方也是 2s 一次重查它）。
pub const EP_PAY_INFO: &str = "/charge/pay/getpayinfo";
/// 取消/清理未支付订单（**必须 JSON body**，见 [`cancel_order`]）。
pub const EP_DELETE_ORDER: &str = "/charge/order/deleteOrder";

/// 建单 `flag`（电费恒 `choose`；`mustbe`/`addcart`/`library` 是其它缴费形态，本项目不用）。
pub const FLAG_CHOOSE: &str = "choose";
/// 建单 `source`（App 口径固定值）。
pub const SOURCE_APP: &str = "app";
/// 电费级联的末级请求类型（`getThirdData` 取最终房间视图；与 `charge.rs` 同口径）。
const IEC_TYPE: &str = "IEC";
/// `paystep`：0 = 建单，2 = 查账户 / 提交支付。
const PAYSTEP_CREATE: &str = "0";
const PAYSTEP_PAY: &str = "2";

/// 支付方式 `code` 白名单：**只保留校园卡/电子账户**（展示层过滤，见计划 §1.3）。
/// 其它（`CARD`/`CARDTSM`/`PAYMENTPLATFORM`/`ALIPAY` 等）一律不展示——它们要么跳转第三方，
/// 要么走官方「只剩一种就自动提交」的隐式行为，本项目都不采用。
pub const ACCOUNT_CODES: [&str; 2] = ["ACCOUNT", "ACCOUNTTSM"];

/// 密码键盘位数（官方 6 位、点满自动提交；仅用于入参校验，不参与任何密码运算）。
pub const PASSWORD_LEN: usize = 6;

/// 跳转分支字段（红线 5）：出现任一即拒绝，绝不实现。
const REDIRECT_KEYS: [&str; 4] = ["webUrl", "paysubmit", "paymentcashierStr", "qrCodeUrl"];

/// 一种支付方式（`payList` 条目）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayMethod {
    /// 提交时的 `paytype`（如 `ACCOUNT`/`ACCOUNTTSM`）。
    pub code: String,
    /// 提交时的 `paytypeid`。
    pub payid: String,
    /// 展示名（如「校园卡电子账户」）。
    pub name: String,
    /// **免密**：仅 `nopassword == 1` 为 true（0/缺失/其它真值 → 需密码，见 [`is_no_password`]）。
    pub nopassword: bool,
    /// 备注（`remark`，官方用 `remark.indexOf("weixin")` 判微信渠道；空串则 None）。
    pub remark: Option<String>,
}

/// 安全键盘数据：**只用于渲染**——`keys[i]` 是第 `i` 个键的**显示字符**，`uuid` 是服务端给这次
/// 键盘发的随机标识。
///
/// # 红线（务必逐条遵守）
///
/// - 提交给服务端的是用户**点击的键位下标**（`keys` 的下标序列，如 `"012345"`），**不是** `keys` 里的字符；
/// - **绝不**把 `keys` + 下标反推/还原成真实密码；
/// - **绝不**落盘、**绝不**打日志（本类型的 `Debug` 手写打码）、**绝不**上报、**绝不**回填任何输入框；
/// - `keys`/`uuid` 只在内存存活到该笔支付结束（取新键盘即换新 uuid）。
#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordPad {
    /// 服务端下发的键盘标识（= `passwordMap` 的键），随提交体一起回传。
    pub uuid: String,
    /// 键位显示字符（实测 10 个）；**只渲染，禁止还原/持久化/日志**。
    pub keys: Vec<String>,
}

/// `Debug` 打码：键盘字符**绝不**进日志/panic 消息（`{:?}` 只报个数）。
impl std::fmt::Debug for PasswordPad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PasswordPad {{ uuid: ***, keys: <{} 键，已打码> }}",
            self.keys.len()
        )
    }
}

/// 订单状态视图（`getpayinfo` 顶层 `order` 的判据字段）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RechargeOrder {
    /// 订单号（`orderid`）。
    pub order_id: String,
    /// **0=待支付、1=已完成**（实测口径；缺字段时按 0 待支付处理）。
    pub status: u8,
    /// 支付截止时间（`payexpdate`，服务端本地时间字符串，原样透出）。
    pub pay_exp_date: Option<String>,
    /// 订单金额（元；服务端数值或数字字符串）。
    pub tranamt: Option<f64>,
}

// ---------------- 纯解析（单测覆盖，无网络） ----------------

/// 文本字段：字符串原样（trim），数字转字符串，缺失/其它 → 空串。
fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// 元金额字段：数字或数字字符串均可（服务端形态不定，见 `charge.rs` 同款处理）。
fn yuan_of(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// **免密判据（唯一）**：`nopassword === 1`。
///
/// 官方口径（`submitPay`/`toPay`）是：
/// - `1`（数字）→ 免密，提交体不含 `password`/`uuid`；
/// - `0` / 缺失 / `false` → 需密码，弹安全键盘；
/// - **其它真值（如 `2`）→ 官方是死分支**（`0!==nopassword && nopassword` 成立但 `1===nopassword` 不成立，
///   于是既不提交也不弹键盘，什么都不做）。本项目**按「需密码」处理**——宁可多要一次键盘，
///   也不静默卡死（这条官方死分支永不主动复现，除非学校改下发值）。
///
/// 另：服务端类型漂移在本仓已有先例（`status`/`layout`/`retain_money` 都见过字符串形态），
/// 故字符串 `"1"` 也认作免密（官方的严格 `===1` 会把它当死分支，属官方脆弱点，不照抄）。
fn is_no_password(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        Some(Value::String(s)) => s.trim() == "1",
        _ => false,
    }
}

/// `payList` 条目 → [`PayMethod`]（不做过滤）。
fn parse_pay_method(it: &Value) -> PayMethod {
    let remark = text_of(it.get("remark"));
    PayMethod {
        code: text_of(it.get("code")),
        payid: text_of(it.get("payid")),
        name: text_of(it.get("name")),
        nopassword: is_no_password(it.get("nopassword")),
        remark: (!remark.is_empty()).then_some(remark),
    }
}

/// `payList` → 支付方式列表：**只保留 [`ACCOUNT_CODES`]**（校园卡/电子账户），且必须同时有
/// `code` 与 `payid`，否则该项不可提交（缺一即丢弃，避免前端拿到点不动的项）。
///
/// 不照抄官方「只剩一种支付方式就自动提交」的行为，也不伪造/改写 `code`。
pub fn parse_pay_methods(v: &Value) -> Vec<PayMethod> {
    v["payList"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(parse_pay_method)
                .filter(|m| ACCOUNT_CODES.contains(&m.code.as_str()))
                .filter(|m| !m.payid.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// `getpayinfo` 顶层 `order` → [`RechargeOrder`]。
fn parse_order(v: &Value) -> RechargeOrder {
    let order = v.get("order").cloned().unwrap_or(Value::Null);
    let exp = text_of(order.get("payexpdate"));
    RechargeOrder {
        order_id: text_of(order.get("orderid")),
        status: match order.get("status") {
            Some(Value::Number(n)) => n.as_u64().unwrap_or(0) as u8,
            Some(Value::String(s)) => s.trim().parse::<u8>().unwrap_or(0),
            // 缺字段不视为异常：刚建单未支付即 0，前端按「待支付」继续走支付方式流程
            _ => 0,
        },
        pay_exp_date: (!exp.is_empty()).then_some(exp),
        tranamt: yuan_of(order.get("tranamt")),
    }
}

/// 账号列表：实测两种形态——对象数组 `[{"accountno":"…"}]` 与**裸字符串数组** `["…"]`，
/// 也容忍单字符串（`paystep:2` 带 `accountno` 时服务端回显为字符串）。
fn parse_accounts(data: &Value) -> Vec<String> {
    match data.get("accountno") {
        Some(Value::Array(list)) => list
            .iter()
            .map(|it| match it {
                Value::Object(_) => text_of(it.get("accountno")),
                other => text_of(Some(other)),
            })
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) if !s.trim().is_empty() => vec![s.trim().to_string()],
        // ⚠️ 官方会回落把整个 data 当数组（`a.data.accountno || a.data`），本项目不照抄：
        // 只在明确是 accountno 字段时取值，避免把无关对象当账号传给前端。
        _ => Vec::new(),
    }
}

/// 账户类型（`ccctype`）列表：只取类型码（`balance` 归官方 UI 展示用，本项目暂不需要）。
fn parse_ccctypes(data: &Value) -> Vec<String> {
    data["ccctype"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|it| {
                    if it.is_object() {
                        text_of(it.get("ccctype"))
                    } else {
                        text_of(Some(it))
                    }
                })
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// `passwordMap{uuid:[10 个显示字符]}` → [`PasswordPad`]（单键取值；无键或空键位表 → None）。
///
/// ⚠️ **实测（2026-09-19 live）`passwordMap[uuid]` 是「10 个字符的字符串」，不是数组**
/// （`{"<uuid>": "…"}`，长度 10）——官方前端 `keys` 直接接这个字符串并用 `v-for` 逐字符渲染，
/// 故这里拆成字符数组喂给同一套 UI（[`PasswordPad::keys`] 的下标语义不变）。
/// 数组形态（`["1","2",…]`）也一并接受：服务端类型漂移在本仓有先例，且两种形态下标语义一致。
///
/// **本函数是红线第一道关**：返回的 `keys` 只能交给渲染层，禁止任何形式的还原/落盘/日志（见 [`PasswordPad`]）。
fn parse_password_pad(data: &Value) -> Option<PasswordPad> {
    let map = data.get("passwordMap")?.as_object()?;
    // 实测恒单键；多键时取首个（官方 `for-in` 实际取末个，此处差异无实害——都不还原）
    let (uuid, raw) = map.iter().next()?;
    let keys: Vec<String> = match raw {
        Value::String(s) => s.chars().map(|c| c.to_string()).collect(),
        Value::Array(a) => a.iter().map(|k| text_of(Some(k))).collect(),
        _ => return None,
    };
    (!uuid.trim().is_empty() && !keys.is_empty() && !keys.iter().any(|k| k.is_empty())).then(|| {
        PasswordPad {
            uuid: uuid.trim().to_string(),
            keys,
        }
    })
}

/// 红线 5：响应 `data` 里若出现跳转分支字段 → 报错退出（不实现、不组装表单、不跳浏览器）。
fn reject_redirect(data: &Value) -> Result<(), CampusSynjonesError> {
    if let Some(k) = REDIRECT_KEYS.iter().find(|k| {
        data.get(*k)
            .is_some_and(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
    }) {
        return Err(CampusSynjonesError::Api {
            code: -1,
            msg: format!(
                "学校返回了需要跳转的支付通道（{k}），本项目不支持该通道——请改用「去官网充值」"
            ),
        });
    }
    Ok(())
}

// ---------------- 取数 ----------------

/// 把末级 `map.data`（+ `myCustomInfo`）拼成电费 `third_party` 串（**纯函数，单测覆盖**）。
///
/// 官方口径：`third_party = JSON.stringify(map.data)`，其中 `myCustomInfo` = `"<末级名>：<各级名 空格拼接>"`
/// （App/PC 同款：`(label ? label+"：" : "") + dataStr`；`label` 取 `map.total` 末级的 name，实测「房间」）。
/// 各级名全空时**不加** `myCustomInfo`（官方同样跳过）。
///
/// `map.data` 不是对象（房间非法/服务端不返回）→ 报错：宁可让用户重选房间，也不发一个没有房间上下文的建单。
fn compose_third_party(map_data: &Value, path: &[RoomStep]) -> Result<String, CampusSynjonesError> {
    if !map_data.is_object() {
        return Err(CampusSynjonesError::Parse(
            "级联响应没有房间明细（map.data），请重新选择房间".to_string(),
        ));
    }
    let names: Vec<&str> = path
        .iter()
        .map(|s| s.name.trim())
        .filter(|n| !n.is_empty())
        .collect();
    let mut data = map_data.clone();
    if !names.is_empty() {
        let label = path
            .last()
            .map(|s| s.name.trim())
            .filter(|n| !n.is_empty())
            .unwrap_or_default();
        let text = names.join(" ");
        data["myCustomInfo"] = Value::String(if label.is_empty() {
            text
        } else {
            format!("{label}：{text}")
        });
    }
    serde_json::to_string(&data)
        .map_err(|e| CampusSynjonesError::Parse(format!("房间上下文序列化失败：{e}")))
}

/// 电费 `third_party`：**后端自己组**（见模块头注「电费上下文串」）。
///
/// 按房间路径重放一次 `getThirdData`（`type=IEC`，`level` = 路径深度）取末级 `map.data`，拼成串返回。
/// `map.data` 含户号等 PII：**只在这个函数里出现，只用于建单**，不进日志、不进错误文案、不返回给前端。
pub async fn third_party_for_room(
    client: &SynjonesClient,
    feeitem_id: &str,
    path: &[RoomStep],
) -> Result<String, CampusSynjonesError> {
    let feeitem_id = feeitem_id.trim();
    if feeitem_id.is_empty() {
        return Err(CampusSynjonesError::Parse("缺少片区 id".to_string()));
    }
    if path.is_empty() {
        return Err(CampusSynjonesError::Parse("请先选择房间".to_string()));
    }
    if let Some(bad) = path.iter().position(|s| s.value.trim().is_empty()) {
        return Err(CampusSynjonesError::Parse(format!("第 {} 级未填值", bad + 1)));
    }
    let mut form: Vec<(&str, String)> = vec![
        ("feeitemid", feeitem_id.to_string()),
        ("type", IEC_TYPE.to_string()),
        ("level", path.len().to_string()),
    ];
    form.extend(path.iter().map(|s| (s.code.as_str(), s.value.clone())));
    let v = client
        .post_form(charge::EP_GET_THIRD_DATA, &form, Envelope::Berserker)
        .await?;
    compose_third_party(&v["map"]["data"], path)
}

/// 建单：`POST /blade-pay/pay`（`paystep=0`），返回 `orderid`。
///
/// `tranamt` **原样透传**（字符串，元）；`path` 是当前房间的完整级联路径（校区 → 楼栋 → 房间），
/// `Some(path)` 时 `third_party` 由 [`third_party_for_room`] 在 crate 内合成——**调用方无法
/// （也不需要）自己拼**，房间上下文因此不会被伪造或写错。
///
/// `path = None` ⇒ **无级联片区**（2026-09-19 live 实测：一卡通充值 `feeitemid=401` 的
/// `getThirdData` 回 `code=500`，没有级联上下文），建单体**不带 `third_party` 字段**——
/// 官方建单体对该字段本就是可选的（电费带、其它缴费项不带）。注意 `Some(空路径)` 仍按
/// 电费口径报「请先选择房间」（空路径 ≠ 无级联，见 [`third_party_for_room`]）。
///
/// **副作用请求**：超时不重试（红线 3）；失败后若已产生订单，调用方须 `cancel_order` 清理（红线 6）。
pub async fn create_order(
    client: &SynjonesClient,
    feeitem_id: &str,
    tranamt: &str,
    path: Option<&[RoomStep]>,
) -> Result<String, CampusSynjonesError> {
    let feeitem_id = feeitem_id.trim().to_string();
    if feeitem_id.is_empty() {
        return Err(CampusSynjonesError::Parse("缺少片区 id".to_string()));
    }
    let tranamt = tranamt.trim().to_string();
    // 金额只做「像不像一个正数」的校验，不做任何改写（红线 2）
    match tranamt.parse::<f64>() {
        Ok(n) if n > 0.0 => {}
        _ => {
            return Err(CampusSynjonesError::Parse(
                "充值金额必须是一个大于 0 的数字".to_string(),
            ))
        }
    }
    let third_party = match path {
        Some(path) => Some(third_party_for_room(client, &feeitem_id, path).await?),
        None => None,
    };
    let mut form: Vec<(&str, String)> = vec![
        ("feeitemid", feeitem_id),
        ("tranamt", tranamt),
        ("flag", FLAG_CHOOSE.to_string()),
        ("source", SOURCE_APP.to_string()),
        ("paystep", PAYSTEP_CREATE.to_string()),
    ];
    if let Some(third_party) = third_party {
        form.push(("third_party", third_party));
    }
    let v = client.post_form(EP_BLADE_PAY, &form, Envelope::Berserker).await?;
    let order_id = text_of(v["data"].get("orderid"));
    if order_id.is_empty() {
        return Err(CampusSynjonesError::Parse(
            "建单成功但响应缺 data.orderid".to_string(),
        ));
    }
    Ok(order_id)
}

/// 支付方式 + 订单状态（`GET /charge/pay/getpayinfo?orderid=`）。
pub async fn fetch_pay_methods(
    client: &SynjonesClient,
    order_id: &str,
) -> Result<(RechargeOrder, Vec<PayMethod>), CampusSynjonesError> {
    let v = payinfo(client, order_id).await?;
    Ok((parse_order(&v), parse_pay_methods(&v)))
}

/// 结果轮询用的同一端点查询（`order.status`：0=待支付、1=已完成）。
///
/// 与 [`fetch_pay_methods`] 是**同一端点同一解析**（官方 `payResult` 页也是 2s 一次重查 `getpayinfo`），
/// 保留两个名字只为调用点语义清晰：轮询时并不关心 `payList`。
pub async fn fetch_order_status(
    client: &SynjonesClient,
    order_id: &str,
) -> Result<(RechargeOrder, Vec<PayMethod>), CampusSynjonesError> {
    fetch_pay_methods(client, order_id).await
}

/// `getpayinfo` 单次请求（两处入口共用，避免重复拼参）。
async fn payinfo(client: &SynjonesClient, order_id: &str) -> Result<Value, CampusSynjonesError> {
    let order_id = order_id.trim();
    if order_id.is_empty() {
        return Err(CampusSynjonesError::Parse("缺少订单号".to_string()));
    }
    client.get(EP_PAY_INFO, &[("orderid", order_id)], Envelope::Berserker).await
}

/// 查账户（`paystep=2`）：返回 `(账号列表, 账户类型列表, 安全键盘?)`。
///
/// `accountno` 语义（官方 `getAccountno` → 选中 → `getAccounttype` 两步）：
/// - **None**：服务端回该支付方式下的**账号列表**（`data.accountno` 数组），据此渲染「选择账号」；
/// - **Some(已选账号)**：服务端回该账号的**账户类型**（`data.ccctype`）与**安全键盘**
///   （`data.passwordMap`）——需密码的支付方式在这一步才拿到键盘。
///   故 `accountno` 是协议的一部分，不是可选的便利参数（计划 §2.1 的签名漏了它，本实现为其超集）。
///
/// # 红线
///
/// 返回的 [`PasswordPad`]（`passwordMap`）**只允许**交给渲染层画键盘；调用方**绝不**打印它、
/// 落盘它、或把 `keys` 还原成密码；提交时只回传**键位下标序列**（见 [`submit_pay`]）。
pub async fn query_account(
    client: &SynjonesClient,
    order_id: &str,
    pay: &PayMethod,
    accountno: Option<&str>,
) -> Result<(Vec<String>, Vec<String>, Option<PasswordPad>), CampusSynjonesError> {
    let order_id = order_id.trim();
    if order_id.is_empty() {
        return Err(CampusSynjonesError::Parse("缺少订单号".to_string()));
    }
    let mut form: Vec<(&str, String)> = vec![
        ("orderid", order_id.to_string()),
        ("paystep", PAYSTEP_PAY.to_string()),
        ("paytype", pay.code.clone()),
        ("paytypeid", pay.payid.clone()),
    ];
    if let Some(acc) = accountno.map(str::trim).filter(|s| !s.is_empty()) {
        form.push(("accountno", acc.to_string()));
    }
    let v = client
        .post_form(EP_BLADE_PAY, &form, Envelope::Berserker)
        .await?;
    let data = &v["data"];
    Ok((
        parse_accounts(data),
        parse_ccctypes(data),
        parse_password_pad(data),
    ))
}

/// 提交支付（`paystep=2`）。
///
/// `password_seq` = 用户点击的**键位下标序列**（`[`PasswordPad::keys`]` 的下标拼成的字符串，
/// 官方 `passwordObj.password.join("")`），配 `uuid` 一起回传；免密时两者都是 `None`。
///
/// # 红线
///
/// - 只转发下标序列：**绝不用 `keys` 还原出真实字符**再提交（那等于明文泄露用户密码）；
/// - 非 6 位数字的下标序列直接拒绝（官方点满 6 位自动提交）；
/// - 命中跳转分支字段（`webUrl` 等）按红线 5 报错退出；
/// - **副作用请求不重试**（红线 3）；本函数被集成测试**只在无效参数下**验证错误分支。
pub async fn submit_pay(
    client: &SynjonesClient,
    order_id: &str,
    pay: &PayMethod,
    accountno: &str,
    ccctype: &str,
    password_seq: Option<&str>,
    uuid: Option<&str>,
) -> Result<(), CampusSynjonesError> {
    let order_id = order_id.trim();
    if order_id.is_empty() {
        return Err(CampusSynjonesError::Parse("缺少订单号".to_string()));
    }
    let accountno = accountno.trim();
    if accountno.is_empty() {
        return Err(CampusSynjonesError::Parse("请先选择账号".to_string()));
    }
    let ccctype = ccctype.trim();
    if ccctype.is_empty() {
        return Err(CampusSynjonesError::Parse("请先选择账户类型".to_string()));
    }
    let (password_seq, uuid) = match (password_seq, uuid) {
        (None, None) => (None, None),
        (Some(seq), Some(u)) => {
            let seq = seq.trim();
            if seq.len() != PASSWORD_LEN || !seq.chars().all(|c| c.is_ascii_digit()) {
                return Err(CampusSynjonesError::Parse(format!(
                    "密码需 {PASSWORD_LEN} 位（提交的是键盘下标序列）"
                )));
            }
            let u = u.trim();
            if u.is_empty() {
                return Err(CampusSynjonesError::Parse("缺少键盘标识".to_string()));
            }
            (Some(seq.to_string()), Some(u.to_string()))
        }
        // 半套凭据一定是调用方 bug：宁可报错，也不发一个「用不了键盘」的提交
        _ => {
            return Err(CampusSynjonesError::Parse(
                "密码与键盘标识必须同时提供".to_string(),
            ))
        }
    };

    let mut form: Vec<(&str, String)> = vec![
        ("orderid", order_id.to_string()),
        ("paystep", PAYSTEP_PAY.to_string()),
        ("paytype", pay.code.clone()),
        ("paytypeid", pay.payid.clone()),
        ("accountno", accountno.to_string()),
        ("ccctype", ccctype.to_string()),
        // 官方按 UA 置 1/0；桌面端恒 0（非微信环境）
        ("isWX", "0".to_string()),
    ];
    if let (Some(seq), Some(u)) = (password_seq, uuid) {
        form.push(("password", seq));
        form.push(("uuid", u));
    }
    let v = client
        .post_form(EP_BLADE_PAY, &form, Envelope::Berserker)
        .await?;
    reject_redirect(&v["data"])
}

/// 取消/清理未支付订单（`POST /charge/order/deleteOrder`）。
///
/// ⚠️ **必须 JSON body**（实测 6 形态矩阵：form / query / GET 一律 `code=500 未知异常，请联系管理员`，
/// 只有 `Content-Type: application/json` + `{"orderid":"…"}` 回 `code=200 success`）——学校侧该端点
/// 是 `@RequestBody` 风格，form 形态取不到 `orderid` 直接 NPE。
///
/// 副作用请求：仅在服务端**已明确拒绝**（401/403x）时静默重进并重发一次（那种情况不会产生副作用）。
pub async fn cancel_order(
    client: &SynjonesClient,
    order_id: &str,
) -> Result<(), CampusSynjonesError> {
    let order_id = order_id.trim();
    if order_id.is_empty() {
        return Err(CampusSynjonesError::Parse("缺少订单号".to_string()));
    }
    json_post(client, EP_DELETE_ORDER, &serde_json::json!({ "orderid": order_id }))
        .await
        .map(|_| ())
}

/// 单请求超时（与 `client.rs` 同值）。
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// JSON body 的 POST（[`SynjonesClient`] 只提供 GET / form POST，而取消订单端点要求 JSON）。
///
/// 自带最小头组（`synjones-auth` + `synAccessSource`）与「明确拒绝→静默重进→重发一次」纪律
/// （与 `client.rs::send` 同口径，只是这里不需要 form 与 `Envelope` 之外的设施）。
/// 不共享 `CasClient` 的 Cookie jar：实测该端点只认 token 头（裸 reqwest client 即可成功）。
async fn json_post(
    client: &SynjonesClient,
    path: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, CampusSynjonesError> {
    let mut last = CampusSynjonesError::NotLogin;
    for attempt in 0..2 {
        let token = match client.token().filter(|t| !t.is_empty()) {
            Some(t) => t,
            None => client.reenter().await?,
        };
        let resp = reqwest::Client::new()
            .post(format!("{}{path}", crate::BERSERKER_BASE))
            .header("synjones-auth", token.auth_value())
            .header("synAccessSource", crate::SYN_ACCESS_SOURCE)
            .json(body)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|e| CampusSynjonesError::Http(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        match parse_envelope(Envelope::Berserker, status, &text) {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = e;
                if attempt == 0 && matches!(last, CampusSynjonesError::NotLogin) {
                    client.reenter().await.map_err(|_| CampusSynjonesError::NotLogin)?;
                    continue;
                }
                break;
            }
        }
    }
    Err(last)
}

/// 本模块所有请求都走同一套头组（`synjones-auth` + 双份 `synAccessSource=app`），由
/// [`SynjonesClient`] 统一注入；App 口径不发 Basic 的实测依据见模块头注（匿名探针结论：
/// 带/不带 `Authorization: Basic …` 响应完全一致 ⇒ 该头非强制）。
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 订单解析：实测字段名 `orderid`/`status`/`payexpdate`/`tranamt`；status 容忍字符串形态。
    #[test]
    fn parse_order_reads_status_and_expiry() {
        let v = json!({"order": {
            "orderid": "202609190000001",
            "status": 0,
            "payexpdate": "2026-09-19 17:30:00",
            "tranamt": "1.00"
        }});
        let o = parse_order(&v);
        assert_eq!(o.order_id, "202609190000001");
        assert_eq!(o.status, 0, "0=待支付");
        assert_eq!(o.pay_exp_date.as_deref(), Some("2026-09-19 17:30:00"));
        assert_eq!(o.tranamt, Some(1.0), "金额字符串 → 元");

        let done = json!({"order": {"orderid": "9", "status": "1", "tranamt": 5}});
        let o = parse_order(&done);
        assert_eq!(o.status, 1, "1=已完成");
        assert_eq!(o.tranamt, Some(5.0));
        assert_eq!(o.pay_exp_date, None);

        // 缺 order / 缺 status → 不 panic，按「待支付」处理
        let empty = parse_order(&json!({}));
        assert_eq!(empty.status, 0);
        assert!(empty.order_id.is_empty());
    }

    /// `payList` 过滤：**只保留 ACCOUNT/ACCOUNTTSM**（CARD/PAYMENTPLATFORM/ALIPAY 等一律不展示）。
    #[test]
    fn parse_pay_methods_keeps_only_account_codes() {
        let v = json!({"payList": [
            {"code": "ACCOUNT", "payid": "p1", "name": "校园卡电子账户", "nopassword": 1},
            {"code": "ACCOUNTTSM", "payid": "p2", "name": "校园卡电子账户(TSM)", "nopassword": 0,
             "remark": "weixin"},
            {"code": "CARD", "payid": "p3", "name": "银行卡", "nopassword": 0},
            {"code": "CARDTSM", "payid": "p4", "name": "银行卡(TSM)"},
            {"code": "PAYMENTPLATFORM", "payid": "p5", "name": "聚合支付"},
            {"code": "ALIPAY", "payid": "p6", "name": "支付宝"}
        ]});
        let ms = parse_pay_methods(&v);
        assert_eq!(ms.len(), 2, "只应有 ACCOUNT/ACCOUNTTSM 两条");
        assert_eq!(ms[0].code, "ACCOUNT");
        assert_eq!(ms[0].payid, "p1");
        assert_eq!(ms[0].name, "校园卡电子账户");
        assert!(ms[0].nopassword);
        assert_eq!(ms[0].remark, None, "空备注 → None");
        assert_eq!(ms[1].code, "ACCOUNTTSM");
        assert!(!ms[1].nopassword);
        assert_eq!(ms[1].remark.as_deref(), Some("weixin"));
    }

    /// 缺 `code`/`payid` 的项丢弃（不可提交）；缺 `payList` 不 panic。
    #[test]
    fn parse_pay_methods_drops_incomplete_items() {
        let v = json!({"payList": [
            {"code": "ACCOUNT", "name": "没有 payid"},
            {"payid": "p2", "name": "没有 code"},
            {"code": "ACCOUNT", "payid": "p3", "name": "正常的"}
        ]});
        let ms = parse_pay_methods(&v);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].payid, "p3");

        assert!(parse_pay_methods(&json!({"code": 200})).is_empty());
        assert!(parse_pay_methods(&json!({"payList": []})).is_empty());
    }

    /// `nopassword` 三态：**1 免密 / 0 需密码 / 其它真值按需密码**（官方对其它真值是死分支，
    /// 本项目按需密码处理，见 [`is_no_password`] 注释）。
    #[test]
    fn nopassword_three_states() {
        assert!(is_no_password(Some(&json!(1))), "1 → 免密");
        assert!(!is_no_password(Some(&json!(0))), "0 → 需密码");
        assert!(!is_no_password(Some(&json!(2))), "其它真值（官方死分支）→ 按需密码");
        assert!(!is_no_password(Some(&json!(false))), "false → 需密码");
        assert!(!is_no_password(None), "缺失 → 需密码");
        assert!(is_no_password(Some(&json!("1"))), "字符串 \"1\" 宽容认作免密（类型漂移先例）");
        assert!(!is_no_password(Some(&json!("0"))));
    }

    /// `accountno` 两种形态：对象数组 `{accountno}` 与裸字符串数组（还有带 accountno 时的单字符串回显）。
    #[test]
    fn parse_accounts_two_shapes() {
        let objects = json!({"accountno": [{"accountno": "A1", "name": "x"}, {"accountno": "A2"}]});
        assert_eq!(parse_accounts(&objects), vec!["A1", "A2"], "对象数组");

        let bare = json!({"accountno": ["A1", "A2"]});
        assert_eq!(parse_accounts(&bare), vec!["A1", "A2"], "裸字符串数组");

        let mixed = json!({"accountno": [{"accountno": "A1"}, "A2"]});
        assert_eq!(parse_accounts(&mixed), vec!["A1", "A2"], "混合形态");

        let single = json!({"accountno": "A1"});
        assert_eq!(parse_accounts(&single), vec!["A1"], "单字符串（带 accountno 请求时的回显）");

        // 空/异常形态：不 panic、不透出垃圾
        assert!(parse_accounts(&json!({})).is_empty());
        assert!(parse_accounts(&json!({"accountno": []})).is_empty());
        assert!(parse_accounts(&json!({"accountno": [{"x": 1}, ""]})).is_empty());
    }

    /// `ccctype`：对象数组取 `ccctype` 码（`balance` 不在此层消费）；裸字符串也容忍。
    #[test]
    fn parse_ccctypes_takes_codes() {
        let v = json!({"ccctype": [{"ccctype": "1", "balance": "12.34"}, {"ccctype": "2"}]});
        assert_eq!(parse_ccctypes(&v), vec!["1", "2"]);
        assert_eq!(parse_ccctypes(&json!({"ccctype": ["1"]})), vec!["1"]);
        assert!(parse_ccctypes(&json!({})).is_empty());
        assert!(parse_ccctypes(&json!({"ccctype": null})).is_empty());
    }

    /// `passwordMap` → 键盘：单键取值，`uuid` 与服务端下发的 10 个键都要在；
    /// **断言只碰结构与长度，绝不校验/打印 keys 内容**。
    ///
    /// ⚠️ 实测 live 的形态是 **10 字符字符串**（不是数组），故两种形态都要覆盖。
    #[test]
    fn parse_password_pad_single_key() {
        // 实测形态：值是 10 个字符的字符串
        let live_shape = json!({"passwordMap": {"uuid-abc": "1a2b3c4d5e"}});
        let pad = parse_password_pad(&live_shape).expect("字符串形态（实测）应解析出键盘");
        assert_eq!(pad.uuid, "uuid-abc");
        assert_eq!(pad.keys.len(), 10, "实测恒 10 键");
        assert_eq!(pad.keys[0], "1", "按字符拆开，下标语义不变");
        assert_eq!(pad.keys[9], "e");

        // 数组形态（服务端类型漂移先例；两种形态下标语义一致）
        let v = json!({"passwordMap": {"uuid-abc": ["a","b","c","d","e","f","g","h","i","j"]}});
        let pad = parse_password_pad(&v).expect("数组形态也应解析");
        assert_eq!(pad.keys.len(), 10);
        // 多键时取首个（实测恒单键；官方 for-in 取末个，无实害）
        let two = json!({"passwordMap": {"u1": "12", "u2": "34"}});
        assert_eq!(parse_password_pad(&two).unwrap().uuid, "u1");
        // 缺失 / 空键位 / 键位含空串 → None（宁可不显示键盘，也不显示一个残缺键盘）
        assert!(parse_password_pad(&json!({})).is_none());
        assert!(parse_password_pad(&json!({"passwordMap": {}})).is_none());
        assert!(parse_password_pad(&json!({"passwordMap": {"u": ""}})).is_none());
        assert!(parse_password_pad(&json!({"passwordMap": {"u": []}})).is_none());
        assert!(parse_password_pad(&json!({"passwordMap": {"": "12"}})).is_none());
        assert!(parse_password_pad(&json!({"passwordMap": {"u": ["1", ""]}})).is_none());
        assert!(parse_password_pad(&json!({"passwordMap": {"u": 123}})).is_none());
    }

    /// 红线：`PasswordPad` 的 `Debug` 必须打码（防它被 `{:?}` 带进日志/panic 消息）。
    #[test]
    fn password_pad_debug_masks_keys() {
        let pad = PasswordPad {
            uuid: "secret-uuid-1234".to_string(),
            keys: vec!["9".to_string(), "8".to_string()],
        };
        let dbg = format!("{pad:?}");
        assert!(!dbg.contains("secret-uuid-1234"), "uuid 不得进 Debug：{dbg}");
        assert!(!dbg.contains('9'), "键位字符不得进 Debug：{dbg}");
        assert!(dbg.contains("2 键"), "只报个数：{dbg}");
    }

    /// 红线 5：跳转分支字段一律报错退出（不实现、不返回成功）。
    #[test]
    fn reject_redirect_catches_all_channels() {
        for k in REDIRECT_KEYS {
            let mut data = json!({"orderid": "1"});
            data[k] = json!("https://example.com/pay");
            let e = reject_redirect(&data).expect_err("跳转分支应被拒绝");
            assert!(e.to_string().contains(k), "文案应点名通道：{e}");
        }
        // 空串/缺失 = 没走跳转分支 → 放行
        assert!(reject_redirect(&json!({"orderid": "1", "webUrl": ""})).is_ok());
        assert!(reject_redirect(&json!({"orderid": "1"})).is_ok());
        assert!(reject_redirect(&json!(null)).is_ok());
    }

    /// 入参校验（无网络）：空片区 id / 非法金额 / 空路径 / 空房间号 / 空订单号 / 空账号 / 半套密码
    /// → 报 Parse 且不发请求。
    #[tokio::test]
    async fn invalid_input_is_rejected_before_any_request() {
        // 未登录且无 TGT：若校验没拦住，就会在发请求前先撞上 NotLogin（`ensure_token`），据此反证「没发请求」
        let c = SynjonesClient::new(
            campus_auth::cas::CasClient::new().expect("创建 CasClient 失败"),
            None,
            None,
        );
        let pay = PayMethod {
            code: "ACCOUNT".to_string(),
            payid: "p1".to_string(),
            name: "校园卡".to_string(),
            nopassword: false,
            remark: None,
        };
        let room_path = |value: &str| {
            vec![
                RoomStep {
                    level: 1,
                    code: "campus".to_string(),
                    value: "1&无锡学院".to_string(),
                    name: "无锡学院".to_string(),
                },
                RoomStep {
                    level: 2,
                    code: "building".to_string(),
                    value: "2309&1号楼".to_string(),
                    name: "1号楼".to_string(),
                },
                RoomStep {
                    level: 3,
                    code: "room".to_string(),
                    value: value.to_string(),
                    name: value.to_string(),
                },
            ]
        };

        let e = create_order(&c, " ", "1", Some(&room_path("101")))
            .await
            .expect_err("空片区 id 应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");

        for bad in ["", "0", "-1", "abc"] {
            let e = create_order(&c, "450", bad, Some(&room_path("101")))
                .await
                .expect_err("非法金额应报错");
            assert!(matches!(e, CampusSynjonesError::Parse(_)), "{bad} 实际 {e:?}");
        }

        let e = create_order(&c, "450", "1", Some(&[][..]))
            .await
            .expect_err("空路径应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");
        assert!(e.to_string().contains("请先选择房间"), "实际 {e}");

        let e = create_order(&c, "450", "1", Some(&room_path("  ")))
            .await
            .expect_err("空房间号应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");
        assert!(e.to_string().contains("第 3 级未填值"), "实际 {e}");

        // 无级联片区（path=None，如一卡通充值 401）：入参校验全过、**不合成 third_party**，
        // 直接走到发请求——无 TGT 客户端在此必然撞 `NotLogin`（不发网络），据此反证
        // 「没有先去 third_party_for_room 报『请先选择房间』」。
        let e = create_order(&c, "401", "1", None).await.expect_err("无会话应报错");
        assert!(matches!(e, CampusSynjonesError::NotLogin), "实际 {e:?}");

        let e = third_party_for_room(&c, "450", &[])
            .await
            .expect_err("空路径应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");

        let e = query_account(&c, " ", &pay, None).await.expect_err("空订单号应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");

        let e = cancel_order(&c, "  ").await.expect_err("空订单号应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");

        let e = submit_pay(&c, "1", &pay, "", "1", None, None)
            .await
            .expect_err("空账号应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");

        let e = submit_pay(&c, "1", &pay, "A1", "", None, None)
            .await
            .expect_err("空账户类型应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");

        // 下标序列必须是 6 位数字；且密码/键盘标识必须成对
        for bad in ["", "123", "1234567", "abcdef", "12345a"] {
            let e = submit_pay(&c, "1", &pay, "A1", "1", Some(bad), Some("u1"))
                .await
                .expect_err("非法下标序列应报错");
            assert!(matches!(e, CampusSynjonesError::Parse(_)), "{bad} 实际 {e:?}");
        }
        let e = submit_pay(&c, "1", &pay, "A1", "1", Some("012345"), None)
            .await
            .expect_err("缺 uuid 应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");
        let e = submit_pay(&c, "1", &pay, "A1", "1", None, Some("u1"))
            .await
            .expect_err("缺密码应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");
    }

    /// `third_party` 合成（纯函数）：保留 `map.data` 全部明细 + 并入 `myCustomInfo`；
    /// 明细缺失/非对象 → 报错（宁可不建单）。
    #[test]
    fn compose_third_party_keeps_details_and_adds_label() {
        let path = vec![
            RoomStep {
                level: 1,
                code: "campus".to_string(),
                value: "1&无锡学院".to_string(),
                name: "无锡学院".to_string(),
            },
            RoomStep {
                level: 2,
                code: "building".to_string(),
                value: "2309&1号楼".to_string(),
                name: "1号楼".to_string(),
            },
            RoomStep {
                level: 3,
                code: "room".to_string(),
                value: "101".to_string(),
                name: "101".to_string(),
            },
        ];
        // 实测 map.data 的键（值用假样本，真实户号绝不入测试）
        let data = json!({
            "account": "0000000000", "aid": "1", "area": "1", "areaName": "A",
            "building": "2309", "buildingName": "1号楼", "floor": "", "floorName": "",
            "room": "101", "roomName": "梅园1号101"
        });
        let s = compose_third_party(&data, &path).expect("应合成成功");
        let v: Value = serde_json::from_str(&s).expect("应是 JSON 串");
        assert_eq!(v["room"], "101", "明细原样保留");
        assert_eq!(v["account"], "0000000000");
        assert_eq!(
            v["myCustomInfo"], "101：无锡学院 1号楼 101",
            "官方口径：<末级名>：<各级名空格拼接>"
        );
        // 各级名全空 → 不加 myCustomInfo（官方同样跳过）
        let nameless: Vec<RoomStep> = path
            .iter()
            .map(|s| RoomStep {
                name: String::new(),
                ..s.clone()
            })
            .collect();
        let no_label: Value =
            serde_json::from_str(&compose_third_party(&data, &nameless).unwrap()).unwrap();
        assert!(no_label.get("myCustomInfo").is_none(), "无标签时不应塞空 myCustomInfo");
        assert_eq!(no_label["room"], "101");
        // 明细缺失/非对象 → 报错，不建单
        assert!(compose_third_party(&Value::Null, &path).is_err());
        assert!(compose_third_party(&json!("x"), &path).is_err());
        assert!(compose_third_party(&json!([]), &path).is_err());
    }

    /// 端点常量与白名单防漂移（改动必须是有意识的协议变更）。
    #[test]
    fn constants_are_stable() {
        assert_eq!(EP_BLADE_PAY, "/blade-pay/pay", "绝对路径，勿改成 /charge 下");
        assert_eq!(EP_PAY_INFO, "/charge/pay/getpayinfo");
        assert_eq!(EP_DELETE_ORDER, "/charge/order/deleteOrder");
        assert_eq!(ACCOUNT_CODES, ["ACCOUNT", "ACCOUNTTSM"]);
        assert_eq!(FLAG_CHOOSE, "choose", "电费恒 choose");
        assert_eq!(PASSWORD_LEN, 6);
    }
}
