// Adapted in part from shiguangschedule
// Copyright (C) 2025 XingHeYuZhuan
// Source: https://github.com/XingHeYuZhuan/shiguangschedule (Apache-2.0)
// Modified for Wxxy-CampusHub: Rust + serde port, added course source isolation
// and Zhengfang (正方) academic system parser.

//! 课表领域数据模型。
//!
//! 字段设计对齐 shiguangschedule 的 Room 实体（Course / CourseWeek /
//! CourseTableConfig / TimeSlot），两点关键差异：
//! 1. 周次沿用其 `weeks: 显式列表` 设计（单双周不用标志位）；
//! 2. 新增 `source`（导入/手动来源隔离）与 `override`（调课通知叠加）模型，
//!    为「自动更新只作用于导入课程」提供数据基础。

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// 课程来源。自动更新与调课解析只作用于 [`CourseSource::Import`] 的课程。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CourseSource {
    /// 从教务系统导入（自动更新可触碰）
    Import,
    /// 用户手动添加（永不触碰）
    Manual,
}

/// 一门课程（同一课程在多个周次复用同一条记录，周次见 [`Course::weeks`]）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Course {
    pub id: String,
    /// 所属课表 ID（多课表隔离）
    pub course_table_id: String,
    pub name: String,
    #[serde(default)]
    pub teacher: String,
    #[serde(default)]
    pub position: String,
    /// 星期几，1=周一 … 7=周日
    pub day: u8,
    /// 起始节次（含）。`is_custom_time = true` 时忽略
    pub start_section: Option<u8>,
    /// 结束节次（含）
    pub end_section: Option<u8>,
    #[serde(default)]
    pub is_custom_time: bool,
    /// 自定义开始时间 "HH:MM"
    pub custom_start_time: Option<String>,
    /// 自定义结束时间 "HH:MM"
    pub custom_end_time: Option<String>,
    /// 课程卡片颜色索引（样式表下标）
    pub color_index: u16,
    #[serde(default)]
    pub remark: Option<String>,
    /// 来源标签（Wxxy-CampusHub 扩展）
    pub source: CourseSource,
    /// 该课程出现的周次（1-based 显式列表，替代单双周标志位）
    pub weeks: Vec<u32>,
    /// 教学班 ID（正方 `jxb_id`），自动更新 diff 的匹配键之一
    #[serde(default)]
    pub class_id: Option<String>,
}

/// 课表配置（对齐 CourseTableConfig）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseTableConfig {
    pub course_table_id: String,
    /// 是否显示周末列
    #[serde(default)]
    pub show_weekends: bool,
    /// 学期开始日期 "yyyy-MM-dd"（周次计算的锚点）
    pub semester_start_date: Option<NaiveDate>,
    /// 学期总周数
    #[serde(default = "default_total_weeks")]
    pub semester_total_weeks: u32,
    /// 一周起始日：1=周一 … 7=周日
    #[serde(default = "default_first_day")]
    pub first_day_of_week: u8,
}

fn default_total_weeks() -> u32 {
    20
}
fn default_first_day() -> u8 {
    1
}

/// 节次时间段（对齐 TimeSlot）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeSlot {
    pub number: u8,
    /// "HH:MM"
    pub start_time: String,
    /// "HH:MM"
    pub end_time: String,
    #[serde(default)]
    pub alias: Option<String>,
}

/// 单次调课叠加（Wxxy-CampusHub 自建，shiguangschedule 无此模型）。
///
/// 语义：叠加在 `course_id` 指向的导入课程之上，原数据保留可回滚；
/// 撤销某条通知 = 删除 `source_notice_id` 匹配的全部 override。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseOverride {
    pub id: String,
    pub course_id: String,
    /// 生效周次（1-based）
    pub weeks: Vec<u32>,
    /// 调整类型：调课 / 停课 / 补课（变体名序列化为 snake_case，见 OverrideKind）
    pub change_type: OverrideKind,
    /// 调整后的星期几（停课时为 None）
    pub new_day: Option<u8>,
    pub new_start_section: Option<u8>,
    pub new_end_section: Option<u8>,
    pub new_position: Option<String>,
    /// 来源通知 ID（撤销与去重键）
    pub source_notice_id: String,
    /// 解析置信度：高置信自动应用，低置信进待确认
    pub auto_applied: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverrideKind {
    Rescheduled,
    Cancelled,
    Extra,
}

/// 周次位掩码（正方 `oldzc` 字段，bit0 = 第 1 周）展开为 1-based 周次列表。
pub fn expand_week_mask(mask: u64) -> Vec<u32> {
    (0..64).filter(|b| mask & (1u64 << b) != 0).map(|b| b + 1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn week_mask_expands_correctly() {
        // 4095 = 0b111111111111 = 第 1-12 周（真实教务数据）
        assert_eq!(expand_week_mask(4095), (1..=12).collect::<Vec<_>>());
        // 单双周：0b010101010101 → 1,3,5,7,9,11（单周）
        assert_eq!(expand_week_mask(0b0101_0101_0101), vec![1, 3, 5, 7, 9, 11]);
        assert!(expand_week_mask(0).is_empty());
    }
}
