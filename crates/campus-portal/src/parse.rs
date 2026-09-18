//! 门户响应解析纯函数（`parse_*(&str) -> Result<T>`），离线单测用脱敏 fixture。
//!
//! 两种响应信封，解析器不混用（计划 §1.1）：
//! - 门户自身服务：`{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":...}`
//!   —— 本模块三个接口全部属此信封；
//! - 日程服务 `bs-schedule/*`（`code=="0"`）：M2 批次 3 再支持。

use crate::{CourseBrief, PortalError, SemesterInfo, WalletSummary};
use campus_schedule::TimeSlot;

/// 门户统一信封校验：`meta.success==true` 时返回 `data` 引用，否则 Err（保留服务端 message）。
fn envelope_data<'a>(
    v: &'a serde_json::Value,
    api: &str,
) -> Result<&'a serde_json::Value, PortalError> {
    let success = v
        .get("meta")
        .and_then(|m| m.get("success"))
        .and_then(|s| s.as_bool());
    if success != Some(true) {
        let msg = v
            .get("meta")
            .and_then(|m| m.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("服务端未返回原因");
        return Err(PortalError::Parse(format!("{api} 失败: {msg}")));
    }
    v.get("data")
        .ok_or_else(|| PortalError::Parse(format!("{api} 响应缺少 data")))
}

/// 宽松取字符串：字符串原样，整数转字符串（实测字段以字符串为主，容数字形态）。
fn jstr(v: &serde_json::Value) -> Option<String> {
    v.as_str()
        .map(str::to_string)
        .or_else(|| v.as_i64().map(|n| n.to_string()))
}

/// 宽松取数字：数字原样，字符串数字 trim 后解析。
fn jnum(v: &serde_json::Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
}

/// 解析 `api/upp/config/querySemesterInfo` 响应（纯函数供离线单测）。
///
/// `data` 七个字段（grade/semester/currentWeek/weekCount/startDate/endDate/
/// currentWeekDay）任一缺失或类型异常 → [`PortalError::Parse`]（上层把 semester
/// 置 None，不阻塞其余字段）；`currentDate` 不在 IPC 契约内，忽略。
pub fn parse_semester_info(body: &str) -> Result<SemesterInfo, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("querySemesterInfo 响应解析失败: {e}")))?;
    let d = envelope_data(&v, "querySemesterInfo")?;
    let s = |key: &str| -> Result<String, PortalError> {
        let raw = d
            .get(key)
            .ok_or_else(|| PortalError::Parse(format!("querySemesterInfo 缺少 {key}")))?;
        jstr(raw)
            .ok_or_else(|| PortalError::Parse(format!("querySemesterInfo 字段 {key} 类型异常")))
    };
    Ok(SemesterInfo {
        grade: s("grade")?,
        semester: s("semester")?,
        current_week: s("currentWeek")?,
        week_count: s("weekCount")?,
        start_date: s("startDate")?,
        end_date: s("endDate")?,
        current_week_day: s("currentWeekDay")?,
    })
}

/// 解析钱包卡 `api/upp/contentDisplay/queryAppointCard/<cardId>` 响应（纯函数供离线单测）。
///
/// 实测形态：`data.data` 是 **JSON 字符串**，二次解析后为数组，取首项
/// `YE`（余额）/`SL`（在借图书）/`mailNewCount`（未读邮件）。`loginUrl`
/// （邮箱免密链接，内含 authkey）**直接丢弃**——结构体不定义该字段。
/// 单字段缺失/类型异常 → 该项 `None`（部分成功）；信封失败 / `data.data` 非字符串 /
/// 内层数组为空 → [`PortalError::Parse`]。
pub fn parse_wallet_summary(body: &str) -> Result<WalletSummary, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("钱包卡响应解析失败: {e}")))?;
    let d = envelope_data(&v, "钱包卡")?;
    let inner = d
        .get("data")
        .and_then(|x| x.as_str())
        .ok_or_else(|| PortalError::Parse("钱包卡响应缺少 data.data 字符串".to_string()))?;
    let items: serde_json::Value = serde_json::from_str(inner)
        .map_err(|e| PortalError::Parse(format!("钱包卡内层 JSON 解析失败: {e}")))?;
    let first = items
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| PortalError::Parse("钱包卡无数据".to_string()))?;
    // 字段级宽松：缺失/类型异常只影响自身项，不影响其余
    let balance = first.get("YE").and_then(jnum);
    let books = first.get("SL").and_then(jnum).map(|n| n.max(0.0) as u32);
    let mail = first
        .get("mailNewCount")
        .and_then(jnum)
        .map(|n| n.max(0.0) as u32);
    Ok(WalletSummary {
        card_balance: balance,
        book_borrowed: books,
        mail_unread: mail,
    })
}

/// `queryAWeekSchedule` 原始形态（解析结果供「下一节课」计算）。
#[derive(Debug, Clone, PartialEq)]
pub struct WeekSchedule {
    /// 服务端已按当前周返回的矩阵：行=周一..周日；10 列 = **5 大节 × 2 小节**
    ///（列对 (0,1)→大节1 … (8,9)→大节5，与 swskjc+xwskjc+wsskjc=4+4+2 吻合，
    /// 实测四门课逐条验证列对→大节映射），一门课占同一大节的相邻两列；
    /// 空串=无课。
    pub grid: Vec<Vec<String>>,
    /// 今天星期几（服务端 `xqj`，1=周一 … 7=周日）。
    pub weekday: u8,
}

/// 解析 `api/uppcard/kbsz/queryAWeekSchedule` 响应（纯函数供离线单测）。
///
/// 只取「下一节课」所需最小字段：`resultsJsonArr` 矩阵与 `xqj`（周次/学期等
/// 其余字段用不上，忽略）。格子为 `"课程名,教室,教学班,姓名"`，此处不拆分
/// （拆分见 [`next_course`]）。
pub fn parse_week_schedule(body: &str) -> Result<WeekSchedule, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("周课表响应解析失败: {e}")))?;
    let d = envelope_data(&v, "周课表")?;
    let rows = d
        .get("resultsJsonArr")
        .and_then(|x| x.as_array())
        .ok_or_else(|| PortalError::Parse("周课表响应缺少 resultsJsonArr".to_string()))?;
    if rows.is_empty() {
        return Err(PortalError::Parse("周课表矩阵为空".to_string()));
    }
    let grid = rows
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or_else(|| PortalError::Parse("周课表矩阵行结构异常".to_string()))
                .map(|cells| {
                    cells
                        .iter()
                        .map(|c| c.as_str().unwrap_or_default().to_string())
                        .collect::<Vec<_>>()
                })
        })
        .collect::<Result<Vec<_>, PortalError>>()?;
    let weekday = d
        .get("xqj")
        .and_then(|x| x.as_u64().or_else(|| x.as_str()?.trim().parse().ok()))
        .unwrap_or(0) as u8;
    Ok(WeekSchedule { grid, weekday })
}

/// 校本「大节」作息表（100 分钟/大节；矩阵 10 列 = 5 大节 × 2 小节）。
///
/// 时间锚定 2026-09-18 主智能体日程服务实测（`findScheduleBetweenTime` 的
/// `Default-class` 事件真实上课时间）：大节2 10:10-11:50、大节3 13:45-15:25 为
/// 实测值；其余档位为推算，M2.5 校本化作息时校准。**不改动
/// `campus-schedule::default_time_slots()`**（上游默认值，被金标测试钉住）。
fn block_time_slots() -> Vec<TimeSlot> {
    const RAW: [(u8, &str, &str); 5] = [
        // ponytail: 未实测——由大节2 10:10 开始反推（30 分钟大课间）；M2.5 校准
        (1, "08:00", "09:40"),
        // 2026-09-18 日程服务实测（信息隐藏与取证技术 10:10→11:50）
        (2, "10:10", "11:50"),
        // 2026-09-18 日程服务实测（信息安全等 13:45→15:25）
        (3, "13:45", "15:25"),
        // ponytail: 未实测——由大节3 结束 15:25 + 10 分钟课间推出；M2.5 校准
        (4, "15:35", "17:15"),
        // ponytail: 未实测——晚上档位占位；M2.5 校准
        (5, "18:30", "20:10"),
    ];
    RAW.into_iter()
        .map(|(number, start_time, end_time)| TimeSlot {
            number,
            start_time: start_time.into(),
            end_time: end_time.into(),
            alias: None,
        })
        .collect()
}

/// 当前时刻「已开始」的节数（按传入节次表口径；传 [`block_time_slots`] 即大节数，
/// 0..=slots.len()，早于第一节为 0，晚于末节为全部）。
///
/// `"HH:MM"` 等长字符串字典序即时间序。
pub fn elapsed_slot_count(now_hm: &str, slots: &[TimeSlot]) -> usize {
    slots
        .iter()
        .filter(|s| s.start_time.as_str() <= now_hm)
        .count()
}

/// 周课表矩阵单格 → [`CourseBrief`]（空格 None；第 4 段任课教师名丢弃）。
///
/// 列号 → 大节号：10 列 = 5 大节 × 2 小节，`(0,1)→大节1 … (8,9)→大节5`，
/// 即 `col / 2 + 1`；课程起始时间取该大节开始时刻（真机缺陷教训：按小节号
/// 查默认 13 节表会把大节4 的课标成 14:50，正确为 15:35）。
fn course_from_cell(cell: &str, col: usize, slots: &[TimeSlot]) -> Option<CourseBrief> {
    let cell = cell.trim();
    if cell.is_empty() {
        return None;
    }
    let mut seg = cell.split(',').map(str::trim);
    let name = seg.next().filter(|s| !s.is_empty())?.to_string();
    let room = seg.next().unwrap_or_default().to_string();
    let teaching_class = seg.next().unwrap_or_default().to_string();
    let slot = col as u32 / 2 + 1;
    let start_time = slots
        .iter()
        .find(|s| s.number as u32 == slot)
        .map(|s| s.start_time.clone());
    Some(CourseBrief {
        name,
        room,
        teaching_class,
        slot,
        start_time,
    })
}

/// 「下一节课」：从（今天 `weekday_now`，已开始 `elapsed` 个大节）起在矩阵内向后
/// 线性扫第一个非空格——当天从大节 `elapsed+1` 的首列（= `elapsed*2`）起，之后
/// 各天从第 0 列起；**只在本周矩阵内**（服务端矩阵即当前周，跨周单双周课表
/// 可能不同，返回 None 隐藏横幅）。
///
/// `weekday_now`：1=周一…7=周日，越界 → None；`grid` 与 `weekday_now` 行列不符 → None。
pub fn next_course(
    grid: &[Vec<String>],
    weekday_now: u8,
    elapsed: usize,
    slots: &[TimeSlot],
) -> Option<CourseBrief> {
    let day_idx = (weekday_now as usize).checked_sub(1)?;
    if day_idx >= grid.len() {
        return None;
    }
    // 今天从 elapsed*2 列起（大节 elapsed 进行/已过，其列对 (elapsed-1)*2 起跳过），
    // 后续天从 0 列起（(row_offset, start_col) 对）
    let scans = std::iter::once((0usize, elapsed * 2))
        .chain((1..grid.len() - day_idx).map(|off| (off, 0usize)));
    for (row_off, start_col) in scans {
        let Some(row) = grid.get(day_idx + row_off) else {
            continue;
        };
        for (col, cell) in row.iter().enumerate().skip(start_col) {
            if let Some(c) = course_from_cell(cell, col, slots) {
                return Some(c);
            }
        }
    }
    None
}

/// 由本地当前时刻计算「下一节课」（[`next_course`] 的命令层薄包装；
/// 星期与 HH:MM 取本机时钟，节次表用校本大节表 [`block_time_slots`]）。
pub fn next_course_from_now(ws: &WeekSchedule) -> Option<CourseBrief> {
    use chrono::Datelike;
    let now = chrono::Local::now();
    let weekday = now.weekday().number_from_monday() as u8;
    let now_hm = now.format("%H:%M").to_string();
    let slots = block_time_slots();
    let elapsed = elapsed_slot_count(&now_hm, &slots);
    next_course(&ws.grid, weekday, elapsed, &slots)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- fixture（全部脱敏：占位学号/姓名/课程，无真实数据） ----------

    /// 学期信息实测形态（学期字段非敏感，与计划 §1.2 同构）。
    const SEMESTER_FIXTURE: &str = r#"{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":{"grade":"2026","semester":"1","currentWeek":"2","weekCount":"19","startDate":"20260907","endDate":"20270117","currentDate":"2026-09-18","currentWeekDay":"星期五"}}"#;

    /// 钱包卡实测形态：`data.data` 为内嵌 JSON 字符串数组；`loginUrl` 用
    /// TEST-PLACEHOLDER 占位（解析后必须被丢弃，结构体无该字段）。
    const WALLET_FIXTURE: &str = r#"{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":{"contentDisplay":"（略）","data":"[{\"ZJ\":\"暂无\",\"ZHYE\":102.51,\"JYE\":0.45,\"YE\":102.51,\"SSYE\":102.51,\"SL\":8,\"mailNewCount\":\"1\",\"mailFolderNewCount\":\"0\",\"totalCount\":\"1\",\"pageTotalCount\":1,\"loginUrl\":\"https://mail.example.edu/redirect?k=TEST-PLACEHOLDER\"}]"}}"#;

    /// 7×10 周课表（形状与实测同构：周五列 1,2（大节1）与列 7,8（大节4）有课、
    /// 周末全空；内容全脱敏）。
    fn fixture_grid() -> Vec<Vec<String>> {
        let mut g = vec![vec![String::new(); 10]; 7];
        let put = |g: &mut Vec<Vec<String>>, day: usize, slots: &[usize], name: &str| {
            for &s in slots {
                g[day][s] = format!("{name},教A-101,2023级1班,李四");
            }
        };
        put(&mut g, 0, &[2, 3, 4, 5], "高等数学");
        put(&mut g, 1, &[4, 5], "大学英语");
        put(&mut g, 2, &[2, 3], "数据结构");
        put(&mut g, 3, &[4, 5], "大学英语");
        put(&mut g, 4, &[0, 1, 6, 7], "高等数学");
        g
    }

    fn week_schedule_fixture(weekday: u8) -> String {
        let grid: Vec<Vec<String>> = fixture_grid();
        let rows: Vec<String> = grid
            .iter()
            .map(|row| {
                let cells: Vec<String> = row
                    .iter()
                    .map(|c| serde_json::to_string(c).unwrap())
                    .collect();
                format!("[{}]", cells.join(","))
            })
            .collect();
        format!(
            r#"{{"meta":{{"success":true,"statusCode":200,"message":"ok"}},"data":{{"resultsJsonArr":[{}],"zs":"2","xn":"2026","xq":"1","xqj":{weekday},"swskjc":4,"xwskjc":4,"wsskjc":2,"weekcount":"19"}}}}"#,
            rows.join(",")
        )
    }

    // ---------- parse_semester_info ----------

    #[test]
    fn semester_info_parses_all_fields() {
        let info = parse_semester_info(SEMESTER_FIXTURE).unwrap();
        assert_eq!(info.grade, "2026");
        assert_eq!(info.semester, "1");
        assert_eq!(info.current_week, "2");
        assert_eq!(info.week_count, "19");
        assert_eq!(info.start_date, "20260907");
        assert_eq!(info.end_date, "20270117");
        assert_eq!(info.current_week_day, "星期五");
    }

    #[test]
    fn semester_info_missing_field_is_error() {
        let body = r#"{"meta":{"success":true},"data":{"grade":"2026"}}"#;
        assert!(parse_semester_info(body).is_err());
        assert!(parse_semester_info("not json").is_err());
        let bad_meta = r#"{"meta":{"success":false,"message":"请重新登录"},"data":{}}"#;
        let err = parse_semester_info(bad_meta).unwrap_err();
        assert!(err.to_string().contains("请重新登录"));
    }

    // ---------- parse_wallet_summary ----------

    #[test]
    fn wallet_summary_parses_first_item_and_tolerant_types() {
        let w = parse_wallet_summary(WALLET_FIXTURE).unwrap();
        assert_eq!(w.card_balance, Some(102.51));
        assert_eq!(w.book_borrowed, Some(8));
        // mailNewCount 实测为字符串 "1"
        assert_eq!(w.mail_unread, Some(1));
    }

    #[test]
    fn wallet_summary_partial_success_on_missing_fields() {
        // YE/SL 缺失 → 对应项 None，mailNewCount 正常（部分成功不整体报错）
        let body = r#"{"meta":{"success":true},"data":{"data":"[{\"mailNewCount\":\"2\"}]"}}"#;
        let w = parse_wallet_summary(body).unwrap();
        assert_eq!(w.card_balance, None);
        assert_eq!(w.book_borrowed, None);
        assert_eq!(w.mail_unread, Some(2));
    }

    #[test]
    fn wallet_summary_error_paths() {
        // data.data 非字符串
        assert!(parse_wallet_summary(r#"{"meta":{"success":true},"data":{"data":123}}"#).is_err());
        // 内层非数组 / 空数组
        assert!(parse_wallet_summary(r#"{"meta":{"success":true},"data":{"data":"{}"}}"#).is_err());
        let empty =
            parse_wallet_summary(r#"{"meta":{"success":true},"data":{"data":"[]"}}"#).unwrap_err();
        assert!(empty.to_string().contains("无数据"));
        // 信封失败 / 非法 JSON
        assert!(parse_wallet_summary(r#"{"meta":{"success":false}}"#).is_err());
        assert!(parse_wallet_summary("not json").is_err());
    }

    // ---------- parse_week_schedule ----------

    #[test]
    fn week_schedule_parses_grid_and_weekday_number() {
        let ws = parse_week_schedule(&week_schedule_fixture(5)).unwrap();
        assert_eq!(ws.weekday, 5);
        assert_eq!(ws.grid.len(), 7);
        assert_eq!(ws.grid[0].len(), 10);
        assert!(ws.grid[4][0].contains("高等数学"));
        assert!(ws.grid[5][0].is_empty());
    }

    #[test]
    fn week_schedule_tolerates_string_weekday_and_errors_on_missing_grid() {
        let body = week_schedule_fixture(5).replace("\"xqj\":5", "\"xqj\":\"5\"");
        assert_eq!(parse_week_schedule(&body).unwrap().weekday, 5);
        assert!(parse_week_schedule(r#"{"meta":{"success":true},"data":{}}"#).is_err());
    }

    // ---------- next_course / elapsed_slot_count ----------

    fn slots() -> Vec<TimeSlot> {
        block_time_slots()
    }

    #[test]
    fn elapsed_block_count_by_time() {
        let s = slots();
        assert_eq!(elapsed_slot_count("07:00", &s), 0);
        // 大节1（08:00-09:40）进行中 → 已开始 1 大节
        assert_eq!(elapsed_slot_count("08:30", &s), 1);
        // 上午大课间（09:40-10:10）→ 大节2 未开始，仍为 1
        assert_eq!(elapsed_slot_count("10:00", &s), 1);
        // 大节3（13:45-15:25）进行中 → 3
        assert_eq!(elapsed_slot_count("14:00", &s), 3);
        // 大节4（15:35-17:15）进行中 → 4
        assert_eq!(elapsed_slot_count("16:00", &s), 4);
        // 深夜 → 全部 5 大节
        assert_eq!(elapsed_slot_count("22:00", &s), 5);
    }

    #[test]
    fn next_course_splits_cell_and_drops_fourth_segment() {
        let grid = fixture_grid();
        let c = next_course(&grid, 5, 0, &slots()).unwrap();
        assert_eq!(c.name, "高等数学");
        assert_eq!(c.room, "教A-101");
        assert_eq!(c.teaching_class, "2023级1班");
        assert_eq!(c.slot, 1);
        assert_eq!(c.start_time.as_deref(), Some("08:00"));
    }

    /// 主智能体验收缺陷回归：周五列 7,8（1-based；0-based 6,7 = 大节4）有课
    /// 「信息安全」，当前时间在上午 → 起点必须是大节4 的 15:35（此前按小节号
    /// 查默认 13 节表误报 14:50）。
    #[test]
    fn next_course_maps_column_pair_to_block_start() {
        let mut grid = vec![vec![String::new(); 10]; 7];
        grid[4][6] = "信息安全,C5科教中心313,2023级1班,李四".into();
        grid[4][7] = "信息安全,C5科教中心313,2023级1班,李四".into();
        // 上午（大节1 已开始，elapsed=1）→ 命中大节4
        let c = next_course(&grid, 5, 1, &slots()).unwrap();
        assert_eq!(c.name, "信息安全");
        assert_eq!(c.room, "C5科教中心313");
        assert_eq!(c.slot, 4);
        assert_eq!(c.start_time.as_deref(), Some("15:35"));
        // 边界：大节4 进行中（elapsed=4）→ 本周无更多课 → None
        assert!(next_course(&grid, 5, 4, &slots()).is_none());
        // 边界：全部大节已过（elapsed=5）→ None
        assert!(next_course(&grid, 5, 5, &slots()).is_none());
    }

    #[test]
    fn next_course_skips_ongoing_block_and_crosses_days() {
        let grid = fixture_grid();
        let s = slots();
        // 周五列 1,2（大节1）与 7,8（大节4）有课：大节1 进行中（elapsed=1）
        // → 当天从 col2 起 → 下一节是大节4 15:35
        let c = next_course(&grid, 5, 1, &s).unwrap();
        assert_eq!(c.slot, 4);
        assert_eq!(c.start_time.as_deref(), Some("15:35"));
        // 周四大节4 进行中（elapsed=4）→ 周四无更多课 → 周五大节1
        let c = next_course(&grid, 4, 4, &s).unwrap();
        assert_eq!(c.slot, 1);
        assert_eq!(c.start_time.as_deref(), Some("08:00"));
        // 周日（第 7 行）无课 → None
        assert!(next_course(&grid, 7, 0, &s).is_none());
        // 全空矩阵 → None
        let empty = vec![vec![String::new(); 10]; 7];
        assert!(next_course(&empty, 5, 0, &s).is_none());
        // weekday 越界防御
        assert!(next_course(&grid, 0, 0, &s).is_none());
        assert!(next_course(&grid, 8, 0, &s).is_none());
    }

    #[test]
    fn course_from_cell_partial_segments_and_unknown_block() {
        let s = slots();
        // 只有两段：教学班为空串，仍有效（col4 → 大节3，13:45 开始）
        let c = course_from_cell("体育,操场,", 4, &s).unwrap();
        assert_eq!(c.name, "体育");
        assert_eq!(c.room, "操场");
        assert_eq!(c.teaching_class, "");
        assert_eq!(c.slot, 3);
        assert_eq!(c.start_time, Some("13:45".to_string()));
        // 列号超出 5 大节（异常长行）→ 大节6 无常量 → start_time None
        let c = course_from_cell("晚课,教B-202,x,李四", 10, &s).unwrap();
        assert_eq!(c.slot, 6);
        assert_eq!(c.start_time, None);
        // 空格 / 纯逗号 → None
        assert!(course_from_cell("", 0, &s).is_none());
        assert!(course_from_cell("  ", 0, &s).is_none());
        assert!(course_from_cell(",教室,班,李四", 0, &s).is_none());
    }

    #[test]
    fn next_course_from_now_weekend_guard() {
        // 直接验证薄包装在「本地周末」场景的防御不 panic（结果取决于运行日）
        let ws = parse_week_schedule(&week_schedule_fixture(5)).unwrap();
        let _ = next_course_from_now(&ws);
    }
}
