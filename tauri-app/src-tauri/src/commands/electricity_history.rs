//! 电费历史命令面（M4 批 2）：账单 / 月度缴费 / 订单（含待支付）/ 自采余额历史 / 绑定宿舍 / 手动采集。
//!
//! 与 [`super::electricity`]（片区目录、级联、常用房间 CRUD、充值六条）分文件，避免一个模块撑爆；
//! 两者共用同一份「常用房间」存储与同一个进程级 synjones 客户端（**token 单活**，见
//! `commands::synjones` 头注——本模块绝不自建第二套客户端）。
//!
//! # 历史三源并存（任务书 §2 架构决策）
//!
//! | 源 | 端点 | 性质 |
//! |---|---|---|
//! | 缴费账单 | [`get_electricity_bills`] / [`get_electricity_monthly`] | 学校侧**权威**缴费记录（历史完整，但只有「交过多少钱」） |
//! | 订单 | [`get_electricity_orders`] | 含**待支付**（清理遗留单的入口；取消复用已提交的 `recharge_cancel`） |
//! | 日余额快照 | [`get_electricity_history`] / [`run_electricity_snapshot`] | **学校侧没有**，客户端自采（唯一能画出余额趋势的来源） |
//!
//! # 只读纪律（本模块只读学校侧）
//!
//! 本模块只调 `campus_synjones::turnover`（**纯 GET**）与级联查询（`POST /charge/feeitem/getThirdData`，
//! 官方语义是「查询」、零副作用）。**禁止**在此出现建单/支付/删单/退款/绑卡
//!（`/blade-pay/pay`、`/charge/order/deleteOrder`、`addRefundOrder`、`sceneBind/add`、`updateReceivable`）——
//! 取消遗留订单由前端复用 `commands::electricity::recharge_cancel`（不在本模块重新实现）。
//!
//! # 采集策略（写进文档，便于将来复核）
//!
//! - **只在应用运行时采**：启动补采（[`startup_snapshot`]，今天没采过 + 有内存会话时采一次）+ 用户手动采集。
//!   **不注册 Windows 计划任务**（用户未选该路线，且 token 单活会让后台采集顶掉正在用的会话）；
//!   应用没开的日子就是空档，曲线如实留空。
//! - **采集对象恒为「绑定的宿舍房间」**（[`electricity::SavedRoom::bound`]）：未绑定 → 可读中文原因。
//! - **余额提不到数字也照样落盘**（`balance: None` + 原文），UI 显示「无数据」——不臆造 0、不插值。
//! - **房间号无效/级联没到末级 → 不落盘**（返回可读原因）：那种情况下 `showData` 是空的或只有 `tipinfo`，
//!   存进去只会污染历史。
//!
//! # 敏感纪律
//!
//! 日志只出「采集成功/失败 + 余额数值 + 错误文案」，**不出** token / 账号 / 户号 / 房间路径；
//! 落盘内容只有 `map.showData` 的余额句与房间路径（crate 层已不透出 `map.data` 的户号等 PII）。
//! 单元测试全部离线（内联 JSON），不跑 `#[ignore]` live 测试。

use super::auth::CommandResult;
use super::electricity::{elec_err, load_rooms, set_bound, write_rooms, SavedRoom};
use super::synjones::synjones_session;
use crate::infra::electricity_history as store;
use crate::infra::electricity_history::HistoryEntry;
use crate::infra::state::{self, AppState};
use campus_synjones::charge::{self, ElectricityQuery, ElectricityView};
use campus_synjones::turnover::{self, BillPage, MonthTotal, Order};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

/// 无会话时的约定文案（与 portal.rs / profile.rs / synjones.rs / electricity.rs 同口径）。
const ERR_NO_SESSION: &str = "请先登录";

/// 未绑定宿舍时的可读原因（`snapshot_round` / 启动补采共用同一句）。
const ERR_NOT_BOUND: &str = "尚未绑定宿舍房间：请先在电费页选好房间并点「绑定为我的宿舍」";
/// 账单/订单默认每页条数（前端首屏；「加载更多」用返回的 `total` 判断）。
const DEFAULT_PAGE_SIZE: u32 = 20;

// ---------------- 学校侧只读取数 ----------------

/// 缴费账单列表（`/charge/turnover/app_account`，**金额单位元**）。
///
/// `feeitem_id` 为空 = 不按片区过滤（含一卡通充值等其它缴费项）；`page` 从 1 起。
#[tauri::command]
pub async fn get_electricity_bills(
    state: State<'_, AppState>,
    page: Option<u32>,
    size: Option<u32>,
    feeitem_id: Option<String>,
) -> Result<CommandResult<BillPage>, String> {
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let fid = feeitem_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
    Ok(
        match turnover::fetch_bills(
            &sess.client,
            fid,
            page.unwrap_or(1),
            size.unwrap_or(DEFAULT_PAGE_SIZE),
        )
        .await
        {
            Ok(p) => CommandResult::ok(p),
            Err(e) => CommandResult::err(&elec_err(&e)),
        },
    )
}

/// 某年 12 个月的缴费合计（逐月打 `pie_account`；`year` 缺省 = 当前年）。
///
/// ⚠️ 一次调用发 **12 个串行请求**（空月服务端回空数组 ⇒ 该月 0 元），期间持有 synjones 全局锁
/// （token 单活，不可并发）——前端要给它独立的三态与「加载中」提示，别和别的请求叠着发。
#[tauri::command]
pub async fn get_electricity_monthly(
    state: State<'_, AppState>,
    year: Option<i32>,
) -> Result<CommandResult<Vec<MonthTotal>>, String> {
    let year = year.unwrap_or_else(|| chrono::Datelike::year(&chrono::Local::now()));
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(match turnover::fetch_monthly(&sess.client, year).await {
        Ok(v) => CommandResult::ok(v),
        Err(e) => CommandResult::err(&elec_err(&e)),
    })
}

/// 订单列表（`/charge/order/personal_data`，**含待支付**；`status`：0 待支付 / 1 已完成 / None 全部）。
///
/// 待支付单的 `orderId` 可直接交给已提交的 `recharge_status` / `recharge_cancel` 处理
/// （任务书 §1.6：旧结论「该端点恒 500」已被实测推翻，缺的是 App 口径头组——crate 已统一注入）。
#[tauri::command]
pub async fn get_electricity_orders(
    state: State<'_, AppState>,
    status: Option<i64>,
) -> Result<CommandResult<Vec<Order>>, String> {
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(match turnover::fetch_orders(&sess.client, status).await {
        Ok(v) => CommandResult::ok(v),
        Err(e) => CommandResult::err(&elec_err(&e)),
    })
}

// ---------------- 自采余额历史（本地） ----------------

/// 本地余额快照，**时间升序**（图表直接消费）。
///
/// `room_id` = 常用房间的 `SavedRoom.id`（None = 全部房间）；`days` = 最近 N 天含今天
/// （None = 不限）。用 `room_id` 而不是内部 `roomKey`：前端只认识常用房间的 id，
/// `roomKey` 是后端为跨端合并造的稳定键，不下发给 UI。
#[tauri::command]
pub async fn get_electricity_history(
    room_id: Option<String>,
    days: Option<u32>,
) -> Result<CommandResult<Vec<HistoryEntry>>, String> {
    let dir = state::data_dir()?;
    let entries = store::load_history(&dir);
    let room_key = match room_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => None,
        Some(id) => match load_rooms(&dir).into_iter().find(|r| r.id == id) {
            Some(r) => Some(store::room_key_of(&r.feeitem_id, &r.path)),
            None => {
                return Ok(CommandResult::err(
                    "该房间不在常用列表里（可能已被删除），请刷新后重试",
                ))
            }
        },
    };
    let today = chrono::Local::now().date_naive();
    Ok(CommandResult::ok(store::filter(
        &entries,
        room_key.as_deref(),
        days,
        today,
    )))
}

/// 绑定/解绑宿舍房间（**同一时刻最多一个绑定**，绑新的自动解绑旧的），返回更新后的完整列表。
///
/// 绑定关系只由本命令改：`save_electricity_room` 保存房间时**保留**原有绑定状态，
/// 不会因为前端没带 `bound` 字段而静默丢绑定（见 `electricity::upsert_room`）。
#[tauri::command]
pub async fn bind_electricity_room(
    id: String,
    bound: bool,
) -> Result<CommandResult<Vec<SavedRoom>>, String> {
    let dir = state::data_dir()?;
    match set_bound(load_rooms(&dir), &id, bound) {
        Ok(rooms) => match write_rooms(&dir, &rooms) {
            Ok(()) => Ok(CommandResult::ok(rooms)),
            Err(e) => Ok(CommandResult::err(&e)),
        },
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

/// `run_electricity_snapshot` → data。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotOutcome {
    /// 本次写入的那条快照（同日已有则被覆盖）。
    pub entry: HistoryEntry,
    /// 是否覆盖了当天已有的那条（前端可提示「已更新今日记录」而不是「新增」）。
    pub replaced: bool,
}

/// 从级联结果构造一条快照（**纯函数**：时间由调用方给 ⇒ 单测能钉住 `id`/`date` 的推导）。
///
/// 余额走 `charge::balance_from_fields`（关键词+分隔符+数字的**相邻形态**，提不到返回 `None`），
/// `raw` 存学校侧那句原文留证据。**绝不做位置解构**（三片区文案格式互不相同，解构会随文案漂移静默失效）。
fn build_entry(
    room: &SavedRoom,
    view: &ElectricityView,
    source: &str,
    now: chrono::DateTime<chrono::Local>,
) -> HistoryEntry {
    let (balance, raw) = charge::balance_from_fields(&view.fields);
    let date = now.format("%Y-%m-%d").to_string();
    let room_key = store::room_key_of(&room.feeitem_id, &room.path);
    HistoryEntry {
        id: store::make_id(&room_key, &date),
        device: store::device_name(),
        room_key,
        room_name: room.label.clone(),
        feeitem_id: room.feeitem_id.clone(),
        feeitem_name: room.feeitem_name.clone(),
        collected_at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        date,
        balance,
        raw,
        source: source.to_string(),
    }
}

/// 绑定的宿舍房间（最多一个；`None` = 未绑定）。
fn bound_room(rooms: &[SavedRoom]) -> Option<&SavedRoom> {
    rooms.iter().find(|r| r.bound)
}

/// 末级视图的可用性判定：`tipinfo` 有值 = 房间号无效（`showData` 为空），不采集。
fn usable_view(q: &ElectricityQuery) -> Result<&ElectricityView, String> {
    let view = q
        .view
        .as_ref()
        .ok_or_else(|| "房间信息不完整，请重新选择房间后再采集".to_string())?;
    match view.tip.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(tip) => Err(format!("学校未返回该房间的电费信息：{tip}")),
        None => Ok(view),
    }
}

/// 采集一轮并落盘（命令与启动补采共用）：读绑定房间 → 级联查询 → 提取余额 → 写历史。
///
/// `Err(中文原因)` 表示**未采集**（未绑定 / 未登录 / 查询失败 / 房间号无效），
/// 由调用方决定是回给用户（手动采集）还是只记日志（启动补采）。
pub(crate) async fn snapshot_round(
    state: &State<'_, AppState>,
    source: &str,
) -> Result<SnapshotOutcome, String> {
    let dir = state::data_dir()?;
    let room = bound_room(&load_rooms(&dir)).cloned().ok_or(ERR_NOT_BOUND)?;

    // 网络取数：锁（token 单活）只覆盖请求本身，落盘在锁外
    let queried = {
        let Some(guard) = synjones_session(state).await else {
            return Err(ERR_NO_SESSION.to_string());
        };
        let Some(sess) = guard.as_ref() else {
            return Err(ERR_NO_SESSION.to_string());
        };
        charge::query_cascade(&sess.client, &room.feeitem_id, &room.path).await
    };
    let q = queried.map_err(|e| elec_err(&e))?;
    let view = usable_view(&q)?.clone();

    let entry = build_entry(&room, &view, source, chrono::Local::now());
    let history = store::load_history(&dir);
    let replaced = store::has_entry_today(&history, &entry.room_key, &entry.date);
    store::save_history(&dir, &store::upsert(history, entry.clone()))?;
    Ok(SnapshotOutcome { entry, replaced })
}

/// **手动采集一次**（对绑定的宿舍房间跑一次级联查询并落盘）。
///
/// 未登录 / 未绑定 一律返回可读中文原因（不 panic）——前端据此给「去绑定」的引导。
#[tauri::command]
pub async fn run_electricity_snapshot(
    state: State<'_, AppState>,
) -> Result<CommandResult<SnapshotOutcome>, String> {
    Ok(match snapshot_round(&state, store::SOURCE_MANUAL).await {
        Ok(o) => CommandResult::ok(o),
        Err(msg) => CommandResult::err(&msg),
    })
}

/// **启动补采**（任务书 §2：不弹窗、不阻塞启动、失败只记日志）。
///
/// 三道静默护栏（任一不满足即安静返回）：今日已采过 / 未绑定宿舍 / 当前无内存会话
///（无会话时不进 SSO——不能因为一次后台补采就去签发 token 顶掉用户正在用的会话，
///  这是「不注册计划任务」同一条理由：token 单活）。
pub async fn startup_snapshot(app: AppHandle) {
    let state = app.state::<AppState>();
    let Ok(dir) = state::data_dir() else {
        return;
    };
    let Some(room) = bound_room(&load_rooms(&dir)).cloned() else {
        log::debug!("[elec-history] 未绑定宿舍房间，跳过启动补采");
        return;
    };
    let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
    let room_key = store::room_key_of(&room.feeitem_id, &room.path);
    if store::has_entry_today(&store::load_history(&dir), &room_key, &today) {
        log::debug!("[elec-history] 今日已有采集记录，跳过启动补采");
        return;
    }
    if state.session.lock().await.is_none() {
        log::debug!("[elec-history] 当前无内存会话，跳过启动补采");
        return;
    }
    match snapshot_round(&state, store::SOURCE_AUTO).await {
        Ok(o) => log::info!(
            "[elec-history] 启动补采完成（balance={:?}，覆盖今日旧记录={}）",
            o.entry.balance,
            o.replaced
        ),
        Err(msg) => log::warn!("[elec-history] 启动补采未完成：{msg}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use campus_synjones::charge::{Field, RoomStep};

    fn room() -> SavedRoom {
        SavedRoom {
            id: "r1".to_string(),
            feeitem_id: "450".to_string(),
            feeitem_name: "梅园1号-梅园3号".to_string(),
            path: vec![
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
            ],
            label: "1号楼 101".to_string(),
            bound: true,
        }
    }

    fn at(rfc3339: &str) -> chrono::DateTime<chrono::Local> {
        chrono::DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&chrono::Local)
    }

    /// `build_entry` 用**实测原文**（450 片区那句自由文本）构造快照：余额、原文、id、date 全钉住。
    #[test]
    fn build_entry_extracts_balance_from_live_text() {
        // 实测样本：KEY 是「信息」，值含负余额与单价（另一个数字不许被误取）
        let view = ElectricityView {
            fields: vec![Field {
                label: "信息".to_string(),
                value: "房间号：101,剩余金额：-545.70，单价：0.5400".to_string(),
            }],
            money: None,
            tip: None,
        };
        let e = build_entry(&room(), &view, store::SOURCE_MANUAL, at("2026-09-19T17:20:31+08:00"));
        assert_eq!(e.balance, Some(-545.70));
        assert_eq!(e.raw, "房间号：101,剩余金额：-545.70，单价：0.5400");
        assert_eq!(e.date, "2026-09-19");
        assert_eq!(e.collected_at, "2026-09-19T17:20:31+08:00");
        assert_eq!(e.id, store::make_id(&store::room_key_of("450", &room().path), "2026-09-19"));
        assert_eq!(e.room_name, "1号楼 101");
        assert_eq!(e.feeitem_id, "450");
        assert_eq!(e.source, "manual");
    }

    /// 提不到余额（只有电量/单价）→ `balance: None` **但记录仍然成立**（原文留证据，UI 显示无数据）。
    #[test]
    fn build_entry_keeps_none_balance_with_raw_text() {
        let view = ElectricityView {
            fields: vec![Field {
                label: "信息".to_string(),
                value: "当前剩余电量957.50度".to_string(),
            }],
            money: None,
            tip: None,
        };
        let e = build_entry(&room(), &view, store::SOURCE_AUTO, at("2026-09-19T08:00:00+08:00"));
        assert_eq!(e.balance, None, "电量是 kWh，绝不当余额");
        assert_eq!(e.raw, "当前剩余电量957.50度");
        assert_eq!(e.id, store::make_id(&e.room_key, &e.date), "id 与余额无关");
    }

    /// 末级视图可用性：无 view / 有 `tipinfo`（房间号无效）→ 拒绝采集并给可读中文原因；空 tip 视为可用。
    #[test]
    fn usable_view_rejects_tip_and_missing_view() {
        let no_view = ElectricityQuery {
            levels: Vec::new(),
            options: Vec::new(),
            is_final: false,
            view: None,
        };
        let e = usable_view(&no_view).unwrap_err();
        assert!(e.contains("重新选择房间"), "实际 {e}");

        let tip = ElectricityQuery {
            levels: Vec::new(),
            options: Vec::new(),
            is_final: true,
            view: Some(ElectricityView {
                fields: Vec::new(),
                money: None,
                tip: Some("缴费系统返回数据错误child==NULL！".to_string()),
            }),
        };
        let e = usable_view(&tip).unwrap_err();
        assert!(e.contains("child==NULL"), "服务端原文要透出便于排查：{e}");

        let blank_tip = ElectricityQuery {
            levels: Vec::new(),
            options: Vec::new(),
            is_final: true,
            view: Some(ElectricityView {
                fields: vec![Field {
                    label: "信息".to_string(),
                    value: "当前余额517.05元,当前剩余电量957.50度".to_string(),
                }],
                money: None,
                tip: Some("   ".to_string()),
            }),
        };
        assert!(usable_view(&blank_tip).is_ok(), "空 tip 是常态，不是错误");
    }

    /// 绑定房间的查找：只认 `bound == true`，多个（理论上不该有）取第一个，无绑定 → None。
    #[test]
    fn bound_room_picks_the_flagged_one() {
        let mut a = room();
        a.bound = false;
        let mut b = room();
        b.id = "r2".to_string();
        assert!(bound_room(&[a.clone()]).is_none());
        assert_eq!(bound_room(&[a, b.clone()]).map(|r| r.id.as_str()), Some("r2"));
        assert!(bound_room(&[]).is_none());
    }
}
