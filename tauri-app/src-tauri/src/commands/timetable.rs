//! 课表命令面（M2.5：批次 1 本地读取；批次 2 导入对比 / 手动增删改 / ICS 导出）。
//!
//! - [`get_timetable`]：纯本地读取（无网络、无需登录态），缺失/损坏 → 空课表。
//! - [`import_timetable`]：需登录。门户学期信息推导 `xnm`/`xqm`（冻结契约 §1.2：
//!   `xnm` = `start_date` 前 4 位、`semester` `"1"→3 / "2"→12`；**不用 `grade`**
//!  ——它是入学年级）→ 教务拉 JSON（901 → TGT 静默重进由 campus-auth 内部处理）→
//!   `parse_kb_response` → [`campus_schedule::diff_courses`] 合并旧库 → 落库。
//! - 手动课程三命令：本地数据操作（无需登录）；`source=Manual` 的课程不参与
//!   导入 diff（冻结契约 §2.4）。
//! - [`export_ics`]：展开式 VEVENT 文本（**不落盘**，前端 Blob 下载）。时间取
//!   校本大节作息 `campus_portal::block_time_slots`（与今日页同一事实来源），
//!   日期由 `semester_start_date` + 周次 + 星期推出。
//!
//! 统一口径：业务失败一律 `Ok(CommandResult::err(中文消息))`（`Err(String)` 仅限
//! IPC 框架层）；敏感纪律——本模块不输出任何 cookie/TGT/凭据字段。

use super::auth::CommandResult;
use crate::infra::state::AppState;
use crate::infra::{state, timetable};
use campus_portal::block_time_slots;
use campus_schedule::model::{Course, Timetable};
use campus_schedule::{diff_courses, parse_kb_response, Semester};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use tauri::State;

/// 无会话时的约定文案（与 profile.rs / portal.rs 同口径）。
const ERR_NO_SESSION: &str = "请先登录";

/// 本地课表读取（无入参）。
#[tauri::command]
pub async fn get_timetable() -> Result<CommandResult<Timetable>, String> {
    let dir = state::data_dir()?;
    Ok(CommandResult::ok(timetable::load_timetable(&dir)))
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

/// 手动课程 id：`manual-<纳秒时间戳>`。与导入课程 `<table_id>-<jxb_id>` 前缀
/// 不同（永不冲突）；创建后即固定，且 Manual 不参与 diff，id 天然稳定。
fn new_manual_id() -> String {
    let n = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("manual-{n}")
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
    let slots = block_time_slots();
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

/// 导出 ICS 文本（不落盘，前端 Blob 下载）。
#[tauri::command]
pub async fn export_ics() -> Result<CommandResult<String>, String> {
    let dir = state::data_dir()?;
    let tt = timetable::load_timetable(&dir);
    match build_ics(&tt) {
        Ok(text) => Ok(CommandResult::ok(text)),
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
}
