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
//! | 全卡列表 | GET | `/berserker-app/ykt/tsm/getCampusCards`（无参） | Berserker |
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
/// 全卡列表（无参；`data.card[]`，字段与 [`EP_QUERY_CARD`] 同源）。
pub const EP_CARDS_FULL: &str = "/berserker-app/ykt/tsm/getCampusCards";
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
    /// 流水单号（`orderId`，实测存在；单条详情查询用）。
    pub order_id: String,
    /// 分类 id（`typeId`，如 `"1"` 消费）。
    pub type_id: String,
    /// 分类名（`turnoverType`，如 `"消费"`）。
    pub turnover_type: String,
    /// 标签（`labelName`，常为空串）。
    pub label_name: String,
    /// 标签备注（`labelRemark`，常为空串）。
    pub label_remark: String,
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
pub fn int_of(v: Option<&Value>) -> Option<i64> {
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
pub fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// 分 → 元（整数分先算后除）。
pub fn yuan(fen: i64) -> f64 {
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

/// 卡号脱敏（契约 §2.5）：够长时前 5 + `****` + 后 2；**短卡号**（本校实测 5 位，如 `42940`）
/// 首位 + `****` + 末 2 —— 旧实现短号一律给 `****`，界面等于什么都没显示。
fn mask_account(acc: &str) -> String {
    let chars: Vec<char> = acc.trim().chars().collect();
    if chars.len() >= 8 {
        let head: String = chars[..5].iter().collect();
        let tail: String = chars[chars.len() - 2..].iter().collect();
        format!("{head}****{tail}")
    } else if chars.len() > 3 {
        let head = chars[0];
        let tail: String = chars[chars.len() - 2..].iter().collect();
        format!("{head}****{tail}")
    } else {
        "****".to_string()
    }
}

/// 只保留末尾 `n` 位（银行卡号尾号）；不足 `n` 位 → 空串（不回显全号）。
fn tail_of(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.trim().chars().collect();
    if chars.len() >= n {
        chars[chars.len() - n..].iter().collect()
    } else {
        String::new()
    }
}

/// 一个子账户（卡 `accinfo[]` 条目，单位全为分→元）。
///
/// ⚠️ 服务端的 `accinfo[].name` 实测形态未知（live 探针按 PII 键名打了码），按「PII 不透出」红线
/// **不解析**；子账户的展示名由前端按 `type` 自行映射。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccInfo {
    /// 子账户类型码（`type`，实测如 `"42940-000"`）。
    #[serde(rename = "type")]
    pub kind: String,
    /// 子账户余额（元，`balance` 分→元）。
    pub balance_yuan: f64,
    /// 当日已消费（元，`daycostamt`）。
    pub day_cost_amt_yuan: f64,
    /// 单日消费限额（元，`daycostlimit`）。
    pub day_cost_limit_yuan: f64,
    /// 免密限额（元，`nonpwdlimit`）。
    pub nonpwd_limit_yuan: f64,
    /// 单笔限额（元，`singlelimit`）。
    pub single_limit_yuan: f64,
}

/// 解析一个子账户（纯函数）。
fn parse_acc_info(v: &Value) -> AccInfo {
    AccInfo {
        kind: text_of(v.get("type")),
        balance_yuan: yuan(int_of(v.get("balance")).unwrap_or(0)),
        day_cost_amt_yuan: yuan(int_of(v.get("daycostamt")).unwrap_or(0)),
        day_cost_limit_yuan: yuan(int_of(v.get("daycostlimit")).unwrap_or(0)),
        nonpwd_limit_yuan: yuan(int_of(v.get("nonpwdlimit")).unwrap_or(0)),
        single_limit_yuan: yuan(int_of(v.get("singlelimit")).unwrap_or(0)),
    }
}

/// 一张卡的完整视图（`getCampusCards` 口径，卡设置/卡详情页用；单位一律元）。
///
/// # 脱敏（契约 §2.5）
///
/// - 卡号只给 [`Self::account_masked`]（前 5 + `****` + 后 2），**不含原号**；
/// - 银行卡只给 [`Self::bankacc_tail`] 尾号；
/// - 持卡人姓名 / 手机号 / 证件 / 学号一律不解析（`getCampusCards` 返回里有，全部丢弃）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardDetail {
    /// 脱敏卡号。
    pub account_masked: String,
    /// 卡类型名（回落链同 [`CardInfo::cardname`]：cardname → card_name → cardtype）。
    pub card_type_name: String,
    /// 中文状态标签（同 [`CardInfo::status_label`]）。
    pub status_label: String,
    /// 卡账户余额（元）= `(db_balance + unsettle_amount) / 100`。
    pub balance_yuan: f64,
    /// 电子账户余额（元，`elec_accamt` 分→元）。
    pub elec_balance_yuan: f64,
    /// 已挂失（`lostflag == 1`）。
    pub lost: bool,
    /// 已冻结（`freezeflag == 1`）。
    pub frozen: bool,
    /// 卡状态码（`acc_status`，实测 0 = 正常；缺失 → None）。
    pub acc_status: Option<i64>,
    /// 卡有效期（`expdate` 原文）。
    pub exp_date: String,
    /// 自动转账（圈存）开关（**非 0 即开启**——官方真机实测档位 `2` 也表示开启）。
    pub autotrans_flag: bool,
    /// 圈存档位原值（0=禁止 1=只允许自助 2=自助及自动；官方 setCard 页按此显示）。
    pub autotrans_flag_kind: i64,
    /// 自动转账金额（元，`autotrans_amt` 分→元）。
    pub autotrans_amt_yuan: f64,
    /// 自动转账余额下限（元，`autotrans_limite` 分→元）。
    pub autotrans_limite_yuan: f64,
    /// 单日消费限额（元，`daycostlimit` 分→元；实测 0 = 未设置）。
    pub day_cost_limit_yuan: f64,
    /// 免密限额（元，`nonpwdlimit`）。
    pub nonpwd_limit_yuan: f64,
    /// 单笔限额（元，`singlelimit`）。
    pub single_limit_yuan: f64,
    /// 绑定银行卡**尾号**（`bankacc` 末 4 位；未绑定 → 空串）。
    pub bankacc_tail: String,
    /// 子账户列表（`accinfo[]`）。
    pub acc_infos: Vec<AccInfo>,
}

/// 解析一张完整卡（纯函数，单测覆盖；金额/状态复用 [`parse_card`] 的已测口径）。
pub fn parse_card_detail(v: &Value) -> CardDetail {
    let info = parse_card(v);
    CardDetail {
        account_masked: mask_account(&info.account),
        card_type_name: info.cardname,
        status_label: info.status_label,
        balance_yuan: info.balance_yuan,
        elec_balance_yuan: info.elec_accamt_yuan,
        lost: int_of(v.get("lostflag")) == Some(1),
        frozen: int_of(v.get("freezeflag")) == Some(1),
        acc_status: int_of(v.get("acc_status")),
        exp_date: text_of(v.get("expdate")),
        // ⚠️ 官方真机 queryCard 卡级 `autotrans_flag: 2` 表示「自助及自动转账」——
        // 旧实现 `== Some(1)` 把它读成 false，圈存写入成功后界面仍显示「关闭」。
        autotrans_flag: int_of(v.get("autotrans_flag")).unwrap_or(0) != 0,
        autotrans_flag_kind: int_of(v.get("autotrans_flag")).unwrap_or(0),
        autotrans_amt_yuan: yuan(int_of(v.get("autotrans_amt")).unwrap_or(0)),
        autotrans_limite_yuan: yuan(int_of(v.get("autotrans_limite")).unwrap_or(0)),
        day_cost_limit_yuan: yuan(int_of(v.get("daycostlimit")).unwrap_or(0)),
        nonpwd_limit_yuan: yuan(int_of(v.get("nonpwdlimit")).unwrap_or(0)),
        single_limit_yuan: yuan(int_of(v.get("singlelimit")).unwrap_or(0)),
        bankacc_tail: tail_of(&text_of(v.get("bankacc")), 4),
        acc_infos: v
            .get("accinfo")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(parse_acc_info).collect())
            .unwrap_or_default(),
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
        order_id: text_of(rec.get("orderId")),
        type_id: text_of(rec.get("typeId")),
        turnover_type: text_of(rec.get("turnoverType")),
        label_name: text_of(rec.get("labelName")),
        label_remark: text_of(rec.get("labelRemark")),
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

/// 全卡列表（无参）→ 完整卡视图（脱敏后）。缺 `data.card` → 空列表。
pub async fn fetch_cards_full(
    client: &SynjonesClient,
) -> Result<Vec<CardDetail>, CampusSynjonesError> {
    let v = client.get(EP_CARDS_FULL, &[], Envelope::Berserker).await?;
    Ok(v["data"]["card"]
        .as_array()
        .map(|a| a.iter().map(parse_card_detail).collect())
        .unwrap_or_default())
}

/// 「当前卡」的**卡号原号**——写操作（挂失/解挂/改密/限额/圈存/绑卡）的账号来源。
///
/// 为什么放在这里而不是让前端传：卡号原号**不暴露给前端**（[`CardDetail`] 只有 `account_masked`），
/// 而这些写操作恒作用于本人当前卡 ⇒ 账号由后端自解析，前端拿不到也不需要（与「房间上下文串由后端
/// 合成」同一取舍）。前端若要显式指定账号（多卡场景）仍可传，由命令层优先采用。
pub async fn current_account(client: &SynjonesClient) -> Result<String, CampusSynjonesError> {
    let v = client.get(EP_CARDS_FULL, &[], Envelope::Berserker).await?;
    let acc = v["data"]["card"]
        .as_array()
        .and_then(|a| a.first())
        .map(|c| text_of(c.get("account")))
        .unwrap_or_default();
    if acc.trim().is_empty() {
        return Err(CampusSynjonesError::Parse(
            "未取到校园卡号，请稍后重试".to_string(),
        ));
    }
    Ok(acc)
}

/// 流水查询的增强筛选（契约 §1.4：全部实测生效）。
///
/// `None` / 空串的参数**不进 query**（避免改变服务端语义——实测 `type` 不传即全量）。
#[derive(Debug, Clone, Default)]
pub struct TurnoverFilter<'a> {
    /// 卡号（空 = 不带 `account` 参数，查本人全部流水）。
    pub account: &'a str,
    /// 收支方向（`"1"` 收入 / `"2"` 支出 / None 全量）。
    pub direction: Option<&'a str>,
    /// 分类 id（`typeId`，取自 `turnoverType` 字典）。
    pub type_id: Option<&'a str>,
    /// 关键词搜索（自动附带 `highlightFieldsClass=text-primary`，官方同款）。
    pub info: Option<&'a str>,
    /// 单条详情（`orderId`，实测命中时 `total=1`）。
    pub order_id: Option<&'a str>,
    /// 排序字段（如 `tranamt`，实测生效）。
    pub sort_fields: Option<&'a str>,
    /// 排序方向（`asc` / `desc`）。
    pub sort_type: Option<&'a str>,
}

/// 组装流水 query 参数（纯函数，单测覆盖：未传的可选参数**不出现**）。
fn build_turnover_params(f: &TurnoverFilter<'_>, page: u32, size: u32) -> Vec<(String, String)> {
    let mut p = vec![
        ("current".to_string(), page.max(1).to_string()),
        ("size".to_string(), size.to_string()),
    ];
    let account = f.account.trim();
    if !account.is_empty() {
        p.push(("account".to_string(), account.to_string()));
    }
    if let Some(d) = f.direction.map(str::trim).filter(|s| !s.is_empty()) {
        p.push(("type".to_string(), d.to_string()));
    }
    if let Some(t) = f.type_id.map(str::trim).filter(|s| !s.is_empty()) {
        p.push(("typeId".to_string(), t.to_string()));
    }
    if let Some(i) = f.info.map(str::trim).filter(|s| !s.is_empty()) {
        p.push(("info".to_string(), i.to_string()));
        p.push(("highlightFieldsClass".to_string(), "text-primary".to_string()));
    }
    if let Some(o) = f.order_id.map(str::trim).filter(|s| !s.is_empty()) {
        p.push(("orderId".to_string(), o.to_string()));
    }
    if let Some(sf) = f.sort_fields.map(str::trim).filter(|s| !s.is_empty()) {
        p.push(("sortFields".to_string(), sf.to_string()));
    }
    if let Some(st) = f.sort_type.map(str::trim).filter(|s| !s.is_empty()) {
        p.push(("sortType".to_string(), st.to_string()));
    }
    p
}

/// 消费流水（分页 + 增强筛选）。
pub async fn fetch_transactions(
    client: &SynjonesClient,
    filter: &TurnoverFilter<'_>,
    page: u32,
    size: u32,
) -> Result<Transactions, CampusSynjonesError> {
    let pairs = build_turnover_params(filter, page, size);
    let params: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
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

    /// 流水新字段（2026-09-19 live 实测记录形态）：orderId/typeId/turnoverType/labelName/labelRemark。
    #[test]
    fn transaction_reads_enhanced_fields() {
        // 实测样本（C1，字段值已脱敏的保持脱敏）
        let rec = json!({
            "orderId": "***",
            "tranamt": 10,
            "typeFrom": "2",
            "typeId": "1",
            "turnoverType": "消费",
            "payName": "电子账户消费",
            "locationName": "1-017",
            "labelName": "",
            "labelRemark": "",
            "cardBalance": 7996
        });
        let t = parse_transaction(&rec);
        assert_eq!(t.type_id, "1");
        assert_eq!(t.turnover_type, "消费");
        assert_eq!(t.pay_name, "电子账户消费");
        assert_eq!(t.location_name, "1-017");
        assert_eq!(t.card_balance_yuan, Some(79.96));
        assert!(!t.order_id.is_empty(), "orderId 原样透传（值不写入测试）");
        assert_eq!(t.label_name, "");
        assert_eq!(t.label_remark, "");
    }

    /// 卡号 / 银行卡脱敏（契约 §2.5）：前 5 + **** + 后 2；短号全掩；银行卡只留末 4。
    #[test]
    fn account_and_bankacc_are_masked() {
        assert_eq!(mask_account("1234567890"), "12345****90");
        assert_eq!(
            mask_account("42940"),
            "4****40",
            "本校实测 5 位卡号：露首位与末 2 位，界面能对上自己的卡"
        );
        assert_eq!(mask_account("1234567"), "1****67");
        assert_eq!(mask_account("123"), "****", "3 位及以下不给可辨识片段");
        assert_eq!(mask_account(""), "****");
        assert_eq!(tail_of("6222021234567890123", 4), "0123");
        assert_eq!(tail_of("123", 4), "", "不足 4 位不给尾号（防全号回显）");
        assert_eq!(tail_of("", 4), "");
    }

    /// **实测样本**（2026-09-19 live，`getCampusCards` 首卡，PII 键已脱敏）：
    /// `elec_accamt=7996`、`db_balance=0`、`unsettle_amount=0`、`autotrans_flag=1`、
    /// `autotrans_amt=5000`、`autotrans_limite=2000`、三项限额 0、`acc_status=0`。
    fn card_detail_fixture() -> Value {
        json!({
            "acc_status": 0,
            "accinfo": [{
                "type": "42940-000",
                "balance": 7996,
                "daycostamt": null,
                "daycostlimit": null,
                "nonpwdlimit": null,
                "singlelimit": null,
                "autotrans_flag": 0
            }],
            "account": "0000000000",
            "autotrans_amt": 5000,
            "autotrans_flag": 1,
            "autotrans_limite": 2000,
            "bankacc": "6222021234567890123",
            "card_name": "本科生卡",
            "cardname": "",
            "cardtype": "800#正式卡",
            "db_balance": 0,
            "elec_accamt": 7996,
            "expdate": "2027-09-01",
            "freezeflag": 0,
            "lostflag": 0,
            "name": "某人",
            "nonpwdlimit": 0,
            "phone": "13800000000",
            "singlelimit": 0,
            "sno": "20230000",
            "unsettle_amount": 0
        })
    }

    /// 卡详情映射：金额全部分→元、状态/开关布尔化、限额三件、autotrans 三件（实测原值）。
    #[test]
    fn card_detail_maps_live_fixture() {
        let c = parse_card_detail(&card_detail_fixture());
        assert_eq!(c.account_masked, "00000****00", "卡号脱敏（前5+****+后2）");
        assert_eq!(c.card_type_name, "本科生卡", "cardname 空回落 card_name");
        assert_eq!(c.status_label, "正常");
        assert_eq!(c.elec_balance_yuan, 79.96, "电子账户分→元（实测 7996 分）");
        assert_eq!(c.balance_yuan, 0.0, "卡账户 = db + unsettle");
        assert!(!c.lost);
        assert!(!c.frozen);
        assert_eq!(c.acc_status, Some(0));
        assert_eq!(c.exp_date, "2027-09-01");
        assert!(c.autotrans_flag, "实测 autotrans_flag=1");
        assert_eq!(c.autotrans_amt_yuan, 50.0, "5000 分 → 50 元");
        assert_eq!(c.autotrans_limite_yuan, 20.0, "2000 分 → 20 元");
        assert_eq!(c.day_cost_limit_yuan, 0.0);
        assert_eq!(c.nonpwd_limit_yuan, 0.0);
        assert_eq!(c.single_limit_yuan, 0.0);
        assert_eq!(c.bankacc_tail, "0123", "银行卡只留末 4 位");
        assert_eq!(c.acc_infos.len(), 1);
        assert_eq!(c.acc_infos[0].kind, "42940-000");
        assert_eq!(c.acc_infos[0].balance_yuan, 79.96);
    }

    /// PII 红线：卡详情 DTO 不得含姓名 / 手机号 / 证件 / 学号 / 原始卡号 / 银行卡全号。
    #[test]
    fn card_detail_dto_carries_no_pii() {
        let text = serde_json::to_string(&parse_card_detail(&card_detail_fixture())).unwrap();
        for leaked in [
            "某人", "13800000000", "20230000", "0000000000", "6222021234567890123", "phone",
            "cert", "sno", "bankacc\",", "\"account\"",
        ] {
            assert!(!text.contains(leaked), "不得透出 {leaked}：{text}");
        }
        assert!(text.contains("accountMasked"));
    }

    /// 卡详情容错：空对象 / null 字段不 panic（开关全 false、金额 0、accinfo 缺失为空表）。
    #[test]
    fn card_detail_tolerates_missing_fields() {
        let c = parse_card_detail(&json!({}));
        assert_eq!(c.account_masked, "****");
        assert!(!c.lost && !c.frozen && !c.autotrans_flag);
        assert_eq!(c.acc_status, None);
        assert!(c.acc_infos.is_empty());
        // 挂失 / 冻结状态
        let lost = parse_card_detail(&json!({"lostflag": 1, "freezeflag": 1, "account": "12345678"}));
        assert!(lost.lost && lost.frozen);
    }

    /// 流水增强参数：**未传的可选参数不进 query**（实测语义：不传 = 不过滤）。
    #[test]
    fn turnover_params_omit_unset_options() {
        let f = TurnoverFilter::default();
        let p = build_turnover_params(&f, 1, 15);
        assert_eq!(p, vec![("current".into(), "1".into()), ("size".into(), "15".into())]);
        assert!(!p.iter().any(|(k, _)| k == "type"), "缺省不得带 type");

        let f = TurnoverFilter {
            account: " 123 ",
            direction: Some("2"),
            type_id: Some("1"),
            info: Some("食堂"),
            order_id: Some("ORD1"),
            sort_fields: Some("tranamt"),
            sort_type: Some("desc"),
        };
        let p = build_turnover_params(&f, 0, 20);
        assert_eq!(
            p,
            vec![
                ("current".to_string(), "1".to_string()),
                ("size".to_string(), "20".to_string()),
                ("account".to_string(), "123".to_string()),
                ("type".to_string(), "2".to_string()),
                ("typeId".to_string(), "1".to_string()),
                ("info".to_string(), "食堂".to_string()),
                ("highlightFieldsClass".to_string(), "text-primary".to_string()),
                ("orderId".to_string(), "ORD1".to_string()),
                ("sortFields".to_string(), "tranamt".to_string()),
                ("sortType".to_string(), "desc".to_string()),
            ]
        );

        // info 传了才带 highlightFieldsClass；只给 sortFields 不强加 sortType
        let partial = build_turnover_params(
            &TurnoverFilter { info: Some("x"), ..Default::default() },
            3,
            10,
        );
        assert!(partial.contains(&("info".to_string(), "x".to_string())));
        assert!(partial.contains(&("highlightFieldsClass".to_string(), "text-primary".to_string())));
        assert!(!partial.iter().any(|(k, _)| k == "typeId"));
        assert!(!partial.iter().any(|(k, _)| k == "sortType"));
    }
}
