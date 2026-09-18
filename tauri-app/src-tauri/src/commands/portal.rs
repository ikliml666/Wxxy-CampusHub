//! 门户数据命令（M2 批次 1：今日页总览；IPC 契约见计划 §2.1，camelCase）。
//!
//! `get_portal_overview` 聚合三个门户接口（学期 / 钱包卡 / 本周课表），**每个
//! 子字段取失败不阻塞其余字段**（失败项为 null，前端回落空态，不整页报错）。
//! 敏感纪律：JWT 与邮箱 `loginUrl` 在 campus-portal 内部消化，本命令只透出
//! 钱包数字与课程简报，不含任何凭据字段。

use super::auth::CommandResult;
use crate::infra::state::AppState;
use campus_portal::{next_course_from_now, CourseBrief, SemesterInfo, WalletSummary};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

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
