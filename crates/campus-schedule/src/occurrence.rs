//! 课程在某教学周的「生效实例」展开（Wxxy-CampusHub 自建，蓝图决策 5）。
//!
//! [`expand_occurrences`] 把一门课程 × 一个周次展开为经调课/停课/补课 override
//! 叠加后的全部实例：ICS 导出与今日页（批 9）只消费 [`OccurrenceKind::Solid`]，
//! ghost 实例（`MovedOut`/`Cancelled`）供前端语义对照与批 4 拖拽复用。
//!
//! ⚠️ 语义基准是前端 `buildWeekBlocks`（`TimetablePanel.tsx`）——两处注释互锚：
//! 改一处必须同步另一处（受控双写，Rust 单测钉住语义，前端无法调 Rust 纯函数）。
//!
//! **custom 课 override 语义（P3-c 登记，与契约 §8.4 互锚）**：custom 课
//!（`start_section`/`end_section` 为 None）不进网格（前端 `buildWeekBlocks` 对
//! 节次 None 的课程 continue 跳过）；被 resched 时本函数产出带节次的 Solid
//!（新节次取 override 的 new_*），消费方走大节表取时刻，不用 custom 时刻；
//! 未被调整时产出节次 None 的 Solid，消费方按 `custom_*_time` 取时刻。

use crate::model::{Course, CourseOverride, OverrideKind};

/// 实例形态（对齐前端 PlacedBlock.ghost 的三态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccurrenceKind {
    /// 实体课（网格实块；ICS/今日页只取这一种）
    Solid,
    /// 已调出：该周该课原时段被调走，原时段只剩虚线占位
    MovedOut,
    /// 已停：虚线「已停」占位，不生成 ICS、不进今日页
    Cancelled,
}

/// 一门课程在一个周次的一次上课实例。
#[derive(Debug, Clone, PartialEq)]
pub struct CourseOccurrence {
    pub kind: OccurrenceKind,
    /// 星期几，1=周一 … 7=周日
    pub day: u8,
    /// 起止节次（教务小节口径，与 [`Course::start_section`] 同尺度）。
    /// `None` 仅当课程本身 custom（`is_custom_time` 且无节次）——消费方按
    /// `custom_start_time`/`custom_end_time` 取时刻。
    pub start_section: Option<u8>,
    pub end_section: Option<u8>,
    pub position: String,
    /// 产生本实例的 override id；普通原位实例为 `None`。供 ICS DESCRIPTION
    /// 追加「调课/补课」标注（Solid 无法仅凭几何字段区分原位与新位/补课）
    /// 与批 9 今日页溯源。
    pub source_override_id: Option<String>,
}

/// 逆序取最后一条匹配 override（后采纳的通知覆盖先采纳的；与前端
/// `lastOverride` 同语义，upsert_override 的覆盖写入与之呼应）。
fn last_override<'a>(
    overrides: &'a [CourseOverride],
    course_id: &str,
    kind: OverrideKind,
    week: u32,
) -> Option<&'a CourseOverride> {
    overrides
        .iter()
        .rev()
        .find(|o| o.course_id == course_id && o.change_type == kind && o.weeks.contains(&week))
}

/// 课程在某周经 override 展开后的全部实例（契约 §8.4）。
///
/// 展开分支与前端 `buildWeekBlocks` 逐条对照（受控双写）：
/// 1. `disabled` 或 `week = 0` → 空；
/// 2. 本周无课（`week` 不在 `course.weeks`）→ 无课程实体实例（但 **extra 补课
///    仍生效**，见第 6 条——前端 extra 循环不看出 `course.weeks`）；
/// 3. 停课两档（契约 §2.5.1）：`new_day = None` = 整周全停；`Some(d)` = 仅
///    `d == course.day` 那一次停（否则该 override 对本课不生效）；停课命中即
///    短路实体块分支（**停课优先于调课**），但**不短路 extra**；
/// 4. 调课且新时间 ≠ 原时间 → 原时段 `MovedOut` + 新时段 `Solid`（跨天/同天
///    换节都算；结束节次缺省 = 起始节次）；
/// 5. 仅换教室（新时间 = 原时间）→ 原位单条 `Solid`，教室取 `new_position`；
/// 6. extra 补课 → **独立于以上所有分支**追加 `Solid` 新实体（day 缺省原 day，
///    节次缺省第 1 小节起）——停课/调课周同时补课 → ghost/移位实例与补课实例
///    并存（复核 P1-a 修复：cancel/resched 分支不得吞掉 extra）。
pub fn expand_occurrences(
    course: &Course,
    overrides: &[CourseOverride],
    week: u32,
) -> Vec<CourseOccurrence> {
    if course.disabled || week == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let resched = last_override(overrides, &course.id, OverrideKind::Rescheduled, week);
    let cancel = last_override(overrides, &course.id, OverrideKind::Cancelled, week);

    // ---- 课程实体块分支（仅当本周有课；对照前端课程循环） ----
    if course.weeks.contains(&week) {
        // 停课优先（冻结契约 §2.5.1 两档，对照前端同名分支）；命中即短路实体块
        // 分支，但不短路下方 extra 循环
        let cancel_hit = cancel
            .map(|c| c.new_day.is_none() || c.new_day == Some(course.day))
            .unwrap_or(false);
        if cancel_hit {
            let c = cancel.unwrap();
            out.push(CourseOccurrence {
                kind: OccurrenceKind::Cancelled,
                day: course.day,
                start_section: course.start_section,
                end_section: course.end_section,
                position: course.position.clone(),
                source_override_id: Some(c.id.clone()),
            });
        } else if let Some(r) = resched {
            // 调课且新时间 ≠ 原时间 → 原时段虚线占位 + 新时段实体块（对照前端同名
            // 分支；结束节次缺省 = 起始，前端以大节 blockOf 表达同一语义）
            let moved = r.new_day.is_some()
                && r.new_start_section.is_some()
                && (r.new_day != Some(course.day)
                    || r.new_start_section.map(|s| (u32::from(s) + 1) / 2)
                        != course
                            .start_section
                            .map(|s| (u32::from(s) + 1) / 2));
            if moved {
                let start = r.new_start_section;
                out.push(CourseOccurrence {
                    kind: OccurrenceKind::MovedOut,
                    day: course.day,
                    start_section: course.start_section,
                    end_section: course.end_section,
                    position: course.position.clone(),
                    source_override_id: Some(r.id.clone()),
                });
                out.push(CourseOccurrence {
                    kind: OccurrenceKind::Solid,
                    day: r.new_day.unwrap_or(course.day),
                    start_section: start,
                    end_section: Some(r.new_end_section.unwrap_or(start.unwrap_or(1))),
                    position: r
                        .new_position
                        .clone()
                        .unwrap_or_else(|| course.position.clone()),
                    source_override_id: Some(r.id.clone()),
                });
            } else {
                // 原地（可能仅换教室，对照前端「原地」分支）
                out.push(CourseOccurrence {
                    kind: OccurrenceKind::Solid,
                    day: course.day,
                    start_section: course.start_section,
                    end_section: course.end_section,
                    position: r
                        .new_position
                        .clone()
                        .unwrap_or_else(|| course.position.clone()),
                    source_override_id: Some(r.id.clone()),
                });
            }
        } else {
            // 原位无调整
            out.push(CourseOccurrence {
                kind: OccurrenceKind::Solid,
                day: course.day,
                start_section: course.start_section,
                end_section: course.end_section,
                position: course.position.clone(),
                source_override_id: None,
            });
        }
    }

    // ---- 补课叠加：独立循环，不被停课/调课分支短路（对照前端 extra 循环，
    //      同样不看出 course.weeks；课程删除时后端级联清理 override，正常必命中） ----
    for ov in overrides {
        if ov.change_type != OverrideKind::Extra || !ov.weeks.contains(&week) {
            continue;
        }
        if ov.course_id != course.id {
            continue;
        }
        let start = ov.new_start_section.unwrap_or(1);
        out.push(CourseOccurrence {
            kind: OccurrenceKind::Solid,
            day: ov.new_day.unwrap_or(course.day),
            start_section: Some(start),
            end_section: Some(ov.new_end_section.unwrap_or(start)),
            position: ov
                .new_position
                .clone()
                .unwrap_or_else(|| course.position.clone()),
            source_override_id: Some(ov.id.clone()),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Course, CourseOverride, CourseSource, OverrideKind};

    fn course(id: &str, day: u8, start: u8, end: u8, weeks: Vec<u32>) -> Course {
        Course {
            id: id.into(),
            course_table_id: "default".into(),
            name: "信息安全".into(),
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

    fn ov(id: &str, course_id: &str, kind: OverrideKind, week: u32) -> CourseOverride {
        CourseOverride {
            id: id.into(),
            course_id: course_id.into(),
            weeks: vec![week],
            change_type: kind,
            new_day: None,
            new_start_section: None,
            new_end_section: None,
            new_position: None,
            source_notice_id: format!("notice-{id}"),
            auto_applied: false,
        }
    }

    /// 分支 1：本周无课 / 停开 / week=0 → 空。
    #[test]
    fn expand_returns_empty_for_out_of_week_or_disabled() {
        let c = course("a", 1, 1, 2, vec![1, 3]);
        assert!(expand_occurrences(&c, &[], 2).is_empty());
        assert!(expand_occurrences(&c, &[], 0).is_empty());
        let mut d = c.clone();
        d.disabled = true;
        assert!(expand_occurrences(&d, &[], 1).is_empty());
    }

    /// 分支 2a：停课 `new_day = None` → 整周全停（单条 Cancelled，无 Solid）。
    #[test]
    fn cancel_without_day_cancels_whole_week() {
        let c = course("a", 1, 1, 2, vec![3]);
        let o = ov("ov1", "a", OverrideKind::Cancelled, 3);
        let occ = expand_occurrences(&c, &[o], 3);
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].kind, OccurrenceKind::Cancelled);
        assert_eq!((occ[0].day, occ[0].start_section), (1, Some(1)));
    }

    /// 分支 2b：停课 `new_day` = 原星期 → 只停那一次；new_day ≠ 原星期 → 不生效
    /// （原时段照常 Solid）。
    #[test]
    fn cancel_with_day_only_bites_matching_day() {
        let c = course("a", 1, 1, 2, vec![3]);
        let hit = ov("ov1", "a", OverrideKind::Cancelled, 3);
        let mut hit = hit;
        hit.new_day = Some(1);
        let occ = expand_occurrences(&c, &[hit], 3);
        assert_eq!(occ[0].kind, OccurrenceKind::Cancelled);

        let miss = ov("ov2", "a", OverrideKind::Cancelled, 3);
        let mut miss = miss;
        miss.new_day = Some(4);
        let occ = expand_occurrences(&c, &[miss], 3);
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].kind, OccurrenceKind::Solid, "new_day 不匹配原星期 → 停课不生效");
    }

    /// 分支 3：调课跨天 → 原时段 MovedOut + 新时段 Solid（新节次/新教室）。
    #[test]
    fn resched_across_days_yields_moved_out_plus_solid() {
        let c = course("a", 1, 1, 2, vec![5]);
        let mut o = ov("ov1", "a", OverrideKind::Rescheduled, 5);
        o.new_day = Some(4);
        o.new_start_section = Some(3);
        o.new_end_section = Some(4);
        o.new_position = Some("D4-305".into());
        let occ = expand_occurrences(&c, &[o], 5);
        assert_eq!(occ.len(), 2);
        assert_eq!(occ[0].kind, OccurrenceKind::MovedOut);
        assert_eq!(occ[0].day, 1);
        assert_eq!(occ[0].position, "D4-207");
        assert_eq!(occ[1].kind, OccurrenceKind::Solid);
        assert_eq!((occ[1].day, occ[1].start_section, occ[1].end_section), (4, Some(3), Some(4)));
        assert_eq!(occ[1].position, "D4-305");
    }

    /// 分支 3b：调课结束节次缺省 = 起始节次（单节补调，对照前端「结束=起始」）。
    #[test]
    fn resched_defaults_end_section_to_start() {
        let c = course("a", 1, 1, 2, vec![5]);
        let mut o = ov("ov1", "a", OverrideKind::Rescheduled, 5);
        o.new_day = Some(2);
        o.new_start_section = Some(5);
        o.new_end_section = None;
        let occ = expand_occurrences(&c, &[o], 5);
        assert_eq!(occ[1].end_section, Some(5));
    }

    /// 分支 4：仅换教室（新时间 = 原时间）→ 原位单条 Solid、新教室、带 override 溯源。
    #[test]
    fn resched_room_only_keeps_slot_with_new_room() {
        let c = course("a", 1, 1, 2, vec![5]);
        let mut o = ov("ov1", "a", OverrideKind::Rescheduled, 5);
        o.new_day = Some(1);
        o.new_start_section = Some(1);
        o.new_end_section = Some(2);
        o.new_position = Some("D4-305".into());
        let occ = expand_occurrences(&c, &[o], 5);
        assert_eq!(occ.len(), 1, "同天同节仅换教室不产生 MovedOut");
        assert_eq!(occ[0].kind, OccurrenceKind::Solid);
        assert_eq!(occ[0].position, "D4-305");
        assert_eq!(occ[0].source_override_id.as_deref(), Some("ov1"));
    }

    /// 分支 5：原位无调整 → 单条 Solid 原样、无溯源。
    #[test]
    fn plain_course_yields_single_solid() {
        let c = course("a", 1, 1, 2, vec![5]);
        let occ = expand_occurrences(&c, &[], 5);
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].kind, OccurrenceKind::Solid);
        assert_eq!(occ[0].position, "D4-207");
        assert_eq!(occ[0].source_override_id, None);
    }

    /// 分支 6：补课 → 追加 Solid 新实体（day 缺省原 day，节次缺省第 1 小节起）。
    #[test]
    fn extra_appends_new_solid_occurrence() {
        let c = course("a", 1, 1, 2, vec![5]);
        let mut o = ov("ov1", "a", OverrideKind::Extra, 5);
        o.new_day = Some(6);
        o.new_start_section = Some(3);
        o.new_end_section = Some(4);
        let occ = expand_occurrences(&c, &[o], 5);
        assert_eq!(occ.len(), 2, "原时段照常 + 补课新实体");
        assert_eq!(occ[0].kind, OccurrenceKind::Solid);
        assert_eq!(occ[1].kind, OccurrenceKind::Solid);
        assert_eq!((occ[1].day, occ[1].start_section, occ[1].end_section), (6, Some(3), Some(4)));
        assert_eq!(occ[1].source_override_id.as_deref(), Some("ov1"));

        // 全缺省：day = 原 day，节次 = 第 1 小节起
        let bare = ov("ov2", "a", OverrideKind::Extra, 5);
        let occ = expand_occurrences(&c, &[bare], 5);
        assert_eq!((occ[1].day, occ[1].start_section, occ[1].end_section), (1, Some(1), Some(1)));
    }

    /// 分支 7：多条 override 逆序取最后（后采纳覆盖先采纳）。
    #[test]
    fn later_override_wins_over_earlier() {
        let c = course("a", 1, 1, 2, vec![5]);
        let mut first = ov("ov1", "a", OverrideKind::Rescheduled, 5);
        first.new_day = Some(2);
        first.new_start_section = Some(1);
        let mut second = ov("ov2", "a", OverrideKind::Rescheduled, 5);
        second.new_day = Some(3);
        second.new_start_section = Some(3);
        // 先采纳 ov1（调到周二）后采纳 ov2（调到周三）→ 生效的是 ov2
        let occ = expand_occurrences(&c, &[first.clone(), second.clone()], 5);
        assert_eq!(occ[1].day, 3);
        assert_eq!(occ[1].source_override_id.as_deref(), Some("ov2"));
        // 顺序反转 → 生效 ov1
        let occ = expand_occurrences(&c, &[second, first], 5);
        assert_eq!(occ[1].day, 2);
        assert_eq!(occ[1].source_override_id.as_deref(), Some("ov1"));
    }

    /// 分支 8：停课优先于调课（cancel 与 resched 同周并存 → 只出 Cancelled）。
    #[test]
    fn cancel_takes_priority_over_resched() {
        let c = course("a", 1, 1, 2, vec![5]);
        let mut res = ov("ov1", "a", OverrideKind::Rescheduled, 5);
        res.new_day = Some(4);
        res.new_start_section = Some(3);
        let cancel = ov("ov2", "a", OverrideKind::Cancelled, 5);
        let occ = expand_occurrences(&c, &[res, cancel], 5);
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].kind, OccurrenceKind::Cancelled);
    }

    /// 分支 9：custom 课（节次 None）→ 实例节次字段为 None（消费方按 custom 时刻取值）。
    #[test]
    fn custom_course_occurrence_has_none_sections() {
        let mut c = course("a", 1, 1, 2, vec![5]);
        c.is_custom_time = true;
        c.start_section = None;
        c.end_section = None;
        c.custom_start_time = Some("18:00".into());
        c.custom_end_time = Some("19:30".into());
        let occ = expand_occurrences(&c, &[], 5);
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].kind, OccurrenceKind::Solid);
        assert_eq!(occ[0].start_section, None);
        assert_eq!(occ[0].end_section, None);
    }

    /// 其他课程的 override 不串扰（course_id 过滤）。
    #[test]
    fn overrides_of_other_courses_are_ignored() {
        let c = course("a", 1, 1, 2, vec![5]);
        let mut o = ov("ov1", "b", OverrideKind::Cancelled, 5);
        o.new_day = Some(1);
        let occ = expand_occurrences(&c, &[o], 5);
        assert_eq!(occ[0].kind, OccurrenceKind::Solid);
    }

    // ---------------- 复核修复回归（P1-a / P1-b / P3-c） ----------------

    /// ① 停课命中 + 同周补课 → Cancelled + Solid 并存（extra 不被停课短路，
    ///    P1-a：前端网格有补课块，展开结果必须一致）。
    #[test]
    fn cancel_hit_and_extra_coexist() {
        let c = course("a", 1, 1, 2, vec![5]);
        let cancel = ov("ov1", "a", OverrideKind::Cancelled, 5); // new_day None = 整周停
        let mut extra = ov("ov2", "a", OverrideKind::Extra, 5);
        extra.new_day = Some(6);
        extra.new_start_section = Some(3);
        let occ = expand_occurrences(&c, &[cancel, extra], 5);
        assert_eq!(occ.len(), 2);
        assert_eq!(occ[0].kind, OccurrenceKind::Cancelled);
        assert_eq!(occ[1].kind, OccurrenceKind::Solid);
        assert_eq!((occ[1].day, occ[1].start_section), (6, Some(3)));
    }

    /// ② 调课移位 + 同周补课 → MovedOut + 2×Solid（extra 不被调课短路）。
    #[test]
    fn resched_move_and_extra_coexist() {
        let c = course("a", 1, 1, 2, vec![5]);
        let mut res = ov("ov1", "a", OverrideKind::Rescheduled, 5);
        res.new_day = Some(4);
        res.new_start_section = Some(3);
        let mut extra = ov("ov2", "a", OverrideKind::Extra, 5);
        extra.new_day = Some(6);
        let occ = expand_occurrences(&c, &[res, extra], 5);
        assert_eq!(occ.len(), 3);
        assert_eq!(occ[0].kind, OccurrenceKind::MovedOut);
        assert_eq!(occ[1].kind, OccurrenceKind::Solid);
        assert_eq!(occ[1].day, 4);
        assert_eq!(occ[2].kind, OccurrenceKind::Solid);
        assert_eq!(occ[2].day, 6);
    }

    /// ⑥ 补课与原位实例同日同起始节次 → 两条独立实例（base 无溯源、extra 有），
    ///    ICS 侧由 UID 后缀保证不撞（见 commands 层 ics_extra_same_slot_uids_distinct）。
    #[test]
    fn extra_same_slot_as_original_yields_two_solids() {
        let c = course("a", 1, 1, 2, vec![5]);
        let extra = ov("ov1", "a", OverrideKind::Extra, 5); // 全缺省：day=1, start=1
        let occ = expand_occurrences(&c, &[extra], 5);
        assert_eq!(occ.len(), 2);
        assert_eq!(occ[0].source_override_id, None, "原位实例无溯源");
        assert_eq!(occ[1].source_override_id.as_deref(), Some("ov1"));
        assert_eq!((occ[1].day, occ[1].start_section), (1, Some(1)));
    }

    /// ⑦ custom 课被 resched → MovedOut（节次 None）+ Solid（节次取 override 的
    ///    new_*，走大节表取值，P3-c 登记口径）。
    #[test]
    fn custom_course_resched_yields_sectioned_solid() {
        let mut c = course("a", 1, 1, 2, vec![5]);
        c.is_custom_time = true;
        c.start_section = None;
        c.end_section = None;
        c.custom_start_time = Some("18:00".into());
        c.custom_end_time = Some("19:30".into());
        let mut res = ov("ov1", "a", OverrideKind::Rescheduled, 5);
        res.new_day = Some(2);
        res.new_start_section = Some(7);
        res.new_end_section = Some(8);
        let occ = expand_occurrences(&c, &[res], 5);
        assert_eq!(occ.len(), 2);
        assert_eq!(occ[0].kind, OccurrenceKind::MovedOut);
        assert_eq!(occ[0].start_section, None);
        assert_eq!(occ[1].kind, OccurrenceKind::Solid);
        assert_eq!((occ[1].day, occ[1].start_section, occ[1].end_section), (2, Some(7), Some(8)));
    }

    /// ⑦ course.weeks 为空数组 → 无实体实例，但 extra 补课仍展开
    ///    （前端 extra 循环不看出 course.weeks，P1-b 的函数侧前提）。
    #[test]
    fn weeks_empty_but_extra_still_expands() {
        let c = course("a", 1, 1, 2, vec![]);
        let mut extra = ov("ov1", "a", OverrideKind::Extra, 1);
        extra.new_day = Some(3);
        extra.new_start_section = Some(5);
        assert!(expand_occurrences(&c, &[], 1).is_empty(), "无 override → 空");
        let occ = expand_occurrences(&c, &[extra], 1);
        assert_eq!(occ.len(), 1);
        assert_eq!(occ[0].kind, OccurrenceKind::Solid);
        assert_eq!((occ[0].day, occ[0].start_section), (3, Some(5)));
    }
}
