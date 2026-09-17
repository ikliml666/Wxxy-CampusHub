//! 默认节次表常量。
//!
//! Kotlin 原文：data/repository/CourseTableRepository.kt:382-396 `defaultTimeSlots`。
//! Wxxy-CampusHub 后续按本校作息覆盖（用户可在设置中编辑）。

use crate::model::TimeSlot;

/// 13 节默认作息（08:00 起，午休 12:15-14:00，晚餐 17:20-18:30）。
pub fn default_time_slots() -> Vec<TimeSlot> {
    const RAW: [(u8, &str, &str); 13] = [
        (1, "08:00", "08:45"),
        (2, "08:50", "09:35"),
        (3, "09:50", "10:35"),
        (4, "10:40", "11:25"),
        (5, "11:30", "12:15"),
        (6, "14:00", "14:45"),
        (7, "14:50", "15:35"),
        (8, "15:45", "16:30"),
        (9, "16:35", "17:20"),
        (10, "18:30", "19:15"),
        (11, "19:20", "20:05"),
        (12, "20:10", "20:55"),
        (13, "21:10", "21:55"),
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
