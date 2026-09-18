//! 门户数据命令（M2 批次 1：今日页总览 + 批次 2：资讯页/待办页 + 批次 3：
//! 应用页/日程页；IPC 契约见计划 §2.1，camelCase）。
//!
//! `get_portal_overview` 聚合三个门户接口（学期 / 钱包卡 / 本周课表），**每个
//! 子字段取失败不阻塞其余字段**（失败项为 null，前端回落空态，不整页报错）。
//! 批次 2/3 的其余命令为单接口透传（资讯、待办、应用目录、日程、打开应用），
//! 统一经 [`portal_of`]：未登录 → `success:false, message:"请先登录"`；协议错误
//! 映射为可读中文 message，不 panic。
//!
//! 敏感纪律：JWT 与邮箱 `loginUrl` 在 campus-portal 内部消化，本命令只透出
//! 钱包数字、课程简报、资讯条目与清洗后的正文 HTML，不含任何凭据字段。

use super::auth::CommandResult;
use crate::infra::state::AppState;
use campus_portal::{
    is_allowed_info_url, is_http_url, next_course_from_now, AppCatalog, CourseBrief, InfoColumn,
    InfoDetail, InfoPage, PortalClient, ScheduleClassify, ScheduleEvent, SemesterInfo, TodoPage,
    TodoTab, WalletSummary,
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

// ---------------- M2 批次 3：应用页 / 日程页 ----------------

/// 应用目录（部门分组 + 收藏/常用钉选；图标已由后端代拉为 data URL，失败的
/// 条目 iconUrl 为 null，前端显示占位图标）。
#[tauri::command]
pub async fn get_app_catalog(
    state: State<'_, AppState>,
) -> Result<CommandResult<AppCatalog>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_app_catalog()
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 日程分类（实测 5 类，会话内后端已缓存）。
#[tauri::command]
pub async fn get_schedule_classify(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<ScheduleClassify>>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_schedule_classify()
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 日程区间明细（周/月视图取数；startMs/endMs 为本地周（月）首尾毫秒时间戳，
/// codes 为选中的分类 code 列表）。
#[tauri::command]
pub async fn get_schedule_month(
    state: State<'_, AppState>,
    start_ms: u64,
    end_ms: u64,
    codes: Vec<String>,
) -> Result<CommandResult<Vec<ScheduleEvent>>, String> {
    // 区间倒挂视为非法入参（前端 bug 防御），不透传给服务端
    if end_ms <= start_ms {
        return Ok(CommandResult::err("日程区间无效"));
    }
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_schedule_events(start_ms, end_ms, &codes)
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 打开应用（`isCas` 契约保留字段）。
///
/// 校验用**协议白名单**（[`campus_portal::is_http_url`]，仅 http/https），而非
/// [`open_url_in_browser`] 的域名白名单：该 URL 来自校方应用目录（受信来源），
/// 且只在**系统浏览器**打开、我们的后端不抓取它（无 SSRF 面）——实测 30 条目录
/// 数据中 16 条为非校园域（一卡通 `10.3.100.110`、知网、万方、超星、虚拟图书馆），
/// 域名白名单会把学校自己的合法应用全部拦掉。域名白名单（SSRF 面）保留在
/// 「后端抓取的正文 URL」路径（`fetch_info_detail`）与 `open_in_browser` 上。
///
/// ⚠️ 残留（计划附录 A / 批次 3 范围）：**B 类应用的 WebVPN 会话包装未实现**
/// ——WebVPN 会话尚未打通，需 WebVPN 环境的应用（外购数据库、内网 IP 直链）在
/// 校外网络下由浏览器侧自行报错。待 WebVPN 会话打通后在此处按 isCas/link
/// 分类包装，届时一并处理打开前的会话预置。
#[tauri::command]
pub async fn open_app(
    app: tauri::AppHandle,
    url: String,
    is_cas: bool,
) -> Result<CommandResult<()>, String> {
    // isCas 当前不影响打开策略（协议直开，见 doc comment），保留入参以冻结 IPC 契约
    let _ = is_cas;
    if !is_http_url(&url) {
        return Ok(CommandResult::err("仅支持 http/https 链接"));
    }
    // 官方 tauri-plugin-opener 的 Rust API（不手写 Win32 调用）
    Ok(match app.opener().open_url(&url, None::<&str>) {
        Ok(()) => CommandResult::empty(),
        Err(e) => CommandResult::err(&format!("打开浏览器失败: {e}")),
    })
}
