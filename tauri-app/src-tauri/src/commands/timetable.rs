//! 课表命令面（M2.5 批次 1：本地读取；批次 2 起补导入/增删改/ICS 导出）。
//!
//! `get_timetable` 为纯本地读取（无网络、无需登录态）：读
//! `%APPDATA%/campushub/timetable.json`，缺失/损坏 → 空课表（冻结契约 §2.3，
//! 前端据 `courses: []` 呈现空态）。出参直接透出 `campus_schedule::Timetable`
//!（域类型已 serde camelCase，`updatedAt` 等与冻结契约字段名一致）。

use super::auth::CommandResult;
use crate::infra::{state, timetable};
use campus_schedule::model::Timetable;

/// 本地课表读取（无入参）。
#[tauri::command]
pub async fn get_timetable() -> Result<CommandResult<Timetable>, String> {
    let dir = state::data_dir()?;
    Ok(CommandResult::ok(timetable::load_timetable(&dir)))
}
