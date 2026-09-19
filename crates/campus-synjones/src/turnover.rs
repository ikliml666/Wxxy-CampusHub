//! 缴费历史读取（M4）：账单流水 / 月度合计 / 累计缴费 / 片区配置 / 订单列表。
//!
//! # 只读纪律（改本文件前先读）
//!
//! 本模块**只发 GET**，端点全部来自官方前端 bundle 反查 + 2026-09-19 内网 live 探针实测
//!（`tests/m4_history_probe_live.rs`）。**禁止**在此新增任何建单 / 支付 / 删除 / 退款的调用
//!（`/blade-pay/pay`、`/charge/order/deleteOrder`、`/charge/order/thirdOrder`、
//! `/charge/order/addRefundOrder`、`/charge/sceneBind/add`、`/charge/receivable/updateReceivable`
//! 一律不碰——充值写路径在 [`crate::recharge`]，本模块不出现在那条链路上）。
//!
//! # 端点与实测事实（2026-09-19 live，授权账号只有极少量历史，样本里的金额是真实原值）
//!
//! | 用途 | 方法 | 路径 | 参数 | 关键字段 |
//! |---|---|---|---|---|
//! | 账单列表（历史） | GET | [`EP_APP_ACCOUNT`] | `current` / `size` / 可选 `feeitemid` | 顶层 `count` + `accountList[]`（`SUCCESSDATE`/`TRANAMT`/`TURNOVERID`/`ITEMNAME`/`ABSTRACTS`/`TYPENAME`） |
//! | 某月合计 | GET | [`EP_PIE_ACCOUNT`] | `createdate=YYYY-MM` | `pieAccountList[]`（`tranamt`/`createdate`；**无数据的月份回空数组**） |
//! | 累计缴费额 | GET | [`EP_TOTAL_ACCOUNT`] | 可选 `feeitemid` | `accountTotal`（单个数，**可能为 null**） |
//! | 订单列表（含待支付） | GET | [`EP_ORDER_PERSONAL_DATA`] | 可选 `status`（`0` 待支付 / `1` 已完成 / `2`） | `orderList[]`（`orderid`/`tranamt`/`actulamt`/`status`/`commitdate`/`successdate`/`source`/`abstracts`/`feeitemlist[].feeitemid`/`orderDetailList[]`） |
//! | 片区配置 | GET | [`EP_SHOW_FEEITEM`] | `feeitemid` | `list[0]` 的 `price`/`maxmoney`/`daymaxmoney`/`retain_money`/`billing_unit`/`layout` |
//!
//! ## ⚠️ 单位红线（元 vs 分）
//!
//! `/charge/*` 侧的金额字段 `TRANAMT` / `tranamt` / `accountTotal` / `pieAccountList[].tranamt`
//! 实测单位是**元**：同一时刻一卡通流水扣 `tranamt=100`（分）而电费账单是 `TRANAMT=1`。
//! **不要**照抄官方 PC 页对 `TRANAMT` 的 `/100`——那会得到 0.01 元。
//! 一卡通侧（[`crate::ecard`]）相反：一切金额字段是**分**，由该模块的 `yuan()` 换算。
//!
//! ## ⚠️ 旧结论的修订：`/charge/order/personal_data?status=0` 并不是「恒 500」
//!
//! 项目早前记录「待支付订单列表无解、该端点任何形态恒 500」（见
//! `commands/electricity.rs` 模块头注红线 6）。M4 探针实测：**参数与路径都对，缺的是 App 口径的
//! 请求头组**——把 `synAccessSource=app` 同时放进 query **与**同名头（[`crate::client`] 的
//! `with_headers` 正是这样做的）即回 `code=200` + `orderList`。用旧形态（只有来源头、无 query 一份）
//! 复跑同样 500。故本模块可直接复用 [`SynjonesClient::get`]，无需任何特判。
//!
//! ## 解析口径
//!
//! - 服务端同一字段的类型会漂移（`maxmoney` 是字符串 `"200"`、`price` 是数字 `0`、`accountTotal`
//!   可能是 `null`），故金额一律走 [`num_of`]（数字/数字字符串都吃得下）。
//! - `size` / `page` 由调用方给，本模块只做下限保护（`page >= 1`、`size ∈ [1,100]`）。
//! - 订单列表**不分页**（服务端不收 `current/size`，实测带这两个参数同样回全量 `orderList`），
//!   故 [`fetch_orders`] 不提供分页参数。

use crate::client::{Envelope, SynjonesClient};
use crate::CampusSynjonesError;
use serde::Serialize;
use serde_json::Value;

/// 缴费账单列表（历史，**含已支付流水**；`current`/`size` 分页 + 可选片区过滤）。
pub const EP_APP_ACCOUNT: &str = "/charge/turnover/app_account";
/// 某月缴费合计（`createdate=YYYY-MM`，参数**生效**：可逐月循环拼年度曲线）。
pub const EP_PIE_ACCOUNT: &str = "/charge/turnover/pie_account";
/// 累计缴费额（可选 `feeitemid` 过滤）。
pub const EP_TOTAL_ACCOUNT: &str = "/charge/turnover/app_totalAccount";
/// 订单列表（**含待支付**，`status` 可过滤；见模块头注的旧结论修订）。
pub const EP_ORDER_PERSONAL_DATA: &str = "/charge/order/personal_data";
/// 片区配置（单价 / 单笔上限 / 单日上限）。
pub const EP_SHOW_FEEITEM: &str = "/charge/feeitem/showFeeitem";

/// 账单列表每页条数上限（服务端实测可吃 `size=200`，此处收紧到 100：内网单页够用且慢网更稳）。
pub const MAX_PAGE_SIZE: u32 = 100;

/// 一条缴费账单（`accountList[]` 条目）。
///
/// 金额单位**元**（见模块头注单位红线）。`id`（`TURNOVERID`）是该笔流水号，与
/// [`crate::recharge::RechargeOrder::order_id`] 同级敏感度（用户自己的单据号，官方 App 同样展示），
/// 用于列表稳定 key；**不落盘、不进日志**。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bill {
    /// 流水号（`TURNOVERID`，实测为字符串）。
    pub id: String,
    /// 成功时间（`SUCCESSDATE` 原文，形如 `2026-09-19 17:08:16`）。
    pub time: String,
    /// 缴费项名（`ITEMNAME`，如 `桃园1号-李园8号`）。
    pub item_name: String,
    /// 摘要（`ABSTRACTS`，含房间路径等文本，学校侧原文）。
    pub abstracts: String,
    /// 渠道名（`TYPENAME`，实测 `移动服务平台`）。
    pub type_name: String,
    /// 金额（元）；缺失/非数值 → None（**不臆造 0**）。
    pub amount_yuan: Option<f64>,
}

/// 账单分页结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BillPage {
    /// 服务端顶层 `count`（符合条件的**全量**条数，不是本页条数）。
    pub total: u32,
    pub records: Vec<Bill>,
}

/// 某月的缴费合计。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthTotal {
    /// `YYYY-MM`。
    pub month: String,
    /// 该月合计（元）。服务端回空数组 = 该月**确实没有缴费** ⇒ `0.0`（见 [`fetch_monthly`]）。
    pub amount_yuan: f64,
}

/// 片区配置（`showFeeitem`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeitemConfig {
    pub feeitem_id: String,
    pub name: String,
    /// 计费单位（实测 `"元"`）。
    pub billing_unit: String,
    /// 快捷金额档（`layout` `"10,50,100"` → `[10,50,100]`）。
    pub layout: Vec<u32>,
    /// 单价（元/单位）。⚠️ 实测 `448` 的 `price` 是数字 `0`，而末级自由文本里单价是 `0.5400`——
    /// 两个口径不一致，**展示层不要把它当权威单价**（见 `charge::balance_from_text` 的同类告诫）。
    pub price_yuan: Option<f64>,
    /// 单笔上限（元；服务端字符串 `"200"`）。
    pub max_money_yuan: Option<f64>,
    /// 单日上限（元；服务端字符串 `"500"`）。建单前的日消费上限校验用的就是它（`recharge` 头注坑 3）。
    pub day_max_money_yuan: Option<f64>,
    /// 起充下限（元；服务端字符串 `"1"`）。
    pub retain_money_yuan: Option<f64>,
}

/// 一条订单（`orderList[]` 条目；`status == 0` 即待支付）。
///
/// 敏感字段（`userno` / `sno` / `third_party` / `paycard` / `verifyFname` / `payBean` 等）**一律不解析**，
/// 见模块头注与项目「户号/PII 不透出」纪律。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    /// 订单号（`orderid`）。待支付单要把它回传给 `recharge_pay_methods` / `recharge_status` / `recharge_cancel`。
    pub order_id: String,
    /// 状态（`0` 待支付 / `1` 已完成 / `2`）；字段缺失按 `0` 处理（与 `recharge::RechargeOrder` 同口径）。
    pub status: i64,
    /// 下单金额（元，`tranamt`）。
    pub amount_yuan: Option<f64>,
    /// 实付金额（元，`actulamt`）——**待支付单实测为 null**，故是 Option。
    pub actual_yuan: Option<f64>,
    /// 下单时间（`commitdate` 原文）。
    pub commit_date: String,
    /// 成功时间（`successdate` 原文；待支付单为 null → 空串）。
    pub success_date: String,
    /// 来源（`source`，实测 `app`）。
    pub source: String,
    /// 摘要（`abstracts`，如 `无锡学院 1号楼 101`）。
    pub abstracts: String,
    /// 片区 id（取自 `feeitemlist[0].feeitemid`，顶层 `feeitemid` 实测恒为 0；都缺则空串）。
    pub feeitem_id: String,
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

/// 金额字段（**元口径**）：数字或数字字符串均可，缺失/空串/非数值 → None。
///
/// 此处**不做任何隐式缩放**：`/charge` 侧实测就是元，除以 100 是错（见模块头注单位红线）。
fn num_of(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                t.parse::<f64>().ok()
            }
        }
        _ => None,
    }
}

/// 解析账单列表信封（`{code,count,accountList[]}`）。
pub fn parse_bills(v: &Value) -> BillPage {
    let records: Vec<Bill> = v["accountList"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|r| Bill {
                    id: text_of(r.get("TURNOVERID")),
                    time: text_of(r.get("SUCCESSDATE")),
                    item_name: text_of(r.get("ITEMNAME")),
                    abstracts: text_of(r.get("ABSTRACTS")),
                    type_name: text_of(r.get("TYPENAME")),
                    amount_yuan: num_of(r.get("TRANAMT")),
                })
                .collect()
        })
        .unwrap_or_default();
    // `count` 容忍字符串形态（服务端类型漂移先例），缺失按本页条数兜底（不报错）
    let total = v["count"]
        .as_u64()
        .or_else(|| text_of(v.get("count")).trim().parse::<u64>().ok())
        .map(|n| n as u32)
        .unwrap_or(records.len() as u32);
    BillPage { total, records }
}

/// 解析某月合计：`pieAccountList[]` 的 `tranamt` 求和（空数组 → `0.0`）。
pub fn parse_month_total(v: &Value) -> f64 {
    v["pieAccountList"]
        .as_array()
        .map(|a| a.iter().filter_map(|r| num_of(r.get("tranamt"))).sum())
        .unwrap_or(0.0)
}

/// 解析累计缴费额（`accountTotal`：单个数/数字字符串/`null`）。
pub fn parse_total(v: &Value) -> Option<f64> {
    num_of(v.get("accountTotal"))
}

/// 解析片区配置：取 `list[0]`；缺 `list` / 空数组 → None。
pub fn parse_feeitem_config(v: &Value) -> Option<FeeitemConfig> {
    let it = v["list"].as_array()?.first()?;
    Some(FeeitemConfig {
        feeitem_id: text_of(it.get("feeitemid")),
        name: text_of(it.get("name")),
        billing_unit: text_of(it.get("billing_unit")),
        layout: text_of(it.get("layout"))
            .split(',')
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .collect(),
        price_yuan: num_of(it.get("price")),
        max_money_yuan: num_of(it.get("maxmoney")),
        day_max_money_yuan: num_of(it.get("daymaxmoney")),
        retain_money_yuan: num_of(it.get("retain_money")),
    })
}

/// 解析订单列表信封（`{code,orderList[]}`）。
pub fn parse_orders(v: &Value) -> Vec<Order> {
    v["orderList"]
        .as_array()
        .map(|a| a.iter().map(parse_order).collect())
        .unwrap_or_default()
}

/// 解析单条订单。
fn parse_order(o: &Value) -> Order {
    // 片区 id：顶层 `feeitemid` 实测恒 0（占位），真值在 `feeitemlist[0].feeitemid`；
    // 两条都缺时回落 `orderDetailList[0].feeitemid`（实测同值）。
    let feeitem_id = o["feeitemlist"]
        .as_array()
        .and_then(|a| a.first())
        .map(|f| text_of(f.get("feeitemid")))
        .filter(|s| !s.is_empty() && s != "0")
        .or_else(|| {
            o["orderDetailList"]
                .as_array()
                .and_then(|a| a.first())
                .map(|d| text_of(d.get("feeitemid")))
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_default();
    Order {
        order_id: text_of(o.get("orderid")),
        status: o["status"].as_i64().unwrap_or(0),
        amount_yuan: num_of(o.get("tranamt")),
        actual_yuan: num_of(o.get("actulamt")),
        commit_date: text_of(o.get("commitdate")),
        success_date: text_of(o.get("successdate")),
        source: text_of(o.get("source")),
        abstracts: text_of(o.get("abstracts")),
        feeitem_id,
    }
}

// ---------------- 取数 ----------------

/// 分页参数收敛：`page >= 1`、`size ∈ [1,`[`MAX_PAGE_SIZE`]`]`。
fn clamp_page(page: u32, size: u32) -> (String, String) {
    (
        page.max(1).to_string(),
        size.clamp(1, MAX_PAGE_SIZE).to_string(),
    )
}

/// 缴费账单列表（`current`/`size` 分页；`feeitem_id` 非空时按片区过滤——实测生效）。
pub async fn fetch_bills(
    client: &SynjonesClient,
    feeitem_id: Option<&str>,
    page: u32,
    size: u32,
) -> Result<BillPage, CampusSynjonesError> {
    let (current, size) = clamp_page(page, size);
    let mut params: Vec<(&str, &str)> = vec![("current", &current), ("size", &size)];
    let feeitem = feeitem_id.unwrap_or("").trim();
    if !feeitem.is_empty() {
        params.push(("feeitemid", feeitem));
    }
    let v = client.get(EP_APP_ACCOUNT, &params, Envelope::Charge).await?;
    Ok(parse_bills(&v))
}

/// 某年 12 个月的缴费合计（逐月循环 [`EP_PIE_ACCOUNT`]，`createdate=YYYY-MM`）。
///
/// - **空月 = `0.0`**：服务端明确回空数组，语义就是「该月没有缴费」⇒ 曲线保留完整 12 个月的 x 轴；
/// - **请求失败 = 整年失败**（不把网络/业务错误当成 0 元——那会把「取不到」误报成「没交钱」）。
pub async fn fetch_monthly(
    client: &SynjonesClient,
    year: i32,
) -> Result<Vec<MonthTotal>, CampusSynjonesError> {
    if !(2000..=2100).contains(&year) {
        return Err(CampusSynjonesError::Parse(format!("年份越界：{year}")));
    }
    let mut out = Vec::with_capacity(12);
    for month in 1..=12u32 {
        let key = format!("{year}-{month:02}");
        let v = client
            .get(EP_PIE_ACCOUNT, &[("createdate", &key)], Envelope::Charge)
            .await?;
        out.push(MonthTotal {
            month: key,
            amount_yuan: parse_month_total(&v),
        });
    }
    Ok(out)
}

/// 累计缴费额（元）；`feeitem_id` 非空时按片区过滤。服务端可能回 `null` → None。
pub async fn fetch_total(
    client: &SynjonesClient,
    feeitem_id: Option<&str>,
) -> Result<Option<f64>, CampusSynjonesError> {
    let mut params: Vec<(&str, &str)> = Vec::new();
    let feeitem = feeitem_id.unwrap_or("").trim();
    if !feeitem.is_empty() {
        params.push(("feeitemid", feeitem));
    }
    let v = client.get(EP_TOTAL_ACCOUNT, &params, Envelope::Charge).await?;
    Ok(parse_total(&v))
}

/// 片区配置（单价 / 单笔上限 / 单日上限 / 起充下限）。`feeitem_id` 为空 → `Parse` 错误。
pub async fn fetch_feeitem_config(
    client: &SynjonesClient,
    feeitem_id: &str,
) -> Result<FeeitemConfig, CampusSynjonesError> {
    let feeitem = feeitem_id.trim();
    if feeitem.is_empty() {
        return Err(CampusSynjonesError::Parse("缺少片区 id".to_string()));
    }
    let v = client
        .get(EP_SHOW_FEEITEM, &[("feeitemid", feeitem)], Envelope::Charge)
        .await?;
    parse_feeitem_config(&v)
        .ok_or_else(|| CampusSynjonesError::Parse("片区配置响应缺少 list".to_string()))
}

/// 订单列表（含待支付）。`status`：`Some(0)` 待支付 / `Some(1)` 已完成 / `None` 全部。
///
/// ⚠️ 该端点**必须**带 App 口径请求头组（query 与同名头各一份 `synAccessSource=app`），
/// [`SynjonesClient::get`] 已统一注入——不要绕过它直接发请求（否则实测 HTTP 500）。
pub async fn fetch_orders(
    client: &SynjonesClient,
    status: Option<i64>,
) -> Result<Vec<Order>, CampusSynjonesError> {
    let status_text = status.map(|s| s.to_string());
    let mut params: Vec<(&str, &str)> = Vec::new();
    if let Some(s) = status_text.as_deref() {
        params.push(("status", s));
    }
    let v = client
        .get(EP_ORDER_PERSONAL_DATA, &params, Envelope::Charge)
        .await?;
    Ok(parse_orders(&v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// **实测样本**（2026-09-19 live，`GET /charge/turnover/app_account?current=1&size=10&feeitemid=448`）：
    /// 顶层 `count` + 单条 `accountList`，字段名全大写，金额是**元**。
    fn bills_sample() -> Value {
        json!({
            "accountList": [
                {
                    "ABSTRACTS": "无锡国际校区 李园捌号A12 101",
                    "ITEMNAME": "桃园1号-李园8号",
                    "SUCCESSDATE": "2026-09-19 17:08:16",
                    "TRANAMT": 1,
                    "TURNOVERID": "T0001",
                    "TYPENAME": "移动服务平台"
                },
                {
                    "ABSTRACTS": "{\"type\":\"recharge\"}",
                    "ITEMBILLING": 0,
                    "ITEMNAME": "慧新易校一卡通充值",
                    "SUCCESSDATE": "2026-09-15 16:18:54",
                    "TRANAMT": 1,
                    "TURNOVERID": "T0002",
                    "TYPENAME": "移动服务平台"
                }
            ],
            "code": 200,
            "count": 2,
            "msg": "success"
        })
    }

    /// 账单解析：`count` 是全量条数而非本页条数；`TRANAMT` 是**元**（不除 100）。
    #[test]
    fn parse_bills_reads_count_and_yuan_amount() {
        let page = parse_bills(&bills_sample());
        assert_eq!(page.total, 2, "count 是全量条数");
        assert_eq!(page.records.len(), 2);
        let first = &page.records[0];
        assert_eq!(first.id, "T0001");
        assert_eq!(first.time, "2026-09-19 17:08:16");
        assert_eq!(first.item_name, "桃园1号-李园8号");
        assert_eq!(first.type_name, "移动服务平台");
        assert_eq!(first.amount_yuan, Some(1.0), "TRANAMT 单位是元（不是分）");
    }

    /// 账单容错：缺 `accountList` / 空数组 / `count` 为字符串 / 金额非数值 → 不 panic、不臆造金额。
    #[test]
    fn parse_bills_tolerates_odd_shapes() {
        let empty = parse_bills(&json!({"code": 200, "count": 0, "accountList": []}));
        assert_eq!(empty.total, 0);
        assert!(empty.records.is_empty());

        let no_list = parse_bills(&json!({"code": 200}));
        assert_eq!(no_list.total, 0, "缺 count 且缺列表 → 0（不 panic）");

        let odd = parse_bills(&json!({
            "count": 7,
            "accountList": [{"TRANAMT": "abc", "TURNOVERID": 12345, "SUCCESSDATE": null}]
        }));
        assert_eq!(odd.total, 7, "count 缺失形态容忍字符串");
        assert_eq!(odd.records[0].amount_yuan, None, "非数值金额 → None，不臆造 0");
        assert_eq!(odd.records[0].id, "12345", "数字型 id 也吃得下");
        assert_eq!(odd.records[0].time, "");
    }

    /// 月度合计：`pieAccountList` 求和；**空数组 = 0.0**；字符串金额也吃得下。
    #[test]
    fn parse_month_total_sums_or_zero() {
        let one = json!({"code": 200, "pieAccountList": [
            {"createdate": "2026-09", "tranamt": 1, "balance_amount": null}
        ]});
        assert_eq!(parse_month_total(&one), 1.0);

        let many = json!({"pieAccountList": [{"tranamt": 12.5}, {"tranamt": "7.5"}]});
        assert_eq!(parse_month_total(&many), 20.0, "多条求和 + 字符串金额");

        assert_eq!(parse_month_total(&json!({"pieAccountList": []})), 0.0, "空月 = 0");
        assert_eq!(parse_month_total(&json!({"code": 200})), 0.0, "缺列表 = 0");
    }

    /// 累计额：数字 / 字符串 / `null` 三态。
    #[test]
    fn parse_total_three_shapes() {
        assert_eq!(parse_total(&json!({"accountTotal": 2})), Some(2.0));
        assert_eq!(parse_total(&json!({"accountTotal": "2.5"})), Some(2.5));
        assert_eq!(parse_total(&json!({"accountTotal": null})), None, "实测 449 片区回 null");
        assert_eq!(parse_total(&json!({"code": 200})), None);
    }

    /// 片区配置：实测 `maxmoney`/`daymaxmoney`/`retain_money` 是**字符串**、`price` 是数字，
    /// `billing_unit` 是 `"元"`。
    #[test]
    fn parse_feeitem_config_mixed_number_types() {
        let v = json!({"code": 200, "list": [{
            "feeitemid": 448,
            "name": "桃园1号-李园8号",
            "billing_unit": "元",
            "layout": "10,50,100",
            "price": 0,
            "maxmoney": "200",
            "daymaxmoney": "500",
            "retain_money": "1"
        }]});
        let c = parse_feeitem_config(&v).expect("应解析出配置");
        assert_eq!(c.feeitem_id, "448");
        assert_eq!(c.billing_unit, "元");
        assert_eq!(c.layout, vec![10, 50, 100]);
        assert_eq!(c.price_yuan, Some(0.0), "price 实测是数字 0");
        assert_eq!(c.max_money_yuan, Some(200.0));
        assert_eq!(c.day_max_money_yuan, Some(500.0), "日上限 500（recharge 头注坑 3 的同一字段）");
        assert_eq!(c.retain_money_yuan, Some(1.0));

        assert!(parse_feeitem_config(&json!({"code": 200})).is_none());
        assert!(parse_feeitem_config(&json!({"list": []})).is_none());
    }

    /// **实测样本**（2026-09-19 live，`GET /charge/order/personal_data?status=0`）：待支付单
    /// `actulamt=null`/`successdate=null`，片区 id 在 `feeitemlist[0]`（顶层 `feeitemid` 是 0）。
    fn orders_sample() -> Value {
        json!({
            "code": 200,
            "msg": "success",
            "orderList": [
                {
                    "abstracts": "无锡学院 1号楼 101",
                    "actulamt": null,
                    "commitdate": "2026-09-19 16:40:11",
                    "feeitemid": 0,
                    "feeitemlist": [{"feeitemid": 450, "chargeunit": "56321", "userno": "U0001"}],
                    "orderDetailList": [{
                        "id": "D1", "orderid": "O0001", "turnoverid": "T0009",
                        "actulamt": 1, "feeitemid": 450, "status": 0, "userno": "U0001"
                    }],
                    "orderid": "O0001",
                    "source": "app",
                    "status": 0,
                    "successdate": null,
                    "tranamt": 1,
                    "userno": "U0001"
                },
                {
                    "abstracts": "无锡国际校区 李园捌号A12 101",
                    "actulamt": 1,
                    "commitdate": "2026-09-19 17:08:04",
                    "feeitemid": 0,
                    "feeitemlist": [{"feeitemid": 448}],
                    "orderid": "O0002",
                    "source": "app",
                    "status": 1,
                    "successdate": "2026-09-19 17:08:16",
                    "tranamt": 1,
                    "third_party": "{\"account\":\"H0001\"}"
                }
            ]
        })
    }

    /// 订单解析：待支付/已完成两态；**片区 id 取 `feeitemlist[0]`**（顶层 `feeitemid` 恒 0）。
    #[test]
    fn parse_orders_reads_pending_and_done() {
        let os = parse_orders(&orders_sample());
        assert_eq!(os.len(), 2);
        let pending = &os[0];
        assert_eq!(pending.order_id, "O0001", "待支付单要能拿到 orderid 供后续查询/取消");
        assert_eq!(pending.status, 0);
        assert_eq!(pending.amount_yuan, Some(1.0));
        assert_eq!(pending.actual_yuan, None, "待支付单 actulamt 实测为 null");
        assert_eq!(pending.success_date, "");
        assert_eq!(pending.commit_date, "2026-09-19 16:40:11");
        assert_eq!(pending.source, "app");
        assert_eq!(pending.feeitem_id, "450", "顶层 feeitemid=0 ⇒ 取 feeitemlist[0]");
        let done = &os[1];
        assert_eq!(done.status, 1);
        assert_eq!(done.actual_yuan, Some(1.0));
        assert_eq!(done.success_date, "2026-09-19 17:08:16");
        assert_eq!(done.feeitem_id, "448");
    }

    /// 订单 PII 纪律：`userno`/`third_party`/`sno`/`paycard` 一律不进对外结构。
    #[test]
    fn order_dto_carries_no_pii() {
        let text = serde_json::to_string(&parse_orders(&orders_sample())).unwrap();
        for leaked in ["userno", "U0001", "third_party", "H0001", "turnoverid", "T0009"] {
            assert!(!text.contains(leaked), "不得透出 {leaked}：{text}");
        }
        assert!(text.contains("O0001"), "订单号是必须透出的（待支付单后续操作要用）");
    }

    /// 订单容错：缺 `orderList` / 空数组 / 缺 status（按 0 待支付，与 recharge 同口径）/ 片区两条都缺。
    #[test]
    fn parse_orders_tolerates_missing() {
        assert!(parse_orders(&json!({"code": 200})).is_empty());
        assert!(parse_orders(&json!({"orderList": []})).is_empty());

        let no_status = parse_orders(&json!({"orderList": [{"orderid": "O1"}]}));
        assert_eq!(no_status[0].status, 0, "缺 status 按待支付（与 RechargeOrder 同口径）");
        assert_eq!(no_status[0].feeitem_id, "");
        assert_eq!(no_status[0].amount_yuan, None);

        // orderDetailList 兜底片区 id（feeitemlist 缺失时）
        let fallback = parse_orders(&json!({"orderList": [
            {"orderid": "O2", "feeitemlist": [], "orderDetailList": [{"feeitemid": 449}]}
        ]}));
        assert_eq!(fallback[0].feeitem_id, "449");
    }

    /// 分页收敛：`page` 至少 1、`size` 落在 [1,100]（服务端实测可吃 200，此处收紧）。
    #[test]
    fn clamp_page_bounds() {
        assert_eq!(clamp_page(0, 0), ("1".to_string(), "1".to_string()));
        assert_eq!(clamp_page(2, 15), ("2".to_string(), "15".to_string()));
        assert_eq!(clamp_page(1, 999), ("1".to_string(), "100".to_string()));
    }

    /// 跨端 IPC 契约：全 camelCase（前端 `types.ts` 按这些键名取值，改键名即破坏前端）。
    #[test]
    fn dtos_are_camel_case() {
        let bill = serde_json::to_value(Bill {
            id: "T1".into(),
            time: "t".into(),
            item_name: "n".into(),
            abstracts: "a".into(),
            type_name: "y".into(),
            amount_yuan: Some(1.0),
        })
        .unwrap();
        assert!(bill.get("itemName").is_some(), "实际 {bill}");
        assert!(bill.get("typeName").is_some());
        assert!(bill.get("amountYuan").is_some());

        let page = serde_json::to_value(BillPage {
            total: 1,
            records: Vec::new(),
        })
        .unwrap();
        assert!(page.get("total").is_some());
        assert!(page.get("records").is_some());

        let order = serde_json::to_value(parse_orders(&orders_sample()))
            .unwrap()
            .as_array()
            .unwrap()[0]
            .clone();
        for k in [
            "orderId",
            "status",
            "amountYuan",
            "actualYuan",
            "commitDate",
            "successDate",
            "source",
            "abstracts",
            "feeitemId",
        ] {
            assert!(order.get(k).is_some(), "缺 {k}：{order}");
        }

        let cfg = serde_json::to_value(FeeitemConfig {
            feeitem_id: "448".into(),
            name: "n".into(),
            billing_unit: "元".into(),
            layout: vec![1],
            price_yuan: None,
            max_money_yuan: None,
            day_max_money_yuan: None,
            retain_money_yuan: None,
        })
        .unwrap();
        for k in [
            "feeitemId",
            "billingUnit",
            "priceYuan",
            "maxMoneyYuan",
            "dayMaxMoneyYuan",
            "retainMoneyYuan",
        ] {
            assert!(cfg.get(k).is_some(), "缺 {k}：{cfg}");
        }

        let month = serde_json::to_value(MonthTotal {
            month: "2026-09".into(),
            amount_yuan: 0.0,
        })
        .unwrap();
        assert!(month.get("amountYuan").is_some());
    }

    /// 年份越界：不发请求，直接报 Parse（12 次请求的参数面必须可信）。
    #[tokio::test]
    async fn fetch_monthly_rejects_bad_year() {
        let cas = campus_auth::cas::CasClient::new().expect("创建 CasClient 失败");
        let c = SynjonesClient::new(cas, None, None);
        let e = fetch_monthly(&c, 1999).await.expect_err("越界年份应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");
    }

    /// 空片区 id：配置查询直接报 Parse（不发请求）。
    #[tokio::test]
    async fn fetch_feeitem_config_rejects_empty_id() {
        let cas = campus_auth::cas::CasClient::new().expect("创建 CasClient 失败");
        let c = SynjonesClient::new(cas, None, None);
        let e = fetch_feeitem_config(&c, "  ").await.expect_err("空 id 应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");
    }

    /// 无 token 无 TGT：读取命令一律 NotLogin（不静默空列表，避免把「未登录」显示成「没有历史」）。
    #[tokio::test]
    async fn reads_require_login() {
        let cas = campus_auth::cas::CasClient::new().expect("创建 CasClient 失败");
        let c = SynjonesClient::new(cas, None, None);
        for e in [
            fetch_bills(&c, None, 1, 15).await.expect_err("应报 NotLogin"),
            fetch_monthly(&c, 2026).await.expect_err("应报 NotLogin"),
            fetch_total(&c, None).await.expect_err("应报 NotLogin"),
            fetch_feeitem_config(&c, "448").await.expect_err("应报 NotLogin"),
            fetch_orders(&c, Some(0)).await.expect_err("应报 NotLogin"),
        ] {
            assert!(matches!(e, CampusSynjonesError::NotLogin), "实际 {e:?}");
        }
    }
}
