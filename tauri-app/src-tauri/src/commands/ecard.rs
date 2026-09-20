//! 一卡通面板命令（批 1 读类 + 批 3 写类，契约 §2.2）：卡列表+客户端配置 / 分类字典 /
//! 统计三件套 / 转账账户 / 安全键盘 / 挂失解挂改密限额转账绑卡等写操作。
//!
//! 与 [`super::synjones`]（钱包页 `get_ecard`、流水 `get_ecard_transactions`）分文件；
//! 共用同一把进程级 synjones 锁与同一个客户端（**token 单活**，见 `commands::synjones`
//! 头注——本模块绝不自建第二套客户端）。
//!
//! # 写操作（批 3，协议 §1.8：bundle 反查，**未 live 验证**）
//!
//! - 密码**永不回传前端**：前端只提交 `padId` + 点击位置序列（[`PasswordInput`]），
//!   `pwd` 串由 crate 层 [`campus_synjones::ecard_ops::assemble_pwd`] 在后端拼装，用完即弃。
//! - 金额：前端传元，crate 层 ×100 成分。
//! - 本模块不做参数校验以外的业务判断；二次确认与输入校验由前端负责（契约 §2.2）。
//!
//! # 配置与 PII 纪律
//!
//! - `frontInfo` 的 `getEcardConfig` / `getFrontConfig` 是**JSON 字符串**，且 `getFrontConfig`
//!   含学校侧下发的 `privateKey`——本模块**只取白名单键**（见 [`parse_client_config`]），
//!   整串绝不落盘 / 落日志 / 透传前端（单测 [`tests::config_dto_leaks_no_private_material`] 钉死）。
//! - 卡列表走 [`campus_synjones::ecard::fetch_cards_full`]（crate 层已脱敏：只给
//!   `accountMasked` / `bankaccTail`，姓名/手机/证件/学号不解析）。
//! - 安全键盘见 [`campus_synjones::ecard_ops`]：前端只拿 `padId`（进程随机 id），真实 uuid
//!   绝不外泄；失败文案不含任何键盘内容。

use super::auth::CommandResult;
use super::synjones::{err_text, synjones_session, ERR_NO_SESSION};
use crate::infra::state::AppState;
use campus_synjones::ecard::{current_account, fetch_bank_number, int_of, yuan, CardDetail};
use campus_synjones::ecard_ops::{
    bind_bank, bind_user, cancel_bank, check_pwd, fetch_secure_keyboard, find_pwd, KeyboardKind,
    lost_card, modify_pwd, send_bind_bank_code, send_bind_user_code, send_find_pwd_code,
    set_autotrans, set_limits, unbind_user, unlost_card, PasswordInput, SecurePad,
};
use campus_synjones::ecard_stats::{
    fetch_stats_assort, fetch_stats_series, fetch_stats_summary, fetch_turnover_types,
    StatsAssortItem, StatsPoint, StatsSummary, TurnoverType,
};
use campus_synjones::client::Envelope;
use campus_synjones::ecard_face;
use campus_synjones::plat;
use serde::Serialize;
use serde_json::Value;
use tauri::State;

/// 与 `commands::electricity` 的同名宏同一份展开（取全局唯一会话/客户端、未登录回
/// 「请先登录」）。那边是 `macro_rules!` 文本作用域、未导出，跨文件复用需要改
/// `electricity.rs`（不在本批允许改动清单内），故此处按同构展开就地定义；
/// 语义与那边完全一致，后续可上提去重。
macro_rules! with_synjones {
    ($state:expr, |$client:ident| $body:expr) => {{
        let Some(guard) = synjones_session(&$state).await else {
            return Ok(CommandResult::err(ERR_NO_SESSION));
        };
        let Some(sess) = guard.as_ref() else {
            return Ok(CommandResult::err(ERR_NO_SESSION));
        };
        let $client = &sess.client;
        $body
    }};
}

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

// ---------------- 写操作命令（批 3，契约 §2.2 写类清单） ----------------

/// `ecard_check_pwd` / 发码类 → data。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EcardOpId {
    /// 发码接口响应 `data.account`（后续提交命令原样回传的 `id`/`uuid`）。
    pub id: String,
}

/// `ecard_check_pwd` → data。`bank_card_no` 本校无「校验密码查银行卡号」需求，恒 None
/// （契约 §2.5：全号仅校验通过才回——回读值来自本人卡列表 `bankacc`，无凭不回）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckPwdResult {
    pub ok: bool,
    /// 完整银行卡号（校验通过后回读；未绑定/学校未下发 → None）。
    pub bank_card_no: Option<String>,
}

/// 写操作的账号解析：卡号原号**不出后端**（前端只持脱敏号），缺省时由后端取「当前卡」。
///
/// 与「电费房间上下文串由后端合成」同一取舍：能由后端解析出的事实，就不让前端传。
/// 显式传入（多卡场景）时优先采用传入值。
async fn resolve_account(
    client: &campus_synjones::SynjonesClient,
    account: Option<String>,
) -> Result<String, String> {
    if let Some(a) = account
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
    {
        return Ok(a);
    }
    current_account(client).await.map_err(|e| err_text(&e))
}

/// 组装可选密码输入（`ecard_unlost` 用）：`padId`/`positions` 要么都给要么都不给。
/// 只给一半 → 可读错误（防止前端半截提交被静默当成免密解挂）。
fn optional_pad(
    pad_id: Option<String>,
    positions: Option<Vec<usize>>,
) -> Result<Option<PasswordInput>, String> {
    match (pad_id, positions) {
        (Some(pad_id), Some(positions)) => {
            Ok(Some(PasswordInput { pad_id, positions }))
        }
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Err("密码参数不完整：请重新获取键盘并完整输入密码".to_string()),
    }
}

/// 挂失（免密）。⚠️ 服务端会立即冻结卡片，调用前由前端完成二次确认。
#[tauri::command]
pub async fn ecard_lost(
    state: State<'_, AppState>,
    account: Option<String>,
) -> Result<CommandResult<()>, String> {
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match lost_card(client, &account).await {
            Ok(()) => CommandResult::ok(()),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 解挂（本校无 `unlockFlag` → 需密码；`padId`/`positions` 缺省 = 免密形态提交）。
#[tauri::command]
pub async fn ecard_unlost(
    state: State<'_, AppState>,
    account: Option<String>,
    pad_id: Option<String>,
    positions: Option<Vec<usize>>,
) -> Result<CommandResult<()>, String> {
    let pad = match optional_pad(pad_id, positions) {
        Ok(p) => p,
        Err(msg) => return Ok(CommandResult::err(&msg)),
    };
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match unlost_card(client, &account, pad).await {
            Ok(()) => CommandResult::ok(()),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 校验查询密码（通过 → `{ok:true}`；`bankCardNo` 本校恒 null）。
#[tauri::command]
pub async fn ecard_check_pwd(
    state: State<'_, AppState>,
    account: Option<String>,
    pad_id: String,
    positions: Vec<usize>,
) -> Result<CommandResult<CheckPwdResult>, String> {
    let pad = PasswordInput { pad_id, positions };
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match check_pwd(client, &account, pad).await {
            Ok(()) => {
                // 官方同款：完整银行卡号随卡列表早已下发，checkPwd 只是显示闸门；
                // 校验通过后回读 bankacc 填充（未绑定/学校未下发 → None，前端如实提示）。
                let bank_card_no = fetch_bank_number(client).await.unwrap_or_default();
                CommandResult::ok(CheckPwdResult {
                    ok: true,
                    bank_card_no: (!bank_card_no.is_empty()).then_some(bank_card_no),
                })
            }
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 修改查询密码（旧密 / 新密 / 确认新密各占一把键盘）。
#[tauri::command]
pub async fn ecard_modify_pwd(
    state: State<'_, AppState>,
    account: Option<String>,
    old_pad_id: String,
    old_positions: Vec<usize>,
    new_pad_id: String,
    new_positions: Vec<usize>,
    renew_pad_id: String,
    renew_positions: Vec<usize>,
) -> Result<CommandResult<()>, String> {
    let (old, new, renew) = (
        PasswordInput { pad_id: old_pad_id, positions: old_positions },
        PasswordInput { pad_id: new_pad_id, positions: new_positions },
        PasswordInput { pad_id: renew_pad_id, positions: renew_positions },
    );
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match modify_pwd(client, &account, old, new, renew).await {
            Ok(()) => CommandResult::ok(()),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 找回密码-发验证码 → `{id}`（后续 `ecard_find_pwd` 的 `id`）。
#[tauri::command]
pub async fn ecard_send_find_pwd_code(
    state: State<'_, AppState>,
    account: Option<String>,
) -> Result<CommandResult<EcardOpId>, String> {
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match send_find_pwd_code(client, &account).await {
            Ok(id) => CommandResult::ok(EcardOpId { id }),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 找回密码-提交新密码（免旧密，凭验证码）。
#[tauri::command]
pub async fn ecard_find_pwd(
    state: State<'_, AppState>,
    account: Option<String>,
    new_pad_id: String,
    new_positions: Vec<usize>,
    renew_pad_id: String,
    renew_positions: Vec<usize>,
    vercode: String,
    id: String,
) -> Result<CommandResult<()>, String> {
    let (new, renew) = (
        PasswordInput { pad_id: new_pad_id, positions: new_positions },
        PasswordInput { pad_id: renew_pad_id, positions: renew_positions },
    );
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match find_pwd(client, &account, new, renew, &vercode, &id).await {
            Ok(()) => CommandResult::ok(()),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 限额设置（`acc_type`：`CARD`/`ACCOUNT`；金额前端传元，后端 ×100 成分）。
#[tauri::command]
pub async fn ecard_set_limits(
    state: State<'_, AppState>,
    account: Option<String>,
    acc_type: String,
    daycost_limit_yuan: f64,
    nonpwd_limit_yuan: f64,
    single_limit_yuan: f64,
) -> Result<CommandResult<()>, String> {
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match set_limits(
            client,
            &account,
            &acc_type,
            daycost_limit_yuan,
            nonpwd_limit_yuan,
            single_limit_yuan,
        )
        .await
        {
            Ok(()) => CommandResult::ok(()),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 转账标识（`flag`：官方语义见 bundle；`amt_yuan` 金额元、`limite_yuan` 单笔限额元可缺省）。
#[tauri::command]
pub async fn ecard_set_autotrans(
    state: State<'_, AppState>,
    account: Option<String>,
    flag: i64,
    amt_yuan: f64,
    limite_yuan: Option<f64>,
) -> Result<CommandResult<()>, String> {
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match set_autotrans(client, &account, flag, amt_yuan, limite_yuan).await {
            Ok(()) => CommandResult::ok(()),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 绑定银行卡-发验证码 → `{id}`（本校 `specialversion=0`，`phone`/`bankacc` 不传即不带）。
#[tauri::command]
pub async fn ecard_send_bind_bank_code(
    state: State<'_, AppState>,
    account: Option<String>,
    phone: Option<String>,
    bankacc: Option<String>,
) -> Result<CommandResult<EcardOpId>, String> {
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match send_bind_bank_code(client, &account, phone.as_deref(), bankacc.as_deref()).await {
            Ok(id) => CommandResult::ok(EcardOpId { id }),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 绑定银行卡-提交。
#[tauri::command]
pub async fn ecard_bind_bank(
    state: State<'_, AppState>,
    account: Option<String>,
    bankacc: String,
    vercode: String,
    id: String,
    pad_id: String,
    positions: Vec<usize>,
) -> Result<CommandResult<()>, String> {
    let pad = PasswordInput { pad_id, positions };
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match bind_bank(client, &account, &bankacc, &vercode, &id, pad).await {
            Ok(()) => CommandResult::ok(()),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 解绑银行卡（免密）。
#[tauri::command]
pub async fn ecard_cancel_bank(
    state: State<'_, AppState>,
    account: Option<String>,
) -> Result<CommandResult<()>, String> {
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match cancel_bank(client, &account).await {
            Ok(()) => CommandResult::ok(()),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 绑定校园卡（电子账户）-发验证码 → `{id}`（即后续 `ecard_bind_user` 的 `id`/协议 `uuid`）。
#[tauri::command]
pub async fn ecard_send_bind_user_code(
    state: State<'_, AppState>,
    account: Option<String>,
) -> Result<CommandResult<EcardOpId>, String> {
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match send_bind_user_code(client, &account).await {
            Ok(id) => CommandResult::ok(EcardOpId { id }),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 绑定校园卡-提交。
#[tauri::command]
pub async fn ecard_bind_user(
    state: State<'_, AppState>,
    account: Option<String>,
    ver_code: String,
    id: String,
    pad_id: String,
    positions: Vec<usize>,
) -> Result<CommandResult<()>, String> {
    let pad = PasswordInput { pad_id, positions };
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match bind_user(client, &account, &ver_code, &id, pad).await {
            Ok(()) => CommandResult::ok(()),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 解绑校园卡。
#[tauri::command]
pub async fn ecard_unbind_user(
    state: State<'_, AppState>,
    account: Option<String>,
    remark: Option<String>,
    pad_id: String,
    positions: Vec<usize>,
) -> Result<CommandResult<()>, String> {
    let pad = PasswordInput { pad_id, positions };
    with_synjones!(state, |client| {
        // 卡号原号不透出前端：缺省时由后端取「当前卡」（见 ecard::current_account）
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        Ok(match unbind_user(client, &account, remark.as_deref(), pad).await {
            Ok(()) => CommandResult::ok(()),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
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

    /// 写类命令返回 DTO（批 3）键名契约：前端按这些键取值。
    #[test]
    fn write_command_dtos_are_camel_case() {
        let op_id = serde_json::to_value(EcardOpId { id: "acc-1".into() }).unwrap();
        assert_eq!(op_id["id"], "acc-1");

        let check = serde_json::to_value(CheckPwdResult { ok: true, bank_card_no: None }).unwrap();
        for k in ["ok", "bankCardNo"] {
            assert!(check.get(k).is_some(), "缺 {k}：{check}");
        }
        assert_eq!(check["ok"], true);
        assert!(check["bankCardNo"].is_null(), "本校无查卡号需求，恒 null");
    }

    /// `optional_pad`：padId/positions 必须成对出现——只给一半要报错，
    /// 防止「半截提交」被静默当成免密解挂发出。
    #[test]
    fn optional_pad_requires_paired_args() {
        assert!(optional_pad(None, None).unwrap().is_none(), "都不给 = 免密形态");
        let pad = optional_pad(Some("p1".into()), Some(vec![0, 1])).unwrap().unwrap();
        assert_eq!(pad.pad_id, "p1");
        assert_eq!(pad.positions, vec![0, 1]);
        assert!(optional_pad(Some("p1".into()), None).is_err(), "缺 positions 应报错");
        assert!(optional_pad(None, Some(vec![0])).is_err(), "缺 padId 应报错");
    }

    /// 红线：写类命令的任何 DTO 都不携带密码/键盘材料（结构面上就不存在这些字段）。
    #[test]
    fn write_dtos_carry_no_secret_fields() {
        for text in [
            serde_json::to_value(EcardOpId { id: "x".into() }).unwrap().to_string(),
            serde_json::to_value(CheckPwdResult { ok: true, bank_card_no: None }).unwrap().to_string(),
        ] {
            for leaked in ["pwd", "password", "keys", "positions", "uuid"] {
                assert!(!text.contains(leaked), "DTO 不得出现 {leaked}：{text}");
            }
        }
    }
}

// ---------------- 人脸采集（fapi 智慧校园服务；官方 overLightMobileH5 复刻） ----------------

/// `ecard_face_detail` → data：人脸采集状态与基础信息。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceDetailDto {
    pub name: String,
    pub number: String,
    pub school_name: String,
    /// 是否已采集（学校侧 `avatar` 非空）。
    pub collected: bool,
}

/// 读人脸采集状态（零写副作用；H5 账号用官方 autoLogin 固定密码登录）。
#[tauri::command]
pub async fn ecard_face_detail(
    state: State<'_, AppState>,
) -> Result<CommandResult<FaceDetailDto>, String> {
    with_synjones!(state, |client| {
        Ok(match ecard_face::face_detail(client).await {
            Ok(d) => CommandResult::ok(FaceDetailDto {
                collected: d.avatar_path.is_some(),
                name: d.name,
                number: d.number,
                school_name: d.school_name,
            }),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 上传人脸照片（**写操作**：写入学校人脸库；前端必须已让用户选照片并二次确认）。
/// `photo_base64`：前端 FileReader 读出的 dataURL base64 部分。
#[tauri::command]
pub async fn ecard_face_upload(
    state: State<'_, AppState>,
    photo_base64: String,
) -> Result<CommandResult<()>, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(photo_base64.trim())
        .map_err(|e| format!("照片数据解码失败：{e}"))?;
    if bytes.is_empty() {
        return Ok(CommandResult::err("照片数据为空"));
    }
    with_synjones!(state, |client| {
        Ok(match ecard_face::replace_face(client, &bytes).await {
            Ok(()) => CommandResult::ok(()),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

// ---------------- plat（移动服务平台）只读面（批 14；鉴权与一卡通同源，见 plat.rs） ----------------

/// 用户资料（`/berserker-base/user`）。本人查看本人资料，`Value` 原样透传（服务端
/// `idNumber` 已掩码）；**绝不进日志**。
#[tauri::command]
pub async fn get_plat_profile(state: State<'_, AppState>) -> Result<CommandResult<Value>, String> {
    with_synjones!(state, |client| {
        Ok(match plat::user_profile(client).await {
            Ok(v) => CommandResult::ok(v),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 绑定设备列表（`status`: `"1"` 已登录在线 / `"0"` 已授权手机设备）。
#[tauri::command]
pub async fn get_plat_equipment(
    state: State<'_, AppState>,
    status: String,
) -> Result<CommandResult<Value>, String> {
    with_synjones!(state, |client| {
        Ok(match plat::equipment(client, &status).await {
            Ok(v) => CommandResult::ok(v),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 登录日志（分页）。
#[tauri::command]
pub async fn get_plat_login_logs(
    state: State<'_, AppState>,
    page: u32,
    size: u32,
) -> Result<CommandResult<Value>, String> {
    with_synjones!(state, |client| {
        Ok(match plat::login_logs(client, page, size).await {
            Ok(v) => CommandResult::ok(v),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 脱机二维码开关状态（付款码页用；写端点由用户显式触发，不在本命令面）。
#[tauri::command]
pub async fn get_plat_offline_switch(
    state: State<'_, AppState>,
) -> Result<CommandResult<bool>, String> {
    with_synjones!(state, |client| {
        Ok(match plat::offline_switch(client).await {
            Ok(b) => CommandResult::ok(b),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

// ---------------- 付款码（一期；官方 H5 plat/pay 对齐，协议与红线见 plat.rs） ----------------

/// `get_ecard_paycode` → data。
///
/// **红线**：`codebarPayinfo` 项里的 `bandacc`（绑定银行卡全号）在此路径上**根本不解析**
/// ——DTO 结构面上就不存在该字段（同 [`parse_client_config`] 对 `privateKey` 的「不取」口径）；
/// `barcode` 是动态支付凭据，前端不得落日志 / localStorage / 错误文案。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EcardPaycode {
    /// 条码内容（一期取官方 `barcode` 数组首段，20 位数字串；条码与二维码同串）。
    pub barcode: String,
    /// 官方 `expires` 原样透传（秒级时间戳或有效期秒数，前端兼容判定；缺失/异常为 0
    /// ⇒ 前端不显示倒计时、不自动重取，只保留手动刷新）。
    pub expires: i64,
    /// 支付方式名（实测「一卡通电子钱包」）。
    pub pay_name: String,
    /// 三账户合计余额（元）：`elec_accamt + db_balance + unsettle_amount`（分→元）。
    pub balance_yuan: f64,
    /// 取码账号（当前卡原号；仅 IPC 内存在、不落日志——同 `get_ecard_transactions` 口径）。
    pub account: String,
    pub payacc: String,
    pub paytype: String,
}

/// `get_ecard_paycode_settings` → data（脱机二维码开关状态，一期只读展示）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EcardPaycodeSettings {
    pub offline_switch: bool,
}

/// 付款码：解析「当前卡」→ 查支付方式（`status==1 && code=="ACCOUNT"` 的电子账户项）
/// → 用其 `payacc`/`paytype` 取动态条码。取不到可用支付方式时报可读错误，不猜参数。
#[tauri::command]
pub async fn get_ecard_paycode(
    state: State<'_, AppState>,
    account: Option<String>,
) -> Result<CommandResult<EcardPaycode>, String> {
    with_synjones!(state, |client| {
        // 卡号原号不透出前端（前端只持脱敏号）：缺省时由后端取「当前卡」
        let account = match resolve_account(client, account).await {
            Ok(a) => a,
            Err(msg) => return Ok(CommandResult::err(&msg)),
        };
        let info = match plat::codebar_payinfo(client).await {
            Ok(v) => v,
            Err(e) => return Ok(CommandResult::err(&err_text(&e))),
        };
        let Some(pick) = info.as_array().and_then(|a| {
            a.iter()
                .find(|it| flag_of(it.get("status")) && text_of(it.get("code")) == "ACCOUNT")
        }) else {
            return Ok(CommandResult::err("无可用支付方式"));
        };
        let payacc = text_of(pick.get("payacc"));
        let paytype = text_of(pick.get("paytype"));
        Ok(match plat::pay_code(client, &account, &payacc, &paytype).await {
            Ok(data) => {
                // 官方一次给多段（页面轮换），一期取首段
                let barcode = data
                    .get("barcode")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if barcode.is_empty() {
                    CommandResult::err("学校未返回条码内容")
                } else {
                    // 余额口径 = 电子账户 + 卡账户两块之和（均分），统一 yuan() 转元
                    let fen = ["elec_accamt", "db_balance", "unsettle_amount"]
                        .iter()
                        .map(|k| int_of(pick.get(k)).unwrap_or(0))
                        .sum::<i64>();
                    CommandResult::ok(EcardPaycode {
                        barcode,
                        expires: int_of(data.get("expires")).unwrap_or(0),
                        pay_name: text_of(pick.get("name")),
                        balance_yuan: yuan(fen),
                        account,
                        payacc,
                        paytype,
                    })
                }
            }
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}

/// 付款码页设置：脱机二维码开关状态（`getUserOfflienSwitch`，只读）。
#[tauri::command]
pub async fn get_ecard_paycode_settings(
    state: State<'_, AppState>,
) -> Result<CommandResult<EcardPaycodeSettings>, String> {
    with_synjones!(state, |client| {
        Ok(match plat::offline_switch(client).await {
            Ok(b) => CommandResult::ok(EcardPaycodeSettings { offline_switch: b }),
            Err(e) => CommandResult::err(&err_text(&e)),
        })
    })
}
