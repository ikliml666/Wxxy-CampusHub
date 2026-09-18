//! 课表命令面（M2.5：批次 1 本地读取；批次 2 导入对比 / 手动增删改 / ICS 导出；
//! 批次 3 调课通知 L1/L2 解析与 override 命令）。
//!
//! - [`get_timetable`]：纯本地读取（无网络、无需登录态），缺失/损坏 → 空课表。
//! - [`import_timetable`]：需登录。门户学期信息推导 `xnm`/`xqm`（冻结契约 §1.2：
//!   `xnm` = `start_date` 前 4 位、`semester` `"1"→3 / "2"→12`；**不用 `grade`**
//!  ——它是入学年级）→ 教务拉 JSON（901 → TGT 静默重进由 campus-auth 内部处理）→
//!   `parse_kb_response` → [`campus_schedule::diff_courses`] 合并旧库 → 落库。
//! - 手动课程三命令：本地数据操作（无需登录）；`source=Manual` 的课程不参与
//!   导入 diff（冻结契约 §2.4）。
//! - [`export_ics`]：生成展开式 VEVENT 后**由后端写入用户下载目录**并返回写入
//!   路径（2026-09-18 真机验收：WebView2 不处理下载，前端 Blob 交付不可用）。
//!   时间取校本大节作息 `campus_portal::block_time_slots`（与今日页同一事实来源），
//!   日期由 `semester_start_date` + 周次 + 星期推出。
//! - [`parse_notice`] / [`apply_override`] / [`revoke_notice`]（批次 3）：解析
//!   纯函数在 campus_schedule::notice（契约 §2.5），本层只做接线——本地课表 +
//!   当前周传入、候选采纳写 overrides（noticeId+courseId 幂等覆盖）、按
//!   noticeId 整批撤销。
//! - [`save_time_slots`]（2026-09-18 收尾轮）：保存/清空自定义作息；
//!   [`effective_slots`] 是生效作息的单点取值（`config.slots` 优先、回落内置
//!   校本大节表），`build_timetable_view` 与 `build_ics` 共用。
//!
//! 统一口径：业务失败一律 `Ok(CommandResult::err(中文消息))`（`Err(String)` 仅限
//! IPC 框架层）；敏感纪律——本模块不输出任何 cookie/TGT/凭据字段。

use super::auth::CommandResult;
use crate::infra::state::AppState;
use crate::infra::{state, timetable};
use campus_portal::block_time_slots;
use campus_schedule::model::{Course, CourseOverride, TimeSlot, Timetable};
use campus_schedule::{current_week, diff_courses, parse_kb_response, parse_notice_text, Semester, NoticeConfidence};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tauri::State;

/// 无会话时的约定文案（与 profile.rs / portal.rs 同口径）。
const ERR_NO_SESSION: &str = "请先登录";

/// get_timetable → data（批次 4 修订契约 §2.3）：课表本体 + 校本大节作息 +
/// 当前教学周 + 今天。`slots` 是时间标签的唯一事实源（前端不得硬编码时间），
/// `currentWeek` 为 None 表示未配置开学日或今天不在学期范围内。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimetableView {
    pub timetable: Timetable,
    pub slots: Vec<TimeSlot>,
    pub current_week: Option<u32>,
    /// 本机今天 "YYYY-MM-DD"
    pub today: String,
}

/// 生效作息的**单点取值**（冻结契约 §2.3 slots 取值口径，2026-09-18 收尾轮修订）：
/// `config.slots` 有值且非空 → 用户自定义作息（唯一事实源）；`None`/空 → 回落
/// 内置校本大节表 [`campus_portal::block_time_slots`]。
/// 网格行（[`TimetableView::slots`]）、ICS 展开、大节号→时间查找必须全部经本函数
/// 取值，不得一处分发一处硬编码。
fn effective_slots(config: &campus_schedule::model::CourseTableConfig) -> Vec<TimeSlot> {
    match config.slots.as_deref() {
        Some(custom) if !custom.is_empty() => custom.to_vec(),
        _ => block_time_slots(),
    }
}

/// 纯函数组装（便于单测）：周次口径与 [`parse_notice`] 一致
/// （`campus_schedule::current_week`）。
fn build_timetable_view(tt: Timetable, today: chrono::NaiveDate) -> TimetableView {
    TimetableView {
        slots: effective_slots(&tt.config),
        current_week: current_week(today, &tt.config),
        today: today.format("%Y-%m-%d").to_string(),
        timetable: tt,
    }
}

/// 本地课表读取（无入参）。
#[tauri::command]
pub async fn get_timetable() -> Result<CommandResult<TimetableView>, String> {
    let dir = state::data_dir()?;
    Ok(CommandResult::ok(build_timetable_view(
        timetable::load_timetable(&dir),
        chrono::Local::now().date_naive(),
    )))
}

// ---------------- M2.5 批次 2：导入 / 手动课程 / ICS ----------------

/// import_timetable → data（变更摘要；`changes` 为人类可读条目）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub added: u32,
    pub changed: u32,
    pub removed: u32,
    /// 合并后本地课表课程总数（含已停开的保留记录）。
    pub total: u32,
    pub changes: Vec<String>,
}

/// add_course_manual 入参（冻结契约 §2.3，camelCase）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualCourseInput {
    pub name: String,
    #[serde(default)]
    pub teacher: String,
    #[serde(default)]
    pub position: String,
    /// 星期几，1=周一 … 7=周日
    pub day: u8,
    pub start_section: u8,
    pub end_section: u8,
    /// 出现周次（1-based）
    pub weeks: Vec<u32>,
    /// 课程卡片颜色索引（前端色板下标）
    pub color_index: u16,
    #[serde(default)]
    pub remark: Option<String>,
}

/// 从教务导入课表：SSO → 拉取 → diff 旧库 → 落库 → 返回变更摘要。
#[tauri::command]
pub async fn import_timetable(
    state: State<'_, AppState>,
) -> Result<CommandResult<ImportResult>, String> {
    // 锁内只 clone（client/tgt/portal 均廉价），drop guard 后再 await
    let session = {
        let guard = state.session.lock().await;
        guard
            .as_ref()
            .map(|s| (s.client.clone(), s.tgt.clone(), s.portal.clone()))
    };
    let Some((client, tgt, portal)) = session else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };

    // 学期信息（门户会话内已缓存）：推导口径 = 冻结契约 §1.2（不用 grade）
    let sem = portal.query_semester_info().await;
    let sem = match sem {
        Ok(s) => s,
        Err(e) => return Ok(CommandResult::err(&e.to_string())),
    };
    let Some(xnm) = sem
        .start_date
        .get(0..4)
        .and_then(|s| s.parse::<u32>().ok())
    else {
        return Ok(CommandResult::err("学期信息异常，无法推导学年"));
    };
    let semester = match sem.semester.trim() {
        "1" => Semester::First,
        "2" => Semester::Second,
        other => {
            return Ok(CommandResult::err(&format!("暂不支持该学期序号（{other}）")));
        }
    };

    // 拉取课表 JSON（内部：901 → TGT 静默重进 → 重试一次；仍失败 → JwglNotLogin，
    // Display 文案「教务会话已失效，请重新登录」直接透出）
    let json = match client
        .fetch_timetable_json(tgt.as_deref(), xnm, semester.xqm())
        .await
    {
        Ok(j) => j,
        Err(e) => return Ok(CommandResult::err(&e.to_string())),
    };
    let incoming = match parse_kb_response(&json, timetable::DEFAULT_TABLE_ID) {
        Ok(c) => c,
        Err(e) => return Ok(CommandResult::err(&e.to_string())),
    };

    let dir = state::data_dir()?;
    let old = timetable::load_timetable(&dir);
    let diff = diff_courses(&old.courses, &incoming);

    let mut tt = old;
    // 用学期信息初始化/更新配置（解析失败保留旧值，不因个别字段坏数据丢课表）
    tt.config.semester_start_date = semester_start_from_info(&sem.start_date)
        .or(tt.config.semester_start_date);
    if let Ok(w) = sem.week_count.trim().parse::<u32>() {
        if w > 0 {
            tt.config.semester_total_weeks = w;
        }
    }
    tt.courses = diff.courses;
    tt.updated_at = chrono::Local::now().to_rfc3339();

    match timetable::save_timetable(&dir, &tt) {
        Ok(()) => Ok(CommandResult::ok(ImportResult {
            added: diff.added,
            changed: diff.changed,
            removed: diff.removed,
            total: tt.courses.len() as u32,
            changes: diff.changes,
        })),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

/// 门户学期信息 `"YYYYMMDD"` → `NaiveDate`（周次计算锚点）。
fn semester_start_from_info(start_date: &str) -> Option<NaiveDate> {
    if start_date.len() != 8 {
        return None;
    }
    let y: i32 = start_date.get(0..4)?.parse().ok()?;
    let m: u32 = start_date.get(4..6)?.parse().ok()?;
    let d: u32 = start_date.get(6..8)?.parse().ok()?;
    NaiveDate::from_ymd_opt(y, m, d)
}

/// load → 修改 → save 的公共骨架（单文件整体读写；写失败向上返回中文错误，
/// 由命令层转 `CommandResult::err`）。前端交互天然串行，本模块不做进程内互斥。
fn mutate_timetable<T>(
    dir: &std::path::Path,
    f: impl FnOnce(&mut Timetable) -> Result<T, String>,
) -> Result<T, String> {
    let mut tt = timetable::load_timetable(dir);
    let out = f(&mut tt)?;
    timetable::save_timetable(dir, &tt)?;
    Ok(out)
}

/// 本地实体 id：`<前缀>-<纳秒时间戳>`（手动课程 `manual-` / 调课叠加 `ov-`）。
/// 与导入课程 `<table_id>-<jxb_id>` 前缀不同（永不冲突）；创建后即固定，
/// 且两者都不参与导入 diff，id 天然稳定。
fn fresh_id(prefix: &str) -> String {
    let n = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{n}")
}

fn new_manual_id() -> String {
    fresh_id("manual")
}

fn validate_manual_input(input: &ManualCourseInput) -> Result<(), String> {
    if input.name.trim().is_empty() {
        return Err("课程名不能为空".to_string());
    }
    if !(1..=7).contains(&input.day) {
        return Err("星期必须在周一至周日之间".to_string());
    }
    if input.start_section == 0 || input.end_section < input.start_section {
        return Err("节次范围无效".to_string());
    }
    if input.weeks.is_empty() || input.weeks.iter().any(|&w| w == 0) {
        return Err("周次不能为空且必须从第 1 周起".to_string());
    }
    Ok(())
}

/// 手动添加课程（source=Manual，colorIndex 由入参指定）。
#[tauri::command]
pub async fn add_course_manual(input: ManualCourseInput) -> Result<CommandResult<Course>, String> {
    if let Err(e) = validate_manual_input(&input) {
        return Ok(CommandResult::err(&e));
    }
    let course = Course {
        id: new_manual_id(),
        course_table_id: timetable::DEFAULT_TABLE_ID.to_string(),
        name: input.name.trim().to_string(),
        teacher: input.teacher.trim().to_string(),
        position: input.position.trim().to_string(),
        day: input.day,
        start_section: Some(input.start_section),
        end_section: Some(input.end_section),
        is_custom_time: false,
        custom_start_time: None,
        custom_end_time: None,
        color_index: input.color_index,
        remark: input.remark,
        source: campus_schedule::model::CourseSource::Manual,
        weeks: input.weeks,
        class_id: None,
        disabled: false,
    };
    let dir = state::data_dir()?;
    match mutate_timetable(&dir, |tt| {
        tt.courses.push(course.clone());
        Ok(course)
    }) {
        Ok(c) => Ok(CommandResult::ok(c)),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

/// 用户手工编辑课程（任意来源；自动更新仍只覆盖 Import 课程）。
#[tauri::command]
pub async fn update_course(course: Course) -> Result<CommandResult<Course>, String> {
    let dir = state::data_dir()?;
    match mutate_timetable(&dir, |tt| {
        match tt.courses.iter_mut().find(|c| c.id == course.id) {
            Some(slot) => {
                *slot = course.clone();
                Ok(())
            }
            None => Err("课程不存在".to_string()),
        }
    }) {
        Ok(()) => Ok(CommandResult::ok(course)),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

/// 删除课程（同时级联清理其挂载的调课 override，避免孤儿记录）。
#[tauri::command]
pub async fn delete_course(id: String) -> Result<CommandResult<()>, String> {
    let dir = state::data_dir()?;
    match mutate_timetable(&dir, |tt| {
        let before = tt.courses.len();
        tt.courses.retain(|c| c.id != id);
        tt.overrides.retain(|o| o.course_id != id);
        if tt.courses.len() == before {
            Err("课程不存在".to_string())
        } else {
            Ok(())
        }
    }) {
        Ok(()) => Ok(CommandResult::empty()),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

// ---------------- ICS 导出 ----------------

/// RFC 5545 TEXT 转义（`,` `;` `\` 换行）。
fn ics_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' | '\r' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out
}

/// 展开式 VEVENT 日历文本（不依赖 RRULE：每门未停开课程 × 其每个教学周各一个
/// VEVENT，主流日历客户端直接识别）。
///
/// 时区取舍：使用 RFC 5545 的 **floating local time**（`DTSTART:20260907T080000`，
/// 无 `Z`/`TZID`）——作息表时刻是「本地墙钟」语义，floating 形态合法且被
/// Outlook / Google 日历按导入时区正确解释，免去手写 VTIMEZONE 块。
///
/// 行长说明：SUMMARY/LOCATION/DESCRIPTION 均为短文本（课名/教室/教师，实测远
/// 低于 75 字节），不做 RFC 5545 §3.1 行折叠；超长自定义文本由客户端容忍。
fn build_ics(tt: &Timetable) -> Result<String, String> {
    let Some(start_date) = tt.config.semester_start_date else {
        return Err("尚未导入课表（缺少学期开学日期），请先完成一次导入".to_string());
    };
    // 作息单点取值（契约 §2.3）：自定义优先，回落内置——与 TimetableView.slots 同源
    let slots = effective_slots(&tt.config);
    let dtstamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//Wxxy-CampusHub//Timetable//CN".into(),
        "CALSCALE:GREGORIAN".into(),
        "METHOD:PUBLISH".into(),
    ];
    for course in &tt.courses {
        if course.disabled {
            continue; // 停开课程不进日历
        }
        // 大节号 = ceil(起始小节 / 2)；起始/结束大节任一超出校本 5 大节表
        //（无作息时刻可展开）→ 跳过该课程
        let (Some(start_sec), Some(end_sec)) = (course.start_section, course.end_section) else {
            continue;
        };
        let (block_start, block_end) = (
            (u32::from(start_sec) + 1) / 2,
            (u32::from(end_sec) + 1) / 2,
        );
        let Some(s) = slots.iter().find(|t| t.number as u32 == block_start) else {
            continue;
        };
        let Some(e) = slots.iter().find(|t| t.number as u32 == block_end) else {
            continue;
        };
        let start_hm = s.start_time.replace(':', "");
        let end_hm = e.end_time.replace(':', "");
        for &week in &course.weeks {
            if week == 0 {
                continue;
            }
            // 教学周 → 日期：开学日（第 1 周周一）+ (周次-1)×7 + (星期-1) 天
            let day_offset = (week - 1) * 7 + u32::from(course.day) - 1;
            let date = start_date + chrono::Duration::days(i64::from(day_offset));
            let day_basic = date.format("%Y%m%d").to_string();
            let mut desc_parts: Vec<String> = Vec::new();
            if !course.teacher.is_empty() {
                desc_parts.push(format!("教师 {}", course.teacher));
            }
            desc_parts.push(format!("第 {week} 周"));
            if let Some(r) = &course.remark {
                if !r.is_empty() {
                    desc_parts.push(r.clone());
                }
            }
            lines.extend([
                "BEGIN:VEVENT".into(),
                format!(
                    "UID:{}-w{}d{}s{}@campushub",
                    ics_escape(&course.id),
                    week,
                    course.day,
                    start_sec
                ),
                format!("DTSTAMP:{dtstamp}"),
                format!("DTSTART:{day_basic}T{start_hm}00"),
                format!("DTEND:{day_basic}T{end_hm}00"),
                format!("SUMMARY:{}", ics_escape(&course.name)),
                format!("LOCATION:{}", ics_escape(&course.position)),
                format!("DESCRIPTION:{}", ics_escape(&desc_parts.join("，"))),
                "END:VEVENT".into(),
            ]);
        }
    }
    lines.push("END:VCALENDAR".into());
    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    Ok(out)
}

/// 把 ICS 文本写入 `dir` 下固定文件名「课表.ics」（已存在直接覆盖），返回完整路径。
fn write_ics_to(dir: &Path, text: &str) -> Result<PathBuf, String> {
    let path = dir.join("课表.ics");
    std::fs::write(&path, text).map_err(|e| format!("写入 {} 失败：{e}", path.display()))?;
    Ok(path)
}

/// 导出 ICS：生成文本后写入用户下载目录并返回写入的完整路径（WebView2 不处理
/// 前端 Blob 下载，交付必须由后端落盘）。
#[tauri::command]
pub async fn export_ics() -> Result<CommandResult<String>, String> {
    let dir = state::data_dir()?;
    let tt = timetable::load_timetable(&dir);
    let text = match build_ics(&tt) {
        Ok(t) => t,
        Err(e) => return Ok(CommandResult::err(&e)),
    };
    let Some(dl_dir) = dirs::download_dir() else {
        return Ok(CommandResult::err(
            "无法定位系统下载目录（当前平台不受支持或目录不可用）",
        ));
    };
    match write_ics_to(&dl_dir, &text) {
        Ok(path) => Ok(CommandResult::ok(path.display().to_string())),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

// ---------------- M2.5 批次 3：调课通知解析与 override ----------------

/// L1/L2 解析（冻结契约 §2.5）：读本地课表 + 当前教学周，**不入库**；
/// 无课程/无学期锚点时 current_week=None，候选自然降级 Low。
#[tauri::command]
pub async fn parse_notice(
    text: String,
) -> Result<CommandResult<Vec<campus_schedule::NoticeCandidate>>, String> {
    let dir = state::data_dir()?;
    let tt = timetable::load_timetable(&dir);
    let today = chrono::Local::now().date_naive();
    let cw = current_week(today, &tt.config);
    Ok(CommandResult::ok(parse_notice_text(&text, &tt.courses, cw)))
}

/// 同一 `noticeId + courseId` 重复采纳幂等：先移除旧叠加再写入（覆盖而非堆叠）。
fn upsert_override(tt: &mut Timetable, ov: CourseOverride) {
    tt.overrides
        .retain(|o| !(o.source_notice_id == ov.source_notice_id && o.course_id == ov.course_id));
    tt.overrides.push(ov);
}

/// 撤销某条通知产生的全部 override，返回删除条数（0 条也幂等成功）。
fn revoke_by_notice(tt: &mut Timetable, notice_id: &str) -> u32 {
    let before = tt.overrides.len();
    tt.overrides
        .retain(|o| o.source_notice_id != notice_id);
    (before - tt.overrides.len()) as u32
}

/// 候选 → 叠加记录（契约 §2.3：autoApplied 区分高置信自动应用与低置信人工采纳；
/// 字段级拷贝——停课通知的星期/节次也保留，供前端定位「停哪一次」，见 model 注释）。
fn candidate_to_override(candidate: &campus_schedule::NoticeCandidate, course_id: &str) -> CourseOverride {
    CourseOverride {
        id: fresh_id("ov"),
        course_id: course_id.to_string(),
        weeks: candidate.weeks.clone(),
        change_type: candidate.change_type,
        new_day: candidate.new_day,
        new_start_section: candidate.new_start_section,
        new_end_section: candidate.new_end_section,
        new_position: candidate.new_position.clone(),
        source_notice_id: candidate.notice_id.clone(),
        auto_applied: candidate.confidence == NoticeConfidence::High,
    }
}

/// 采纳候选为调课叠加（高置信自动应用也走这条，`autoApplied` 区分来源）。
/// 候选未落到本地课程（courseId 缺失或课程已删）→ 业务失败。
#[tauri::command]
pub async fn apply_override(
    candidate: campus_schedule::NoticeCandidate,
) -> Result<CommandResult<CourseOverride>, String> {
    let dir = state::data_dir()?;
    match mutate_timetable(&dir, |tt| {
        let Some(course) = tt
            .courses
            .iter()
            .find(|c| Some(&c.id) == candidate.course_id.as_ref())
        else {
            return Err(
                "通知未匹配到本地课程（courseId 缺失或课程已删除），无法应用".to_string()
            );
        };
        let ov = candidate_to_override(&candidate, &course.id);
        upsert_override(tt, ov.clone());
        Ok(ov)
    }) {
        Ok(ov) => Ok(CommandResult::ok(ov)),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

/// 撤销通知：删除该 noticeId 产生的全部 override，返回删除条数。
#[tauri::command]
pub async fn revoke_notice(notice_id: String) -> Result<CommandResult<u32>, String> {
    let dir = state::data_dir()?;
    match mutate_timetable(&dir, |tt| Ok(revoke_by_notice(tt, &notice_id))) {
        Ok(n) => Ok(CommandResult::ok(n)),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

// ---------------- 作息时间表编辑（冻结契约 §2.3，2026-09-18 收尾轮追加） ----------------

/// 自定义作息条数上限（防误填：正常学校大节不会超过这个量级）。
const MAX_TIME_SLOTS: usize = 20;

/// `"HH:MM"` → 分钟数（0..=1439）；格式非法返回 None。
fn parse_hm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(h * 60 + m)
}

/// 自定义作息校验（非法一律中文原因，转 `CommandResult::err`）：至少 1 条、
/// 不超 [`MAX_TIME_SLOTS`]；`number` 正整数且严格递增不重复；时间匹配 `HH:MM`
/// 且 `end_time > start_time`（等长数字串字典序即时间序）。
fn validate_time_slots(slots: &[TimeSlot]) -> Result<(), String> {
    if slots.is_empty() {
        return Err("作息至少需要一条".to_string());
    }
    if slots.len() > MAX_TIME_SLOTS {
        return Err(format!("作息条数不能超过 {MAX_TIME_SLOTS} 条"));
    }
    let mut prev_number: u8 = 0;
    for (i, s) in slots.iter().enumerate() {
        if s.number == 0 {
            return Err(format!("第 {} 条的大节号必须是正整数", i + 1));
        }
        if s.number <= prev_number {
            return Err(format!(
                "大节号必须严格递增且不重复（第 {} 条：{}）",
                i + 1,
                s.number
            ));
        }
        let (Some(start), Some(end)) = (parse_hm(&s.start_time), parse_hm(&s.end_time)) else {
            return Err(format!(
                "第 {} 条的时间格式必须是 HH:MM（如 08:00）",
                i + 1
            ));
        };
        if end <= start {
            return Err(format!(
                "第 {} 条的结束时间（{}）必须晚于开始时间（{}）",
                i + 1,
                s.end_time,
                s.start_time
            ));
        }
        prev_number = s.number;
    }
    Ok(())
}

/// 保存自定义作息时间表（冻结契约 §2.3 `save_time_slots`）：
/// `None` → 清空自定义（`config.slots = None`，恢复内置校本默认）；
/// `Some(v)` → 校验通过后保存。返回刷新后的 [`TimetableView`]（前端免二次拉取）。
#[tauri::command]
pub async fn save_time_slots(
    slots: Option<Vec<TimeSlot>>,
) -> Result<CommandResult<TimetableView>, String> {
    if let Some(v) = &slots {
        if let Err(e) = validate_time_slots(v) {
            return Ok(CommandResult::err(&e));
        }
    }
    let dir = state::data_dir()?;
    match mutate_timetable(&dir, |tt| {
        // Some(空数组) 按恢复内置处理（与「None/空 = 内置」口径一致），防御性归一
        tt.config.slots = slots.clone().filter(|v| !v.is_empty());
        Ok(tt.clone())
    }) {
        Ok(tt) => Ok(CommandResult::ok(build_timetable_view(
            tt,
            chrono::Local::now().date_naive(),
        ))),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use campus_schedule::model::{CourseSource, CourseTableConfig};

    fn course(id: &str, name: &str, day: u8, start: u8, end: u8, weeks: Vec<u32>) -> Course {
        Course {
            id: id.into(),
            course_table_id: "default".into(),
            name: name.into(),
            teacher: "张老师".into(),
            position: "D4-207".into(),
            day,
            start_section: Some(start),
            end_section: Some(end),
            is_custom_time: false,
            custom_start_time: None,
            custom_end_time: None,
            color_index: 2,
            remark: None,
            source: CourseSource::Import,
            weeks,
            class_id: Some("abc".into()),
            disabled: false,
        }
    }

    fn timetable_with(start_date: Option<NaiveDate>, courses: Vec<Course>) -> Timetable {
        Timetable {
            config: CourseTableConfig {
                course_table_id: "default".into(),
                show_weekends: false,
                semester_start_date: start_date,
                semester_total_weeks: 20,
                first_day_of_week: 1,
                slots: None,
            },
            courses,
            overrides: vec![],
            updated_at: String::new(),
        }
    }

    /// 开学日 2026-09-07（周一，实测学期锚点）。
    fn fixture() -> Timetable {
        timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![
                course("default-a", "信息安全", 1, 1, 2, vec![1, 3]),
                course("default-b", "密码学", 3, 3, 4, vec![2]),
            ],
        )
    }

    fn vevent_count(ics: &str) -> usize {
        ics.matches("BEGIN:VEVENT").count()
    }

    /// VEVENT 数量 = 未停开课程数 × 周次数；时间字符串正确（大节 1/2 作息 +
    /// 开学日锚点）。
    #[test]
    fn ics_expands_events_with_correct_times() {
        let ics = build_ics(&fixture()).unwrap();
        // 2 门课 × 各自周次数 = 2 + 1
        assert_eq!(vevent_count(&ics), 3);
        // 大节1（1-2节）08:00-09:40，第 1 周周一 = 开学日当天
        assert!(ics.contains("DTSTART:20260907T080000"));
        assert!(ics.contains("DTEND:20260907T094000"));
        // 第 3 周周一 = 开学日 + 14 天 = 2026-09-21
        assert!(ics.contains("DTSTART:20260921T080000"));
        // 大节2（3-4节）10:10-11:50，第 2 周周三 = 2026-09-16
        assert!(ics.contains("DTSTART:20260916T101000"));
        assert!(ics.contains("DTEND:20260916T115000"));
        // SUMMARY/LOCATION/DESCRIPTION
        assert!(ics.contains("SUMMARY:信息安全"));
        assert!(ics.contains("LOCATION:D4-207"));
        assert!(ics.contains("DESCRIPTION:教师 张老师，第 1 周"));
    }

    /// 停开课程不进 ICS。
    #[test]
    fn ics_skips_disabled_courses() {
        let mut tt = fixture();
        tt.courses[1].disabled = true;
        let ics = build_ics(&tt).unwrap();
        assert_eq!(vevent_count(&ics), 2); // 只剩 信息安全 × 2 周
        assert!(!ics.contains("密码学"));
    }

    /// 起始/结束大节超出校本 5 大节表 → 该课程跳过（无时刻可展开）。
    #[test]
    fn ics_skips_sections_outside_block_table() {
        let tt = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-x", "晚自习", 4, 11, 12, vec![1])],
        );
        let ics = build_ics(&tt).unwrap();
        assert_eq!(vevent_count(&ics), 0);
        assert!(ics.contains("END:VCALENDAR"));
    }

    /// 缺少学期开学日期 → 报错（无锚点无法展开日期）。
    #[test]
    fn ics_requires_semester_start_date() {
        let tt = timetable_with(None, vec![course("default-a", "信息安全", 1, 1, 2, vec![1])]);
        let err = build_ics(&tt).unwrap_err();
        assert!(err.contains("请先完成一次导入"));
    }

    /// TEXT 转义 + CRLF 行尾 + VCALENDAR 信封完整。
    #[test]
    fn ics_escapes_text_and_uses_crlf() {
        let mut tt = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-a", "信息安全", 1, 1, 2, vec![1])],
        );
        tt.courses[0].position = "D4,207;东侧".into();
        let ics = build_ics(&tt).unwrap();
        assert!(ics.contains("LOCATION:D4\\,207\\;东侧"));
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
        assert!(!ics.contains("LOCATION:D4,207"));
    }

    /// 写盘 helper：不依赖真实下载目录，在临时目录断言固定文件名、内容与覆盖语义。
    #[test]
    fn write_ics_to_overwrites_at_target_path() {
        let dir = std::env::temp_dir().join(format!("campushub-ics-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_ics_to(&dir, "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n").unwrap();
        assert_eq!(path.file_name().unwrap(), "课表.ics");
        assert!(std::fs::read_to_string(&path).unwrap().contains("BEGIN:VCALENDAR"));
        // 已存在 → 直接覆盖，不加时间戳后缀
        write_ics_to(&dir, "overwritten").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "overwritten");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `semester_start_from_info`：门户 `"YYYYMMDD"` 解析与坏数据防御。
    #[test]
    fn semester_start_parses_portal_format() {
        assert_eq!(
            semester_start_from_info("20260907"),
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap())
        );
        assert_eq!(semester_start_from_info(""), None);
        assert_eq!(semester_start_from_info("202609"), None);
        assert_eq!(semester_start_from_info("20261399"), None);
    }

    /// 手动课程校验：非法入参逐项拒绝。
    #[test]
    fn manual_input_validation_rejects_bad_fields() {
        let ok = ManualCourseInput {
            name: "自习".into(),
            teacher: String::new(),
            position: String::new(),
            day: 6,
            start_section: 1,
            end_section: 2,
            weeks: vec![1, 2],
            color_index: 0,
            remark: None,
        };
        assert!(validate_manual_input(&ok).is_ok());
        let blank = ManualCourseInput { name: "  ".into(), ..ok.clone() };
        assert!(validate_manual_input(&blank).unwrap_err().contains("课程名"));
        let bad_day = ManualCourseInput { day: 8, ..ok.clone() };
        assert!(validate_manual_input(&bad_day).unwrap_err().contains("星期"));
        let bad_sections = ManualCourseInput { start_section: 5, end_section: 4, ..ok.clone() };
        assert!(validate_manual_input(&bad_sections).unwrap_err().contains("节次"));
        let bad_weeks = ManualCourseInput { weeks: vec![], ..ok.clone() };
        assert!(validate_manual_input(&bad_weeks).unwrap_err().contains("周次"));
        let zero_week = ManualCourseInput { weeks: vec![0], ..ok };
        assert!(validate_manual_input(&zero_week).unwrap_err().contains("周次"));
    }

    /// TEXT 转义函数全覆盖。
    #[test]
    fn ics_escape_covers_all_specials() {
        assert_eq!(ics_escape("a\\b;c,d\ne"), "a\\\\b\\;c\\,d\\ne");
        assert_eq!(ics_escape("正常文本"), "正常文本");
    }

    // ---------------- 批次 3：override 幂等与撤销 ----------------

    use campus_schedule::model::OverrideKind;

    fn override_of(notice: &str, course: &str) -> CourseOverride {
        CourseOverride {
            id: fresh_id("ov"),
            course_id: course.into(),
            weeks: vec![3],
            change_type: OverrideKind::Rescheduled,
            new_day: Some(4),
            new_start_section: Some(3),
            new_end_section: Some(4),
            new_position: Some("D4-305".into()),
            source_notice_id: notice.into(),
            auto_applied: false,
        }
    }

    /// 同一 noticeId+courseId 重复采纳 → 覆盖（1 条），不堆叠；不同课程同通知可并存。
    #[test]
    fn upsert_override_idempotent_per_notice_and_course() {
        let mut tt = fixture();
        upsert_override(&mut tt, override_of("manual:aaa", "default-a"));
        upsert_override(&mut tt, override_of("manual:aaa", "default-a"));
        assert_eq!(tt.overrides.len(), 1, "重复采纳应覆盖而非堆叠");
        upsert_override(&mut tt, override_of("manual:aaa", "default-b"));
        assert_eq!(tt.overrides.len(), 2, "同一通知不同课程应并存");
        upsert_override(&mut tt, override_of("manual:bbb", "default-a"));
        assert_eq!(tt.overrides.len(), 3, "不同通知互不影响");
    }

    /// revoke 按 noticeId 整批删除并返回条数；未知 noticeId 返回 0。
    #[test]
    fn revoke_notice_removes_all_by_notice_id() {
        let mut tt = fixture();
        upsert_override(&mut tt, override_of("manual:aaa", "default-a"));
        upsert_override(&mut tt, override_of("manual:aaa", "default-b"));
        upsert_override(&mut tt, override_of("manual:bbb", "default-a"));
        assert_eq!(revoke_by_notice(&mut tt, "manual:aaa"), 2);
        assert_eq!(tt.overrides.len(), 1);
        assert_eq!(revoke_by_notice(&mut tt, "manual:aaa"), 0, "重复撤销幂等");
        assert_eq!(revoke_by_notice(&mut tt, "manual:none"), 0);
        assert_eq!(tt.overrides[0].source_notice_id, "manual:bbb");
    }

    /// 高置信候选 → auto_applied=true；低置信 → false（契约 §2.3 的 autoApplied 区分）。
    #[test]
    fn auto_applied_follows_confidence() {
        let tt = fixture();
        let courses = tt.courses.clone();
        let high = &parse_notice_text("第5周周四3-4节 信息安全 调整到 D4-305", &courses, Some(2))[0];
        let low = &parse_notice_text("第5周 信息安全 调整到 D4-305", &courses, Some(2))[0];
        assert_eq!(high.confidence, NoticeConfidence::High);
        assert_eq!(low.confidence, NoticeConfidence::Low);
        let mut tt = tt;
        upsert_override(
            &mut tt,
            candidate_to_override(high, high.course_id.as_deref().unwrap()),
        );
        upsert_override(&mut tt, candidate_to_override(low, "default-a"));
        assert!(tt.overrides[0].auto_applied);
        assert!(!tt.overrides[1].auto_applied);
        // 字段级拷贝：noticeId 进 source_notice_id，时间字段原样
        assert_eq!(tt.overrides[0].source_notice_id, high.notice_id);
        assert_eq!(tt.overrides[0].new_day, high.new_day);
        assert_eq!(tt.overrides[0].new_position, high.new_position);
    }

    // ---------------- 批次 4：TimetableView 组装 ----------------

    /// 开学日 2026-09-07（周一）+ 今天 2026-09-17（周四）→ 第 2 周（与门户
    /// 「第2周」实测一致）；slots = 校本 5 大节（时间标签唯一事实源）；
    /// today 序列化为 "YYYY-MM-DD"。
    #[test]
    fn timetable_view_assembles_slots_week_and_today() {
        let view = build_timetable_view(
            fixture(),
            NaiveDate::from_ymd_opt(2026, 9, 17).unwrap(),
        );
        assert_eq!(view.current_week, Some(2));
        assert_eq!(view.today, "2026-09-17");
        assert_eq!(view.slots.len(), 5);
        assert_eq!(view.slots[0].number, 1);
        assert_eq!(view.slots[0].start_time, "08:00");
        assert_eq!(view.slots[4].end_time, "20:10");
        assert_eq!(view.timetable.courses.len(), 2);

        // camelCase 序列化键（前端镜像契约）
        let json = serde_json::to_string(&view).unwrap();
        assert!(json.contains("\"currentWeek\":2"));
        assert!(json.contains("\"timetable\":"));
    }

    /// 未配置开学日 → currentWeek=null（前端据此提示先设置开学日）；
    /// 今天越出学期范围（第 0 周）同样为 null。
    #[test]
    fn timetable_view_current_week_none_without_anchor() {
        let view = build_timetable_view(
            timetable_with(None, vec![]),
            NaiveDate::from_ymd_opt(2026, 9, 17).unwrap(),
        );
        assert_eq!(view.current_week, None);

        // 开学 2026-09-07，今天 2026-09-01（开学前）→ None
        let view = build_timetable_view(
            fixture(),
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
        );
        assert_eq!(view.current_week, None);
    }

    // ---------------- 收尾轮：作息时间表编辑 ----------------

    fn slot(number: u8, start: &str, end: &str) -> TimeSlot {
        TimeSlot {
            number,
            start_time: start.into(),
            end_time: end.into(),
            alias: None,
        }
    }

    /// 自定义作息校验：合法样例通过；空 / 超上限 / 重复号 / 倒序 / 0 号 /
    /// 坏格式 / 结束不晚于开始逐项拒绝。
    #[test]
    fn time_slot_validation_covers_bad_inputs() {
        let ok = vec![slot(1, "08:00", "09:40"), slot(2, "10:10", "11:50")];
        assert!(validate_time_slots(&ok).is_ok());

        // 空数组
        assert!(validate_time_slots(&[]).unwrap_err().contains("至少"));
        // 超上限
        let too_many: Vec<TimeSlot> = (1..=21)
            .map(|n| slot(n, "08:00", "09:40"))
            .collect();
        assert!(validate_time_slots(&too_many).unwrap_err().contains("不能超过"));
        // 重复号
        let dup = vec![slot(1, "08:00", "09:40"), slot(1, "10:10", "11:50")];
        assert!(validate_time_slots(&dup).unwrap_err().contains("严格递增"));
        // 倒序
        let reversed = vec![slot(2, "08:00", "09:40"), slot(1, "10:10", "11:50")];
        assert!(validate_time_slots(&reversed).unwrap_err().contains("严格递增"));
        // 0 号
        assert!(validate_time_slots(&[slot(0, "08:00", "09:40")])
            .unwrap_err()
            .contains("正整数"));
        // 坏格式：缺前导零 / 非数字 / 越界分钟 / 非 HH:MM 形态
        for bad in ["8:00", "08:0a", "08:70", "24:00", "0800"] {
            assert!(
                validate_time_slots(&[slot(1, bad, "09:40")])
                    .unwrap_err()
                    .contains("HH:MM"),
                "坏格式 {bad} 应被拒绝"
            );
        }
        // 结束时间不晚于开始
        assert!(validate_time_slots(&[slot(1, "10:10", "09:40")])
            .unwrap_err()
            .contains("晚于"));
        assert!(validate_time_slots(&[slot(1, "09:40", "09:40")])
            .unwrap_err()
            .contains("晚于"));
    }

    /// `parse_hm`：合法 "HH:MM" → 分钟数；越界小时/分钟与坏格式 → None。
    #[test]
    fn parse_hm_accepts_only_valid_clock_time() {
        assert_eq!(parse_hm("08:00"), Some(480));
        assert_eq!(parse_hm("23:59"), Some(1439));
        assert_eq!(parse_hm("24:00"), None);
        assert_eq!(parse_hm("08:60"), None);
        assert_eq!(parse_hm("8:00"), None);
        assert_eq!(parse_hm("0800"), None);
        assert_eq!(parse_hm("08-00"), None);
    }

    /// 生效作息单点取值（契约 §2.3 口径）：`config.slots` 有值且非空 → 自定义；
    /// `None` 或空 → 回落内置校本 5 大节表。
    #[test]
    fn effective_slots_prefers_custom_and_falls_back_to_builtin() {
        let custom = vec![slot(1, "08:30", "10:00"), slot(2, "10:20", "11:50"), slot(3, "14:00", "15:30")];
        let mut tt = timetable_with(None, vec![]);
        tt.config.slots = Some(custom.clone());
        assert_eq!(effective_slots(&tt.config), custom);

        tt.config.slots = None;
        assert_eq!(effective_slots(&tt.config), block_time_slots());

        tt.config.slots = Some(vec![]);
        assert_eq!(effective_slots(&tt.config), block_time_slots(), "空自定义回落内置");
    }

    /// `TimetableView` 组装：带自定义 slots 时行数 = 自定义条数（前端网格按
    /// slots.len() 渲染，不写死 5）。
    #[test]
    fn timetable_view_uses_custom_slots_row_count() {
        let mut tt = timetable_with(Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()), vec![]);
        tt.config.slots = Some(vec![
            slot(1, "08:30", "10:00"),
            slot(2, "10:20", "11:50"),
            slot(3, "14:00", "15:30"),
            slot(4, "15:40", "17:10"),
        ]);
        let view = build_timetable_view(tt, NaiveDate::from_ymd_opt(2026, 9, 17).unwrap());
        assert_eq!(view.slots.len(), 4);
        assert_eq!(view.slots[0].start_time, "08:30");
    }
}
