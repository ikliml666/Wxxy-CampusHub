//! 通知中心命令面 + 后台轮询（M5 批 2）：`get_notifications` /
//! `mark_notifications_read` / `get_notification_settings` / `save_notification_settings`
//! 与单一 `poll_tick` 循环（lib.rs setup spawn，参照 `auto_sync_tick` 先例）。
//!
//! # 轮询形态
//!
//! 一个循环三个源，**每 tick 热读 settings**（改设置即时生效，无需重启）：
//! ① 门户资讯——订阅栏目各 `query_info_list(col, 1, 10)`，与已见游标比对产出新公告；
//! ② 待办——`query_todo_list("todo", 1, 10)` 新条目；③ 电费——有绑定房间时
//! 复用 [`super::electricity_history::snapshot_round`]（**不复制采集逻辑**），
//! `balance < 阈值` 触发提醒（24h 节流，[`infra::notification`] 纯函数判定）。
//!
//! 纪律（与 `auto_sync_tick` 同款）：无会话**静默跳过**（绝不后台拉起登录链路）；
//! 单源失败不影响其他源；失败**指数退避**（倍数 ×2 封顶 2h，成功归 1）；
//! 每条新通知经 `tauri-plugin-notification` 发系统通知（标题「锡院助手」+ 分类
//! 前缀，失败只 log）并写入未读列表 + 推进游标（落盘失败则跳过系统通知，防止
//! 下一轮游标丢失后重复弹）。
//!
//! # 敏感纪律
//!
//! 日志与通知内容不含 token / 账号 / cookie / 户号；电费通知只用房间显示名与余额。

use super::auth::CommandResult;
use super::electricity::load_rooms;
use super::electricity_history::snapshot_round;
use crate::infra::electricity_history::SOURCE_AUTO;
use crate::infra::notification as store;
use crate::infra::state::{self, AppState};
use campus_portal::PortalClient;
use serde::Serialize;
use std::path::Path;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// 轮询每源每轮取的首页条数（设置里的间隔单位是分钟，条数固定轻量）。
const POLL_PAGE_SIZE: u32 = 10;
/// 全部源都关闭 / 数据目录不可用时的兜底轮询间隔（秒）——慢速空转，等开关打开。
const POLL_IDLE_SECS: u64 = 300;
/// 退避倍数上限（×24，配最小间隔 5 分钟 = 封顶 2 小时；`effective_interval_secs`
/// 内部还有 2h 硬封顶双保险）。
const BACKOFF_MAX_MULT: u32 = 24;
/// 系统通知标题（分类前缀进 body）。
const NOTIFY_TITLE: &str = "锡院助手";

// ---------------- 后台轮询 ----------------

/// 单源游标推进 + 基线标志写回（资讯/待办两个轮询源复用），返回本轮新条目 id。
///
/// 基线判定用显式标志（[`NotificationState::baselined_sources`]）：源无标志 =
/// 首次拉取，无论有没有条目都建基线、不通知；旧版本文件无标志字段时以「游标
/// 非空」视作已基线迁移（历史游标保留，不吞升级后第一轮的新条目）。
fn advance_cursor(
    st: &mut store::NotificationState,
    key: &str,
    fetched: &[String],
) -> Vec<String> {
    let cursor = st.cursors.get(key).cloned().unwrap_or_default();
    let has_flag = st.baselined_sources.contains(key);
    let already = has_flag || !cursor.is_empty();
    let d = store::diff_seen(&cursor, already, fetched);
    st.cursors.insert(key.to_string(), d.next);
    // 本轮刚建基线，或旧文件迁移（游标非空但无标志）→ 补写显式标志，之后
    // 判定不再依赖「游标非空」的隐式启发
    if d.baselined || (already && !has_flag) {
        st.baselined_sources.insert(key.to_string());
    }
    d.new_ids
}

/// 单条新通知的收敛：写未读列表 + 落盘 + 系统通知（落盘失败跳过系统通知防重复弹）。
///
/// **落盘前重新读盘**（P0）：`st` 是本轮开始时的快照（其 `unread` 不再使用），
/// 资讯源在拉取网络响应的 await 窗口期内用户可能已执行「标记已读」（独立读改写
/// 同一文件）；若整份回写旧快照会覆盖掉用户的已读（已读复活）。故未读列表以
/// 盘上最新为准、只把本轮 `fresh` 追加其上；游标与基线标志按快照覆盖（每源游标
/// 只有 poll_tick 单写者，快照里的推进即最新推进；用户操作不动这两者）。
fn commit_fresh(
    dir: &Path,
    app: &AppHandle,
    mute: bool,
    st_snapshot: store::NotificationState,
    fresh: Vec<store::NotificationItem>,
) {
    let mut st = store::load_state(dir);
    for (k, v) in st_snapshot.cursors {
        st.cursors.insert(k, v);
    }
    st.baselined_sources.extend(st_snapshot.baselined_sources);
    for f in &fresh {
        st.unread = store::push_unread(st.unread, f.clone());
    }
    if let Err(e) = store::save_state(dir, &st) {
        log::warn!("[notify] 通知状态落盘失败（跳过系统通知防重复弹）: {e}");
        return;
    }
    for f in &fresh {
        notify_system(app, mute, f);
    }
}

/// 系统通知：静音开关只拦系统弹窗，不影响通知中心落盘（调用方保证）。
fn notify_system(app: &AppHandle, mute: bool, item: &store::NotificationItem) {
    if mute {
        return;
    }
    use tauri_plugin_notification::NotificationExt;
    let body = format!("【{}】{}", store::kind_label(&item.kind), item.title);
    if let Err(e) = app.notification().builder().title(NOTIFY_TITLE).body(body).show() {
        log::warn!("[notify] 系统通知发送失败: {e}");
    }
}

/// ① 门户资讯：订阅栏目各拉首页 10 条，与游标比对产出新公告。
/// 任一栏目失败 → Err（该源整体退避，本轮部分成果不落盘，下轮重试幂等）。
async fn check_info(
    portal: &PortalClient,
    s: &store::NotificationSettings,
    dir: &Path,
    app: &AppHandle,
) -> Result<(), String> {
    let mut st = store::load_state(dir);
    let mut fresh = Vec::new();
    for col in &s.info_columns {
        let page = portal
            .query_info_list(col, 1, POLL_PAGE_SIZE)
            .await
            .map_err(|e| e.to_string())?;
        let key = format!("info:{}", col);
        let ids: Vec<String> = page.items.iter().map(|i| i.id.clone()).collect();
        let new_ids = advance_cursor(&mut st, &key, &ids);
        for it in &page.items {
            if new_ids.contains(&it.id) {
                fresh.push(store::info_notification(
                    col, &it.id, &it.title, &it.column_title, &it.publish_time, &it.url,
                ));
            }
        }
    }
    commit_fresh(dir, app, s.mute_system_notify, st, fresh);
    Ok(())
}

/// ② 门户待办：`tabId=todo` 首页 10 条（实测账号常无数据——空页首轮也建显式
/// 基线，之后首条真待办正常报新，不再被静默吞掉）。
async fn check_todo(
    portal: &PortalClient,
    s: &store::NotificationSettings,
    dir: &Path,
    app: &AppHandle,
) -> Result<(), String> {
    let page = portal
        .query_todo_list("todo", 1, POLL_PAGE_SIZE)
        .await
        .map_err(|e| e.to_string())?;
    let mut st = store::load_state(dir);
    let ids: Vec<String> = page.items.iter().map(|i| i.id.clone()).collect();
    let new_ids = advance_cursor(&mut st, "todo", &ids);
    let mut fresh = Vec::new();
    for it in &page.items {
        if new_ids.contains(&it.id) {
            fresh.push(store::todo_notification(&it.id, &it.title, &it.applicant, &it.apply_time));
        }
    }
    commit_fresh(dir, app, s.mute_system_notify, st, fresh);
    Ok(())
}

/// ③ 电费低余额：复用 [`snapshot_round`]（绑定房间校验/级联/落盘全在内，本层
/// 只做提醒判定与节流）。`balance` 提不到数字（None）→ 不提醒但算成功。
async fn check_elec(
    app_st: &tauri::State<'_, AppState>,
    s: &store::NotificationSettings,
    dir: &Path,
    app: &AppHandle,
) -> Result<(), String> {
    let out = snapshot_round(app_st, SOURCE_AUTO).await?;
    let Some(bal) = out.entry.balance else {
        return Ok(());
    };
    let mut st = store::load_state(dir);
    let now = store::now_epoch_secs();
    if !store::elec_should_alert(
        Some(bal),
        s.electricity_threshold_yuan,
        st.last_elec_alert_at,
        now,
    ) {
        return Ok(());
    }
    st.last_elec_alert_at = Some(now);
    let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
    let item = store::elec_notification(&out.entry.room_name, bal, s.electricity_threshold_yuan, &today);
    st.unread = store::push_unread(st.unread, item.clone());
    if let Err(e) = store::save_state(dir, &st) {
        log::warn!("[notify] 通知状态落盘失败（跳过系统通知防重复弹）: {e}");
        return Ok(());
    }
    notify_system(app, s.mute_system_notify, &item);
    Ok(())
}

/// 单源执行后统一收敛退避与休眠时长（成功归 1、失败 ×2，源关闭不动）。
fn settle_backoff(mult: &mut u32, base_min: u32, result: &Result<(), String>, sleep_secs: &mut u64) {
    match result {
        Ok(()) => {
            *mult = 1;
        }
        Err(e) => {
            *mult = (*mult * 2).min(BACKOFF_MAX_MULT);
            log::warn!("[notify] 轮询失败（退避 ×{mult}）: {e}");
        }
    }
    *sleep_secs = (*sleep_secs).min(store::effective_interval_secs(base_min, *mult));
}

/// 通知轮询主循环（lib.rs setup spawn，单一实例）：三类检查合一，每 tick 热读
/// settings；休眠时长 = 各**开启**源有效间隔的最小值（含失败退避）。
pub(crate) async fn poll_tick(app: tauri::AppHandle) {
    // [资讯, 待办, 电费] 的退避倍数（成功归 1、失败 ×2）
    let mut backoff = [1u32, 1, 1];
    loop {
        let Ok(dir) = state::data_dir() else {
            tokio::time::sleep(Duration::from_secs(POLL_IDLE_SECS)).await;
            continue;
        };
        let s = store::load_settings(&dir);
        let app_st = app.state::<AppState>();
        // 锁纪律：锁内只 clone portal（Arc 廉价），drop guard 后再 await
        let portal = {
            let guard = app_st.session.lock().await;
            guard.as_ref().map(|x| x.portal.clone())
        };

        let mut sleep_secs = u64::MAX;

        if let Some(portal) = portal.as_ref() {
            if !s.info_columns.is_empty() {
                let r = check_info(portal, &s, &dir, &app).await;
                settle_backoff(&mut backoff[0], s.info_interval_min, &r, &mut sleep_secs);
            }
            if s.todo_enabled {
                let r = check_todo(portal, &s, &dir, &app).await;
                settle_backoff(&mut backoff[1], s.todo_interval_min, &r, &mut sleep_secs);
            }
        } else {
            log::debug!("[notify] 当前无内存会话，本轮跳过（不拉起登录）");
        }

        // 电费源：需要会话 + 绑定房间（无绑定就不发请求，省 token 锁竞争）
        if s.electricity_enabled && portal.is_some() && load_rooms(&dir).iter().any(|r| r.bound) {
            let r = check_elec(&app_st, &s, &dir, &app).await;
            settle_backoff(&mut backoff[2], s.electricity_interval_min, &r, &mut sleep_secs);
        }

        // 无任何开启源 → 兜底 5 分钟慢轮询（等开关被打开 / 会话出现）
        let sleep = if sleep_secs == u64::MAX { POLL_IDLE_SECS } else { sleep_secs };
        tokio::time::sleep(Duration::from_secs(sleep)).await;
    }
}

// ---------------- IPC 命令 ----------------

/// 各类未读计数（`get_notifications` → data.counts）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationCounts {
    pub total: u32,
    pub info: u32,
    pub todo: u32,
    pub electricity: u32,
}

/// `get_notifications` / `mark_notifications_read` → data。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationsView {
    /// 未读列表（后端按产生时间升序存放，前端倒序展示最新的在前）。
    pub items: Vec<store::NotificationItem>,
    pub counts: NotificationCounts,
}

/// 计数（纯函数，按 kind 归类；未知 kind 只进 total）。
pub fn count_by_kind(items: &[store::NotificationItem]) -> NotificationCounts {
    let mut c = NotificationCounts {
        total: items.len() as u32,
        info: 0,
        todo: 0,
        electricity: 0,
    };
    for it in items {
        match it.kind.as_str() {
            store::KIND_INFO => c.info += 1,
            store::KIND_TODO => c.todo += 1,
            store::KIND_ELECTRICITY => c.electricity += 1,
            _ => {}
        }
    }
    c
}

/// 已读过滤（纯函数）：None/空 = 全读（清空）；Some(ids) = 只移除命中项。
/// 返回（剩余未读, 移除数）。
pub fn remove_read(
    items: Vec<store::NotificationItem>,
    ids: Option<&[String]>,
) -> (Vec<store::NotificationItem>, usize) {
    match ids {
        None => (Vec::new(), items.len()),
        Some(ids) if ids.is_empty() => (Vec::new(), items.len()),
        Some(ids) => {
            let before = items.len();
            let rest: Vec<_> = items.into_iter().filter(|it| !ids.contains(&it.id)).collect();
            let removed = before - rest.len();
            (rest, removed)
        }
    }
}

/// 通知中心未读列表 + 各类计数。
#[tauri::command]
pub async fn get_notifications() -> Result<CommandResult<NotificationsView>, String> {
    let dir = state::data_dir()?;
    let st = store::load_state(&dir);
    Ok(CommandResult::ok(NotificationsView {
        counts: count_by_kind(&st.unread),
        items: st.unread,
    }))
}

/// 标记已读：`ids` 缺省/空 = 全部已读；否则只移除命中 id 的项。
/// 返回剩余未读（前端免二次拉取）。
#[tauri::command]
pub async fn mark_notifications_read(
    ids: Option<Vec<String>>,
) -> Result<CommandResult<NotificationsView>, String> {
    let dir = state::data_dir()?;
    let st = store::load_state(&dir);
    let (rest, removed) = remove_read(st.unread, ids.as_deref());
    if removed > 0 {
        if let Err(e) = store::save_state(&dir, &store::NotificationState { unread: rest.clone(), ..st }) {
            return Ok(CommandResult::err(&format!("写通知状态失败: {e}")));
        }
    }
    Ok(CommandResult::ok(NotificationsView {
        counts: count_by_kind(&rest),
        items: rest,
    }))
}

/// 读取通知设置（文件缺失/损坏 → 默认设置）。
#[tauri::command]
pub async fn get_notification_settings() -> Result<CommandResult<store::NotificationSettings>, String> {
    let dir = state::data_dir()?;
    Ok(CommandResult::ok(store::load_settings(&dir)))
}

/// 保存通知设置：间隔必须 5..=720 分钟、阈值 ≥ 0，非法给中文原因不落盘。
#[tauri::command]
pub async fn save_notification_settings(
    settings: store::NotificationSettings,
) -> Result<CommandResult<()>, String> {
    if let Err(e) = store::validate_settings(&settings) {
        return Ok(CommandResult::err(&e));
    }
    let dir = state::data_dir()?;
    match store::save_settings(&dir, &settings) {
        Ok(()) => Ok(CommandResult::empty()),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::notification::NotificationItem;

    fn item(id: &str, kind: &str) -> NotificationItem {
        NotificationItem {
            id: id.to_string(),
            kind: kind.to_string(),
            title: format!("标题-{id}"),
            body: "正文".to_string(),
            created_at: "2026-09-20T08:00:00+08:00".to_string(),
            url: None,
        }
    }

    /// 计数：按 kind 归类、未知 kind 只进 total、空列表全 0。
    #[test]
    fn count_by_kind_groups_and_tolerates_unknown() {
        let items = vec![
            item("a", store::KIND_INFO),
            item("b", store::KIND_INFO),
            item("c", store::KIND_TODO),
            item("d", store::KIND_ELECTRICITY),
            item("e", "mystery"),
        ];
        let c = count_by_kind(&items);
        assert_eq!(c.total, 5);
        assert_eq!(c.info, 2);
        assert_eq!(c.todo, 1);
        assert_eq!(c.electricity, 1);
        assert_eq!(count_by_kind(&[]), NotificationCounts { total: 0, info: 0, todo: 0, electricity: 0 });
    }

    /// 已读过滤：None/空 = 全读；按 id 只移除命中项；未命中 id 原样保留。
    #[test]
    fn remove_read_all_or_by_ids() {
        let items = vec![item("a", store::KIND_INFO), item("b", store::KIND_TODO), item("c", store::KIND_INFO)];

        let (rest, removed) = remove_read(items.clone(), None);
        assert_eq!(removed, 3);
        assert!(rest.is_empty(), "全读清空");

        let (rest, removed) = remove_read(items.clone(), Some(&[]));
        assert_eq!(removed, 3, "空 ids 等效全读");

        let ids = vec!["b".to_string(), "ghost".to_string()];
        let (rest, removed) = remove_read(items, Some(&ids));
        assert_eq!(removed, 1, "未命中 id 不计移除数");
        assert_eq!(rest.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["a", "c"]);
    }

    /// settle_backoff：成功归 1、失败 ×2 封顶；休眠取有效间隔与当前值的最小者。
    #[test]
    fn settle_backoff_doubles_on_failure_resets_on_success() {
        let mut mult = 1u32;
        let mut sleep = u64::MAX;
        settle_backoff(&mut mult, 10, &Err("x".to_string()), &mut sleep);
        assert_eq!(mult, 2);
        assert_eq!(sleep, 2 * 600, "失败后退避间隔生效");

        settle_backoff(&mut mult, 10, &Err("y".to_string()), &mut sleep);
        assert_eq!(mult, 4);

        let mut sleep2 = u64::MAX;
        settle_backoff(&mut mult, 10, &Ok(()), &mut sleep2);
        assert_eq!(mult, 1, "成功归 1");
        assert_eq!(sleep2, 600, "恢复默认间隔");

        // 失败封顶：×2 到 24 为止（5 分钟 ×24 = 2 小时）
        let mut m = BACKOFF_MAX_MULT;
        let mut sleep3 = u64::MAX;
        settle_backoff(&mut m, 5, &Err("z".to_string()), &mut sleep3);
        assert_eq!(m, BACKOFF_MAX_MULT, "封顶不再翻倍");
        assert_eq!(sleep3, 2 * 3600, "最小间隔 × 上限倍数 = 2 小时");
    }

    /// 待办源空页起步 → 显式基线 → 首条真待办必须通知（P1：以「游标为空」当
    /// 未基线时首条真待办会被静默吞进基线）。
    #[test]
    fn advance_cursor_baselines_empty_first_pull_then_reports_first_todo() {
        let mut st = store::NotificationState::default();
        // 首轮：空页（实测待办常无数据）→ 建基线、无通知
        let new_ids = advance_cursor(&mut st, "todo", &[]);
        assert!(new_ids.is_empty(), "首轮空页不产生通知");
        assert!(st.baselined_sources.contains("todo"), "空页也写入显式基线标志");
        // 次轮：第一条真待办出现 → 必须报新
        let new_ids = advance_cursor(&mut st, "todo", &["t1".to_string()]);
        assert_eq!(new_ids, vec!["t1"], "基线后首条真待办必须通知");
        assert_eq!(st.cursors.get("todo").map(|v| v.as_slice()), Some(&["t1".to_string()][..]));
        // 三轮：同 id 不再报
        let new_ids = advance_cursor(&mut st, "todo", &["t1".to_string(), "t2".to_string()]);
        assert_eq!(new_ids, vec!["t2"]);
    }

    /// 旧版本文件迁移：游标非空但无基线标志 → 视作已基线，历史游标保留、
    /// 新条目照常报新（不吞升级后第一轮）。
    #[test]
    fn advance_cursor_migrates_legacy_nonempty_cursor_as_baselined() {
        let mut st = store::NotificationState::default();
        st.cursors.insert("todo".to_string(), vec!["t0".to_string()]);
        assert!(!st.baselined_sources.contains("todo"), "旧文件无标志");
        let new_ids = advance_cursor(&mut st, "todo", &["t0".to_string(), "t9".to_string()]);
        assert_eq!(new_ids, vec!["t9"], "旧游标保留，只报真新增");
        assert!(st.baselined_sources.contains("todo"), "迁移后补写标志");
    }
}
