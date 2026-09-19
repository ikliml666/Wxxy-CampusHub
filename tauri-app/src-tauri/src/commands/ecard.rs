//! 一卡通面板读类命令（批 1，契约 §2.2 读类清单）：卡列表+客户端配置 / 分类字典 /
//! 统计三件套 / 转账账户 / 安全键盘。
//!
//! 与 [`super::synjones`]（钱包页 `get_ecard`、流水 `get_ecard_transactions`）分文件；
//! 共用同一把进程级 synjones 锁与同一个客户端（**token 单活**，见 `commands::synjones`
//! 头注——本模块绝不自建第二套客户端）。
//!
//! # 批 1 只读红线
//!
//! 本模块只发 GET（含 `queryCardByTransfer` / `frontInfo` / `getAllApps` / `keyboard`），
//! **不碰任何写接口**（`/blade-pay/pay`、`lostCard`、`modifyPwd`、`payLimiteModify`、
//! `modifyAcc`、`cardTransfer`、`buildBankCardRelation`、`bindUser`/`unBind` 一律不出现在此）。
//!
//! # 配置与 PII 纪律
//!
//! - `frontInfo` 的 `getEcardConfig` / `getFrontConfig` 是**JSON 字符串**，且 `getFrontConfig`
//!   含学校侧下发的 `privateKey`——本模块**只取白名单键**（见 [`parse_client_config`]），
//!   整串绝不落盘 / 落日志 / 透传前端（单测 [`tests::config_dto_leaks_no_private_material`] 钉死）。
//! - 卡列表走 [`campus_synjones::ecard::fetch_cards_full`]（crate 层已脱敏：只给
//!   `accountMasked` / `bankaccTail`，姓名/手机/证件/学号不解析）。
//! - 转账账户（`queryCardByTransfer`）的 `name` 实测疑似持卡人姓名（live 探针按 PII 键打码、
//!   与契约 §2.5「DTO 不含 name」冲突）——**不透出**，展示名由后端按 `code` 派生（见
//!   [`TransferAccount::label`]）。
//! - 安全键盘见 [`campus_synjones::ecard_ops`]：前端只拿 `padId`（进程随机 id），真实 uuid
//!   绝不外泄；失败文案不含任何键盘内容。

use super::auth::CommandResult;
use super::synjones::{err_text, synjones_session, ERR_NO_SESSION};
use crate::infra::state::AppState;
use campus_synjones::ecard::CardDetail;
use campus_synjones::ecard_ops::{fetch_secure_keyboard, KeyboardKind, SecurePad};
use campus_synjones::ecard_stats::{
    fetch_stats_assort, fetch_stats_series, fetch_stats_summary, fetch_turnover_types,
    StatsAssortItem, StatsPoint, StatsSummary, TurnoverType,
};
use campus_synjones::client::Envelope;
use serde::Serialize;
use serde_json::Value;
use tauri::State;

/// `get_ecard_overview` → data。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EcardOverview {
    /// 全卡列表（脱敏视图，`getCampusCards`）。
    pub cards: Vec<CardDetail>,
    /// 客户端配置（官方开关白名单，见 [`EcardClientConfig`]）。
    pub config: EcardClientConfig,
}

/// 一卡通客户端配置（`frontInfo` 两个 JSON 串的**白名单键**；其余键——含 `privateKey`——
/// 一律不解析、不透传）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EcardClientConfig {
    /// 主余额口径是否为电子账户（`getEcardConfig.type !== "2"`；本校 type=1 ⇒ true）。
    pub balance_shows_electronic: bool,
    /// 是否展示学号（`showSno`；学号值本身仍不下发，显示由前端另行脱敏）。
    pub show_sno: bool,
    /// 是否启用挂失入口（`showLost`）。
    pub show_lost: bool,
    /// 冻结期间禁止充值（`freezeRecharge`）。
    pub freeze_recharge: bool,
    /// 是否收管理费（`manageFee`）。
    pub manage_fee: bool,
    /// 一卡通充值片区 id（`frontConfig.recharge`，本校 `"401"`）。
    pub recharge_feeitem_id: String,
    /// 扫码充值片区 id（`frontConfig.scan`，本校 `"407"`）。
    pub scan_feeitem_id: String,
    /// 密码规则（`passwordRule`，本校 `"A/a/Num/#/leng_6"`）。
    pub password_rule: String,
    /// 服务时段（`eCardServiceTime`，形如 `["05:00","23:50"]`）。
    pub service_time: Vec<String>,
    /// 启用的功能 appCode 白名单（`getAllApps` 中 `status==1` 的 `appCode`）。
    pub enabled_apps: Vec<String>,
}

// ---------------- 配置解析（纯函数，单测覆盖） ----------------

/// 标志位：`"1"` / 数字 1 → true，其余（`"0"`、缺失、其它真值）→ false。
fn flag_of(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        Some(Value::String(s)) => s.trim() == "1",
        _ => false,
    }
}

/// 文本字段（与 crate 层 `text_of` 同口径；命令层独立小实现避免跨 crate 泄漏助手）。
fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// 把 frontInfo `data` 里的一个 JSON **字符串**字段解析成对象（解析失败 → Null，不报错）。
fn embedded_json(data: &Value, key: &str) -> Value {
    data.get(key)
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null)
}

/// 解析 `frontInfo` → 白名单配置。`enabled_apps` 由调用方另查 `getAllApps` 后填入。
///
/// **白名单之外的一个键都不读**——`getFrontConfig` 里的 `privateKey` 等敏感键在这条
/// 路径上根本不会被触碰（不是「取了再滤」，是「不取」）。
pub fn parse_client_config(data: &Value, enabled_apps: Vec<String>) -> EcardClientConfig {
    let ecard_cfg = embedded_json(data, "getEcardConfig");
    let front_cfg = embedded_json(data, "getFrontConfig");
    // 实测（§1.5）：本校 type="1" ⇒ 主口径电子账户；契约口径 `balanceShowsElectronic = type !== "2"`
    let cfg_type = text_of(ecard_cfg.get("type"));
    EcardClientConfig {
        balance_shows_electronic: cfg_type != "2",
        show_sno: flag_of(ecard_cfg.get("showSno")),
        show_lost: flag_of(ecard_cfg.get("showLost")),
        freeze_recharge: flag_of(ecard_cfg.get("freezeRecharge")),
        manage_fee: flag_of(ecard_cfg.get("manageFee")),
        recharge_feeitem_id: text_of(front_cfg.get("recharge")),
        scan_feeitem_id: text_of(front_cfg.get("scan")),
        password_rule: text_of(front_cfg.get("passwordRule")),
        service_time: front_cfg
            .get("eCardServiceTime")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|v| text_of(Some(v))).collect())
            .unwrap_or_default(),
        enabled_apps,
    }
}

/// `getAllApps` → `status==1` 的 `appCode` 列表（宫格门控白名单）。
pub fn parse_enabled_apps(v: &Value) -> Vec<String> {
    v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|it| flag_of(it.get("status")))
                .map(|it| text_of(it.get("appCode")))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

// ---------------- 命令 ----------------

/// 一卡通面板总览：全卡列表（脱敏）+ 客户端配置。
///
/// 卡列表失败 → 整体报错（没有卡就没有面板）；配置 / 应用清单失败**静默降级**
/// （配置给官方缺省、`enabledApps` 空）——宫格少几个入口比整页打不开好。
#[tauri::command]
pub async fn get_ecard_overview(
    state: State<'_, AppState>,
) -> Result<CommandResult<EcardOverview>, String> {
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let client = &sess.client;

    let cards = match campus_synjones::ecard::fetch_cards_full(client).await {
        Ok(c) => c,
        Err(e) => return Ok(CommandResult::err(&err_text(&e))),
    };

    // frontInfo（实测 type=app 与 type=pc 响应完全相同，按官方 PC 页口径取 pc）
    let config = match client
        .get(
            "/berserker-app/frontInfo",
            &[("type", "pc")],
            Envelope::Berserker,
        )
        .await
    {
        Ok(v) => parse_client_config(&v["data"], Vec::new()),
        Err(_) => parse_client_config(&Value::Null, Vec::new()),
    };
    // getAllApps：失败 → 空白名单（宫格按「无入口」降级，不阻塞余额展示）
    let mut config = config;
    config.enabled_apps = match client
        .get(
            "/berserker-app/app/getAllApps",
            &[
                ("platType", "pc"),
                ("userType", "user"),
                ("websiteRequired", "false"),
            ],
            Envelope::Berserker,
        )
        .await
    {
        Ok(v) => parse_enabled_apps(&v),
        Err(_) => Vec::new(),
    };

    Ok(CommandResult::ok(EcardOverview { cards, config }))
}

/// 分类字典（消费/充值/退款/扫码付/补贴）。
#[tauri::command]
pub async fn get_ecard_types(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<TurnoverType>>, String> {
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(match fetch_turnover_types(&sess.client).await {
        Ok(t) => CommandResult::ok(t),
        Err(e) => CommandResult::err(&err_text(&e)),
    })
}

/// 收支汇总（`time_from`/`time_to`，形如 `2026-01-01`；空串 = 全量合计）。
#[tauri::command]
pub async fn get_ecard_stats_summary(
    state: State<'_, AppState>,
    time_from: String,
    time_to: String,
) -> Result<CommandResult<StatsSummary>, String> {
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(match fetch_stats_summary(&sess.client, &time_from, &time_to).await {
        Ok(s) => CommandResult::ok(s),
        Err(e) => CommandResult::err(&err_text(&e)),
    })
}

/// 日期序列（折线）：月视图 `date_str="2026-09"` + `date_type="month"` + `statistics_date_str="day"`；
/// 年视图 `date_str="2026"` + `date_type="year"` + `statistics_date_str="month"`。
/// `type`：`"1"` 收入 / `"2"` 支出。返回按日期升序、**零值保留**的数组。
#[tauri::command]
pub async fn get_ecard_stats_series(
    state: State<'_, AppState>,
    date_str: String,
    date_type: String,
    statistics_date_str: String,
    r#type: String,
) -> Result<CommandResult<Vec<StatsPoint>>, String> {
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(
        match fetch_stats_series(&sess.client, &date_str, &date_type, &statistics_date_str, &r#type)
            .await
        {
            Ok(p) => CommandResult::ok(p),
            Err(e) => CommandResult::err(&err_text(&e)),
        },
    )
}

/// 分类聚合（饼图）：`type`：`"1"` 收入 / `"2"` 支出。
#[tauri::command]
pub async fn get_ecard_stats_assort(
    state: State<'_, AppState>,
    r#type: String,
    time_from: String,
    time_to: String,
) -> Result<CommandResult<Vec<StatsAssortItem>>, String> {
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(match fetch_stats_assort(&sess.client, &r#type, &time_from, &time_to).await {
        Ok(a) => CommandResult::ok(a),
        Err(e) => CommandResult::err(&err_text(&e)),
    })
}

/// 一个可转账账户（`queryCardByTransfer` 条目，`data` 是**数组**不是 `data.card`）。
///
/// ⚠️ 服务端 `name` 实测疑似持卡人姓名（live 探针按 PII 键打码）——**不透出**，
/// 展示名 [`Self::label`] 由后端按 `code` 派生（`CARD`→卡账户 / `ACCOUNT`→电子账户）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferAccount {
    /// 账户号（卡号/电子账户号原文：转账操作（批 4）需要前端原样回传，契约 §2.2 明确保留）。
    pub account: String,
    /// 支付账户标识（`payacc`，实测如 `"42940-000"`）。
    pub pay_acc: String,
    /// 账户类型码（`CARD` 卡账户 / `ACCOUNT` 电子账户）。
    pub code: String,
    /// 展示名（按 `code` 派生；未识别的 code 原样透出兜底）。
    pub label: String,
    /// 余额（元）：`db_balance + unsettle_amount + elec_accamt` 求和（实测两类账户各只填自己的）。
    pub balance_yuan: f64,
    /// 是否可转出（`canTransferOut == "1"`）。
    pub can_transfer_out: bool,
    /// 已挂失（`lostflag == 1`）。
    pub lost_flag: bool,
}

/// `code` → 展示名（官方语义：两种账户）。
fn account_label(code: &str) -> String {
    match code {
        "CARD" => "卡账户".to_string(),
        "ACCOUNT" => "电子账户".to_string(),
        other => other.to_string(),
    }
}

/// 解析转账账户列表（纯函数，单测覆盖）。
pub fn parse_transfer_accounts(v: &Value) -> Vec<TransferAccount> {
    let int = |v: Option<&Value>| campus_synjones::ecard::int_of(v).unwrap_or(0);
    v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|it| {
                    let code = text_of(it.get("code"));
                    TransferAccount {
                        account: text_of(it.get("account")),
                        pay_acc: text_of(it.get("payacc")),
                        label: account_label(&code),
                        code,
                        balance_yuan: campus_synjones::ecard::yuan(
                            int(it.get("db_balance"))
                                + int(it.get("unsettle_amount"))
                                + int(it.get("elec_accamt")),
                        ),
                        can_transfer_out: text_of(it.get("canTransferOut")) == "1",
                        lost_flag: int(it.get("lostflag")) == 1,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 可转账账户列表（卡账户 ⇄ 电子账户转账页用；批 4 的写操作也用这里的 `account`）。
#[tauri::command]
pub async fn get_ecard_transfer_accounts(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<TransferAccount>>, String> {
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let v = match sess
        .client
        .get(
            "/berserker-app/ykt/tsm/queryCardByTransfer",
            &[],
            Envelope::Berserker,
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return Ok(CommandResult::err(&err_text(&e))),
    };
    Ok(CommandResult::ok(parse_transfer_accounts(&v)))
}

/// 取一把安全键盘（前端渲染用；真实服务端 uuid 留在后端缓存，`padId` 是进程随机 id）。
///
/// `kind`：`"number"`（10 键数字盘，密码输入）/ `"standard"`（数字+大小写+符号四组）。
#[tauri::command]
pub async fn get_ecard_secure_keyboard(
    state: State<'_, AppState>,
    kind: String,
) -> Result<CommandResult<SecurePad>, String> {
    let Some(parsed) = KeyboardKind::parse(&kind) else {
        return Ok(CommandResult::err("键盘类型只支持 number / standard"));
    };
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(match fetch_secure_keyboard(&sess.client, parsed).await {
        Ok(p) => CommandResult::ok(p),
        Err(e) => CommandResult::err(&err_text(&e)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// **实测样本**（2026-09-19 live `frontInfo?type=pc`，节选白名单键 + 敏感键 privateKey）。
    fn front_info_fixture() -> Value {
        json!({
            "code": 200,
            "data": {
                "getEcardConfig": "{\"freezeRecharge\":\"0\",\"showSno\":\"1\",\"type\":\"1\",\"showLost\":\"1\",\"manageFee\":\"1\",\"msCardFlag\":\"0\"}",
                "getFrontConfig": "{\"specialversion\":\"0\",\"passwordRule\":\"A/a/Num/#/leng_6\",\"paySettings\":\"101\",\"scan\":\"407\",\"recharge\":\"401\",\"eCardServiceTime\":[\"05:00\",\"23:50\"],\"privateKey\":\"-----BEGIN PRIVATE KEY-----XXXX\"}"
            }
        })
    }

    /// 配置解析：实测原值逐项钉住（本校 type=1 ⇒ 电子账户主口径、showLost=1、401/407）。
    #[test]
    fn parse_client_config_reads_live_whitelist() {
        let cfg = parse_client_config(&front_info_fixture()["data"], vec!["bill".into()]);
        assert!(cfg.balance_shows_electronic, "type=1 ⇒ 电子账户主口径");
        assert!(cfg.show_sno, "本校 showSno=1");
        assert!(cfg.show_lost, "本校 showLost=1");
        assert!(!cfg.freeze_recharge, "freezeRecharge=0");
        assert!(cfg.manage_fee);
        assert_eq!(cfg.recharge_feeitem_id, "401", "一卡通充值片区");
        assert_eq!(cfg.scan_feeitem_id, "407");
        assert_eq!(cfg.password_rule, "A/a/Num/#/leng_6");
        assert_eq!(cfg.service_time, vec!["05:00".to_string(), "23:50".to_string()]);
        assert_eq!(cfg.enabled_apps, vec!["bill".to_string()], "由调用方填入");
    }

    /// type="2"（官方另一形态：主口径卡账户）与 type 缺失的判定。
    #[test]
    fn config_balance_semantics_follow_type() {
        let mut v = front_info_fixture();
        v["data"]["getEcardConfig"] = json!("{\"type\":\"2\"}");
        assert!(!parse_client_config(&v["data"], vec![]).balance_shows_electronic);
        // 缺配置串 / 非法 JSON → 全缺省，不 panic
        let d = parse_client_config(&json!({}), vec![]);
        assert!(d.balance_shows_electronic, "缺 type 按电子账户（本校口径）");
        assert!(!d.show_lost);
        assert_eq!(d.recharge_feeitem_id, "");
        assert!(d.service_time.is_empty());
        let bad = json!({"getEcardConfig": "not json", "getFrontConfig": "{oops"});
        let d = parse_client_config(&bad, vec![]);
        assert_eq!(d.password_rule, "");
    }

    /// 红线：配置 DTO **不含** privateKey 等敏感键（白名单外一个键都不读）。
    #[test]
    fn config_dto_leaks_no_private_material() {
        let cfg = parse_client_config(&front_info_fixture()["data"], vec![]);
        let text = serde_json::to_string(&cfg).unwrap();
        for leaked in ["privateKey", "BEGIN PRIVATE", "smsVercode", "updateApp", "mercInfo",
                       "websocket", "payResult", "loginTitle"] {
            assert!(!text.contains(leaked), "不得透出 {leaked}：{text}");
        }
        assert!(text.contains("rechargeFeeitemId"), "实际 {text}");
        assert!(text.contains("passwordRule"));
        assert!(text.contains("serviceTime"));
        assert!(text.contains("enabledApps"));
        assert!(text.contains("balanceShowsElectronic"));
    }

    /// `getAllApps` → 只留 `status==1` 的 appCode（实测首项 card-lost）。
    #[test]
    fn enabled_apps_filters_by_status() {
        let v = json!({"code": 200, "data": [
            {"appCode": "card-lost", "status": 1},
            {"appCode": "disabled-app", "status": 0},
            {"appCode": "string-status", "status": "1"},
            {"appCode": "", "status": 1},
            {"status": 1}
        ]});
        assert_eq!(
            parse_enabled_apps(&v),
            vec!["card-lost".to_string(), "string-status".to_string()]
        );
        assert!(parse_enabled_apps(&json!({"code": 200})).is_empty());
    }

    /// **实测样本**（A4 `queryCardByTransfer`）：`data` 是数组，CARD 行只填卡账户字段、
    /// ACCOUNT 行只填电子账户字段（其余 null）→ 余额求和口径两类都对。
    #[test]
    fn transfer_accounts_read_live_sample() {
        let v = json!({"code": 200, "data": [
            {"code": "CARD", "account": "***", "payacc": "***", "db_balance": 0,
             "elec_accamt": null, "unsettle_amount": 0, "canTransferOut": "1",
             "lostflag": 0, "freezeflag": 0, "name": "某人", "userName": "某人", "sno": "20230000"},
            {"code": "ACCOUNT", "account": "***", "payacc": "42940-000", "db_balance": null,
             "elec_accamt": 7996, "unsettle_amount": 0, "canTransferOut": "1",
             "lostflag": 0, "name": "某人"}
        ]});
        let accounts = parse_transfer_accounts(&v);
        assert_eq!(accounts.len(), 2);
        let card = &accounts[0];
        assert_eq!(card.code, "CARD");
        assert_eq!(card.label, "卡账户", "展示名按 code 派生");
        assert_eq!(card.balance_yuan, 0.0);
        assert!(card.can_transfer_out);
        assert!(!card.lost_flag);
        let account = &accounts[1];
        assert_eq!(account.label, "电子账户");
        assert_eq!(account.balance_yuan, 79.96, "7996 分 → 79.96 元");
        // PII：name / userName / sno 一律不透出
        let text = serde_json::to_string(&accounts).unwrap();
        for leaked in ["某人", "20230000", "\"name\"", "userName", "sno"] {
            assert!(!text.contains(leaked), "不得透出 {leaked}：{text}");
        }
        assert!(parse_transfer_accounts(&json!({"code": 200})).is_empty());
    }

    /// 命令面 IPC 契约：全 camelCase（前端 `types.ts` 按这些键名取值，改键名即破坏前端）。
    #[test]
    fn command_dtos_are_camel_case() {
        let cfg = serde_json::to_value(parse_client_config(&front_info_fixture()["data"], vec!["bill".into()]))
            .unwrap();
        for k in [
            "balanceShowsElectronic",
            "showSno",
            "showLost",
            "freezeRecharge",
            "manageFee",
            "rechargeFeeitemId",
            "scanFeeitemId",
            "passwordRule",
            "serviceTime",
            "enabledApps",
        ] {
            assert!(cfg.get(k).is_some(), "缺 {k}：{cfg}");
        }

        let accounts = serde_json::to_value(parse_transfer_accounts(&json!({"data": [
            {"code": "ACCOUNT", "account": "A1", "payacc": "42940-000", "elec_accamt": 100,
             "canTransferOut": "1", "lostflag": 0}
        ]})))
        .unwrap();
        let a = &accounts[0];
        for k in ["account", "payAcc", "code", "label", "balanceYuan", "canTransferOut", "lostFlag"] {
            assert!(a.get(k).is_some(), "缺 {k}：{a}");
        }
        assert_eq!(a["balanceYuan"], 1.0);

        let overview = serde_json::to_value(EcardOverview {
            cards: Vec::new(),
            config: parse_client_config(&Value::Null, vec![]),
        })
        .unwrap();
        assert!(overview.get("cards").is_some());
        assert!(overview.get("config").is_some());

        // 卡详情（crate DTO 复用为命令返回）：关键字段 camelCase
        let card = serde_json::to_value(campus_synjones::ecard::parse_card_detail(&json!({
            "account": "1234567890", "elec_accamt": 7996, "autotrans_flag": 1
        })))
        .unwrap();
        for k in [
            "accountMasked",
            "cardTypeName",
            "statusLabel",
            "balanceYuan",
            "elecBalanceYuan",
            "expDate",
            "autotransFlag",
            "autotransAmtYuan",
            "dayCostLimitYuan",
            "nonpwdLimitYuan",
            "singleLimitYuan",
            "bankaccTail",
            "accInfos",
        ] {
            assert!(card.get(k).is_some(), "缺 {k}：{card}");
        }
    }
}
