// Wxxy-CampusHub original code (no upstream counterpart):
// 本地课表 ↔ 教务最新课表的自动对比更新（M2.5 冻结契约 §2.4）。

//! 导入 diff：[`diff_courses`] 把教务最新课程列表合并进本地课程列表。
//!
//! 冻结语义（计划 §2.4，不得改动）：
//! - 匹配键 = **课程名 + `class_id`**（教务 `jxb_id`）；`class_id` 缺失时退化
//!   为仅课程名匹配；
//! - 新出现 → 新增（`source=Import`）；消失 → **置 `disabled=true`，不删记录**
//!  （保留其挂载的调课 override 可回滚）；字段变化 → 只更新 `Import` 课程；
//! - **`source=Manual` 的课程永不参与 diff，永不被打上 `disabled`**。
//!
//! 输出为「合并后的完整课程列表」（直接写入本地库）+ 变更计数 + 人类可读
//! 变更条目（如 `信息安全 教室 D4-207 → D4-305`）。纯函数，无 IO。

use crate::model::{Course, CourseSource};

/// [`diff_courses`] 的合并结果。
#[derive(Debug, Clone, PartialEq)]
pub struct DiffResult {
    /// 合并后的完整课程列表（旧库 + 新增 + 更新；消失的 Import 课程置
    /// `disabled=true` 后原位保留；Manual 课程原样保留）。直接写入本地库。
    pub courses: Vec<Course>,
    /// 新增（Import）课程数。
    pub added: u32,
    /// 字段变化（或停开课程复活）的课程数。
    pub changed: u32,
    /// 本次新发现停开（消失）的课程数。已停开课程再次消失**不重复计数**。
    pub removed: u32,
    /// 人类可读变更条目（新增 / 变化 / 停开），如 `信息安全 教室 D4-207 → D4-305`。
    pub changes: Vec<String>,
}

/// 匹配键：课程名 + `class_id`（缺失 → None，同名无 id 的课程互相匹配——
/// 冻结契约 §2.4 的退化语义）。
fn match_key(c: &Course) -> (&str, Option<&str>) {
    (c.name.as_str(), c.class_id.as_deref())
}

/// 参与字段级 diff 的展示字段：星期 / 节次 / 周次 / 教室 / 教师。
/// （`name`/`class_id` 是匹配键本身；`color_index` 由课程名稳定散列、`remark`
/// 为教务分类拼接，不产生用户可感知的位置变化，不参与。）
fn field_changes(old: &Course, new: &Course) -> Vec<String> {
    let mut out = Vec::new();
    if old.day != new.day {
        out.push(format!("星期{} → 星期{}", weekday_name(old.day), weekday_name(new.day)));
    }
    if old.start_section != new.start_section || old.end_section != new.end_section {
        out.push(format!(
            "节次 {} → {}",
            format_sections(old.start_section, old.end_section),
            format_sections(new.start_section, new.end_section)
        ));
    }
    // 周次列表顺序无关（旧库经手工编辑后顺序可能乱），排序后比较
    let old_weeks = sorted_weeks(&old.weeks);
    let new_weeks = sorted_weeks(&new.weeks);
    if old_weeks != new_weeks {
        out.push(format!(
            "周次 {} → {}",
            format_weeks(&old_weeks),
            format_weeks(&new_weeks)
        ));
    }
    if old.position != new.position {
        out.push(format!("教室 {} → {}", old.position, new.position));
    }
    if old.teacher != new.teacher {
        out.push(format!("教师 {} → {}", old.teacher, new.teacher));
    }
    out
}

fn sorted_weeks(weeks: &[u32]) -> Vec<u32> {
    let mut v = weeks.to_vec();
    v.sort_unstable();
    v
}

/// 星期 1-7 → 中文（越界回落数字，防御手工编辑的脏数据）。
fn weekday_name(day: u8) -> String {
    match day {
        1 => "一".into(),
        2 => "二".into(),
        3 => "三".into(),
        4 => "四".into(),
        5 => "五".into(),
        6 => "六".into(),
        7 => "日".into(),
        other => other.to_string(),
    }
}

fn format_sections(start: Option<u8>, end: Option<u8>) -> String {
    match (start, end) {
        (Some(s), Some(e)) => format!("{s}-{e}节"),
        (Some(s), None) => format!("{s}节"),
        _ => "无".into(),
    }
}

/// 周次列表 → 紧凑文案：连续区间合并（`1,2,3,7,8` → `1-3,7-8`）。
/// 单双周后缀（批 5 §11.6，与前端 fmtWeeks 同语义互锚，改一处必须同步另一处）：
/// 全奇数且 ≥3 项 → 追加 `(单周)`；全偶数（任意项数）→ 追加 `(双周)`；其余不变。
pub fn format_weeks(weeks: &[u32]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut iter = weeks.iter().copied();
    let Some(first) = iter.next() else {
        return String::new();
    };
    let (mut run_start, mut run_prev) = (first, first);
    for w in iter {
        if w == run_prev + 1 {
            run_prev = w;
            continue;
        }
        push_week_run(&mut parts, run_start, run_prev);
        run_start = w;
        run_prev = w;
    }
    push_week_run(&mut parts, run_start, run_prev);
    let out = parts.join(",");
    if weeks.len() >= 3 && weeks.iter().all(|w| w % 2 == 1) {
        return format!("{out}(单周)");
    }
    if weeks.iter().all(|w| w % 2 == 0) {
        return format!("{out}(双周)");
    }
    out
}

fn push_week_run(parts: &mut Vec<String>, start: u32, end: u32) {
    if start == end {
        parts.push(start.to_string());
    } else {
        parts.push(format!("{start}-{end}"));
    }
}

/// 合并教务最新课程列表进本地课程列表（冻结契约 §2.4，见模块文档）。
///
/// `existing`：本地库当前课程列表（可含 Manual 与已停开的 Import 课程）；
/// `incoming`：教务导入列表（`parse_kb_response` 输出，全部 `source=Import`）。
pub fn diff_courses(existing: &[Course], incoming: &[Course]) -> DiffResult {
    let mut courses = Vec::with_capacity(existing.len() + incoming.len());
    let mut changes: Vec<String> = Vec::new();
    let (mut added, mut changed, mut removed) = (0u32, 0u32, 0u32);

    // 1) 旧库逐条：Manual 原样保留；Import 按 key 匹配 incoming
    for old in existing {
        if old.source == CourseSource::Manual {
            // 冻结契约：Manual 永不参与 diff，永不被打 disabled
            courses.push(old.clone());
            continue;
        }
        match incoming.iter().find(|new| match_key(new) == match_key(old)) {
            Some(new) => {
                let field_diff = field_changes(old, new);
                if old.disabled || !field_diff.is_empty() {
                    changed += 1;
                    if old.disabled {
                        changes.push(format!("{} 恢复开课", new.name));
                    }
                    if !field_diff.is_empty() {
                        changes.push(format!("{} {}", new.name, field_diff.join("；")));
                    }
                }
                // 整条采用新数据（教务为准），但 id 沿用旧库——调课 override
                // 挂在 course_id 上，id 必须稳定（class_id 缺失退化匹配时
                // incoming 的 `<table_id>-<空 jxb_id>` 可能与旧 id 不同）
                let mut merged = new.clone();
                merged.id = old.id.clone();
                merged.disabled = false;
                courses.push(merged);
            }
            None => {
                // 消失 → 置 disabled（已停开的不再计数/重复报文案）
                let mut gone = old.clone();
                if !gone.disabled {
                    gone.disabled = true;
                    removed += 1;
                    changes.push(format!("{} 停开", old.name));
                }
                courses.push(gone);
            }
        }
    }

    // 2) incoming 中旧库没有的 key → 新增
    let existing_keys: std::collections::HashSet<(String, Option<String>)> = existing
        .iter()
        .filter(|c| c.source != CourseSource::Manual)
        .map(|c| (c.name.clone(), c.class_id.clone()))
        .collect();
    for new in incoming {
        if !existing_keys.contains(&(new.name.clone(), new.class_id.clone())) {
            added += 1;
            changes.push(format!("新增 {}", new.name));
            courses.push(new.clone());
        }
    }

    DiffResult {
        courses,
        added,
        changed,
        removed,
        changes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小可用课程（其余字段按 parse_kb_response 输出形态缺省）。
    fn course(id: &str, name: &str, class_id: Option<&str>) -> Course {
        Course {
            id: id.into(),
            course_table_id: "default".into(),
            name: name.into(),
            teacher: "张老师".into(),
            position: "D4-207".into(),
            day: 3,
            start_section: Some(3),
            end_section: Some(4),
            is_custom_time: false,
            custom_start_time: None,
            custom_end_time: None,
            color_index: 2,
            remark: None,
            source: CourseSource::Import,
            weeks: vec![1, 2, 3],
            class_id: class_id.map(Into::into),
            disabled: false,
        }
    }

    #[test]
    fn diff_adds_new_courses() {
        let inc = vec![course("default-a", "信息安全", Some("A")), course("default-b", "密码学", Some("B"))];
        let r = diff_courses(&[], &inc);
        assert_eq!((r.added, r.changed, r.removed), (2, 0, 0));
        assert_eq!(r.courses.len(), 2);
        assert_eq!(r.changes, vec!["新增 信息安全", "新增 密码学"]);
    }

    #[test]
    fn diff_field_changes_with_contract_wording() {
        let old = course("default-a", "信息安全", Some("A"));
        let mut new = course("default-a", "信息安全", Some("A"));
        new.position = "D4-305".into();
        new.teacher = "李老师".into();
        let r = diff_courses(&[old.clone()], &[new.clone()]);
        assert_eq!((r.added, r.changed, r.removed), (0, 1, 0));
        // 契约示例文案形态（多字段以「；」连接）
        assert_eq!(r.changes, vec!["信息安全 教室 D4-207 → D4-305；教师 张老师 → 李老师"]);
        // 更新后保留旧库 id（override 挂 course_id，必须稳定）且未停开
        assert_eq!(r.courses[0].id, old.id);
        assert_eq!(r.courses[0].position, "D4-305");
        assert!(!r.courses[0].disabled);
    }

    #[test]
    fn diff_marks_missing_as_disabled_without_deleting() {
        let old = vec![course("default-a", "信息安全", Some("A")), course("default-b", "密码学", Some("B"))];
        let inc = vec![course("default-a", "信息安全", Some("A"))];
        let r = diff_courses(&old, &inc);
        assert_eq!((r.added, r.changed, r.removed), (0, 0, 1));
        assert_eq!(r.courses.len(), 2, "停开课程保留记录不删除");
        let gone = r.courses.iter().find(|c| c.name == "密码学").unwrap();
        assert!(gone.disabled);
        assert!(!r.courses.iter().find(|c| c.name == "信息安全").unwrap().disabled);
        assert_eq!(r.changes, vec!["密码学 停开"]);
    }

    /// 冻结契约 §2.4：Manual 零触碰——不参与匹配、不被更新、永不被置 disabled。
    #[test]
    fn diff_never_touches_manual_courses() {
        let mut manual = course("manual-1", "信息安全", None);
        manual.source = CourseSource::Manual;
        manual.position = "手工填的教室".into();
        manual.disabled = false;
        // incoming 同名同 id 的 Import 课程（教室不同）——不得影响 Manual
        let inc = vec![course("default-a", "信息安全", Some("A"))];
        let r = diff_courses(&[manual.clone()], &inc);
        assert_eq!((r.added, r.changed, r.removed), (1, 0, 0));
        let kept = r.courses.iter().find(|c| c.id == "manual-1").unwrap();
        assert_eq!(kept.source, CourseSource::Manual);
        assert_eq!(kept.position, "手工填的教室");
        assert!(!kept.disabled);
        assert_eq!(r.changes, vec!["新增 信息安全"]);
        // Manual 课程消失也绝不停开：库里只有 Manual，incoming 为空
        let r = diff_courses(&[manual], &[]);
        assert_eq!((r.added, r.changed, r.removed), (0, 0, 0));
        assert!(!r.courses[0].disabled);
        assert!(r.changes.is_empty());
    }

    /// class_id 缺失 → 退化仅课程名匹配。
    #[test]
    fn diff_falls_back_to_name_when_class_id_missing() {
        let old = course("old-1", "体育", None);
        let mut new = course("default-", "体育", None);
        new.position = "操场".into();
        let r = diff_courses(&[old], &[new]);
        assert_eq!((r.added, r.changed, r.removed), (0, 1, 0));
        assert_eq!(r.courses[0].id, "old-1", "退化匹配沿用旧 id");
        assert_eq!(r.courses[0].position, "操场");
    }

    /// 停开课程再次出现 → 复活（disabled=false），计入 changed。
    #[test]
    fn diff_revives_disabled_course() {
        let mut old = course("default-a", "信息安全", Some("A"));
        old.disabled = true;
        let inc = vec![course("default-a", "信息安全", Some("A"))];
        let r = diff_courses(&[old], &inc);
        assert_eq!((r.added, r.changed, r.removed), (0, 1, 0));
        assert!(!r.courses[0].disabled);
        assert_eq!(r.changes, vec!["信息安全 恢复开课"]);
    }

    /// 已停开课程再次消失：不重复计数、不重复报文案。
    #[test]
    fn diff_second_run_no_duplicate_removed() {
        let mut old = course("default-a", "信息安全", Some("A"));
        old.disabled = true;
        let r = diff_courses(&[old], &[]);
        assert_eq!((r.added, r.changed, r.removed), (0, 0, 0));
        assert!(r.changes.is_empty());
        assert!(r.courses[0].disabled);
    }

    #[test]
    fn diff_identical_data_is_noop() {
        let old = vec![course("default-a", "信息安全", Some("A"))];
        let inc = vec![course("default-a", "信息安全", Some("A"))];
        let r = diff_courses(&old, &inc);
        assert_eq!((r.added, r.changed, r.removed), (0, 0, 0));
        assert!(r.changes.is_empty());
        assert_eq!(r.courses, old);
    }

    /// 周次比较顺序无关（旧库手工编辑后顺序乱不影响 diff）。
    #[test]
    fn diff_weeks_order_insensitive() {
        let mut old = course("default-a", "信息安全", Some("A"));
        old.weeks = vec![1, 2, 3];
        let mut new = course("default-a", "信息安全", Some("A"));
        new.weeks = vec![3, 1, 2];
        let r = diff_courses(&[old], &[new]);
        assert_eq!((r.added, r.changed, r.removed), (0, 0, 0));
    }

    #[test]
    fn format_weeks_merges_consecutive_runs() {
        assert_eq!(format_weeks(&[1, 2, 3, 7, 8]), "1-3,7-8");
        assert_eq!(format_weeks(&[5]), "5");
        assert_eq!(format_weeks(&[]), "");
        // 教务 4095 位掩码展开 1..=12 → "1-12"
        assert_eq!(format_weeks(&crate::expand_week_mask(4095)), "1-12");
    }

    /// 单双周后缀（批 5 §11.6，与前端 fmtWeeks 同语义互锚）：全奇 ≥3 项 →
    /// `(单周)`；全偶任意项数 → `(双周)`；混合 / 全奇不足 3 项 / 空列表不变。
    #[test]
    fn format_weeks_odd_even_suffix() {
        assert_eq!(format_weeks(&[1, 3, 5]), "1,3,5(单周)");
        assert_eq!(format_weeks(&[1, 3, 5, 7]), "1,3,5,7(单周)");
        assert_eq!(format_weeks(&[1, 3, 5, 7, 9, 11, 13, 15]), "1,3,5,7,9,11,13,15(单周)");
        assert_eq!(format_weeks(&[2, 4, 6]), "2,4,6(双周)");
        assert_eq!(format_weeks(&[2]), "2(双周)", "全偶任意项数都加后缀");
        assert_eq!(format_weeks(&[1, 2, 3]), "1-3", "混合奇偶不加后缀");
        assert_eq!(format_weeks(&[1, 3]), "1,3", "全奇但不足 3 项不加后缀");
        assert_eq!(format_weeks(&[5]), "5", "单项奇数不加后缀");
        assert_eq!(format_weeks(&[]), "");
    }

    #[test]
    fn weekday_and_section_formatting() {
        assert_eq!(weekday_name(1), "一");
        assert_eq!(weekday_name(7), "日");
        assert_eq!(weekday_name(9), "9");
        assert_eq!(format_sections(Some(3), Some(4)), "3-4节");
        assert_eq!(format_sections(None, None), "无");
    }
}
