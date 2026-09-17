// Adapted in part from shiguangschedule
// Copyright (C) 2025 XingHeYuZhuan
// Source: https://github.com/XingHeYuZhuan/shiguangschedule (Apache-2.0)
// Modified for Wxxy-CampusHub: Rust port.
//   Kotlin original: ui/schedule/WeeklyScheduleViewModel.kt
//   (timeToGridScale :324-364, gridScaleToTime :370-409, mergeCourses :632-754)

//! 课程网格布局：节次/时间 → 浮点网格坐标，同日重叠课程分簇 + 贪心分列。
//! 渲染层（React）直接消费 `MergedCourseBlock` 的几何结果做 absolute 定位。

use crate::model::{Course, TimeSlot};
use chrono::{NaiveTime, Timelike};

/// 分列模式（对齐 ScheduleModeProto）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleMode {
    /// 节次模式：网格以节次为单位
    Section,
    /// 24 小时模式：网格以小时为单位
    Time24h,
}

/// 归一化后的课程块（渲染几何结果）。
#[derive(Debug, Clone, PartialEq)]
pub struct MergedCourseBlock {
    /// 星期几 1..=7
    pub day: u8,
    /// 网格起始（0 起，节次模式下 0.0 = 第 1 节顶部）
    pub start_section: f32,
    pub end_section: f32,
    pub course: Course,
    /// 非本周课程（视觉淡化）
    pub is_visual_demoted: bool,
    /// 本块占据的子列（第几列，从 0 起）与簇内总列数（重叠分列）
    pub column_index: usize,
    pub column_count: usize,
}

fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").ok()
}

/// 统一时间换算器：任意时刻 → 网格 Float 纵坐标（1.0 = 第 1 格顶部）。
/// Kotlin 原文：timeToGridScale。
pub fn time_to_grid_scale(
    time: NaiveTime,
    time_slots: &[TimeSlot],
    mode: ScheduleMode,
) -> f32 {
    match mode {
        ScheduleMode::Time24h => {
            let minutes = time.hour() as f32 * 60.0 + time.minute() as f32;
            1.0 + minutes / 60.0
        }
        ScheduleMode::Section => {
            if time_slots.is_empty() {
                return 1.0;
            }
            let mut sorted: Vec<&TimeSlot> = time_slots.iter().collect();
            sorted.sort_by_key(|s| s.number);

            let first_start = parse_hhmm(&sorted[0].start_time);
            let last_end = parse_hhmm(&sorted[sorted.len() - 1].end_time);
            let (Some(first_start), Some(last_end)) = (first_start, last_end) else {
                return 1.0;
            };
            if time <= first_start {
                return 1.0;
            }
            if time >= last_end {
                return sorted.len() as f32 + 1.0;
            }
            if let Some(slot) = sorted.iter().find(|s| {
                let s0 = parse_hhmm(&s.start_time).unwrap();
                let e0 = parse_hhmm(&s.end_time).unwrap();
                time >= s0 && time <= e0
            }) {
                let s0 = parse_hhmm(&slot.start_time).unwrap();
                let e0 = parse_hhmm(&slot.end_time).unwrap();
                let duration = (e0 - s0).num_minutes();
                let safe_duration = if duration <= 0 { 1 } else { duration };
                let elapsed = (time - s0).num_minutes();
                return slot.number as f32 + elapsed as f32 / safe_duration as f32;
            }
            // 节间空隙：落到下一节的起点
            sorted
                .iter()
                .find(|s| parse_hhmm(&s.start_time).map(|t| t > time).unwrap_or(false))
                .map(|s| s.number as f32)
                .unwrap_or(sorted.len() as f32 + 1.0)
        }
    }
}

/// 反向换算：网格 Float 坐标 → 物理时间（拖拽改课用）。
/// Kotlin 原文：gridScaleToTime。
pub fn grid_scale_to_time(
    grid_section: f32,
    time_slots: &[TimeSlot],
    mode: ScheduleMode,
) -> Option<NaiveTime> {
    match mode {
        ScheduleMode::Time24h => {
            let total_minutes = ((grid_section * 60.0) as i64).clamp(0, 24 * 60 - 1);
            Some(NaiveTime::from_hms_opt(
                (total_minutes / 60) as u32,
                (total_minutes % 60) as u32,
                0,
            )?)
        }
        ScheduleMode::Section => {
            if time_slots.is_empty() {
                return NaiveTime::from_hms_opt(8, 0, 0);
            }
            let mut sorted: Vec<&TimeSlot> = time_slots.iter().collect();
            sorted.sort_by_key(|s| s.number);

            let target = grid_section + 1.0;
            let integer_part = target as u32;
            let fraction = target - integer_part as f32;

            match sorted.iter().find(|s| s.number as u32 == integer_part) {
                Some(slot) => {
                    let s0 = parse_hhmm(&slot.start_time)?;
                    let e0 = parse_hhmm(&slot.end_time)?;
                    let total_duration = (e0 - s0).num_minutes();
                    let added = (fraction * total_duration as f32) as i64;
                    Some(s0 + chrono::Duration::minutes(added))
                }
                None => {
                    if integer_part < sorted[0].number as u32 {
                        parse_hhmm(&sorted[0].start_time)
                    } else {
                        parse_hhmm(&sorted[sorted.len() - 1].end_time)
                    }
                }
            }
        }
    }
}

const EPS: f32 = 0.01;

/// 单日布局结果（内部中间态）。
struct Normalized {
    course: Course,
    start: f32,
    end: f32,
}

/// 无损展平排版引擎：把一周的课程归一化 → 分簇 → 贪心分列。
/// Kotlin 原文：WeeklyScheduleViewModel.mergeCourses（逻辑 1:1，含容差与越界修正）。
pub fn merge_courses(
    courses: &[Course],
    time_slots: &[TimeSlot],
    current_week: u32,
    mode: ScheduleMode,
) -> Vec<MergedCourseBlock> {
    if time_slots.is_empty() && mode == ScheduleMode::Section {
        return Vec::new();
    }

    let max_section = match mode {
        ScheduleMode::Time24h => 24.0f32,
        ScheduleMode::Section => time_slots.len() as f32,
    };
    let limit = max_section + 1.0;
    let min_safe_height = match mode {
        ScheduleMode::Time24h => 0.0f32,
        ScheduleMode::Section => 0.3,
    };

    // 1. 归一化：课程 → (start, end) Float 区间
    let mut normalized: Vec<Normalized> = Vec::new();
    for c in courses {
        let (s_time, e_time) = if c.is_custom_time {
            match (
                c.custom_start_time.as_deref().and_then(parse_hhmm),
                c.custom_end_time.as_deref().and_then(parse_hhmm),
            ) {
                (Some(s), Some(e)) => (s, e),
                _ => continue,
            }
        } else {
            let start_slot = time_slots.iter().find(|s| Some(s.number) == c.start_section);
            let end_slot = time_slots.iter().find(|s| Some(s.number) == c.end_section);
            match (start_slot, end_slot) {
                (Some(s), Some(e)) => match (
                    parse_hhmm(&s.start_time),
                    parse_hhmm(&e.end_time),
                ) {
                    (Some(a), Some(b)) => (a, b),
                    _ => continue,
                },
                _ => continue,
            }
        };
        let s = time_to_grid_scale(s_time, time_slots, mode);
        let e = time_to_grid_scale(e_time, time_slots, mode);

        // 越界与最小高度修正（对齐 Kotlin 原文）
        let mut final_start = s;
        let mut final_end = e;
        if final_start >= limit {
            final_end = limit;
            final_start = limit - min_safe_height;
        } else if final_end <= 1.0 {
            final_start = 1.0;
            final_end = 1.0 + min_safe_height;
        }
        if final_end - final_start < min_safe_height {
            if final_end + min_safe_height <= limit {
                final_end = final_start + min_safe_height;
            } else {
                final_start = final_end - min_safe_height;
            }
        }

        normalized.push(Normalized {
            course: c.clone(),
            start: final_start.clamp(1.0, limit - 0.1),
            end: final_end.clamp(1.1, limit),
        });
    }

    let mut result: Vec<MergedCourseBlock> = Vec::new();

    // 2. 按星期分组
    let mut by_day: std::collections::BTreeMap<u8, Vec<Normalized>> = Default::default();
    for n in normalized {
        by_day.entry(n.course.day).or_default().push(n);
    }

    for (day, mut daily) in by_day {
        // 3. 排序：起点升序，跨度降序
        daily.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    (b.end - b.start)
                        .partial_cmp(&(a.end - a.start))
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        });

        // 4. 分簇：与簇内任一课程重叠（±EPS 容差）则并入
        let mut clusters: Vec<Vec<Normalized>> = Vec::new();
        for item in daily {
            let target = clusters.iter_mut().find(|cluster| {
                cluster
                    .iter()
                    .any(|existing| item.start < existing.end - EPS && item.end > existing.start + EPS)
            });
            match target {
                Some(cluster) => cluster.push(item),
                None => clusters.push(vec![item]),
            }
        }

        // 5. 簇内贪心分列（区间图着色）：复用首个结束时间 <= 起点的列
        for cluster in clusters {
            let mut column_ends: Vec<f32> = Vec::new();
            let mut assignments: Vec<(usize, usize)> = Vec::new(); // (course_idx_in_cluster, column)

            for (idx, item) in cluster.iter().enumerate() {
                let mut assigned = None;
                for (i, end) in column_ends.iter_mut().enumerate() {
                    if *end <= item.start + EPS {
                        *end = item.end;
                        assigned = Some(i);
                        break;
                    }
                }
                let col = match assigned {
                    Some(i) => i,
                    None => {
                        column_ends.push(item.end);
                        column_ends.len() - 1
                    }
                };
                assignments.push((idx, col));
            }

            let total_columns = column_ends.len();

            // 6. 输出（簇内按起点、列号排序）
            let mut order: Vec<usize> = (0..cluster.len()).collect();
            order.sort_by(|&a, &b| {
                cluster[a]
                    .start
                    .partial_cmp(&cluster[b].start)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(assignments[a].1.cmp(&assignments[b].1))
            });

            for idx in order {
                let item = &cluster[idx];
                let col = assignments[idx].1;
                let is_active = item.course.weeks.iter().any(|&w| w == current_week);
                result.push(MergedCourseBlock {
                    day,
                    start_section: (item.start - 1.0).clamp(0.0, max_section),
                    end_section: (item.end - 1.0).clamp(0.0, max_section),
                    course: item.course.clone(),
                    is_visual_demoted: !is_active,
                    column_index: col,
                    column_count: total_columns,
                });
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CourseSource;

    fn slot(n: u8, start: &str, end: &str) -> TimeSlot {
        TimeSlot { number: n, start_time: start.into(), end_time: end.into(), alias: None }
    }
    fn course(day: u8, s: u8, e: u8, weeks: Vec<u32>) -> Course {
        Course {
            id: format!("c{day}-{s}-{e}-{}", weeks.first().unwrap_or(&0)),
            course_table_id: "t".into(),
            name: "信息安全".into(),
            teacher: "林博乐".into(),
            position: "D4思泉楼207".into(),
            day,
            start_section: Some(s),
            end_section: Some(e),
            is_custom_time: false,
            custom_start_time: None,
            custom_end_time: None,
            color_index: 0,
            remark: None,
            source: CourseSource::Import,
            weeks,
            class_id: None,
        }
    }

    fn slots() -> Vec<TimeSlot> {
        vec![
            slot(1, "08:00", "08:45"),
            slot(2, "08:50", "09:35"),
            slot(3, "09:50", "10:35"),
            slot(4, "10:40", "11:25"),
        ]
    }

    #[test]
    fn no_overlap_single_column() {
        let cs = vec![course(1, 1, 2, vec![1]), course(1, 3, 4, vec![1])];
        let out = merge_courses(&cs, &slots(), 1, ScheduleMode::Section);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|b| b.column_count == 1));
        // 1-2 节块：0.0..2.0
        let b0 = &out[0];
        assert!((b0.start_section - 0.0).abs() < 1e-3 && (b0.end_section - 2.0).abs() < 1e-3);
    }

    #[test]
    fn overlap_splits_two_columns() {
        // 周一 1-2 与 2-3（在 08:50-09:35 重叠）→ 分两列
        let cs = vec![course(1, 1, 2, vec![1]), course(1, 2, 3, vec![1])];
        let out = merge_courses(&cs, &slots(), 1, ScheduleMode::Section);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|b| b.column_count == 2));
        assert_eq!(out[0].column_index + out[1].column_index, 1);
    }

    #[test]
    fn chain_reuses_column() {
        // 1-2、2-3、3-4 链式：1-2 与 2-3 重叠，3-4 与 2-3 重叠 → 两列
        let cs = vec![
            course(1, 1, 2, vec![1]),
            course(1, 2, 3, vec![1]),
            course(1, 3, 4, vec![1]),
        ];
        let out = merge_courses(&cs, &slots(), 1, ScheduleMode::Section);
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|b| b.column_count == 2));
    }

    #[test]
    fn other_week_demoted() {
        let cs = vec![course(2, 1, 2, vec![3])]; // 第 3 周才有
        let out = merge_courses(&cs, &slots(), 1, ScheduleMode::Section);
        assert!(out[0].is_visual_demoted);
    }

    #[test]
    fn section_mode_requires_slots() {
        let cs = vec![course(1, 1, 2, vec![1])];
        assert!(merge_courses(&cs, &[], 1, ScheduleMode::Section).is_empty());
    }
}
