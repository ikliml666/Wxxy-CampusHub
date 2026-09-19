//! 一卡通命令面（M3 批 2）：钱包页 `get_ecard` / `get_ecard_transactions` 与
//! 首页钱包卡 `get_wallet_cards`（后端聚合 + 门户降级，计划 §2.4）。
//!
//! # 取数口径
//!
//! 余额与流水全部来自 `campus_synjones::ecard`（慧新E校实时；单位换算在 crate 内做）。
//! 钱包页主数字 = **电子账户** `elec_accamt`（批 1 live 实测口径，计划 §1.4 修订），
//! 卡账户 `(db_balance + unsettle_amount)` 作次级显示。
//!
//! # token 单活 → 全应用一个客户端
//!
//! 慧新E校 token 是**单活**的：同一账号同一时刻只有最新一次 SSO 签发的 token 有效，
//! 并发 SSO 会互相顶掉（计划 §1.3）。故本模块用一个进程级 `static` 缓存**唯一**
//! `SynjonesClient`，并让命令**整段持有**该锁（`tokio::sync::MutexGuard` 跨 await），
//! 把 SSO 与业务请求串行化——开销是几次一卡通调用互相排队，代价可忽略，
//! 换来的是「绝不并发 SSO」这条硬约束。会话变更（换账号 / 重新登录换了 TGT）时重建。
//!
//! # 失败文案
//!
//! 业务失败一律 `Ok(CommandResult::err(中文))`（`Err(String)` 仅限 IPC 框架层）；
//! `NotLogin` 归一为「登录已过期，请重新登录」（与批 0 的门户口径一致）。
//! 敏感纪律：本模块不输出任何 token / TGT / 卡号到日志或错误消息
//! （`EcardOverview.account` 是给前端查流水分页用的卡号，只在 IPC 数据里，不入日志）。

use super::auth::CommandResult;
use crate::infra::state::AppState;
use campus_synjones::ecard::{
    fetch_cards, fetch_current_card, fetch_transactions, CardInfo, Transactions,
};
use campus_synjones::{CampusSynjonesError, SynjonesClient};
use serde::Serialize;
use std::sync::OnceLock;
use tauri::State;

/// 无会话时的约定文案（与 portal.rs / profile.rs / timetable.rs 同口径）。
const ERR_NO_SESSION: &str = "请先登录";

/// 流水每页条数（前端「加载更多」按 `total` 判断是否还有下一页）。
const PAGE_SIZE: u32 = 15;

// ---------------- 全局唯一客户端 ----------------

/// 缓存的慧新E校会话（`username` + `tgt` 任一变化即重建，避免持过期 TGT 的旧客户端）。
struct SynjonesSession {
    username: String,
    tgt: Option<String>,
    client: SynjonesClient,
}

static SYNJONES: OnceLock<tokio::sync::Mutex<Option<SynjonesSession>>> = OnceLock::new();

/// 取全局唯一客户端；返回的 guard **必须活到请求结束**（持锁即串行化，见模块头注）。
/// 未登录 → None（调用方回「请先登录」）。
async fn synjones_session(
    state: &State<'_, AppState>,
) -> Option<tokio::sync::MutexGuard<'static, Option<SynjonesSession>>> {
    // 锁内只 clone（cas/username/tgt 均廉价），drop guard 后再 await（本项目锁纪律）
    let (username, tgt, cas) = {
        let guard = state.session.lock().await;
        let s = guard.as_ref()?;
        (s.username.clone(), s.tgt.clone(), s.client.clone())
    };
    let mut guard = SYNJONES
        .get_or_init(|| tokio::sync::Mutex::new(None))
        .lock()
        .await;
    let stale = match guard.as_ref() {
        Some(sess) => sess.username != username || sess.tgt != tgt,
        None => true,
    };
    if stale {
        let client = SynjonesClient::new(cas, tgt.clone(), None);
        *guard = Some(SynjonesSession {
            username,
            tgt,
            client,
        });
    }
    Some(guard)
}

/// 会话失效归一为可操作文案，其余透出 crate 的中文错误（不回显票据/凭据）。
fn err_text(e: &CampusSynjonesError) -> String {
    match e {
        CampusSynjonesError::NotLogin => "登录已过期，请重新登录".to_string(),
        other => other.to_string(),
    }
}

// ---------------- 钱包页 ----------------

/// get_ecard → data。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EcardOverview {
    /// 电子账户余额（元）——**页面主口径**（`elec_accamt`）。
    pub balance_yuan: f64,
    /// 卡账户余额（元）= `(db_balance + unsettle_amount) / 100`（次级显示）。
    pub card_balance_yuan: f64,
    /// 卡号（前端查流水分页用；不作身份展示）。
    pub account: String,
    pub cards: Vec<CardInfo>,
    /// 今日消费（元）。⚠️ 恒 `None`：`statistics/turnover/sum/user` 五参齐备
    /// （`account`/`dateType`/`dateStr`/`statisticsDateStr`/`type`）仍恒返回 `data:{}`
    /// （2026-09-19 live 实测，见 `tests/synjones_live.rs::synjones_ecard_live`），
    /// 前端据此显示「暂不可用」。
    pub today_spend: Option<f64>,
    /// 本月消费（元）。同 [`Self::today_spend`]，恒 `None`。
    pub month_spend: Option<f64>,
}

/// 钱包页总览：当前卡 + 卡列表 + 流水消费统计位（见 [`EcardOverview`]）。
#[tauri::command]
pub async fn get_ecard(state: State<'_, AppState>) -> Result<CommandResult<EcardOverview>, String> {
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let client = &sess.client;

    let card = match fetch_current_card(client).await {
        Ok(c) => c,
        Err(e) => return Ok(CommandResult::err(&err_text(&e))),
    };
    // 卡列表（多卡场景）：卡明细取失败不让整页失败，回落当前卡本身
    let cards = fetch_cards(client, &card.account)
        .await
        .unwrap_or_else(|_| vec![card.clone()]);

    Ok(CommandResult::ok(EcardOverview {
        balance_yuan: card.elec_accamt_yuan,
        card_balance_yuan: card.balance_yuan,
        account: card.account,
        cards,
        today_spend: None,
        month_spend: None,
    }))
}

/// 流水一页：`account` 为卡号（先取 [`get_ecard`]），`page` 从 1 起。
/// 不带方向过滤 = 全量（实测 1042 = 支出 1006 + 收入 36）。
#[tauri::command]
pub async fn get_ecard_transactions(
    state: State<'_, AppState>,
    account: String,
    page: u32,
) -> Result<CommandResult<Transactions>, String> {
    if account.trim().is_empty() {
        return Ok(CommandResult::err("卡号缺失，请刷新余额后重试"));
    }
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let client = &sess.client;

    Ok(
        match fetch_transactions(client, &account, page, PAGE_SIZE, "").await {
            Ok(t) => CommandResult::ok(t),
            Err(e) => CommandResult::err(&err_text(&e)),
        },
    )
}

// ---------------- 首页钱包卡（后端聚合 + 降级） ----------------

/// 一卡通格：实时值与来源标注。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EcardValue {
    /// 余额（元）；实时与门户都取不到 → None（前端显示 "—"）。
    pub value_yuan: Option<f64>,
    /// `"realtime"`（慧新E校实时）/ `"portal"`（门户快照降级）/ `"none"`。
    pub source: String,
    /// 取数时刻（RFC3339）。
    pub updated_at: String,
}

/// 电费格（计划 §2.4）：电费余额只能走 `/charge/*` 级联，**批 3 接入**，当前恒 None。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElecValue {
    pub value_yuan: Option<f64>,
    /// `"realtime"`（批 3 起）| `"none"`（未接入）。
    pub source: String,
}

/// 未读邮件格（门户唯一来源；取失败 `unread: None` → 前端显示 "—"）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailValue {
    pub unread: Option<u32>,
}

/// 在借图书格（门户唯一来源；取失败 `borrowed: None`）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryValue {
    pub borrowed: Option<u32>,
}

/// get_wallet_cards → data（计划 §2.4）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletCards {
    pub ecard: EcardValue,
    pub mail: MailValue,
    pub library: LibraryValue,
    pub elec: ElecValue,
}

/// 首页钱包卡：一卡通先试慧新E校实时、失败回落门户快照（标 `source:"portal"`）；
/// 邮箱未读 / 图书借阅仍取门户；电费留空（批 3）。
///
/// **失败一律静默降级**：实时失败不弹错，门户也失败则 `value_yuan: None` + `source:"none"`，
/// 由前端显示 "—"（用户裁决：门户数据先渲染、实时到后替换，不阻塞首屏）。
#[tauri::command]
pub async fn get_wallet_cards(state: State<'_, AppState>) -> Result<CommandResult<WalletCards>, String> {
    // 门户客户端（锁内只 clone，drop guard 后再 await）
    let Some(portal) = ({
        let guard = state.session.lock().await;
        guard.as_ref().map(|s| s.portal.clone())
    }) else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };

    // 门户三卡（邮箱/图书唯一来源；余额是降级备胎）——失败不阻塞实时路径
    let summary = portal.query_wallet_summary().await.ok();
    let portal_balance = summary.as_ref().and_then(|s| s.card_balance);

    // 慧新E校实时余额：任何失败（未登录/校外/桥失败）都静默落到门户
    let realtime = match synjones_session(&state).await {
        Some(guard) => match guard.as_ref() {
            Some(sess) => match fetch_current_card(&sess.client).await {
                Ok(card) => Some(card.elec_accamt_yuan),
                Err(_) => None,
            },
            None => None,
        },
        None => None,
    };

    let (value_yuan, source) = match (realtime, portal_balance) {
        (Some(v), _) => (Some(v), "realtime"),
        (None, Some(v)) => (Some(v), "portal"),
        (None, None) => (None, "none"),
    };

    Ok(CommandResult::ok(WalletCards {
        ecard: EcardValue {
            value_yuan,
            source: source.to_string(),
            updated_at: chrono::Local::now().to_rfc3339(),
        },
        mail: MailValue {
            unread: summary.as_ref().and_then(|s| s.mail_unread),
        },
        library: LibraryValue {
            borrowed: summary.as_ref().and_then(|s| s.book_borrowed),
        },
        elec: ElecValue {
            value_yuan: None,
            source: "none".to_string(),
        },
    }))
}
