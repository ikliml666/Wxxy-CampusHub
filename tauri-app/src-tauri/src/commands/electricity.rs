//! 电费命令面（M3 批 3 + M3.1 批 C）：片区目录 / 级联查询 / 常用房间（本地存）/ 去官网充值（系统浏览器）
//! / **充值六条**（建单 → 支付方式 → 账户与安全键盘 → 提交 → 状态 → 取消）。
//!
//! # 取数口径
//!
//! 目录与级联全部来自 `campus_synjones::charge`（协议事实见该模块头注，含两条 live 实测关键结论：
//! 「末级是输入级，服务端不下发下拉」「`showData` 键名恒为 `信息`，值是各片区格式不同的自由文本」）；
//! 充值协议全部来自 `campus_synjones::recharge`（模块头注含 live 实测的 paystep/错误文案/取消订单等事实）。
//!
//! # 充值红线（M3.1 计划 §2 的 Global Constraints，逐条落在这里）
//!
//! 1. **密码**：`recharge_query_account` 把服务端下发的安全键盘（`pad.keys`，**只用于渲染**）交给前端；
//!    提交走 `recharge_submit` 的 `password_seq`（**用户点击的键位下标序列**）。本模块**绝不**把 `keys`
//!    还原成密码、**绝不**落盘/打日志/回填输入框，两个字段都只在一次调用内存在（不写任何 storage）。
//! 2. **金额语义不碰**：`tranamt` 原样透传（crate 只校验「非空且为正数」），下限/上限只由前端提示。
//! 3. **副作用不重试**：`recharge_create` / `recharge_submit` / `recharge_cancel` 各只发一次
//!    （crate 层仅在服务端明确拒绝 401 时静默重进一次），结果一律以 `recharge_status` 兜底判定。
//! 4. **轮询上限**在前端（2s × 15 次）；本模块只提供单次查询。
//! 5. **跳转分支不实现**：响应命中 `webUrl`/`paysubmit`/`paymentcashierStr`/`qrCodeUrl` 时 crate 直接报错，
//!    本模块只把文案转给用户。
//! 6. **清理**：失败/放弃时前端调 `recharge_cancel`（crate 实测该端点**必须 JSON body**）。
//!    ⚠️ 实测学校侧没有可用的「遗留未支付订单列表」接口（`/charge/order/personal_data?status=0` 恒
//!    `code=500`，见 `tests/recharge_live.rs::recharge_diag_live`），故无法在进入流程前预检遗留单。
//! 7. **`third_party` 由后端合成**：`recharge_create` 收「房间路径」而不是上下文串——crate 内部按路径
//!    重放一次 `getThirdData` 取末级 `map.data`（含户号 PII）拼串后随建单发出，PII 全程不出后端，
//!    前端也无从伪造房间上下文（见 `recharge::third_party_for_room`）。
//!
//! # token 单活 → 沿用批 2 的全局唯一客户端
//!
//! 电费接口除目录外都需 token，且慧新E校 token 是**单活**的（并发 SSO 会互相顶掉）。
//! 故本模块**复用** `commands::synjones` 的进程级 `static SYNJONES` 与 `synjones_session`
//! （同锁、同客户端、同 `reenter`），**绝不另起第二套客户端**。匿名目录接口只需一个 reqwest
//! client（未登录时现建一个 `CasClient`），不触碰 token 缓存。
//!
//! # 常用房间：本地存（计划 §2.3）
//!
//! 平台侧 `sceneBind/add` 会改学校数据，不采用；房间只存本机
//! `%APPDATA%/campushub/electricity_rooms.json`。落盘沿用 `commands/profile.rs` 的同款极小
//! helper（`create_dir_all` + `to_string_pretty` + `fs::write`）——`infra/` 里没有通用 JSON
//! helper（`infra/timetable.rs` 是课表专用、`infra/state.rs` 是会话专用），故按既有范式在本模块
//! 内落地，不新造抽象。
//!
//! # 敏感纪律
//!
//! token **只在 Rust 内存**使用（不经前端 JS API、不进日志、不进错误文案）；浏览器兜底 URL 只含
//! 片区 id、不带 token；`showData` 之外的 `map.data` 含户号（PII），crate 层已不透出。

use super::auth::{session_client, CommandResult};
use super::synjones::{err_text, synjones_session};
use crate::infra::state::{self, AppState};
use campus_auth::cas::CasClient;
use campus_synjones::charge::{self, ElectricityQuery, FeeItem, RoomStep};
use campus_synjones::recharge::{self, PayMethod, PasswordPad, RechargeOrder};
use campus_synjones::{CampusSynjonesError, BERSERKER_BASE};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

/// 无会话时的约定文案（与 portal.rs / profile.rs / synjones.rs 同口径）。
const ERR_NO_SESSION: &str = "请先登录";
/// 常用房间数量上限（防无界增长；超出时给可操作文案）。
const MAX_SAVED_ROOMS: usize = 20;
/// 官方充值页路径前缀（官方 SPA；`/charge-pc/pays/<feeitemid>` 是批 1 实测的合法落点）。
const RECHARGE_PATH_PREFIX: &str = "/charge-pc/pays/";

// ---------------- 常用房间（本地存） ----------------

/// 一个常用房间（计划 §2.3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedRoom {
    /// 本机 id（前端留空时由后端生成 epoch 毫秒字符串）。
    pub id: String,
    /// 所属片区（`FeeItem::id`）。
    pub feeitem_id: String,
    /// 片区名（冗余存下来，列表展示不必再拉目录）。
    #[serde(default)]
    pub feeitem_name: String,
    /// 完整选择路径（校区 → 楼栋 → 房间）；点击即按它重放级联。
    pub path: Vec<RoomStep>,
    /// 用户起的名字（留空时后端用路径名拼一个）。
    #[serde(default)]
    pub label: String,
    /// 是否「我的宿舍」（M4 批 2：每天定时采集的采集对象）。
    ///
    /// **同一时刻最多一个为 true**（[`set_bound`] 保证）。`serde(default)` 让旧版
    /// `electricity_rooms.json`（无该字段）照旧读得进来（一律视为未绑定）。
    #[serde(default)]
    pub bound: bool,
}

fn rooms_path(dir: &Path) -> PathBuf {
    dir.join("electricity_rooms.json")
}

/// 生成本机房间 id（epoch 毫秒）：与已存在的 id 撞号就顺延。
///
/// `id` 是绑定（[`set_bound`]）与删除（[`remove_room`]）的定位键 ⇒ **必须唯一**：
/// 同一毫秒内连续新增两个房间时，裸毫秒会给出同一个 id，导致「解绑/删除隔壁」误伤另一个房间
/// （M4 批 2 加绑定字段时发现，单测覆盖）。
fn next_room_id(rooms: &[SavedRoom]) -> String {
    let start = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut n = start;
    while rooms.iter().any(|r| r.id == n.to_string()) {
        n += 1;
    }
    n.to_string()
}

/// 读取本地常用房间。文件缺失/损坏/反序列化失败 → 空列表（不报错、不删坏文件，
/// 与 `infra/timetable.rs` 的宽容读取同款——电费页首屏不能因存储异常白屏）。
pub fn load_rooms(dir: &Path) -> Vec<SavedRoom> {
    let Ok(raw) = fs::read_to_string(rooms_path(dir)) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<SavedRoom>>(&raw).unwrap_or_else(|e| {
        log::warn!("electricity_rooms.json 损坏，回退空列表: {e}");
        Vec::new()
    })
}

/// 整体写入常用房间（电费命令面与 `electricity_history` 的绑定命令共用）。
pub(crate) fn write_rooms(dir: &Path, rooms: &[SavedRoom]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let json = serde_json::to_string_pretty(rooms).map_err(|e| e.to_string())?;
    fs::write(rooms_path(dir), json).map_err(|e| format!("写 electricity_rooms.json 失败: {e}"))
}

/// 同一房间的判定键：片区 + 完整路径（代码与值）。
fn room_key(r: &SavedRoom) -> (String, Vec<(String, String)>) {
    (
        r.feeitem_id.clone(),
        r.path
            .iter()
            .map(|s| (s.code.clone(), s.value.clone()))
            .collect(),
    )
}

/// 房间标题：用户没起名时用路径名拼（取末两级，尽量有信息量）。
fn default_label(r: &SavedRoom) -> String {
    let names: Vec<&str> = r
        .path
        .iter()
        .map(|s| s.name.trim())
        .filter(|n| !n.is_empty())
        .collect();
    let tail = &names[names.len().saturating_sub(2)..];
    let joined = tail.join(" ");
    if joined.is_empty() {
        "未命名房间".to_string()
    } else {
        joined
    }
}

/// 新增/更新一个常用房间（纯逻辑，单测覆盖）：校验 → 同片区同路径视为同一房间（沿用原 id）
/// → 追加或替换 → 上限校验。返回新列表（不改动入参）。
///
/// **绑定状态不由本函数决定**：已有房间的 `bound` 一律沿用存量（改名/换房间号不得静默丢绑定，
/// 前端保存时不带 `bound` 字段也无妨）；新房间自带 `bound: true` 时按唯一性清掉其它房间的绑定
/// ——绑定关系只经 [`set_bound`]（命令 `bind_electricity_room`）显式变更。
pub fn upsert_room(mut rooms: Vec<SavedRoom>, mut room: SavedRoom) -> Result<Vec<SavedRoom>, String> {
    if room.feeitem_id.trim().is_empty() {
        return Err("缺少片区 id".to_string());
    }
    if room.path.is_empty() {
        return Err("房间路径为空，请先在页面上选到房间".to_string());
    }
    if let Some(bad) = room.path.iter().position(|s| s.value.trim().is_empty()) {
        return Err(format!("第 {} 级未填值", bad + 1));
    }
    if room.label.trim().is_empty() {
        room.label = default_label(&room);
    }
    let key = room_key(&room);
    match rooms.iter().position(|r| room_key(r) == key) {
        Some(i) => {
            // 同一房间重复保存 = 改名/刷新元信息，不产生重复项
            room.id = rooms[i].id.clone();
            room.bound = rooms[i].bound;
            rooms[i] = room;
            Ok(rooms)
        }
        None => {
            if rooms.len() >= MAX_SAVED_ROOMS {
                return Err(format!("常用房间已达上限（{MAX_SAVED_ROOMS} 个）"));
            }
            if room.id.trim().is_empty() {
                room.id = next_room_id(&rooms);
            }
            if room.bound {
                // 新房间自带绑定 ⇒ 先清旧的，保证「最多一个绑定」
                let new_id = room.id.clone();
                clear_other_bounds(&mut rooms, &new_id);
            }
            rooms.push(room);
            Ok(rooms)
        }
    }
}

/// 绑定唯一性的唯一实现：清掉除 `keep_id` 之外所有房间的绑定。
fn clear_other_bounds(rooms: &mut [SavedRoom], keep_id: &str) {
    for r in rooms.iter_mut() {
        if r.id != keep_id {
            r.bound = false;
        }
    }
}

/// 绑定/解绑「我的宿舍」（纯逻辑，单测覆盖）：`bound=true` 时把它设为**唯一**绑定
/// （其余房间自动解绑），`bound=false` 时仅解绑它自己。id 不存在 → `Err`（可读中文）。
pub fn set_bound(
    mut rooms: Vec<SavedRoom>,
    id: &str,
    bound: bool,
) -> Result<Vec<SavedRoom>, String> {
    let Some(i) = rooms.iter().position(|r| r.id == id) else {
        return Err("该房间不在常用列表里（可能已被删除），请刷新后重试".to_string());
    };
    if !bound {
        rooms[i].bound = false;
        return Ok(rooms);
    }
    let id = rooms[i].id.clone();
    clear_other_bounds(&mut rooms, &id);
    rooms[i].bound = true;
    Ok(rooms)
}

/// 删除一个常用房间（纯逻辑，单测覆盖）；id 不存在则原样返回。
pub fn remove_room(rooms: Vec<SavedRoom>, id: &str) -> Vec<SavedRoom> {
    rooms.into_iter().filter(|r| r.id != id).collect()
}

// ---------------- 目录 / 级联 ----------------

/// 电费侧失败文案：会话失效沿批 2 口径；网络层失败点明「需校园网」（内网明文 IP，校外不可达）；
/// 服务端空文案（实测房间号给空串时 `code=500` 且 `message` 为空）补一句可读话。
/// 可见性 `pub(crate)`：历史命令面（`commands::electricity_history`）复用同一套文案。
pub(crate) fn elec_err(e: &CampusSynjonesError) -> String {
    match e {
        CampusSynjonesError::Http(msg) => {
            format!("无法访问学校服务，请确认已连校园网（{msg}）")
        }
        CampusSynjonesError::Api { code, msg } if msg.trim().is_empty() => {
            format!("学校服务返回异常（code={code}）")
        }
        other => err_text(other),
    }
}

/// 片区目录（**免登录可调**：`/charge/feeitem` 是该服务唯一匿名端点，见 crate 头注）。
/// 已登录时复用会话 client（同 Cookie jar），未登录时现建一个——不触碰 token 缓存。
#[tauri::command]
pub async fn list_feeitems(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<FeeItem>>, String> {
    let cas = match session_client(&state).await {
        Some(c) => c,
        None => match CasClient::new() {
            Ok(c) => c,
            Err(e) => return Ok(CommandResult::err(&format!("初始化网络客户端失败：{e}"))),
        },
    };
    Ok(match charge::list_feeitems(&cas).await {
        Ok(items) => CommandResult::ok(items),
        Err(e) => CommandResult::err(&elec_err(&e)),
    })
}

/// 级联查询：`path` 为空取第 1 级选项；非空则重放该路径。
/// 末级（房间）是**输入级**（服务端不下发选项）——`options` 为空且 `isFinal == false` 时，
/// 前端按 `FeeItem.lastLevelIsInput` 把该级渲染成输入框（房间号），带上房间号再请求一次即得 `view`。
#[tauri::command]
pub async fn query_electricity(
    state: State<'_, AppState>,
    feeitem_id: String,
    path: Vec<RoomStep>,
) -> Result<CommandResult<ElectricityQuery>, String> {
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(
        match charge::query_cascade(&sess.client, &feeitem_id, &path).await {
            Ok(q) => CommandResult::ok(q),
            Err(e) => CommandResult::err(&elec_err(&e)),
        },
    )
}

// ---------------- 常用房间命令 ----------------

/// 本地常用房间列表（无入参，读盘失败回空列表）。
#[tauri::command]
pub async fn get_electricity_rooms() -> Result<CommandResult<Vec<SavedRoom>>, String> {
    let dir = state::data_dir()?;
    Ok(CommandResult::ok(load_rooms(&dir)))
}

/// 保存常用房间（upsert），返回更新后的完整列表。
#[tauri::command]
pub async fn save_electricity_room(
    room: SavedRoom,
) -> Result<CommandResult<Vec<SavedRoom>>, String> {
    let dir = state::data_dir()?;
    match upsert_room(load_rooms(&dir), room) {
        Ok(rooms) => match write_rooms(&dir, &rooms) {
            Ok(()) => Ok(CommandResult::ok(rooms)),
            Err(e) => Ok(CommandResult::err(&e)),
        },
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

/// 删除常用房间，返回更新后的完整列表。
#[tauri::command]
pub async fn delete_electricity_room(id: String) -> Result<CommandResult<Vec<SavedRoom>>, String> {
    let dir = state::data_dir()?;
    let rooms = remove_room(load_rooms(&dir), &id);
    match write_rooms(&dir, &rooms) {
        Ok(()) => Ok(CommandResult::ok(rooms)),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

// ---------------- 去官网充值（系统浏览器） ----------------

/// 充值入口：在系统浏览器打开官方充值页（2026-09-19 裁决：不再内嵌官方界面，
/// 客户端直调官方接口的充值在后续批次接入）。
/// 入参只允许数字片区 id，URL 由后端拼装，前端无法借它打开任意地址（与 `portal::open_in_browser`
/// 的白名单思路一致，只是这里的合法目标是内网 IP）。
#[tauri::command]
pub async fn open_recharge_in_browser(
    app: AppHandle,
    feeitem_id: String,
) -> Result<CommandResult<()>, String> {
    let id = feeitem_id.trim();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Ok(CommandResult::err("片区 id 非法"));
    }
    let url = format!("{BERSERKER_BASE}{RECHARGE_PATH_PREFIX}{id}");
    Ok(
        match app.opener().open_url(url.clone(), None::<&str>) {
            Ok(()) => CommandResult::empty(),
            Err(e) => CommandResult::err(&format!("打开浏览器失败：{e}")),
        },
    )
}

// ---------------- 充值六条（M3.1 批 C） ----------------

/// 充值命令面取会话（与 `query_electricity` 同一把锁/同一个客户端）。
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

/// `recharge_create` → data。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RechargeCreated {
    pub order_id: String,
}

/// `recharge_pay_methods` / `recharge_status` → data。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RechargePayMethods {
    pub order: RechargeOrder,
    pub methods: Vec<PayMethod>,
}

/// `recharge_query_account` → data。`pad` 只有**需密码**且服务端下发键盘时才有值（见 [`PasswordPad`] 红线）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RechargeAccounts {
    pub accounts: Vec<String>,
    pub ccctypes: Vec<String>,
    pub pad: Option<PasswordPad>,
}

/// **建单**（电费 `paystep=0`，金额 1 元起；副作用请求只发一次）。
///
/// `path` = 当前房间的完整级联路径（校区 → 楼栋 → 房间，与 `query_electricity` 的入参同构）——**必填语义**。
/// `third_party`（房间上下文串）**由后端按该路径合成**——前端拿不到也不需要（见模块头注红线 7）。
///
/// ⚠️ `path` 声明成 `Option` **只为兼容尚未升级的旧前端**：批 D 已提交的调用没传 `path`
/// （它在等后端给上下文串）。不声明 `Option` 时 Tauri 会在反序列化阶段就报 `invalid args`（IPC 层错误，
/// 界面只能显示通用失败），声明后我们能把「请更新客户端」这句可读文案送到用户面前。
///
/// 失败文案：服务端原文一并透出（如 `dayTotalMoney-日消费最大金额判断异常了-null`），便于排查。
#[tauri::command]
pub async fn recharge_create(
    state: State<'_, AppState>,
    feeitem_id: String,
    tranamt: String,
    path: Option<Vec<RoomStep>>,
) -> Result<CommandResult<RechargeCreated>, String> {
    let Some(path) = path.filter(|p| !p.is_empty()) else {
        return Ok(CommandResult::err(
            "缺少房间信息，请返回上一步重新选择房间后再试（客户端需更新）",
        ));
    };
    with_synjones!(state, |client| {
        Ok(match recharge::create_order(client, &feeitem_id, &tranamt, &path).await {
            Ok(order_id) => CommandResult::ok(RechargeCreated { order_id }),
            Err(e) => CommandResult::err(&elec_err(&e)),
        })
    })
}

/// **支付方式**（`getpayinfo`）：返回订单状态 + 账户类支付方式（crate 已过滤到 `ACCOUNT`/`ACCOUNTTSM`）。
#[tauri::command]
pub async fn recharge_pay_methods(
    state: State<'_, AppState>,
    order_id: String,
) -> Result<CommandResult<RechargePayMethods>, String> {
    with_synjones!(state, |client| {
        Ok(
            match recharge::fetch_pay_methods(client, &order_id).await {
                Ok((order, methods)) => {
                    CommandResult::ok(RechargePayMethods { order, methods })
                }
                Err(e) => CommandResult::err(&elec_err(&e)),
            },
        )
    })
}

/// **查账户 / 安全键盘**（`paystep=2`，两步协议，见 `recharge::query_account`）：
///
/// - 不带 `accountno` → 回**账号列表**（渲染「选择账号」）；
/// - 带已选 `accountno` → 回该账号的**账户类型**与**安全键盘**（需密码的渠道才有 `pad`）。
///
/// `pad.keys` **只允许**交给渲染层画键盘；提交只回传下标序列（见 `recharge_submit`）。
#[tauri::command]
pub async fn recharge_query_account(
    state: State<'_, AppState>,
    order_id: String,
    code: String,
    payid: String,
    accountno: Option<String>,
) -> Result<CommandResult<RechargeAccounts>, String> {
    with_synjones!(state, |client| {
        let pay = PayMethod {
            code,
            payid,
            name: String::new(),
            // 提交/查询只用到 code 与 payid；nopassword 由 `recharge_pay_methods` 的结果决定
            nopassword: false,
            remark: None,
        };
        Ok(
            match recharge::query_account(client, &order_id, &pay, accountno.as_deref()).await {
                Ok((accounts, ccctypes, pad)) => {
                    CommandResult::ok(RechargeAccounts {
                        accounts,
                        ccctypes,
                        pad,
                    })
                }
                Err(e) => CommandResult::err(&elec_err(&e)),
            },
        )
    })
}

/// **提交支付**（`paystep=2`，**唯一会扣款的命令**）。
///
/// `password_seq` = 用户点击的**键位下标序列**（6 位数字字符串，如 `"013579"`）配 `uuid`；免密时都不传。
/// 红线：本模块只转发这串下标，**绝不**用 `keys` 还原真实字符，也不把它写进任何日志/存储。
// 8 个扁平入参是 Tauri 命令的既有形态（前端 `invoke("recharge_submit", { orderId, code, … })` 按名传参）；
// 改成结构体会直接破坏已提交的前端契约，故只压制计数告警。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn recharge_submit(
    state: State<'_, AppState>,
    order_id: String,
    code: String,
    payid: String,
    accountno: String,
    ccctype: String,
    password_seq: Option<String>,
    uuid: Option<String>,
) -> Result<CommandResult<()>, String> {
    with_synjones!(state, |client| {
        let pay = PayMethod {
            code,
            payid,
            name: String::new(),
            // 提交路径不消费 `nopassword`（是否免密由前端按 `recharge_pay_methods` 的结果决定）
            nopassword: false,
            remark: None,
        };
        Ok(
            match recharge::submit_pay(
                client,
                &order_id,
                &pay,
                &accountno,
                &ccctype,
                password_seq.as_deref(),
                uuid.as_deref(),
            )
            .await
            {
                Ok(()) => CommandResult::empty(),
                Err(e) => CommandResult::err(&elec_err(&e)),
            },
        )
    })
}

/// **结果查询**（`getpayinfo` 单次；轮询与上限由前端把关：`order.status` 0=待支付、1=已完成）。
#[tauri::command]
pub async fn recharge_status(
    state: State<'_, AppState>,
    order_id: String,
) -> Result<CommandResult<RechargePayMethods>, String> {
    with_synjones!(state, |client| {
        Ok(
            match recharge::fetch_order_status(client, &order_id).await {
                Ok((order, methods)) => {
                    CommandResult::ok(RechargePayMethods { order, methods })
                }
                Err(e) => CommandResult::err(&elec_err(&e)),
            },
        )
    })
}

/// **取消/清理未支付订单**（失败/放弃/超时后调用；crate 实测该端点必须 JSON body）。
#[tauri::command]
pub async fn recharge_cancel(
    state: State<'_, AppState>,
    order_id: String,
) -> Result<CommandResult<()>, String> {
    with_synjones!(state, |client| {
        Ok(match recharge::cancel_order(client, &order_id).await {
            Ok(()) => CommandResult::empty(),
            Err(e) => CommandResult::err(&elec_err(&e)),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("campushub-elec-{tag}-{n}"))
    }

    fn room(label: &str, value: &str) -> SavedRoom {
        SavedRoom {
            id: String::new(),
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
                    value: value.to_string(),
                    name: value.to_string(),
                },
            ],
            label: label.to_string(),
            bound: false,
        }
    }

    /// 落盘往返：save → load 相等；JSON 为 camelCase 明文（与前端 DTO 一致）。
    #[test]
    fn rooms_roundtrip_and_camel_case() {
        let dir = temp_dir("rt");
        let rooms = upsert_room(Vec::new(), room("我的宿舍", "101")).unwrap();
        assert_eq!(rooms.len(), 1);
        assert!(!rooms[0].id.is_empty(), "空 id 应由后端补上");
        write_rooms(&dir, &rooms).unwrap();

        let raw = fs::read_to_string(rooms_path(&dir)).unwrap();
        assert!(raw.contains("\"feeitemId\""), "落盘应 camelCase：{raw}");
        assert!(raw.contains("\"bound\": false"), "绑定字段也要落盘：{raw}");
        assert_eq!(load_rooms(&dir), rooms);
        fs::remove_dir_all(&dir).ok();
    }

    /// 绑定唯一性（M4 批 2）：`set_bound` 至多一个 true；绑新的自动解绑旧的；解绑只影响自己；
    /// 未知 id → 可读中文错误。
    #[test]
    fn set_bound_keeps_at_most_one_and_reports_unknown_id() {
        let mut rooms = upsert_room(Vec::new(), room("宿舍", "101")).unwrap();
        rooms = upsert_room(rooms, room("隔壁", "102")).unwrap();
        let (a, b) = (rooms[0].id.clone(), rooms[1].id.clone());
        assert_ne!(a, b, "同毫秒新增两个房间不得撞号（id 是绑定/删除的定位键）");

        let rooms = set_bound(rooms, &a, true).unwrap();
        assert!(rooms[0].bound, "被绑的应为 true");
        assert!(!rooms[1].bound);

        let rooms = set_bound(rooms, &b, true).unwrap();
        assert!(!rooms[0].bound, "绑新的应自动解绑旧的");
        assert!(rooms[1].bound);

        let rooms = set_bound(rooms, &b, false).unwrap();
        assert!(!rooms[1].bound, "解绑只影响自己");
        assert_eq!(
            set_bound(rooms, "不存在", true).unwrap_err(),
            "该房间不在常用列表里（可能已被删除），请刷新后重试"
        );
    }

    /// 保存房间**不得静默丢绑定**：同房间重复保存（改名/刷新元信息）沿用存量 bound；
    /// 新房间自带 bound=true 时按唯一性清掉其它房间的绑定。
    #[test]
    fn upsert_preserves_and_normalizes_binding() {
        let rooms = upsert_room(Vec::new(), room("宿舍", "101")).unwrap();
        let first_id = rooms[0].id.clone();
        let rooms = set_bound(rooms, &first_id, true).unwrap();
        assert!(rooms[0].bound);

        // 前端保存时不带 bound（serde default = false）也不能把绑定抹掉
        let mut renamed = room("宿舍改名", "101");
        renamed.bound = false;
        let rooms = upsert_room(rooms, renamed).unwrap();
        assert_eq!(rooms.len(), 1);
        assert!(rooms[0].bound, "重复保存应沿用存量绑定：{rooms:?}");

        // 新房间自带绑定 ⇒ 旧的解绑
        let mut fresh = room("新宿舍", "202");
        fresh.bound = true;
        let rooms = upsert_room(rooms, fresh).unwrap();
        assert_eq!(rooms.len(), 2);
        assert!(!rooms[0].bound, "新绑定应清掉旧绑定");
        assert!(rooms[1].bound);
        assert_eq!(rooms.iter().filter(|r| r.bound).count(), 1, "至多一个绑定");
    }

    /// 同一片区同一路径重复保存 → 原地更新（沿用原 id），不产生重复项；换房间号 → 新项。
    #[test]
    fn upsert_dedups_same_path_and_keeps_id() {
        let rooms = upsert_room(Vec::new(), room("宿舍", "101")).unwrap();
        let first_id = rooms[0].id.clone();
        let renamed = upsert_room(rooms, room("宿舍改名", "101")).unwrap();
        assert_eq!(renamed.len(), 1, "同房间重复保存不应新增");
        assert_eq!(renamed[0].id, first_id, "重复保存应沿用原 id");
        assert_eq!(renamed[0].label, "宿舍改名", "新名字生效");

        let more = upsert_room(renamed, room("隔壁", "102")).unwrap();
        assert_eq!(more.len(), 2, "不同房间号应新增");
    }

    /// label 缺省由路径名拼（末两级）；路径名全空 → 「未命名房间」。
    #[test]
    fn missing_label_gets_default_from_path() {
        let rooms = upsert_room(Vec::new(), room("", "101")).unwrap();
        assert_eq!(rooms[0].label, "1号楼 101");
        let mut nameless = room("", "101");
        for s in &mut nameless.path {
            s.name = String::new();
        }
        assert_eq!(default_label(&nameless), "未命名房间");
    }

    /// 校验：缺片区 id / 空路径 / 某级空值 → 报错；超上限 → 报错。
    #[test]
    fn upsert_validates_input_and_cap() {
        let mut no_feeitem = room("x", "101");
        no_feeitem.feeitem_id = "  ".to_string();
        assert!(upsert_room(Vec::new(), no_feeitem).is_err());

        let mut empty_path = room("x", "101");
        empty_path.path.clear();
        assert!(upsert_room(Vec::new(), empty_path).is_err());

        let mut blank_room = room("x", "101");
        blank_room.path[2].value = " ".to_string();
        let e = upsert_room(Vec::new(), blank_room).unwrap_err();
        assert!(e.contains("第 3 级未填值"), "实际 {e}");

        let mut all: Vec<SavedRoom> = Vec::new();
        for i in 0..MAX_SAVED_ROOMS {
            all = upsert_room(all, room(&format!("r{i}"), &format!("{i}01"))).unwrap();
        }
        assert_eq!(all.len(), MAX_SAVED_ROOMS);
        let e = upsert_room(all, room("溢出", "999")).unwrap_err();
        assert!(e.contains("上限"), "实际 {e}");
    }

    /// 删除：命中即移除，未知 id 原样返回；文件缺失/损坏 → 空列表不 panic（坏文件保留现场）。
    #[test]
    fn remove_and_tolerant_load() {
        let rooms = upsert_room(Vec::new(), room("宿舍", "101")).unwrap();
        let id = rooms[0].id.clone();
        assert!(remove_room(rooms.clone(), &id).is_empty());
        assert_eq!(remove_room(rooms, "不存在").len(), 1);

        let dir = temp_dir("missing");
        assert!(load_rooms(&dir).is_empty());
        fs::create_dir_all(&dir).unwrap();
        fs::write(rooms_path(&dir), "{ not json").unwrap();
        assert!(load_rooms(&dir).is_empty());
        assert!(rooms_path(&dir).exists(), "坏文件保留现场");
        fs::remove_dir_all(&dir).ok();
    }

    /// 失败文案：网络层失败点明「需校园网」，空文案的业务错误不出现悬空冒号。
    #[test]
    fn elec_err_messages_are_actionable() {
        let net = elec_err(&CampusSynjonesError::Http("HTTP 502".to_string()));
        assert!(net.contains("校园网"), "实际 {net}");
        let api = elec_err(&CampusSynjonesError::Api {
            code: 500,
            msg: String::new(),
        });
        assert_eq!(api, "学校服务返回异常（code=500）");
        assert_eq!(
            elec_err(&CampusSynjonesError::NotLogin),
            "登录已过期，请重新登录"
        );
        // 服务端原文必须透出（排查用：实测建单失败会给这类文案）
        let raw = elec_err(&CampusSynjonesError::Api {
            code: 500,
            msg: "dayTotalMoney-日消费最大金额判断异常了-null".to_string(),
        });
        assert!(raw.contains("dayTotalMoney"), "实际 {raw}");
    }

    /// 充值命令的 IPC 契约：全 camelCase（前端 `types.ts` 按这些键名取值，改键名即破坏前端）。
    #[test]
    fn recharge_dtos_are_camel_case() {
        let order = RechargeOrder {
            order_id: "1".to_string(),
            status: 0,
            pay_exp_date: Some("2026-09-19 17:30:00".to_string()),
            tranamt: Some(1.0),
        };
        let pay = PayMethod {
            code: "ACCOUNTTSM".to_string(),
            payid: "64".to_string(),
            name: "电子账户".to_string(),
            nopassword: false,
            remark: None,
        };
        let json = serde_json::to_value(RechargePayMethods {
            order,
            methods: vec![pay.clone()],
        })
        .unwrap();
        assert!(json["order"].get("orderId").is_some(), "实际 {json}");
        assert!(json["order"].get("payExpDate").is_some());
        assert_eq!(json["methods"][0]["payid"], "64");
        assert_eq!(json["methods"][0]["nopassword"], false);
        assert!(json["methods"][0].get("remark").is_some(), "None 也要在场（前端判 null）");

        let created = serde_json::to_value(RechargeCreated {
            order_id: "1".to_string(),
        })
        .unwrap();
        assert!(created.get("orderId").is_some(), "实际 {created}");

        let accounts = serde_json::to_value(RechargeAccounts {
            accounts: vec!["A1".to_string()],
            ccctypes: vec!["000".to_string()],
            pad: Some(PasswordPad {
                uuid: "u".to_string(),
                keys: vec!["1".to_string(), "2".to_string()],
            }),
        })
        .unwrap();
        assert_eq!(accounts["accounts"][0], "A1");
        assert_eq!(accounts["ccctypes"][0], "000");
        assert_eq!(accounts["pad"]["uuid"], "u");
        assert_eq!(accounts["pad"]["keys"][1], "2");
        // 无键盘时字段仍在场且为 null（前端据此判断「免密」）
        let no_pad = serde_json::to_value(RechargeAccounts {
            accounts: vec![],
            ccctypes: vec![],
            pad: None,
        })
        .unwrap();
        assert!(no_pad["pad"].is_null());
    }
}
