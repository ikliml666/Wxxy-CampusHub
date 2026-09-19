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
//! - [`save_semester_config`]（2026-09-19 批 1，契约 §7.1）：开学日/总周数/周首日/
//!   显示周末整块保存；`current_week_hint` 有值时由后端反推开学日（口径单点）；
//!   显示约束联动收口在 [`apply_display_constraints`]。
//! - [`save_skipped_dates`]（2026-09-19 批 2，契约 §8.2）：跳过日期整体替换；
//!   [`build_ics`] 经 [`campus_schedule::expand_occurrences`] 按**生效结果**展开
//!   （停课不生成、调课换 UID、补课新增、跳过日剔除、custom 课取自定义时刻）。
//!
//! 统一口径：业务失败一律 `Ok(CommandResult::err(中文消息))`（`Err(String)` 仅限
//! IPC 框架层）；敏感纪律——本模块不输出任何 cookie/TGT/凭据字段。

use super::auth::CommandResult;
use crate::infra::state::AppState;
use crate::infra::{state, timetable};
use campus_portal::block_time_slots;
use campus_schedule::model::{Course, CourseOverride, SlotRule, TimeSlot, Timetable};
use campus_schedule::{
    current_week, diff_courses, expand_occurrences, parse_kb_response, parse_notice_text,
    previous_or_same_day_of_week, semester_start_from_week, OccurrenceKind, NoticeConfidence,
    OverrideKind, Semester,
};
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

/// 生效作息的**单点取值**（冻结契约 §2.3 slots 取值口径；批 3 契约 §9.2 三段回落链）：
/// ① `slot_rules` 首个日期命中的规则（`start_date <= date <= end_date` 含端点；
/// 区间允许重叠、重叠取**先声明**者，对齐上游 firstOrNull）；② 无命中 →
/// `config.slots` 有值且非空 → 用户自定义作息；③ 仍无 → 内置校本大节表
/// [`campus_portal::block_time_slots`]。
/// 网格行（[`TimetableView::slots`]）、ICS 展开、大节号→时间查找必须全部经本函数
/// 取值，不得一处分发一处硬编码。
fn effective_slots_at(
    config: &campus_schedule::model::CourseTableConfig,
    date: NaiveDate,
) -> Vec<TimeSlot> {
    // 规则 slots 恒非空（save_slot_rules 校验）；手改 JSON 的空规则防御性跳过回落
    if let Some(rule) = config
        .slot_rules
        .iter()
        .find(|r| r.start_date <= date && date <= r.end_date && !r.slots.is_empty())
    {
        return rule.slots.clone();
    }
    match config.slots.as_deref() {
        Some(custom) if !custom.is_empty() => custom.to_vec(),
        _ => block_time_slots(),
    }
}

/// 纯函数组装（便于单测）：周次口径与 [`parse_notice`] 一致
/// （`campus_schedule::current_week`）。`slots` 取「今天」的生效作息（契约 §9.4：
/// 跨作息区间的换季周无法逐天变行，与上游周视图同口径的已知取舍，ICS 逐事件
/// 日期精确取值）。
fn build_timetable_view(tt: Timetable, today: chrono::NaiveDate) -> TimetableView {
    TimetableView {
        slots: effective_slots_at(&tt.config, today),
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

/// 展开式 VEVENT 日历文本（不依赖 RRULE：每门未停开课程经
/// [`campus_schedule::expand_occurrences`] 展开其每个教学周的**生效实例**，
/// 只消费 `Solid`——停课不生成、调课原时段不生成而新时段生成、补课新增，
/// 主流日历客户端直接识别。决策 5，契约 §8.5）。
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
    // 周首日对齐（契约 §7.3）：第 1 周首日 = 开学日按 first_day_of_week 回退对齐；
    // firstDay=1 且开学日为周一时与旧公式逐字节等价（golden 保持）。
    let week_first = previous_or_same_day_of_week(start_date, tt.config.first_day_of_week);
    let dtstamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//Wxxy-CampusHub//Timetable//CN".into(),
        "CALSCALE:GREGORIAN".into(),
        "METHOD:PUBLISH".into(),
    ];
    // override 溯源映射（复核 P3-b）：id → 类型，预建一次，防逐事件全局 find
    // 在 id 异常时张冠李戴
    let ov_kind: std::collections::HashMap<&str, OverrideKind> = tt
        .overrides
        .iter()
        .map(|o| (o.id.as_str(), o.change_type))
        .collect();
    for course in &tt.courses {
        if course.disabled {
            continue; // 停开课程不进日历
        }
        // 迭代周次 = course.weeks ∪ 该课各 override.weeks（复核 P1-b）：补课周
        // 可不属于 course.weeks——与前端覆盖面对齐（前端 extra 循环不看出周次）
        let mut weeks: Vec<u32> = course.weeks.clone();
        for ov in tt.overrides.iter().filter(|o| o.course_id == course.id) {
            weeks.extend_from_slice(&ov.weeks);
        }
        weeks.sort();
        weeks.dedup();
        for week in weeks {
            if week == 0 {
                continue;
            }
            // 教学周 → 日期基准列（契约 §7.3）：第 week 周首日 = week_first + (week-1)×7
            let week_start = week_first + chrono::Duration::days(i64::from((week - 1) * 7));
            // override 生效展开（决策 5）：只消费 Solid（停课/调出 ghost 不生成 VEVENT）
            for occ in expand_occurrences(course, &tt.overrides, week) {
                if occ.kind != OccurrenceKind::Solid {
                    continue;
                }
                // 该实例的实际日期：列偏移 col = (day - firstDay + 7) % 7（周首日旋转）
                let col =
                    (u32::from(occ.day) + 7 - u32::from(tt.config.first_day_of_week)) % 7;
                let date = week_start + chrono::Duration::days(i64::from(col));
                // 跳过日期（契约 §8.1）：全校停课日不生成 VEVENT
                if tt.config.skipped_dates.contains(&date) {
                    continue;
                }
                // 时刻取值（契约 §8.5）：节次课查**该日**生效作息（大节 = (s+1)/2
                // 不变，起/止大节任一查不到 → 无时刻可展开，跳过该实例）；
                // custom 课直接取 custom_start_time/custom_end_time（缺失 → 跳过）。
                let (start_hm, end_hm) = match (occ.start_section, occ.end_section) {
                    (Some(s), Some(e)) => {
                        let slots = effective_slots_at(&tt.config, date);
                        let (bs, be) = ((u32::from(s) + 1) / 2, (u32::from(e) + 1) / 2);
                        let (Some(s_slot), Some(e_slot)) = (
                            slots.iter().find(|t| u32::from(t.number) == bs),
                            slots.iter().find(|t| u32::from(t.number) == be),
                        ) else {
                            continue;
                        };
                        (s_slot.start_time.clone(), e_slot.end_time.clone())
                    }
                    _ => {
                        let (Some(cs), Some(ce)) =
                            (&course.custom_start_time, &course.custom_end_time)
                        else {
                            continue;
                        };
                        (cs.clone(), ce.clone())
                    }
                };
                let start_hm = start_hm.replace(':', "");
                let end_hm = end_hm.replace(':', "");
                let day_basic = date.format("%Y%m%d").to_string();
                // override 溯源（P3-b：经预建映射，None = 无 override）
                let kind_of = occ
                    .source_override_id
                    .as_deref()
                    .and_then(|id| ov_kind.get(id).copied());
                let mut desc_parts: Vec<String> = Vec::new();
                if !course.teacher.is_empty() {
                    desc_parts.push(format!("教师 {}", course.teacher));
                }
                desc_parts.push(format!("第 {week} 周"));
                // 调课/补课标注（决策 5）：经 override 溯源判定，普通原位实例不加
                match kind_of {
                    Some(OverrideKind::Rescheduled) => desc_parts.push("调课".into()),
                    Some(OverrideKind::Extra) => desc_parts.push("补课".into()),
                    _ => {}
                }
                if let Some(r) = &course.remark {
                    if !r.is_empty() {
                        desc_parts.push(r.clone());
                    }
                }
                // UID（契约 §8.5 + 复核 P3-a）：`{id}-w{week}d{day}s{start}@campushub`
                // ——原位实例与旧版逐字节一致（golden 保）；**调课新位与补课实例追加
                // `-o{override 短 id}`**，防同周同日同起始小节与原位 UID 逐字节相撞；
                // custom 课（无节次）用 `scustom` 段。
                let sec_tag = occ
                    .start_section
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "custom".into());
                let new_slot =
                    occ.day != course.day || occ.start_section != course.start_section;
                let uid_suffix = occ
                    .source_override_id
                    .as_deref()
                    .filter(|_| match kind_of {
                        // 补课恒加后缀；调课仅新位加（仅换教室的原位 = 同一事件，UID 不变）
                        Some(OverrideKind::Extra) => true,
                        Some(OverrideKind::Rescheduled) => new_slot,
                        _ => false,
                    })
                    .map(|oid| format!("-o{}", oid.chars().take(8).collect::<String>()))
                    .unwrap_or_default();
                lines.extend([
                    "BEGIN:VEVENT".into(),
                    format!(
                        "UID:{}-w{}d{}s{}{}@campushub",
                        ics_escape(&course.id),
                        week,
                        occ.day,
                        sec_tag,
                        uid_suffix
                    ),
                    format!("DTSTAMP:{dtstamp}"),
                    format!("DTSTART:{day_basic}T{start_hm}00"),
                    format!("DTEND:{day_basic}T{end_hm}00"),
                    format!("SUMMARY:{}", ics_escape(&course.name)),
                    format!("LOCATION:{}", ics_escape(&occ.position)),
                    format!("DESCRIPTION:{}", ics_escape(&desc_parts.join("，"))),
                    "END:VEVENT".into(),
                ]);
            }
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

// ---------------- 学期设置与周首日（2026-09-19 批 1，契约 §7） ----------------

/// save_semester_config 入参（契约 §7.1，camelCase；容器级 default 容忍缺省字段）。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SemesterConfigInput {
    /// `None` = 清空开学日（假期态）；被 `current_week_hint` 反推覆盖
    pub semester_start_date: Option<NaiveDate>,
    pub semester_total_weeks: u32,
    /// 一周起始日：1=周一 … 7=周日
    pub first_day_of_week: u8,
    pub show_weekends: bool,
    /// 「今天是第 N 周」手动锚点：有值时后端反推开学日（口径单点，契约 §7.1）
    pub current_week_hint: Option<u32>,
}

/// 显示约束双向联动（契约 §7.2，上游语义）：周日开头必须显示周末；隐藏周末则
/// 周首日回周一。收口在后端 save 单点——前端两处开关各自联动必漏。
fn apply_display_constraints(cfg: &mut campus_schedule::model::CourseTableConfig) {
    if cfg.first_day_of_week == 7 {
        cfg.show_weekends = true;
    }
    if !cfg.show_weekends {
        cfg.first_day_of_week = 1;
    }
}

/// 入参校验（契约 §7.1）：周数 1..=30（上游滚轮范围）、firstDay 1..=7、
/// hint 落在 1..=总周数。
fn validate_semester_input(input: &SemesterConfigInput) -> Result<(), String> {
    if !(1..=30).contains(&input.semester_total_weeks) {
        return Err("学期总周数必须在 1 至 30 之间".to_string());
    }
    if !(1..=7).contains(&input.first_day_of_week) {
        return Err("每周起始日无效（1=周一 … 7=周日）".to_string());
    }
    if let Some(h) = input.current_week_hint {
        if h < 1 || h > input.semester_total_weeks {
            return Err("「今天是第几周」必须在 1 至学期总周数之间".to_string());
        }
    }
    Ok(())
}

/// 学期设置落库纯函数（便于单测）：校验 → 反推/清空开学日 → 套显示约束联动。
fn apply_semester_config(
    tt: &mut Timetable,
    input: &SemesterConfigInput,
    today: NaiveDate,
) -> Result<(), String> {
    validate_semester_input(input)?;
    tt.config.semester_start_date = match input.current_week_hint {
        // 反推放后端（契约 §7.1）：用入参的周首日口径对齐「本周首日」再回退 hint-1 周
        Some(h) => Some(semester_start_from_week(today, h, input.first_day_of_week)),
        None => input.semester_start_date,
    };
    tt.config.semester_total_weeks = input.semester_total_weeks;
    tt.config.first_day_of_week = input.first_day_of_week;
    tt.config.show_weekends = input.show_weekends;
    apply_display_constraints(&mut tt.config);
    Ok(())
}

/// 保存学期设置（契约 §7.1）：返回刷新后的 [`TimetableView`]（前端免二次拉取）。
#[tauri::command]
pub async fn save_semester_config(
    input: SemesterConfigInput,
) -> Result<CommandResult<TimetableView>, String> {
    let dir = state::data_dir()?;
    match mutate_timetable(&dir, |tt| {
        apply_semester_config(tt, &input, chrono::Local::now().date_naive())?;
        Ok(tt.clone())
    }) {
        Ok(tt) => Ok(CommandResult::ok(build_timetable_view(
            tt,
            chrono::Local::now().date_naive(),
        ))),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

// ---------------- 跳过日期（2026-09-19 批 2，契约 §8.1/§8.2） ----------------

/// 保存跳过日期（契约 §8.2 `save_skipped_dates`）：**整体替换**语义（非增量，
/// 前端以完整列表提交）；返回刷新后的 [`TimetableView`]（前端免二次拉取）。
/// 日期合法性由 `NaiveDate` 反序列化保证（非法日期在 IPC 层报错，不落库）；
/// 落库前归一：升序排序 + 去重。
#[tauri::command]
pub async fn save_skipped_dates(
    dates: Vec<NaiveDate>,
) -> Result<CommandResult<TimetableView>, String> {
    let dir = state::data_dir()?;
    match mutate_timetable(&dir, |tt| {
        let mut ds = dates.clone();
        ds.sort();
        ds.dedup();
        tt.config.skipped_dates = ds;
        Ok(tt.clone())
    }) {
        Ok(tt) => Ok(CommandResult::ok(build_timetable_view(
            tt,
            chrono::Local::now().date_naive(),
        ))),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

// ---------------- 按日期生效的作息规则（2026-09-19 批 3，契约 §9） ----------------

/// 作息规则校验（契约 §9.3）：每条 `start_date <= end_date`、`slots` 非空、
/// 且规则内作息复用 [`validate_time_slots`]（号递增 / HH:MM / 结束晚于开始）。
/// 区间是否重叠**不校验**——重叠合法，取先声明者（契约 §9.2）。
fn validate_slot_rules(rules: &[SlotRule]) -> Result<(), String> {
    for (i, rule) in rules.iter().enumerate() {
        let n = i + 1;
        if rule.start_date > rule.end_date {
            return Err(format!("第 {n} 条规则的开始日期必须不晚于结束日期"));
        }
        if rule.slots.is_empty() {
            return Err(format!("第 {n} 条规则的作息至少需要一条"));
        }
        validate_time_slots(&rule.slots).map_err(|e| format!("第 {n} 条规则：{e}"))?;
    }
    Ok(())
}

/// 保存按日期生效的作息规则（契约 §9.3 `save_slot_rules`）：**整体替换**语义；
/// `None` = 清空全部规则（回落主作息/内置）。返回刷新后的 [`TimetableView`]
/// （前端免二次拉取）。
#[tauri::command]
pub async fn save_slot_rules(
    rules: Option<Vec<SlotRule>>,
) -> Result<CommandResult<TimetableView>, String> {
    if let Some(v) = &rules {
        if let Err(e) = validate_slot_rules(v) {
            return Ok(CommandResult::err(&e));
        }
    }
    let dir = state::data_dir()?;
    match mutate_timetable(&dir, |tt| {
        tt.config.slot_rules = rules.clone().unwrap_or_default();
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
                skipped_dates: vec![],
                slot_rules: vec![],
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

    /// 生效作息单点取值（契约 §2.3 口径 + §8.3 签名迁移）：`config.slots` 有值且
    /// 非空 → 自定义；`None` 或空 → 回落内置校本 5 大节表。`date` 参数批 2 暂不
    /// 消费（P3 扩展点），不同日期取值一致由本断言钉住。
    #[test]
    fn effective_slots_at_prefers_custom_and_falls_back_to_builtin() {
        let custom = vec![slot(1, "08:30", "10:00"), slot(2, "10:20", "11:50"), slot(3, "14:00", "15:30")];
        let mut tt = timetable_with(None, vec![]);
        tt.config.slots = Some(custom.clone());
        let d1 = NaiveDate::from_ymd_opt(2026, 9, 17).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        assert_eq!(effective_slots_at(&tt.config, d1), custom);
        assert_eq!(effective_slots_at(&tt.config, d2), custom, "P3 前不同日期同值");

        tt.config.slots = None;
        assert_eq!(effective_slots_at(&tt.config, d1), block_time_slots());

        tt.config.slots = Some(vec![]);
        assert_eq!(effective_slots_at(&tt.config, d1), block_time_slots(), "空自定义回落内置");
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

    // ---------------- 批 1：学期设置、显示约束联动与 ICS 周首日对齐（契约 §7） ----------------

    fn sem_input(
        start: Option<NaiveDate>,
        weeks: u32,
        first_day: u8,
        show_weekends: bool,
        hint: Option<u32>,
    ) -> SemesterConfigInput {
        SemesterConfigInput {
            semester_start_date: start,
            semester_total_weeks: weeks,
            first_day_of_week: first_day,
            show_weekends,
            current_week_hint: hint,
        }
    }

    /// 约束联动双向（契约 §7.2）：firstDay=7 ⇒ 强制显示周末（即使入参 false）；
    /// 隐藏周末 ⇒ firstDay 回周一；其余组合不动。
    #[test]
    fn display_constraints_link_both_directions() {
        let mut cfg = timetable_with(None, vec![]).config;
        cfg.first_day_of_week = 7;
        cfg.show_weekends = false;
        apply_display_constraints(&mut cfg);
        assert!(cfg.show_weekends, "周日开头必须强制显示周末");
        assert_eq!(cfg.first_day_of_week, 7);

        let mut cfg = timetable_with(None, vec![]).config;
        cfg.first_day_of_week = 3;
        cfg.show_weekends = false;
        apply_display_constraints(&mut cfg);
        assert_eq!(cfg.first_day_of_week, 1, "隐藏周末把周首日拉回周一");

        let mut cfg = timetable_with(None, vec![]).config;
        cfg.first_day_of_week = 1;
        cfg.show_weekends = true;
        apply_display_constraints(&mut cfg);
        assert_eq!((cfg.first_day_of_week, cfg.show_weekends), (1, true));
    }

    /// 入参校验（契约 §7.1）：周数与 firstDay 越界、hint 越界逐项拒绝。
    #[test]
    fn semester_input_validation_rejects_out_of_range() {
        assert!(validate_semester_input(&sem_input(None, 0, 1, false, None))
            .unwrap_err()
            .contains("1 至 30"));
        assert!(validate_semester_input(&sem_input(None, 31, 1, false, None))
            .unwrap_err()
            .contains("1 至 30"));
        assert!(validate_semester_input(&sem_input(None, 20, 0, false, None))
            .unwrap_err()
            .contains("每周起始日"));
        assert!(validate_semester_input(&sem_input(None, 20, 8, false, None))
            .unwrap_err()
            .contains("每周起始日"));
        assert!(
            validate_semester_input(&sem_input(None, 20, 1, false, Some(0)))
                .unwrap_err()
                .contains("今天是第几周"),
            "hint=0 应拒绝"
        );
        assert!(
            validate_semester_input(&sem_input(None, 20, 1, false, Some(21)))
                .unwrap_err()
                .contains("今天是第几周"),
            "hint 超出总周数应拒绝"
        );
        assert!(validate_semester_input(&sem_input(None, 30, 7, true, Some(30))).is_ok());
    }

    /// current_week_hint 反推（契约 §7.1）：today=2026-09-17（周四），
    /// hint=2 firstDay=1 → 本周周一 2026-09-07；firstDay=7 → 本周周日 2026-09-13
    /// 回退 1 周 = 2026-09-06。hint 覆盖入参 start_date；hint 空 → 用入参/清空。
    #[test]
    fn apply_semester_config_hint_backfills_start_date() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 17).unwrap();

        let mut tt = timetable_with(None, vec![]);
        apply_semester_config(
            &mut tt,
            &sem_input(None, 20, 1, false, Some(2)),
            today,
        )
        .unwrap();
        assert_eq!(tt.config.semester_start_date, Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()));

        let mut tt = timetable_with(None, vec![]);
        apply_semester_config(
            &mut tt,
            &sem_input(None, 20, 7, true, Some(2)),
            today,
        )
        .unwrap();
        assert_eq!(tt.config.semester_start_date, Some(NaiveDate::from_ymd_opt(2026, 9, 6).unwrap()));

        // hint 有值时覆盖入参 start_date
        let mut tt = timetable_with(None, vec![]);
        apply_semester_config(
            &mut tt,
            &sem_input(Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()), 20, 1, false, Some(2)),
            today,
        )
        .unwrap();
        assert_eq!(tt.config.semester_start_date, Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()));

        // hint 空 + start None → 清空开学日（假期态）；联动照常生效
        let mut tt = timetable_with(Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()), vec![]);
        apply_semester_config(&mut tt, &sem_input(None, 20, 7, false, None), today).unwrap();
        assert_eq!(tt.config.semester_start_date, None);
        assert!(tt.config.show_weekends, "firstDay=7 联动强制显示周末");
        assert_eq!(tt.config.first_day_of_week, 7);
    }

    /// 入参缺省容错（契约 §7.1 容器级 serde default）：空 JSON 反序列化为默认值，
    /// 校验层报中文错误而不是反序列化失败——旧调用方不因新增字段破坏。
    #[test]
    fn semester_input_deserializes_partial_json() {
        let input: SemesterConfigInput = serde_json::from_str("{}").unwrap();
        assert_eq!(input.semester_start_date, None);
        assert_eq!(input.semester_total_weeks, 0);
        assert_eq!(input.current_week_hint, None);
        assert!(validate_semester_input(&input).is_err(), "缺省值应被校验层拒绝");
    }

    /// ICS 周首日对齐（契约 §7.3）：开学日非周一（2026-09-09 周三，firstDay=1）时
    /// 第 1 周周一对齐到 2026-09-07；firstDay=7 时周日课落在周首（2026-09-06）。
    /// firstDay=1 且开学日周一的 golden 由上方 ics_expands_events_with_correct_times 钉住。
    #[test]
    fn ics_aligns_dates_to_first_day_of_week() {
        // 开学日非周一：start 2026-09-09（周三），firstDay=1 → 第 1 周周一 = 09-07
        let mut tt = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 9).unwrap()),
            vec![course("default-a", "信息安全", 1, 1, 2, vec![1])],
        );
        tt.config.first_day_of_week = 1;
        let ics = build_ics(&tt).unwrap();
        assert!(ics.contains("DTSTART:20260907T080000"), "非周一开学日应回退对齐到周首");

        // firstDay=7：start 2026-09-07（周一），day=7（周日）→ 列 0 = 本周周日 09-06
        let mut tt = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-b", "密码学", 7, 1, 2, vec![1])],
        );
        tt.config.first_day_of_week = 7;
        tt.config.show_weekends = true;
        let ics = build_ics(&tt).unwrap();
        assert!(ics.contains("DTSTART:20260906T080000"), "firstDay=7 时周日 = 周首日");

        // 同一配置下周三（day=3）：col=(3-7+7)%7=3 → 09-06+3 = 09-09
        let mut tt = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-c", "数据结构", 3, 1, 2, vec![1])],
        );
        tt.config.first_day_of_week = 7;
        tt.config.show_weekends = true;
        let ics = build_ics(&tt).unwrap();
        assert!(ics.contains("DTSTART:20260909T080000"));
    }

    // ---------------- 批 2：ICS 按生效结果展开（契约 §8.5，决策 5） ----------------

    fn push_override(tt: &mut Timetable, mut o: CourseOverride) {
        o.id = fresh_id("ov");
        tt.overrides.push(o);
    }

    fn cancelled(course_id: &str, week: u32, new_day: Option<u8>) -> CourseOverride {
        CourseOverride {
            id: String::new(),
            course_id: course_id.into(),
            weeks: vec![week],
            change_type: OverrideKind::Cancelled,
            new_day,
            new_start_section: None,
            new_end_section: None,
            new_position: None,
            source_notice_id: "notice-cancel".into(),
            auto_applied: false,
        }
    }

    /// 停课（new_day = None 整周全停）→ 该周 VEVENT 消失，其他周保留。
    #[test]
    fn ics_drops_cancelled_week_events() {
        let mut tt = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-a", "信息安全", 1, 1, 2, vec![1, 5])],
        );
        push_override(&mut tt, cancelled("default-a", 5, None));
        let ics = build_ics(&tt).unwrap();
        assert_eq!(vevent_count(&ics), 1, "第 5 周停课 → 只剩第 1 周");
        assert!(ics.contains("UID:default-a-w1d1s1@campushub"));
        assert!(!ics.contains("default-a-w5"));
    }

    /// 调课 → 原时段 VEVENT 消失、新时段出现（新 UID/新日期/新教室，DESCRIPTION 追加「调课」）。
    #[test]
    fn ics_rescheduled_event_moves_to_new_uid() {
        let mut tt = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-a", "信息安全", 1, 1, 2, vec![5])],
        );
        push_override(&mut tt, CourseOverride {
            id: String::new(),
            course_id: "default-a".into(),
            weeks: vec![5],
            change_type: OverrideKind::Rescheduled,
            new_day: Some(4),
            new_start_section: Some(3),
            new_end_section: Some(4),
            new_position: Some("D4-305".into()),
            source_notice_id: "notice-move".into(),
            auto_applied: false,
        });
        let ics = build_ics(&tt).unwrap();
        assert_eq!(vevent_count(&ics), 1);
        // 原 UID（w5d1s1）消失；新 UID 按契约 §8.5（复核 P3-a）= w{week}d{newDay}s{newStart}-o{短id}
        assert!(!ics.contains("UID:default-a-w5d1s1@campushub"));
        assert!(!ics.contains("UID:default-a-w5d1s1@"));
        assert!(ics.contains("UID:default-a-w5d4s3-o"), "调课新位 UID 带 override 短 id 后缀");
        // 第 5 周周四 = 2026-10-08，大节 2（3-4 节）10:10-11:50
        assert!(ics.contains("DTSTART:20261008T101000"));
        assert!(ics.contains("DTEND:20261008T115000"));
        assert!(ics.contains("LOCATION:D4-305"));
        assert!(ics.contains("DESCRIPTION:教师 张老师，第 5 周，调课"));
    }

    /// 补课 → 新增 VEVENT（DESCRIPTION 追加「补课」），原时段照常。
    #[test]
    fn ics_extra_appends_new_event() {
        let mut tt = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-a", "信息安全", 1, 1, 2, vec![5])],
        );
        push_override(&mut tt, CourseOverride {
            id: String::new(),
            course_id: "default-a".into(),
            weeks: vec![5],
            change_type: OverrideKind::Extra,
            new_day: Some(6),
            new_start_section: Some(5),
            new_end_section: Some(6),
            new_position: None,
            source_notice_id: "notice-extra".into(),
            auto_applied: false,
        });
        let ics = build_ics(&tt).unwrap();
        assert_eq!(vevent_count(&ics), 2);
        // 第 5 周周六 = 2026-10-10，大节 3（5-6 节）13:45-15:25；教室缺省沿用原教室；
        // 补课 UID 带 override 短 id 后缀（复核 P3-a）
        assert!(ics.contains("UID:default-a-w5d6s5-o"));
        assert!(ics.contains("DTSTART:20261010T134500"));
        assert!(ics.contains("DESCRIPTION:教师 张老师，第 5 周，补课"));
        assert!(ics.contains("UID:default-a-w5d1s1@campushub"));
    }

    /// ③ 补课周 ∉ course.weeks → ICS 仍有该事件（复核 P1-b：迭代周次取
    ///    course.weeks ∪ override.weeks，与前端覆盖面对齐）。
    #[test]
    fn ics_extra_week_outside_course_weeks_emits_event() {
        let mut tt = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-a", "信息安全", 1, 1, 2, vec![5])],
        );
        push_override(&mut tt, CourseOverride {
            id: String::new(),
            course_id: "default-a".into(),
            weeks: vec![6],
            change_type: OverrideKind::Extra,
            new_day: Some(3),
            new_start_section: Some(3),
            new_end_section: Some(4),
            new_position: None,
            source_notice_id: "notice-extra".into(),
            auto_applied: false,
        });
        let ics = build_ics(&tt).unwrap();
        assert_eq!(vevent_count(&ics), 2, "第 5 周原课 + 第 6 周补课");
        // 第 6 周周三 = 开学日 09-07 + 35 天 + 2 = 2026-10-14，大节 2 10:10 起
        assert!(ics.contains("UID:default-a-w6d3s3-o"));
        assert!(ics.contains("DTSTART:20261014T101000"));
        assert!(ics.contains("DESCRIPTION:教师 张老师，第 6 周，补课"));
    }

    /// ⑥ 补课与原位实例同日同起始节次 → UID 不重复（补课带 -o 后缀）。
    #[test]
    fn ics_extra_same_slot_uids_distinct() {
        let mut tt = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-a", "信息安全", 1, 1, 2, vec![5])],
        );
        push_override(&mut tt, CourseOverride {
            id: String::new(),
            course_id: "default-a".into(),
            weeks: vec![5],
            change_type: OverrideKind::Extra,
            new_day: Some(1),
            new_start_section: Some(1),
            new_end_section: Some(2),
            new_position: None,
            source_notice_id: "notice-extra".into(),
            auto_applied: false,
        });
        let ics = build_ics(&tt).unwrap();
        assert_eq!(vevent_count(&ics), 2, "原位 + 同位补课并存");
        assert!(ics.contains("UID:default-a-w5d1s1@campushub"), "原位 UID 不变");
        assert!(ics.contains("UID:default-a-w5d1s1-o"), "补课 UID 加后缀不撞原位");
    }

    /// ④⑤ 调课结束节次缺省 = 起始大节高（start=5 → 大节 3，13:45-15:25）；
    ///    firstDay=7 时调课新位的日期按周首日旋转落列（第 1 周周四 = 周日周首
    ///    2026-09-06 + 4 列 = 2026-09-10）。
    #[test]
    fn ics_resched_end_default_and_first_day_7_placement() {
        let mut tt = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-a", "信息安全", 1, 1, 2, vec![1])],
        );
        tt.config.first_day_of_week = 7;
        tt.config.show_weekends = true;
        push_override(&mut tt, CourseOverride {
            id: String::new(),
            course_id: "default-a".into(),
            weeks: vec![1],
            change_type: OverrideKind::Rescheduled,
            new_day: Some(4),
            new_start_section: Some(5),
            new_end_section: None, // 单节补调：结束 = 起始 → 大节 3
            new_position: None,
            source_notice_id: "notice-move".into(),
            auto_applied: false,
        });
        let ics = build_ics(&tt).unwrap();
        assert_eq!(vevent_count(&ics), 1);
        assert!(ics.contains("UID:default-a-w1d4s5-o"));
        assert!(ics.contains("DTSTART:20260910T134500"), "firstDay=7 旋转后周四 = 09-10");
        assert!(ics.contains("DTEND:20260910T152500"), "结束缺省 = 起始大节高（大节 3）");
    }

    /// ⑦ custom 课被 resched → 新位走大节表取时刻（P3-c 口径），UID 带 scustom 之外
    ///    的节次段 + 后缀；skipped_dates 恰为调课新日期 → 该 VEVENT 剔除；
    ///    course.weeks 空数组 + extra → 补课事件照常生成。
    #[test]
    fn ics_custom_resched_skipped_new_date_and_empty_weeks() {
        // custom + resched：新位 day 2 节次 7-8 → 大节 4 15:35-17:15
        let mut c = course("default-c", "科研例会", 3, 1, 2, vec![1]);
        c.is_custom_time = true;
        c.start_section = None;
        c.end_section = None;
        c.custom_start_time = Some("18:00".into());
        c.custom_end_time = Some("19:30".into());
        let mut tt = timetable_with(Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()), vec![c]);
        push_override(&mut tt, CourseOverride {
            id: String::new(),
            course_id: "default-c".into(),
            weeks: vec![1],
            change_type: OverrideKind::Rescheduled,
            new_day: Some(2),
            new_start_section: Some(7),
            new_end_section: Some(8),
            new_position: None,
            source_notice_id: "notice-move".into(),
            auto_applied: false,
        });
        let ics = build_ics(&tt).unwrap();
        assert_eq!(vevent_count(&ics), 1);
        assert!(ics.contains("UID:default-c-w1d2s7-o"));
        assert!(ics.contains("DTSTART:20260908T153500"), "resched 后走大节表而非 custom 时刻");
        assert!(!ics.contains("T180000"), "原 custom 时段不生成");

        // skipped_dates 恰为调课新日期（2026-10-08 = 第 5 周周四）→ VEVENT 剔除
        let mut tt2 = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-a", "信息安全", 1, 1, 2, vec![5])],
        );
        push_override(&mut tt2, CourseOverride {
            id: String::new(),
            course_id: "default-a".into(),
            weeks: vec![5],
            change_type: OverrideKind::Rescheduled,
            new_day: Some(4),
            new_start_section: Some(3),
            new_end_section: Some(4),
            new_position: None,
            source_notice_id: "notice-move".into(),
            auto_applied: false,
        });
        tt2.config.skipped_dates = vec![NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()];
        let ics = build_ics(&tt2).unwrap();
        assert_eq!(vevent_count(&ics), 0, "调课新位命中跳过日 → 无事件");

        // course.weeks 为空数组 + extra override → 补课事件照常
        let mut tt3 = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-d", "讲座", 5, 1, 2, vec![])],
        );
        push_override(&mut tt3, CourseOverride {
            id: String::new(),
            course_id: "default-d".into(),
            weeks: vec![2],
            change_type: OverrideKind::Extra,
            new_day: Some(5),
            new_start_section: Some(9),
            new_end_section: Some(10),
            new_position: None,
            source_notice_id: "notice-extra".into(),
            auto_applied: false,
        });
        let ics = build_ics(&tt3).unwrap();
        assert_eq!(vevent_count(&ics), 1);
        assert!(ics.contains("UID:default-d-w2d5s9-o"));
        assert!(ics.contains("DTSTART:20260918T183000"), "第 2 周周五 = 09-18，大节 5 18:30 起");
    }

    /// 跳过日期 → 命中日期的 VEVENT 剔除（该实例），其他日期不受影响。
    #[test]
    fn ics_skips_events_on_skipped_dates() {
        let mut tt = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-a", "信息安全", 1, 1, 2, vec![1, 3])],
        );
        // 第 3 周周一 = 2026-09-21 标记为跳过
        tt.config.skipped_dates = vec![NaiveDate::from_ymd_opt(2026, 9, 21).unwrap()];
        let ics = build_ics(&tt).unwrap();
        assert_eq!(vevent_count(&ics), 1, "10-01 类停课日 → 该日 VEVENT 剔除");
        assert!(ics.contains("DTSTART:20260907T080000"));
        assert!(!ics.contains("20260921"));
    }

    /// custom 课（is_custom_time，节次 None）→ DTSTART/DTEND 直接取 custom 时刻，
    /// UID 用 `scustom` 段（决策 5：替掉旧版对无节次课程的整体 continue）。
    #[test]
    fn ics_custom_course_uses_custom_times() {
        let mut c = course("default-c", "科研例会", 3, 1, 2, vec![1]);
        c.is_custom_time = true;
        c.start_section = None;
        c.end_section = None;
        c.custom_start_time = Some("18:00".into());
        c.custom_end_time = Some("19:30".into());
        let tt = timetable_with(Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()), vec![c]);
        let ics = build_ics(&tt).unwrap();
        assert_eq!(vevent_count(&ics), 1);
        assert!(ics.contains("UID:default-c-w1d3scustom@campushub"));
        assert!(ics.contains("DTSTART:20260909T180000"));
        assert!(ics.contains("DTEND:20260909T193000"));
    }

    /// 旧 JSON（无 skippedDates 键）反序列化无损 → 空列表，ICS 行为与旧版一致。
    #[test]
    fn legacy_timetable_json_without_skipped_dates_opens_clean() {
        let json = r#"{
            "config": { "courseTableId": "default", "semesterStartDate": "2026-09-07",
                        "semesterTotalWeeks": 20, "firstDayOfWeek": 1 },
            "courses": [], "overrides": [], "updatedAt": ""
        }"#;
        let tt: Timetable = serde_json::from_str(json).unwrap();
        assert!(tt.config.skipped_dates.is_empty());
        assert!(tt.config.slots.is_none());
        // 反序列化后 ICS 链路照常工作（空课程 → 0 VEVENT，不因缺键失败）
        let ics = build_ics(&tt).unwrap();
        assert_eq!(vevent_count(&ics), 0);
        let tt_ok = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-a", "信息安全", 1, 1, 2, vec![1])],
        );
        let ics = build_ics(&tt_ok).unwrap();
        assert!(ics.contains("UID:default-a-w1d1s1@campushub"), "旧数据 UID/时刻 golden 不变");
    }

    /// save_skipped_dates 落库归一：升序 + 去重（mutate 骨架内联逻辑的单测等价验证）。
    #[test]
    fn skipped_dates_normalization_sorts_and_dedups() {
        let mut dates = vec![
            NaiveDate::from_ymd_opt(2026, 10, 8).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        ];
        dates.sort();
        dates.dedup();
        assert_eq!(
            dates,
            vec![
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 8).unwrap(),
            ]
        );
    }

    // ---------------- 批 3：slot_rules 区间命中与三段回落链（契约 §9） ----------------

    fn d(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn slot_rule(start: &str, end: &str, slots: Vec<TimeSlot>) -> SlotRule {
        SlotRule { start_date: d(start), end_date: d(end), slots }
    }

    fn tt_with_slots_and_rules(
        slots: Option<Vec<TimeSlot>>,
        rules: Vec<SlotRule>,
    ) -> campus_schedule::model::CourseTableConfig {
        let mut cfg = timetable_with(None, vec![]).config;
        cfg.slots = slots;
        cfg.slot_rules = rules;
        cfg
    }

    /// 区间含端点命中（契约 §9.2）：`start_date <= date <= end_date` 两端都算；
    /// 区间外回落。冬季规则无 `config.slots` 时区间外进一步回落内置。
    #[test]
    fn effective_slots_at_rule_hit_inclusive_endpoints() {
        let winter = vec![slot(1, "09:00", "10:40")];
        let cfg = tt_with_slots_and_rules(
            None,
            vec![slot_rule("2026-12-01", "2027-02-28", winter.clone())],
        );
        assert_eq!(effective_slots_at(&cfg, d("2026-12-01")), winter, "命中起始日（含）");
        assert_eq!(effective_slots_at(&cfg, d("2027-02-28")), winter, "命中结束日（含）");
        assert_eq!(effective_slots_at(&cfg, d("2026-11-30")), block_time_slots(), "区间外回落内置");
        assert_eq!(effective_slots_at(&cfg, d("2027-03-01")), block_time_slots(), "区间后回落内置");
    }

    /// 重叠区间取先声明者（契约 §9.2，对齐上游 firstOrNull）：A 先声明覆盖 B。
    #[test]
    fn effective_slots_at_overlapping_rules_take_first_declared() {
        let a = vec![slot(1, "08:00", "09:00")];
        let b = vec![slot(1, "10:00", "11:00")];
        let cfg = tt_with_slots_and_rules(
            None,
            vec![
                slot_rule("2026-01-01", "2026-06-30", a.clone()),
                slot_rule("2026-03-01", "2026-05-01", b),
            ],
        );
        assert_eq!(effective_slots_at(&cfg, d("2026-04-01")), a, "重叠区取先声明");
        assert_eq!(effective_slots_at(&cfg, d("2026-02-01")), a);
    }

    /// 三段回落链（契约 §9.2）：rules 命中 → config.slots → 内置；空规则列表、
    /// 命中区间外的日期、以及手改 JSON 的空 slots 规则都正确逐段回落。
    #[test]
    fn effective_slots_at_fallback_chain_rules_then_slots_then_builtin() {
        let custom = vec![slot(1, "08:30", "10:00")];
        let winter = vec![slot(1, "09:00", "10:40")];

        // 空规则列表 + 自定义主作息 → 主作息（第二段）
        let cfg = tt_with_slots_and_rules(Some(custom.clone()), vec![]);
        assert_eq!(effective_slots_at(&cfg, d("2026-12-15")), custom);

        // 有规则但日期未命中 → 主作息
        let cfg = tt_with_slots_and_rules(
            Some(custom.clone()),
            vec![slot_rule("2026-12-01", "2027-02-28", winter.clone())],
        );
        assert_eq!(effective_slots_at(&cfg, d("2026-10-01")), custom);
        assert_eq!(effective_slots_at(&cfg, d("2026-12-15")), winter, "命中 → 规则（第一段）");

        // 未命中 + 无主作息 → 内置（第三段）
        let cfg = tt_with_slots_and_rules(
            None,
            vec![slot_rule("2026-12-01", "2027-02-28", winter.clone())],
        );
        assert_eq!(effective_slots_at(&cfg, d("2026-10-01")), block_time_slots());

        // 防御：手改 JSON 的空 slots 规则不算命中 → 继续回落主作息
        let cfg = tt_with_slots_and_rules(Some(custom.clone()), vec![slot_rule("2026-12-01", "2027-02-28", vec![])]);
        assert_eq!(effective_slots_at(&cfg, d("2026-12-15")), custom);
    }

    /// 旧 JSON（无 slotRules 键）反序列化无损 → 空列表，取值链为
    /// config.slots → 内置，与批 2 行为逐字节一致。
    #[test]
    fn legacy_timetable_json_without_slot_rules_opens_clean() {
        let json = r#"{
            "config": { "courseTableId": "default", "semesterStartDate": "2026-09-07",
                        "semesterTotalWeeks": 20, "firstDayOfWeek": 1,
                        "slots": [{"number": 1, "startTime": "08:30", "endTime": "10:00"}] },
            "courses": [], "overrides": [], "updatedAt": ""
        }"#;
        let tt: Timetable = serde_json::from_str(json).unwrap();
        assert!(tt.config.slot_rules.is_empty());
        assert_eq!(
            effective_slots_at(&tt.config, d("2026-12-15")),
            tt.config.slots.clone().unwrap(),
            "无规则 → 主作息（回落链第二段）"
        );
    }

    /// TimetableView.slots 取「今天」生效作息（契约 §9.4 已知取舍）：同一课表在
    /// 换季日前后组装，slots 行数与起始时刻随 today 切换（网格行数随规则变化）。
    #[test]
    fn timetable_view_slots_follow_today_across_slot_rules() {
        let mut tt = timetable_with(Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()), vec![]);
        tt.config.slots = Some(vec![
            slot(1, "08:00", "09:40"),
            slot(2, "10:10", "11:50"),
            slot(3, "14:00", "15:30"),
            slot(4, "15:40", "17:10"),
        ]);
        tt.config.slot_rules = vec![slot_rule(
            "2026-12-01",
            "2027-02-28",
            vec![slot(1, "09:00", "10:40"), slot(2, "10:50", "12:20"), slot(3, "14:30", "16:00")],
        )];
        let before = build_timetable_view(tt.clone(), d("2026-11-30"));
        assert_eq!(before.slots.len(), 4, "换季前 → 主作息 4 行");
        assert_eq!(before.slots[0].start_time, "08:00");
        let after = build_timetable_view(tt, d("2026-12-15"));
        assert_eq!(after.slots.len(), 3, "换季后 → 冬季规则 3 行");
        assert_eq!(after.slots[0].start_time, "09:00");
    }

    /// save_slot_rules 入参校验（契约 §9.3）：start>end / 空 slots / 规则内非法
    /// 时刻逐项拒绝（带规则序号）；重叠区间合法（不校验，取先声明）。
    #[test]
    fn slot_rule_validation_rejects_bad_rules() {
        let ok = slot_rule("2026-12-01", "2027-02-28", vec![slot(1, "09:00", "10:40")]);
        assert!(validate_slot_rules(&[ok.clone()]).is_ok());

        // start > end
        let reversed = slot_rule("2027-02-28", "2026-12-01", vec![slot(1, "09:00", "10:40")]);
        assert!(validate_slot_rules(&[reversed]).unwrap_err().contains("开始日期必须不晚于结束日期"));
        // 空 slots
        let empty = slot_rule("2026-12-01", "2027-02-28", vec![]);
        assert!(validate_slot_rules(&[empty]).unwrap_err().contains("至少需要一条"));
        // 规则内非法时刻（复用 validate_time_slots，报错带规则序号）
        let bad_time = slot_rule("2026-12-01", "2027-02-28", vec![slot(1, "9:00", "10:40")]);
        let err = validate_slot_rules(&[bad_time]).unwrap_err();
        assert!(err.contains("第 1 条规则"), "报错应带规则序号：{err}");
        assert!(err.contains("HH:MM"));
        // 第二条规则出错时序号正确
        let bad_second = slot_rule("2026-12-01", "2027-02-28", vec![slot(1, "10:40", "09:00")]);
        assert!(validate_slot_rules(&[ok.clone(), bad_second])
            .unwrap_err()
            .contains("第 2 条规则"));
        // 重叠区间合法（命中语义取先声明，校验不拒绝）
        let overlap = slot_rule("2026-01-01", "2026-06-30", vec![slot(1, "08:00", "09:00")]);
        assert!(validate_slot_rules(&[ok, overlap]).is_ok());
    }

    /// 建规则后 ICS 时刻随日期切换（蓝图批 3 验收）：换季日前的 VEVENT 用内置
    /// 作息、之后的 VEVENT 用规则作息——ICS 逐 VEVENT 日期经 effective_slots_at
    /// 取值（收敛点），网格与今日页自动获得同一语义。
    #[test]
    fn ics_event_times_follow_slot_rules_by_date() {
        let mut tt = timetable_with(
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            vec![course("default-a", "信息安全", 1, 1, 2, vec![1, 13])],
        );
        // 冬季作息：第 1 大节 09:00-10:40（与内置 08:00-09:40 可区分）
        tt.config.slot_rules = vec![slot_rule(
            "2026-11-01",
            "2027-02-28",
            vec![slot(1, "09:00", "10:40")],
        )];
        let ics = build_ics(&tt).unwrap();
        assert_eq!(vevent_count(&ics), 2);
        // 第 1 周周一 = 2026-09-07（换季前）→ 内置大节 1：08:00
        assert!(ics.contains("DTSTART:20260907T080000"));
        // 第 13 周周一 = 2026-11-30（换季后）→ 规则大节 1：09:00
        assert!(ics.contains("DTSTART:20261130T090000"));
        assert!(ics.contains("DTEND:20261130T104000"));
        assert!(!ics.contains("DTSTART:20261130T080000"), "换季后不得再用内置时刻");
    }
}
