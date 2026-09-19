//! 电费命令面（M3 批 3）：片区目录 / 级联查询 / 常用房间（本地存）/ 去官网充值（系统浏览器）。
//!
//! # 取数口径
//!
//! 目录与级联全部来自 `campus_synjones::charge`（协议事实见该模块头注，含两条 live 实测关键结论：
//! 「末级是输入级，服务端不下发下拉」「`showData` 键名恒为 `信息`，值是各片区格式不同的自由文本」）。
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
}

fn rooms_path(dir: &Path) -> PathBuf {
    dir.join("electricity_rooms.json")
}

fn now_ms() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default()
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

fn write_rooms(dir: &Path, rooms: &[SavedRoom]) -> Result<(), String> {
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
            rooms[i] = room;
            Ok(rooms)
        }
        None => {
            if rooms.len() >= MAX_SAVED_ROOMS {
                return Err(format!("常用房间已达上限（{MAX_SAVED_ROOMS} 个）"));
            }
            if room.id.trim().is_empty() {
                room.id = now_ms();
            }
            rooms.push(room);
            Ok(rooms)
        }
    }
}

/// 删除一个常用房间（纯逻辑，单测覆盖）；id 不存在则原样返回。
pub fn remove_room(rooms: Vec<SavedRoom>, id: &str) -> Vec<SavedRoom> {
    rooms.into_iter().filter(|r| r.id != id).collect()
}

// ---------------- 目录 / 级联 ----------------

/// 电费侧失败文案：会话失效沿批 2 口径；网络层失败点明「需校园网」（内网明文 IP，校外不可达）；
/// 服务端空文案（实测房间号给空串时 `code=500` 且 `message` 为空）补一句可读话。
fn elec_err(e: &CampusSynjonesError) -> String {
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
        assert_eq!(load_rooms(&dir), rooms);
        fs::remove_dir_all(&dir).ok();
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
    }
}
