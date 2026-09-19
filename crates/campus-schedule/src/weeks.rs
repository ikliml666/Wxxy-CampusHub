// Adapted in part from shiguangschedule
// Copyright (C) 2025 XingHeYuZhuan
// Source: https://github.com/XingHeYuZhuan/shiguangschedule (Apache-2.0)
// Modified for Wxxy-CampusHub: Rust/chrono port.
//   Kotlin original: data/repository/AppSettingsRepository.kt:138-224
//   (getWeekIndexAtDate / calculateSemesterStartDate / getPreviousOrSameDayOfWeek)

//! 周次计算：开学日 ↔ 周次互算，支持自定义一周起始日。

use chrono::{Datelike, NaiveDate};

/// 把日期对齐到「本周（按 `first_day_of_week` 划分）的首日」，不足则回退。
/// Kotlin 原文：getPreviousOrSameDayOfWeek。
/// 前端 `TimetablePanel.tsx::previousOrSame` 是本函数的 JS 镜像，两处注释互锚，
/// 改一处必须同步另一处（契约 §7.4）。
pub fn previous_or_same_day_of_week(date: NaiveDate, first_day_of_week: u8) -> NaiveDate {
    // chrono: weekday().number_from_monday() 1=周一…7=周日
    let current = date.weekday().number_from_monday() as i64;
    let target = first_day_of_week as i64;
    let days_to_subtract = if current >= target {
        current - target
    } else {
        7 - (target - current)
    };
    date - chrono::Duration::days(days_to_subtract)
}

/// 核心周次偏移：`date` 落在以 `semester_start` 为第 1 周的学期的第几周。
/// 返回 None 表示未配置开学日期。
pub fn week_index_at_date(
    target: NaiveDate,
    semester_start: NaiveDate,
    first_day_of_week: u8,
) -> i64 {
    let aligned_start = previous_or_same_day_of_week(semester_start, first_day_of_week);
    let aligned_target = previous_or_same_day_of_week(target, first_day_of_week);
    let diff_days = (aligned_target - aligned_start).num_days();
    diff_days / 7 + 1
}

/// 当前自然周次，越界（不在 1..=total_weeks）返回 None。
pub fn current_week(
    today: NaiveDate,
    cfg: &crate::model::CourseTableConfig,
) -> Option<u32> {
    let start = cfg.semester_start_date?;
    let raw = week_index_at_date(today, start, cfg.first_day_of_week);
    if raw >= 1 && raw <= cfg.semester_total_weeks as i64 {
        Some(raw as u32)
    } else {
        None
    }
}

/// 由「今天是第 `week` 周」反推开学日期（首周引导用）。
/// Kotlin 原文：calculateSemesterStartDate。
pub fn semester_start_from_week(
    today: NaiveDate,
    week: u32,
    first_day_of_week: u8,
) -> NaiveDate {
    let start_of_this_week = previous_or_same_day_of_week(today, first_day_of_week);
    start_of_this_week - chrono::Duration::days((week as i64 - 1) * 7)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CourseSource, CourseTableConfig};

    // 2026-2027 学年第 1 学期：9月7日（周一）开学；2026-09-17（周四）为第 2 周
    // —— 与门户课表「第2周」实测一致（golden）。
    #[test]
    fn week_index_golden() {
        let start = NaiveDate::from_ymd_opt(2026, 9, 7).unwrap();
        let d = NaiveDate::from_ymd_opt(2026, 9, 17).unwrap();
        assert_eq!(week_index_at_date(d, start, 1), 2);
    }

    #[test]
    fn week_index_custom_first_day() {
        // 开学日 2026-09-07 是周一；若一周从周日算，9-12（周六）仍属第 1 周，
        // 9-13（周日）开始进入第 2 周。
        let start = NaiveDate::from_ymd_opt(2026, 9, 7).unwrap();
        assert_eq!(
            week_index_at_date(NaiveDate::from_ymd_opt(2026, 9, 12).unwrap(), start, 7),
            1
        );
        assert_eq!(
            week_index_at_date(NaiveDate::from_ymd_opt(2026, 9, 13).unwrap(), start, 7),
            2
        );
    }

    #[test]
    fn current_week_clamps_out_of_range() {
        let cfg = |start: Option<NaiveDate>| CourseTableConfig {
            course_table_id: "t".into(),
            show_weekends: false,
            semester_start_date: start,
            semester_total_weeks: 20,
            first_day_of_week: 1,
            slots: None,
            skipped_dates: vec![],
            slot_rules: vec![],
            show_non_current_week: false,
        };
        assert_eq!(
            current_week(
                NaiveDate::from_ymd_opt(2026, 9, 17).unwrap(),
                &cfg(Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()))
            ),
            Some(2)
        );
        // 假期（第 30 周）越界 → None
        assert_eq!(
            current_week(
                NaiveDate::from_ymd_opt(2027, 4, 1).unwrap(),
                &cfg(Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()))
            ),
            None
        );
        // 未配置开学日 → None
        assert_eq!(
            current_week(NaiveDate::from_ymd_opt(2026, 9, 17).unwrap(), &cfg(None)),
            None
        );
        let _ = CourseSource::Manual;
    }

    #[test]
    fn reverse_start_date_roundtrip() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 17).unwrap();
        let start = semester_start_from_week(today, 2, 1);
        assert_eq!(start, NaiveDate::from_ymd_opt(2026, 9, 7).unwrap());
        assert_eq!(week_index_at_date(today, start, 1), 2);
    }
}
