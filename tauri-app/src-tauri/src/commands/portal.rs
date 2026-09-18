//! 门户数据命令（M2 批次 1：今日页总览 + 批次 2：资讯页/待办页；IPC 契约见
//! 计划 §2.1，camelCase）。
//!
//! `get_portal_overview` 聚合三个门户接口（学期 / 钱包卡 / 本周课表），**每个
//! 子字段取失败不阻塞其余字段**（失败项为 null，前端回落空态，不整页报错）。
//! 批次 2 的五个命令为单接口透传（资讯栏目/列表/正文、待办分栏/列表），统一经
//! [`portal_call`]：未登录 → `success:false, message:"请先登录"`；协议错误映射
//! 为可读中文 message，不 panic。
//!
//! 敏感纪律：JWT 与邮箱 `loginUrl` 在 campus-portal 内部消化，本命令只透出
//! 钱包数字、课程简报、资讯条目与清洗后的正文 HTML，不含任何凭据字段。

use super::auth::CommandResult;
use crate::infra::state::AppState;
use campus_portal::{
    is_allowed_info_url, next_course_from_now, CourseBrief, InfoColumn, InfoDetail, InfoPage,
    PortalClient, SemesterInfo, TodoPage, TodoTab, WalletSummary,
};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;
use tauri_plugin_opener::OpenerExt;

/// 无会话时的约定文案（与 profile.rs 同口径，前端据此引导登录）。
const ERR_NO_SESSION: &str = "请先登录";

/// get_portal_overview → data。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalOverview {
    pub semester: Option<SemesterInfo>,
    pub wallet: Option<WalletSummary>,
    /// 无课 / 课表取失败为 null（前端隐藏横幅）。
    pub next_course: Option<CourseBrief>,
    /// 聚合完成时刻 epoch 毫秒。
    pub fetched_at: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 今日页总览：学期信息 + 钱包三卡 + 下一节课（子字段各自尽力而为）。
#[tauri::command]
pub async fn get_portal_overview(
    state: State<'_, AppState>,
) -> Result<CommandResult<PortalOverview>, String> {
    // 锁内只 clone portal（Arc 包装，廉价），drop guard 后再 await
    let portal = {
        let guard = state.session.lock().await;
        guard.as_ref().map(|s| s.portal.clone())
    };
    let Some(portal) = portal else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };

    // 子字段独立取数：任一失败置 None，不影响其余（计划 §2.1）
    let semester = portal.query_semester_info().await.ok();
    let wallet = portal.query_wallet_summary().await.ok();
    let next_course = portal
        .query_week_schedule()
        .await
        .ok()
        .and_then(|ws| next_course_from_now(&ws));

    Ok(CommandResult::ok(PortalOverview {
        semester,
        wallet,
        next_course,
        fetched_at: now_ms(),
    }))
}

/// 会话内门户客户端（None = 未登录）。锁内只 clone portal（Arc 廉价），
/// drop guard 后调用方再 await。协议错误由各命令统一映射为可读中文 message。
async fn portal_of(state: &State<'_, AppState>) -> Option<PortalClient> {
    let guard = state.session.lock().await;
    guard.as_ref().map(|s| s.portal.clone())
}

/// 资讯栏目（订阅接口 + 实测全量兜底，7 栏）。
#[tauri::command]
pub async fn get_info_columns(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<InfoColumn>>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_info_columns()
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 资讯列表（total/pageCount 实测不可靠，前端以 items.length 判断分页）。
#[tauri::command]
pub async fn get_info_list(
    state: State<'_, AppState>,
    column_id: String,
    page: u32,
    page_size: u32,
) -> Result<CommandResult<InfoPage>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_info_list(&column_id, page, page_size)
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 资讯正文（后端已清洗：域名白名单 + 标签/属性白名单，前端直接渲染）。
#[tauri::command]
pub async fn get_info_detail(
    state: State<'_, AppState>,
    url: String,
) -> Result<CommandResult<InfoDetail>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .fetch_info_detail(&url)
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 待办分栏（接口实际返回 6 个 tab，全量透传，前端展示 todo/done/apply）。
#[tauri::command]
pub async fn get_todo_tabs(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<TodoTab>>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_todo_tabs()
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 待办列表（tabId 白名单校验见 client::query_todo_list）。
#[tauri::command]
pub async fn get_todo_list(
    state: State<'_, AppState>,
    tab_id: String,
    page: u32,
    page_size: u32,
) -> Result<CommandResult<TodoPage>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_todo_list(&tab_id, page, page_size)
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 系统浏览器打开 URL（可复用 helper，批次 3 `open_app` 复用；实现为官方
/// `tauri-plugin-opener` 的 Rust API，不手写 Win32 调用）。
///
/// 安全：**域名白名单强制**——只允许 `*.cwxu.edu.cn`（复用
/// `campus_portal::is_allowed_info_url`，与正文抓取同一事实来源），非法域名
/// 一律拒绝，防被诱导打开任意 URL。
pub(crate) fn open_url_in_browser(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    if !is_allowed_info_url(url) {
        return Err("仅支持校园官网链接".to_string());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| format!("打开浏览器失败: {e}"))
}

/// 在系统浏览器中打开原文（needsBrowser 的正文 / 官网页面）。
#[tauri::command]
pub async fn open_in_browser(
    app: tauri::AppHandle,
    url: String,
) -> Result<CommandResult<()>, String> {
    Ok(match open_url_in_browser(&app, &url) {
        Ok(()) => CommandResult::empty(),
        Err(e) => CommandResult::err(&e),
    })
}
