// Adapted in part from shiguangschedule
// Copyright (C) 2025 XingHeYuZhuan
// Source: https://github.com/XingHeYuZhuan/shiguangschedule (Apache-2.0)
// Modified for Wxxy-CampusHub: Rust port + Zhengfang parser + course source isolation.

//! 锡院助手课表领域核心 crate。
//!
//! 纯逻辑层（无 Tauri / 无网络依赖），可被桌面端与安卓端以 path 依赖复用：
//! - [`model`]：课程 / 课表配置 / 节次 / 调课叠加数据模型（serde）
//! - [`weeks`]：开学日 ↔ 周次互算（自定义一周起始日）
//! - [`grid`]：课程网格布局（时间换算 + 重叠分簇分列），渲染层消费几何结果
//! - [`timeslots`]：默认 13 节作息常量
//! - [`zhengfang`]：正方教务课表响应解析（周次位掩码展开）
//! - [`diff`]：导入自动对比更新（本地课表 ↔ 教务最新课表，冻结契约 §2.4）
//! - [`notice`]：调课通知 L1/L2 解析（冻结契约 §2.5，产出 NoticeCandidate）
//! - [`occurrence`]：课程周内生效实例展开（调课/停课/补课 override 叠加，决策 5）
//!
//! 算法与数据模型移植自 shiguangschedule（Apache-2.0），见 NOTICE.md。

pub mod diff;
pub mod grid;
pub mod model;
pub mod notice;
pub mod occurrence;
pub mod timeslots;
pub mod weeks;
pub mod zhengfang;

pub use diff::{diff_courses, format_weeks, DiffResult};

pub use grid::{grid_scale_to_time, merge_courses, time_to_grid_scale, MergedCourseBlock, ScheduleMode};
pub use model::{
    expand_week_mask, Course, CourseOverride, CourseSource, CourseTableConfig, OverrideKind,
    TimeSlot, Timetable,
};
pub use notice::{notice_id_for, parse_notice_text, parse_notice_with_semester, NoticeCandidate, NoticeConfidence};
pub use occurrence::{expand_occurrences, CourseOccurrence, OccurrenceKind};
pub use timeslots::default_time_slots;
pub use weeks::{
    current_week, previous_or_same_day_of_week, semester_start_from_week, week_index_at_date,
};
pub use zhengfang::{parse_kb_response, query_body, Semester, ZhengfangError};
