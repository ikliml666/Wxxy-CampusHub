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
//! # 校外 WebVPN 路由（M4）
//!
//! 取会话前先判网络归属（`infra::net_zone::detect`，spawn_blocking + 60s 缓存）：
//! 校内/未知 → 直连现状；校外 → 用 CAS TGT 静默登录深澜网关
//! （`WebVpnSession::login`，无验证码无交互），把网关包装 base 与会话注入
//! `SynjonesClient::set_webvpn`——此后该 client 的业务请求与 SSO 桥自动走网关。
//! 进程级 `static WEBVPN_SESSION` 缓存网关会话（TTL 30 分钟，过期重登；TGT 过期
//! 时给出 [`ERR_OFFCAMPUS_RELOGIN`] 可操作文案）；login 失败另有 60 秒负缓存
//! （[`VPN_LOGIN_BACKOFF`]），网关不可达时高频命令不再反复撞整条 SSO 链。锁纪律：
//! `WEBVPN_SESSION` 用 tokio
//! Mutex 且 guard 允许跨 await（login 是网络 IO，持锁排队正是我们要的串行化），
//! `SYNJONES` 锁内只做同步赋值；`net_zone::detect`（netsh 同步阻塞）绝不持任何锁执行。
//!
//! # 失败文案
//!
//! 业务失败一律 `Ok(CommandResult::err(中文))`（`Err(String)` 仅限 IPC 框架层）；
//! `NotLogin` 归一为「登录已过期，请重新登录」（与批 0 的门户口径一致）。
//! 敏感纪律：本模块不输出任何 token / TGT / 卡号到日志或错误消息
//! （`EcardOverview.account` 是给前端查流水分页用的卡号，只在 IPC 数据里，不入日志）。

use super::auth::CommandResult;
use crate::infra::net_zone;
use crate::infra::state::AppState;
use campus_auth::cas::CasClient;
use campus_synjones::ecard::{
    fetch_cards, fetch_current_card, fetch_transactions, CardInfo, Transactions, TurnoverFilter,
};
use campus_synjones::routing::{route, NetZone as RouteZone, RouteDecision, WebVpnSession};
use campus_synjones::{CampusSynjonesError, SynjonesClient, BERSERKER_BASE};
use campus_synjones::routing::GATEWAY;
use serde::Serialize;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::State;

/// 无会话时的约定文案（与 portal.rs / profile.rs / timetable.rs 同口径）。
pub(crate) const ERR_NO_SESSION: &str = "请先登录";

/// 校外且 WebVPN 会话建立失败（TGT 过期 / 网关不可达）的可操作文案。
pub(crate) const ERR_OFFCAMPUS_RELOGIN: &str = "校外模式：登录态已过期，请重新登录后再试";

/// 流水每页条数（前端「加载更多」按 `total` 判断是否还有下一页）。
const PAGE_SIZE: u32 = 15;

/// WebVPN 网关会话 TTL：过期重登（静默换票 + 302 链，无用户交互，成本一次请求链）。
const VPN_SESSION_TTL: Duration = Duration::from_secs(30 * 60);

/// 网关 login **负缓存**窗口：刚失败过（60 秒内）不再重试整条 SSO 链，直接回
/// 可操作文案（网关不可达 / TGT 过期时，高频命令轮询不必每次都撞一遍网络）。
/// 换 TGT 重新登录后最多再等满 60 秒即恢复。仅内存态，不落盘、不进日志。
const VPN_LOGIN_BACKOFF: Duration = Duration::from_secs(60);

/// 上次网关 login 失败时刻（负缓存）。std Mutex 瞬时读写、绝不跨 await
/// （login 的串行化由 `WEBVPN_SESSION` 的 tokio guard 负责，这里只存时间戳）。
static WEBVPN_LOGIN_FAIL: OnceLock<std::sync::Mutex<Option<Instant>>> = OnceLock::new();

/// 读负缓存：60 秒内失败过 → Some（调用方直接回文案不重试）。
fn vpn_login_in_backoff() -> bool {
    WEBVPN_LOGIN_FAIL
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| *g)
        .is_some_and(|t| t.elapsed() < VPN_LOGIN_BACKOFF)
}

/// 记一次 login 失败时刻（锁中毒按无缓存处理，不影响主流程）。
fn note_vpn_login_fail() {
    if let Ok(mut g) = WEBVPN_LOGIN_FAIL
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
    {
        *g = Some(Instant::now());
    }
}

// ---------------- 全局唯一客户端 ----------------

/// 缓存的慧新E校会话（`username` + `tgt` 任一变化即重建，避免持过期 TGT 的旧客户端）。
/// 对外 `pub(crate)`：电费命令面（`commands::electricity`）与批 2 共用同一个会话/同一把锁。
pub(crate) struct SynjonesSession {
    pub(crate) username: String,
    pub(crate) tgt: Option<String>,
    pub(crate) client: SynjonesClient,
}

static SYNJONES: OnceLock<tokio::sync::Mutex<Option<SynjonesSession>>> = OnceLock::new();

/// 进程级 WebVPN 网关会话（M4）：`(会话, 建立时刻)`，TTL 过期静默重登。
/// 单例语义与 SYNJONES 一致——绝不并发 login（网关侧一次性 token 换发）。
static WEBVPN_SESSION: OnceLock<tokio::sync::Mutex<Option<(WebVpnSession, Instant)>>> =
    OnceLock::new();

/// 网络归属 → 路由决策枚举（两枚举同构，编译期钉住映射完整性）。
fn to_route_zone(z: net_zone::NetZone) -> RouteZone {
    match z {
        net_zone::NetZone::Campus => RouteZone::Campus,
        net_zone::NetZone::OffCampus => RouteZone::OffCampus,
        net_zone::NetZone::Unknown => RouteZone::Unknown,
    }
}

/// 判当前网络归属（spawn_blocking：`detect` 内含 netsh 同步子进程调用，不进 async 线程）。
pub(crate) async fn detect_zone() -> RouteZone {
    let z = tokio::task::spawn_blocking(net_zone::detect)
        .await
        .unwrap_or(net_zone::NetZone::Unknown);
    to_route_zone(z)
}

/// 校外路径下取（必要时新建）WebVPN 网关会话；返回 clone 与原实例共享网关 cookie jar，
/// 锁外使用安全。
///
/// 无 TGT 或 login 失败（TGT 过期 / 网络）→ [`ERR_OFFCAMPUS_RELOGIN`]。持
/// `WEBVPN_SESSION` 的 tokio guard 跨 await 正是设计：并发命中过期时排队，只 login 一次。
/// login 失败有 60 秒负缓存（[`VPN_LOGIN_BACKOFF`]）：窗口内直接回文案不再撞网络。
pub(crate) async fn ensure_webvpn_session(
    cas: &CasClient,
    tgt: Option<&str>,
) -> Result<WebVpnSession, String> {
    let Some(tgt) = tgt.filter(|t| !t.trim().is_empty()) else {
        return Err(ERR_OFFCAMPUS_RELOGIN.to_string());
    };
    let mutex = WEBVPN_SESSION.get_or_init(|| tokio::sync::Mutex::new(None));
    let mut guard = mutex.lock().await;
    let fresh = matches!(guard.as_ref(), Some((_, ts)) if ts.elapsed() < VPN_SESSION_TTL);
    if !fresh {
        if vpn_login_in_backoff() {
            return Err(ERR_OFFCAMPUS_RELOGIN.to_string());
        }
        match WebVpnSession::login(cas, tgt).await {
            Ok(session) => *guard = Some((session, Instant::now())),
            Err(_) => {
                note_vpn_login_fail();
                return Err(ERR_OFFCAMPUS_RELOGIN.to_string());
            }
        }
    }
    Ok(guard.as_ref().expect("fresh 分支必有会话").0.clone())
}

/// 「业务 base 的网关包装形态」（去尾斜杠，供 `format!("{base}{path}")` 直接拼接）。
/// 纯函数单测钉住 golden 形态。可见性 `pub(crate)`：电费命令面的匿名目录路由复用。
pub(crate) fn wrapped_base() -> String {
    campus_synjones::routing::wrap_url(BERSERKER_BASE, GATEWAY)
        .expect("BERSERKER_BASE 恒为合法 http URL")
        .trim_end_matches('/')
        .to_string()
}

/// 取全局唯一客户端（**带校外路由决策**，M4 主路径）；返回的 guard **必须活到请求结束**
/// （持锁即串行化，见模块头注）。未登录 → `Ok(None)`（调用方回「请先登录」）；
/// 校外且网关会话建立失败 → `Err([ERR_OFFCAMPUS_RELOGIN])`。
///
/// 可见性 `pub(crate)`：电费/一卡通历史等命令面复用同一把锁与同一个客户端
///（token 单活，绝不能出现第二套 SSO 缓存），不复制这段逻辑。
pub(crate) async fn synjones_session_routed(
    state: &State<'_, AppState>,
) -> Result<Option<tokio::sync::MutexGuard<'static, Option<SynjonesSession>>>, String> {
    // 锁内只 clone（cas/username/tgt 均廉价），drop guard 后再 await（本项目锁纪律）
    let (username, tgt, cas) = {
        let guard = state.session.lock().await;
        let Some(s) = guard.as_ref() else {
            return Ok(None);
        };
        (s.username.clone(), s.tgt.clone(), s.client.clone())
    };
    // 路由决策（detect 带 60s TTL 缓存；决策目标恒为内网 base）：
    // 校外 + 有 TGT → Wrapped（建/复用网关会话并注入）；校外 + 无 TGT → NeedLogin；
    // Campus/Unknown → Direct（Unknown 直连失败由上层中文报错兜底，不做自动二次尝试）
    let (base_override, vpn) = match route(BERSERKER_BASE, detect_zone().await, tgt.is_some()) {
        RouteDecision::Wrapped(_) => {
            let v = ensure_webvpn_session(&cas, tgt.as_deref()).await?;
            (Some(wrapped_base()), Some(v))
        }
        RouteDecision::NeedLogin => return Err(ERR_OFFCAMPUS_RELOGIN.to_string()),
        // Direct = 校内/未知直连现状；Unreachable 本轮 route 不产生（兜底直连）
        _ => (None, None),
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
        let mut client = SynjonesClient::new(cas, tgt.clone(), None);
        client.set_webvpn(base_override, vpn);
        *guard = Some(SynjonesSession {
            username,
            tgt,
            client,
        });
    } else if let Some(sess) = guard.as_mut() {
        // 非重建路径同样刷新路由态（zone 在 client 生命周期内可能变化：校外 ↔ 校内）
        sess.client.set_webvpn(base_override, vpn);
    }
    Ok(Some(guard))
}

/// 旧入口（兼容既有调用方 `ecard.rs` / `electricity_history.rs` 的签名）：路由照走，
/// 校外网关会话失败降级为 None（调用方回「请先登录」——文案欠精确但不阻塞界外模块）。
pub(crate) async fn synjones_session(
    state: &State<'_, AppState>,
) -> Option<tokio::sync::MutexGuard<'static, Option<SynjonesSession>>> {
    match synjones_session_routed(state).await {
        Ok(g) => g,
        Err(e) => {
            log::warn!("WebVPN 会话建立失败，按无会话处理：{e}");
            None
        }
    }
}

/// 会话失效归一为可操作文案，其余透出 crate 的中文错误（不回显票据/凭据）。
/// 可见性 `pub(crate)`：电费命令面在此之上追加「需校园网」等自有文案（`electricity::elec_err`）。
pub(crate) fn err_text(e: &CampusSynjonesError) -> String {
    match e {
        CampusSynjonesError::NotLogin => "登录已过期，请重新登录".to_string(),
        // 双保险：crate 内已对 URL 凭据打码，这里再兜一层（任何变体带出的凭据都不出后端）
        other => campus_synjones::redact_secrets(&other.to_string()),
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
    let guard = match synjones_session_routed(&state).await {
        Ok(Some(g)) => g,
        Ok(None) => return Ok(CommandResult::err(ERR_NO_SESSION)),
        Err(e) => return Ok(CommandResult::err(&e)),
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

/// 流水一页（一卡通面板批 1 扩展了筛选参数，契约 §2.2）：
/// `account` 可选（缺省查本人全部流水）；`type`（`r#type`）：`"1"` 收入 / `"2"` 支出 /
/// 缺省全量；`type_id` 分类、`info` 关键词、`order_id` 单条详情。`page` 从 1 起，`size` 缺省 15。
///
/// 未传的可选筛选参数不会出现在服务端 query 里（crate 层保证，实测语义：不传 = 不过滤）。
#[tauri::command]
pub async fn get_ecard_transactions(
    state: State<'_, AppState>,
    account: Option<String>,
    page: u32,
    size: Option<u32>,
    r#type: Option<String>,
    type_id: Option<String>,
    info: Option<String>,
    order_id: Option<String>,
) -> Result<CommandResult<Transactions>, String> {
    let guard = match synjones_session_routed(&state).await {
        Ok(Some(g)) => g,
        Ok(None) => return Ok(CommandResult::err(ERR_NO_SESSION)),
        Err(e) => return Ok(CommandResult::err(&e)),
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let client = &sess.client;

    let filter = TurnoverFilter {
        account: account.as_deref().unwrap_or(""),
        direction: r#type.as_deref(),
        type_id: type_id.as_deref(),
        info: info.as_deref(),
        order_id: order_id.as_deref(),
        sort_fields: None,
        sort_type: None,
    };
    Ok(
        match fetch_transactions(client, &filter, page, size.unwrap_or(PAGE_SIZE)).await {
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

    // 慧新E校实时余额：任何失败（未登录/校外网关会话失败/桥失败）都静默落到门户
    let realtime = match synjones_session_routed(&state).await {
        Ok(Some(guard)) => match guard.as_ref() {
            Some(sess) => fetch_current_card(&sess.client).await.ok().map(|c| c.elec_accamt_yuan),
            None => None,
        },
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use campus_synjones::routing::wrap_url;

    /// 同构映射钉死：三个变体逐一对应（任一侧改名/加变体即编译失败或此处失败）。
    #[test]
    fn route_zone_mapping_is_complete() {
        assert_eq!(to_route_zone(net_zone::NetZone::Campus), RouteZone::Campus);
        assert_eq!(to_route_zone(net_zone::NetZone::OffCampus), RouteZone::OffCampus);
        assert_eq!(to_route_zone(net_zone::NetZone::Unknown), RouteZone::Unknown);
    }

    /// WebVPN 业务 base 的 golden 形态：网关 + 内网 host 密文、无尾斜杠（供直接拼 path）。
    #[test]
    fn wrapped_base_is_gateway_prefix_without_trailing_slash() {
        let b = wrapped_base();
        assert_eq!(b, format!("{GATEWAY}/http/{}", hex_encode_103()));
        assert!(!b.ends_with('/'), "尾斜杠必须去掉，否则拼 path 出现双斜杠");
        // 与 wrap_url 对带 path 的目标同源：base + path 恰好等于整条包装 URL
        let path = "/berserker-app/ykt/tsm/queryCurrentCard";
        assert_eq!(
            format!("{b}{path}"),
            wrap_url(&format!("{BERSERKER_BASE}{path}"), GATEWAY).unwrap(),
            "base+path 必须与 wrap_url 整条包装一致（无双斜杠/丢段）"
        );
    }

    fn hex_encode_103() -> String {
        // encrypt_host 的 golden 值（与 wrap.rs 测试同源）：10.3.100.110 的密文段
        "77726476706e69737468656265737421a1a70fcf696138003059d8fc".to_string()
    }

    /// 校外登录失败文案钉住（前端按文案引导重新登录，改动即破坏契约）。
    #[test]
    fn offcampus_relogin_message_is_stable() {
        assert_eq!(
            ERR_OFFCAMPUS_RELOGIN,
            "校外模式：登录态已过期，请重新登录后再试"
        );
    }
}
