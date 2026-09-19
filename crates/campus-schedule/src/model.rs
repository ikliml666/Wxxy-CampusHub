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
    /// 「已停开」标记：自动更新发现课程在教务最新课表中消失时置 `true`（**不删记录**，
    /// 保留用户可能挂载的调课 override）；`CourseSource::Manual` 的课程**永不置位**
    /// （冻结契约 §2.4：Manual 不参与 diff）。前端按 false 渲染、true 灰显或隐藏。
    #[serde(default)]
    pub disabled: bool,
}

/// 课表配置（对齐 CourseTableConfig）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseTableConfig {
    pub course_table_id: String,
    /// 是否显示周末列（契约 §17 修订，2026-09-19 真机反馈：默认**显示**周六
    /// 周日；旧文件显式 false 不受影响，仅缺字段时兜底为 true）
    #[serde(default = "default_show_weekends")]
    pub show_weekends: bool,
    /// 学期开始日期 "yyyy-MM-dd"（周次计算的锚点）
    pub semester_start_date: Option<NaiveDate>,
    /// 学期总周数
    #[serde(default = "default_total_weeks")]
    pub semester_total_weeks: u32,
    /// 一周起始日：1=周一 … 7=周日
    #[serde(default = "default_first_day")]
    pub first_day_of_week: u8,
    /// 自定义作息时间表（冻结契约 §2.1，2026-09-18 收尾轮追加）。`None`/空 =
    /// 用内置校本大节表 `campus_portal::block_time_slots()`（今日页与课表页同一
    /// 事实来源）；有值 = 用户编辑过的作息，视为唯一事实源。取值口径收敛在
    /// tauri 层 `commands::timetable::effective_slots`（网格行 / ICS 展开 /
    /// ceil 映射三处必须共用同一份）。`TimeSlot.alias` 可作大节别名。
    #[serde(default)]
    pub slots: Option<Vec<TimeSlot>>,
    /// 跳过日期（契约 §8.1，2026-09-19 批 2 追加）：全校性停课日（手动维护）。
    /// 网格该列课程不渲染 +「休」徽标；ICS 该日期 VEVENT 剔除；今日页（批 9）
    /// 显 `skipped` 态。旧文件无该键 → 空列表（serde default）。
    #[serde(default)]
    pub skipped_dates: Vec<NaiveDate>,
    /// 按日期生效的作息规则（契约 §9.1，2026-09-19 批 3 追加）：每条规则在
    /// `[start_date, end_date]`（含端点）区间内生效；区间允许重叠，命中取
    /// 首个声明者。无命中回落 [`CourseTableConfig::slots`] → 仍无则内置校本
    /// 大节表（三段回落链，收敛点在 tauri 层 `effective_slots_at`）。
    /// 旧文件无该键 → 空列表（serde default）。
    #[serde(default)]
    pub slot_rules: Vec<SlotRule>,
    /// 非本周课程降级显示（契约 §13.2，2026-09-19 批 7 追加）：`true` = 网格
    /// 同时渲染非展示周课程（40% 透明度、可点详情、不可拖）；`false`（默认 /
    /// 旧文件缺省）= 现状隐藏。仅前端渲染开关，后端不消费。
    #[serde(default)]
    pub show_non_current_week: bool,
    /// 置换日（契约 §22，2026-09-19 节假日轮）：该日期**按 `weekday` 的课表
    /// 执行**（调休补课：「9月20日（周日）补9月28日（周一）课」→
    /// `{date: 2026-09-20, weekday: 1}`）。来源 = 公告置换解析采纳
    /// （`apply_swap_day`，挂 `source_notice_id` 可随通知撤销）+ 设置手动编辑。
    /// 网格该列显示 `weekday` 列课程 +「班」徽标；ICS 追加置换实例；今日页按
    /// 置换课展示。旧文件无该键 → 空列表（serde default）。
    #[serde(default)]
    pub swap_days: Vec<SwapDay>,
    /// 节假日名（契约 §22）：timor.tech API 拉取的法定节日名（如「国庆节」），
    /// **仅供显示**（网格横幅/今日页）；「无课」判定仍以 [`CourseTableConfig::skipped_dates`]
    /// 为准（手动停课日无节日名，横幅回退「放假」）。旧文件无该键 → 空。
    #[serde(default)]
    pub holiday_names: Vec<NamedDate>,
}

/// 按日期区间的作息规则（契约 §9.1，批 3）。`slots` 恒非空（保存时校验）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotRule {
    /// 生效起始日期（含）
    pub start_date: NaiveDate,
    /// 生效结束日期（含）
    pub end_date: NaiveDate,
    /// 该区间的作息
    pub slots: Vec<TimeSlot>,
}

/// 置换日（契约 §22）：某日期按某星期的课表执行。见
/// [`CourseTableConfig::swap_days`]。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapDay {
    pub date: NaiveDate,
    /// 该日按星期几的课表执行（1=周一 … 7=周日）
    pub weekday: u8,
    /// 来源通知 id（`revoke_notice` 的撤销键）；手动添加为 None
    #[serde(default)]
    pub source_notice_id: Option<String>,
}

/// 带名的日期（契约 §22）：法定节假日名，仅显示用。见
/// [`CourseTableConfig::holiday_names`]。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedDate {
    pub date: NaiveDate,
    pub name: String,
}

fn default_total_weeks() -> u32 {
    20
}
fn default_first_day() -> u8 {
    1
}
fn default_show_weekends() -> bool {
    true
}

/// 节次时间段（对齐 TimeSlot）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseOverride {
    pub id: String,
    pub course_id: String,
    /// 生效周次（1-based）
    pub weeks: Vec<u32>,
    /// 调整类型：调课 / 停课 / 补课（变体名序列化为 snake_case，见 OverrideKind）
    pub change_type: OverrideKind,
    /// 调整后的星期几（调课/补课 = 新时间；停课 = 被停那次的星期，供前端定位「停哪一次」，
    /// 通知未提及时为 None）
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

/// 一份本地课表（M2.5 冻结契约 §2.1）：配置 + 课程 + 调课叠加 + 最近更新时刻。
///
/// 持久化形态即 `%APPDATA%/campushub/timetable.json` 的顶层结构（非凭据，明文）；
/// 序列化 camelCase（`updatedAt` 等）。`courses`/`overrides` 带 serde 缺省，
/// 旧文件或手工删节后仍可读取。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timetable {
    pub config: CourseTableConfig,
    #[serde(default)]
    pub courses: Vec<Course>,
    #[serde(default)]
    pub overrides: Vec<CourseOverride>,
    /// 最近更新时刻（RFC3339 文本，由上层写入；空串 = 从未更新）
    #[serde(default)]
    pub updated_at: String,
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

    /// `disabled` serde 缺省（冻结契约 §2.1）：旧数据无该字段 → false；显式 true 保留；
    /// 序列化键为 camelCase `disabled`。
    #[test]
    fn course_disabled_defaults_false_and_roundtrips() {
        let legacy = r#"{
            "id": "t-abc", "courseTableId": "t", "name": "信息安全",
            "day": 1, "colorIndex": 0, "source": "import", "weeks": [1,2]
        }"#;
        let c: Course = serde_json::from_str(legacy).unwrap();
        assert!(!c.disabled, "旧格式（无 disabled 字段）应缺省为 false");

        let mut c = c.clone();
        c.disabled = true;
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"disabled\":true"));
        let back: Course = serde_json::from_str(&json).unwrap();
        assert!(back.disabled);
    }

    /// `Timetable` serde：camelCase 顶层键（`updatedAt`）+ courses/overrides 缺省。
    #[test]
    fn timetable_serde_defaults_and_camel_case() {
        let minimal = r#"{
            "config": { "courseTableId": "default", "semesterTotalWeeks": 20, "firstDayOfWeek": 1 }
        }"#;
        let tt: Timetable = serde_json::from_str(minimal).unwrap();
        assert_eq!(tt.config.course_table_id, "default");
        assert!(tt.courses.is_empty());
        assert!(tt.overrides.is_empty());
        assert_eq!(tt.updated_at, "");
        // slots serde 缺省（冻结契约 §2.1 收尾轮追加）：旧文件无该字段 → None（内置作息）
        assert!(tt.config.slots.is_none());
        // skipped_dates serde 缺省（契约 §8.1）：旧文件无该键 → 空列表
        assert!(tt.config.skipped_dates.is_empty());
        // slot_rules serde 缺省（契约 §9.1）：旧文件无该键 → 空列表
        assert!(tt.config.slot_rules.is_empty());
        // show_non_current_week serde 缺省（契约 §13.2）：旧文件无该键 → false（现状隐藏）
        assert!(!tt.config.show_non_current_week);
        // show_weekends serde 缺省（契约 §17 修订）：旧文件无该键 → true（默认显示周末）
        assert!(tt.config.show_weekends);

        let full = Timetable {
            config: CourseTableConfig {
                course_table_id: "default".into(),
                show_weekends: false,
                semester_start_date: None,
                semester_total_weeks: 20,
                first_day_of_week: 1,
                slots: None,
                skipped_dates: vec![NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()],
                swap_days: vec![],
                holiday_names: vec![],
                show_non_current_week: true,
                slot_rules: vec![SlotRule {
                    start_date: NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
                    end_date: NaiveDate::from_ymd_opt(2027, 2, 28).unwrap(),
                    slots: vec![TimeSlot {
                        number: 1,
                        start_time: "09:00".into(),
                        end_time: "10:40".into(),
                        alias: None,
                    }],
                }],
            },
            courses: vec![],
            overrides: vec![],
            updated_at: "2026-09-18T10:00:00+08:00".into(),
        };
        let json = serde_json::to_string(&full).unwrap();
        assert!(json.contains("\"updatedAt\""));
        assert!(json.contains("\"skippedDates\":[\"2026-10-01\"]"));
        assert!(
            json.contains("\"slotRules\":[{\"startDate\":\"2026-12-01\",\"endDate\":\"2027-02-28\""),
            "slot_rules 序列化为 camelCase 键"
        );
        assert!(json.contains("\"showNonCurrentWeek\":true"), "批 7 字段 camelCase 键");
        let back: Timetable = serde_json::from_str(&json).unwrap();
        assert_eq!(back, full);
    }
}
