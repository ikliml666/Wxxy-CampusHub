//! 一卡通业务：余额 / 卡列表 / 消费流水（慧新E校，2026-09-19 live 实测固化）。
//!
//! # 余额口径（批 1 live 实测，计划 §1.4 修订）
//!
//! | 口径 | 字段 | 含义 |
//! |---|---|---|
//! | 电子账户 | `elec_accamt` | **页面主口径**（`elec` = electronic，**不是电费余额**；电费走 `/charge/*`） |
//! | 卡账户 | `db_balance + unsettle_amount` | 次级显示（本机样本两项皆为 0） |
//!
//! 三处数值实测完全一致：`elec_accamt = accinfo[0].balance = 流水最新一条 cardBalance`。
//! 计划 §1.4 曾把 `elec_accamt` 注为「电费余额」——**已推翻**，详见 `lib.rs` 头注。
//!
//! # 单位
//!
//! 服务端金额单位一律**分**（`"8151"`），对外一律**元**（`f64`）。
//! 换算先做**整数分运算**再除 100（卡账户是两字段相加，先加后除避免浮点累加误差）。
//!
//! # 端点
//!
//! | 用途 | 方法 | 路径 | 信封 |
//! |---|---|---|---|
//! | 当前卡 | GET | `/berserker-app/ykt/tsm/queryCurrentCard`（无参） | Berserker |
//! | 卡明细 | GET | `/berserker-app/ykt/tsm/queryCard?account=…` / `?scene=recharge` | Berserker |
//! | 流水 | GET | `/berserker-search/search/personal/turnover` | Search |
//!
//! 流水 `type` 是**收支方向**：`"1"`=收入 / `"2"`=支出 / `"3"`=空；传空串 = 不带该参数。

use crate::client::{Envelope, SynjonesClient};
use crate::CampusSynjonesError;
use serde::Serialize;
use serde_json::Value;

/// 当前卡（无参）。
pub const EP_CURRENT_CARD: &str = "/berserker-app/ykt/tsm/queryCurrentCard";
/// 卡明细（`account=<卡号>` 或 `scene=recharge`）。
pub const EP_QUERY_CARD: &str = "/berserker-app/ykt/tsm/queryCard";
/// 消费流水（search 系）。
pub const EP_TURNOVER: &str = "/berserker-search/search/personal/turnover";

/// 电子账户余额字段名（`elec` = electronic；**不是电费余额**，见模块头注）。
const F_ELEC_ACCAMT: &str = "elec_accamt";

/// 一张一卡通（对外单位：元）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardInfo {
    pub account: String,
    pub cardname: String,
    /// 卡账户余额（元）= `(db_balance + unsettle_amount) / 100`。
    pub balance_yuan: f64,
    /// 电子账户余额（元）= `elec_accamt / 100`（页面主口径）。
    pub elec_accamt_yuan: f64,
    /// 中文状态标签（`acc_status` / `lostflag` 映射）。
    pub status_label: String,
}

/// 一条流水（对外单位：元）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    /// `jndatetimeStr`（服务端原文，不改格式）。
    pub time: String,
    /// `resume`（摘要）。
    pub summary: String,
    /// 带符号金额（元）：收入为正、支出为负。
    pub amount_yuan: f64,
    /// `typeFrom == "1"`（收入）。
    pub is_income: bool,
    /// 交易方（`payName`，为空回落 `consumeTypeName`）。
    pub pay_name: String,
    /// 消费地点（`locationName`）。
    pub location_name: String,
    /// **该笔交易后的余额快照**（元；`cardBalance` 字段，服务端单位分）。
    ///
    /// M4 新增的「事件级余额」数据源：一卡通流水每笔都带这个字段，故「某时刻的余额」是**可回溯**的
    /// （与服务端恒为 null 的电费每日余额不同）。字段缺失/非数值 → `None`（`Option` 保持向后兼容，
    /// 旧前端不读该键不受影响）。
    pub card_balance_yuan: Option<f64>,
}

/// 一页流水。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transactions {
    /// 该方向下的总条数（服务端 `data.total`）。
    pub total: u32,
    pub records: Vec<Transaction>,
}

// ---------------- 纯解析（单测覆盖，无网络） ----------------

/// 整数字段（金额分 / 状态码共用）：数字或数字字符串均可，缺失/非数值 → None。
fn int_of(v: Option<&Value>) -> Option<i64> {
    let v = v?;
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f.round() as i64)),
        Value::String(s) => {
            let t = s.trim();
            t.parse::<i64>()
                .ok()
                .or_else(|| t.parse::<f64>().ok().map(|f| f.round() as i64))
        }
        _ => None,
    }
}

/// 文本字段：字符串原样（trim），数字转字符串，缺失 → 空串。
fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// 分 → 元（整数分先算后除）。
fn yuan(fen: i64) -> f64 {
    fen as f64 / 100.0
}

/// 状态中文标签：挂失优先（`lostflag == 1`），否则按 `acc_status`（实测 0 = 正常）。
fn status_label_of(acc_status: Option<i64>, lostflag: Option<i64>) -> String {
    if lostflag == Some(1) {
        return "已挂失".to_string();
    }
    match acc_status {
        Some(0) => "正常".to_string(),
        Some(c) => format!("状态异常（{c}）"),
        None => "状态未知".to_string(),
    }
}

/// 卡名：实测首卡 `cardname` 为**空串** → 依次回落 `card_name` / `cardtype`
/// （本机样本 `cardtype = "800#正式卡"`，是非身份信息的卡类别标签）。
fn card_name_of(v: &Value) -> String {
    ["cardname", "card_name", "cardtype"]
        .iter()
        .map(|k| text_of(v.get(k)))
        .find(|s| !s.is_empty())
        .unwrap_or_default()
}

/// 解析单张卡（纯函数，单测覆盖）。
pub fn parse_card(v: &Value) -> CardInfo {
    let card_fen =
        int_of(v.get("db_balance")).unwrap_or(0) + int_of(v.get("unsettle_amount")).unwrap_or(0);
    let elec_fen = int_of(v.get(F_ELEC_ACCAMT)).unwrap_or(0);
    CardInfo {
        account: text_of(v.get("account")),
        cardname: card_name_of(v),
        balance_yuan: yuan(card_fen),
        elec_accamt_yuan: yuan(elec_fen),
        status_label: status_label_of(int_of(v.get("acc_status")), int_of(v.get("lostflag"))),
    }
}

/// 解析单条流水（纯函数，单测覆盖）：`typeFrom` 决定符号。
pub fn parse_transaction(rec: &Value) -> Transaction {
    let is_income = text_of(rec.get("typeFrom")) == "1";
    let fen = int_of(rec.get("tranamt")).unwrap_or(0);
    let pay_name = {
        let pay = text_of(rec.get("payName"));
        if pay.is_empty() {
            text_of(rec.get("consumeTypeName"))
        } else {
            pay
        }
    };
    Transaction {
        time: text_of(rec.get("jndatetimeStr")),
        summary: text_of(rec.get("resume")),
        amount_yuan: yuan(if is_income { fen } else { -fen }),
        is_income,
        pay_name,
        location_name: text_of(rec.get("locationName")),
        // 交易后余额快照（分→元）；缺失/垃圾值 → None（不臆造 0：0 元与「没有该字段」语义不同）
        card_balance_yuan: int_of(rec.get("cardBalance")).map(yuan),
    }
}

/// 解析流水信封的 `data`（纯函数，单测覆盖）。
pub fn parse_transactions(v: &Value) -> Transactions {
    Transactions {
        total: v["data"]["total"].as_u64().unwrap_or(0) as u32,
        records: v["data"]["records"]
            .as_array()
            .map(|a| a.iter().map(parse_transaction).collect())
            .unwrap_or_default(),
    }
}

// ---------------- 取数 ----------------

/// 当前卡（无参）→ 首张。响应缺 `data.card` 或为空数组 → [`CampusSynjonesError::Parse`]。
pub async fn fetch_current_card(client: &SynjonesClient) -> Result<CardInfo, CampusSynjonesError> {
    let v = client.get(EP_CURRENT_CARD, &[], Envelope::Berserker).await?;
    let first = v["data"]["card"]
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| CampusSynjonesError::Parse("当前卡响应缺少 data.card".to_string()))?;
    Ok(parse_card(first))
}

/// 卡明细：`account` 非空按卡号查，空则回落 `scene=recharge`（官方充值页同款）。
pub async fn fetch_cards(
    client: &SynjonesClient,
    account: &str,
) -> Result<Vec<CardInfo>, CampusSynjonesError> {
    let params: Vec<(&str, &str)> = if account.trim().is_empty() {
        vec![("scene", "recharge")]
    } else {
        vec![("account", account.trim())]
    };
    let v = client.get(EP_QUERY_CARD, &params, Envelope::Berserker).await?;
    Ok(v["data"]["card"]
        .as_array()
        .map(|a| a.iter().map(parse_card).collect())
        .unwrap_or_default())
}

/// 消费流水（分页）。`direction` 为收支方向（`"1"` 收入 / `"2"` 支出；空串 = 不过滤）。
pub async fn fetch_transactions(
    client: &SynjonesClient,
    account: &str,
    page: u32,
    size: u32,
    direction: &str,
) -> Result<Transactions, CampusSynjonesError> {
    let current = page.max(1).to_string();
    let size = size.to_string();
    let mut params: Vec<(&str, &str)> = vec![("current", &current), ("size", &size)];
    if !account.trim().is_empty() {
        params.push(("account", account.trim()));
    }
    let direction = direction.trim();
    if !direction.is_empty() {
        params.push(("type", direction));
    }
    let v = client.get(EP_TURNOVER, &params, Envelope::Search).await?;
    Ok(parse_transactions(&v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 卡余额口径（去 PII 样本）：电子账户 = `elec_accamt`（分→元）；卡账户 = db + unsettle。
    #[test]
    fn card_balances_use_electronic_account_for_elec() {
        let v = json!({
            "account": "0000",
            "cardname": "校园卡",
            "db_balance": "0",
            "unsettle_amount": "0",
            "elec_accamt": "8151",
            "acc_status": 0,
            "lostflag": 0
        });
        let c = parse_card(&v);
        assert_eq!(c.elec_accamt_yuan, 81.51, "电子账户余额（分→元）");
        assert_eq!(c.balance_yuan, 0.0, "卡账户 = (db_balance + unsettle_amount)/100");
        assert_eq!(c.status_label, "正常");
        assert_eq!(c.account, "0000");
    }

    /// 卡账户是两字段相加（先整数分相加再除 100，不吃浮点误差）。
    #[test]
    fn card_account_is_sum_of_two_fen_fields() {
        let v = json!({"db_balance": "1751", "unsettle_amount": "49", "elec_accamt": "2500"});
        let c = parse_card(&v);
        assert_eq!(c.balance_yuan, 18.0, "(1751 + 49) 分 = 18.00 元");
        assert_eq!(c.elec_accamt_yuan, 25.0);
    }

    /// 金额字段容错：数字形态 / 缺失 / 垃圾值（缺失按 0，不 panic）。
    #[test]
    fn card_amount_fields_tolerate_number_and_missing() {
        let v = json!({"db_balance": 1751, "elec_accamt": "abc"});
        let c = parse_card(&v);
        assert_eq!(c.balance_yuan, 17.51, "数字形态也要吃得下");
        assert_eq!(c.elec_accamt_yuan, 0.0, "非数值按 0（不 panic）");
        assert_eq!(c.status_label, "状态未知");
    }

    /// 状态标签：挂失优先于 acc_status；未知 acc_status 原样透出便于定位。
    #[test]
    fn status_label_prioritizes_lost_flag() {
        assert_eq!(status_label_of(Some(0), Some(1)), "已挂失");
        assert_eq!(status_label_of(Some(2), None), "状态异常（2）");
        assert_eq!(status_label_of(None, Some(0)), "状态未知");
    }

    /// 卡名回落链（实测首卡 `cardname` 为空串）：cardname → card_name → cardtype。
    #[test]
    fn card_name_falls_back_through_candidate_fields() {
        assert_eq!(
            parse_card(&json!({"cardname": "正式卡", "cardtype": "800#正式卡"})).cardname,
            "正式卡"
        );
        assert_eq!(
            parse_card(&json!({"cardname": "", "card_name": "校园卡"})).cardname,
            "校园卡"
        );
        assert_eq!(
            parse_card(&json!({"cardname": "  ", "cardtype": "800#正式卡"})).cardname,
            "800#正式卡"
        );
        assert_eq!(parse_card(&json!({})).cardname, "");
    }

    /// 流水方向与符号：`typeFrom` `"1"` 收入为正、`"2"` 支出为负（分→元）。
    #[test]
    fn transaction_sign_follows_type_from() {
        let income = parse_transaction(&json!({
            "jndatetimeStr": "2026-09-19 12:00:00",
            "resume": "充值",
            "tranamt": "350",
            "typeFrom": "1",
            "locationName": "圈存机",
            "payName": "支付宝"
        }));
        assert_eq!(income.amount_yuan, 3.50);
        assert!(income.is_income);

        let expense = parse_transaction(&json!({
            "jndatetimeStr": "2026-09-19 12:30:00",
            "resume": "食堂消费",
            "tranamt": "350",
            "typeFrom": "2",
            "consumeTypeName": "餐饮"
        }));
        assert_eq!(expense.amount_yuan, -3.50);
        assert!(!expense.is_income);
        assert_eq!(expense.pay_name, "餐饮", "payName 为空回落 consumeTypeName");
        assert_eq!(expense.summary, "食堂消费");
    }

    /// **交易后余额快照**（M4 新增）：`cardBalance` 单位是**分**（一卡通侧口径），分→元；
    /// 缺失 / `null` / 非数值 → `None`（**不臆造 0**：「0 元」与「没有该字段」语义不同，
    /// 后者若显示成 0 元会在趋势图上造出假点）。
    #[test]
    fn transaction_card_balance_is_fen_to_yuan() {
        // 实测样本：cardBalance = 8151 分，与 queryCurrentCard 的 elec_accamt 一致
        let live = parse_transaction(&json!({
            "jndatetimeStr": "2026-09-19 12:30:00",
            "resume": "食堂消费",
            "tranamt": "350",
            "typeFrom": "2",
            "cardBalance": 8151
        }));
        assert_eq!(live.card_balance_yuan, Some(81.51), "分→元（不是元口径）");

        // 字符串形态也吃得下（服务端类型漂移先例）
        let as_text = parse_transaction(&json!({"cardBalance": "8151"}));
        assert_eq!(as_text.card_balance_yuan, Some(81.51));

        // 0 分是合法余额（与「无该字段」不同）
        assert_eq!(
            parse_transaction(&json!({"cardBalance": 0})).card_balance_yuan,
            Some(0.0)
        );

        // 缺失 / null / 垃圾值 / 小数分（四舍五入）→ None 或取整
        assert_eq!(parse_transaction(&json!({})).card_balance_yuan, None);
        assert_eq!(
            parse_transaction(&json!({"cardBalance": null})).card_balance_yuan,
            None
        );
        assert_eq!(
            parse_transaction(&json!({"cardBalance": "abc"})).card_balance_yuan,
            None
        );
        assert_eq!(
            parse_transaction(&json!({"cardBalance": 8151.4})).card_balance_yuan,
            Some(81.51),
            "非整数分先取整（int_of 口径）"
        );
    }

    /// 流水信封：total 与 records；空/缺字段 → 0 条不 panic。
    #[test]
    fn transactions_envelope_parses_total_and_records() {
        let v = json!({
            "code": 200,
            "data": { "total": 1006, "records": [
                {"tranamt": "100", "typeFrom": "2", "jndatetimeStr": "2026-09-19 08:00:00"}
            ]},
            "msg": "success"
        });
        let t = parse_transactions(&v);
        assert_eq!(t.total, 1006);
        assert_eq!(t.records.len(), 1);
        assert_eq!(t.records[0].amount_yuan, -1.0);

        let empty = parse_transactions(&json!({"code": 200, "data": {}}));
        assert_eq!(empty.total, 0);
        assert!(empty.records.is_empty());

        // total 为字符串（服务端形态不定）→ 按 0 处理，不 panic
        let weird = parse_transactions(&json!({"data": {"total": "12", "records": []}}));
        assert_eq!(weird.total, 0);
    }
}
