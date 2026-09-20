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

/// 匹配键：课程名 + `class_id` + **时段（星期/起止小节）**。正方同一教学班
/// （同 `jxb_id`）会在 kbList 里按不同上课时段拆成多条记录（2026-09-19 实测：
/// 马原 3 条共用同一 `jxb_id`），键若只有名字+class_id，二次导入的更新分支会
/// 把多条本地课全部覆盖成第一条的时段（数据损坏），因此时段必须入键——
/// 时段变更的语义 = 旧时段停开 + 新时段新增（契约 §20 修订）。
fn match_key(c: &Course) -> (&str, Option<&str>, u8, Option<u8>, Option<u8>) {
    (
        c.name.as_str(),
        c.class_id.as_deref(),
        c.day,
        c.start_section,
        c.end_section,
    )
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

/// 合并教务最新课程列表进本地课程列表（冻结契约 §2.4 + §20 修订，见模块文档）。
///
/// `existing`：本地库当前课程列表（可含 Manual 与已停开的 Import 课程）；
/// `incoming`：教务导入列表（`parse_kb_response` 输出，全部 `source=Import`）。
pub fn diff_courses(existing: &[Course], incoming: &[Course]) -> DiffResult {
    let mut courses = Vec::with_capacity(existing.len() + incoming.len());
    let mut changes: Vec<String> = Vec::new();
    let (mut added, mut changed, mut removed) = (0u32, 0u32, 0u32);

    // incoming 按 match_key 分桶（索引队列）：旧库消费式一对一配对。同 key 多条
    // （同班多时段 / 同一时段多条记录）按顺序一一对应，杜绝旧实现 `find` 命中
    // 首条导致的多对一覆盖（2026-09-19 实测把马原 3 条全改写成同一时段的数据损坏）。
    let mut pool: std::collections::HashMap<
        (&str, Option<&str>, u8, Option<u8>, Option<u8>),
        std::collections::VecDeque<usize>,
    > = std::collections::HashMap::new();
    for (i, new) in incoming.iter().enumerate() {
        pool.entry(match_key(new)).or_default().push_back(i);
    }
    let mut consumed = vec![false; incoming.len()];

    // 1) 旧库逐条：Manual 原样保留；Import 按 key 消费式匹配 incoming
    for old in existing {
        if old.source == CourseSource::Manual {
            // 冻结契约：Manual 永不参与 diff，永不被打 disabled
            courses.push(old.clone());
            continue;
        }
        match pool.get_mut(&match_key(old)).and_then(|q| q.pop_front()) {
            Some(idx) => {
                consumed[idx] = true;
                let new = &incoming[idx];
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

    // 2) 未被旧库消费的 incoming → 新增（首次导入 existing 为空时全部走这里；
    //    同 key 多条各自计入，与消费式配对语义一致）
    for (i, new) in incoming.iter().enumerate() {
        if !consumed[i] {
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

/// 教务已覆盖判定（每日自动导入的「教务为准」清理与置换采纳校验共用）：
/// 存在未停开的导入课程，在教学周 `week` 的 `weekday`（1=周一 … 7=周日）有课。
/// 依据（2026-09-20 取证）：教务 kbList 以「新增同教学班条目（xqj=7、zcd 锁定
/// 补课周）」表达调休补课——置换日所在教学周该星期有教务课 ⇒ 调休已进本地
/// 课表，同一事实的公告置换表达即冗余。
pub fn weekday_covered(courses: &[Course], week: u32, weekday: u8) -> bool {
    courses.iter().any(|c| {
        c.source == CourseSource::Import && !c.disabled && c.day == weekday && c.weeks.contains(&week)
    })
}

/// 冗余 extra override 判定（导入后清理）：公告「补课」override 的新时段与
/// 教务课表已有的导入课程条目完全重合（同名、双方都有 jxb_id 时要求一致、
/// 同星期同起止小节、override 周次全覆盖）⇒ 教务已表达该次补课，override
/// 冗余应删，否则同格重复渲染（2026-09-20 用户报「同一门课挤在同一格」）。
/// 原课已删除（course_id 失配）的 override 不判冗余，保守保留。
pub fn redundant_extra_override_ids(
    courses: &[Course],
    overrides: &[crate::model::CourseOverride],
) -> Vec<String> {
    use crate::model::OverrideKind;
    let by_id: std::collections::HashMap<&str, &Course> =
        courses.iter().map(|c| (c.id.as_str(), c)).collect();
    overrides
        .iter()
        .filter(|ov| {
            if ov.change_type != OverrideKind::Extra {
                return false;
            }
            let Some(origin) = by_id.get(ov.course_id.as_str()) else {
                return false;
            };
            let day = ov.new_day.unwrap_or(origin.day);
            let (Some(s), Some(e)) = (ov.new_start_section, ov.new_end_section) else {
                return false;
            };
            courses.iter().any(|c| {
                c.source == CourseSource::Import
                    && !c.disabled
                    && c.name == origin.name
                    && (origin.class_id.is_none() || c.class_id == origin.class_id)
                    && c.day == day
                    && c.start_section == Some(s)
                    && c.end_section == Some(e)
                    && ov.weeks.iter().all(|w| c.weeks.contains(w))
            })
        })
        .map(|ov| ov.id.clone())
        .collect()
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

    /// 回归（2026-09-19 真机数据损坏）：正方同一教学班（同 jxb_id）按多时段
    /// 拆多条，match_key 旧版（名+class_id）在二次导入时把多条本地课全部覆盖成
    /// 第一条的时段。修复 = 时段入键 + 消费式一对一配对。
    #[test]
    fn diff_pairs_same_class_multi_section_one_to_one() {
        let mk = |id: &str, day: u8, s: u8| {
            let mut c = course(id, "马克思主义基本原理", Some("SAME_JXB"));
            c.day = day;
            c.start_section = Some(s);
            c.end_section = Some(s + 1);
            c
        };
        // 教务：三个时段（周一5-6 / 周三3-4 / 周四3-4），class_id 全同
        let inc = vec![mk("new-a", 1, 5), mk("new-b", 3, 3), mk("new-c", 4, 3)];
        // 旧库：同样是这三条（乱序）
        let old = vec![mk("old-1", 4, 3), mk("old-2", 1, 5), mk("old-3", 3, 3)];
        let r = diff_courses(&old, &inc);
        assert_eq!((r.added, r.changed, r.removed), (0, 0, 0), "同数据重导应零变化");
        // 每条本地课的时段各自保留，不被覆盖成同一条
        let mut got: Vec<(u8, u8)> = r
            .courses
            .iter()
            .map(|c| (c.day, c.start_section.unwrap()))
            .collect();
        got.sort();
        assert_eq!(got, vec![(1, 5), (3, 3), (4, 3)]);
        // id 稳定（override 挂靠）
        assert_eq!(r.courses[0].id, "old-1");
    }

    /// 同 key 的本地重复记录只消费一条 incoming，多余条目停开（不再被覆盖复制）。
    #[test]
    fn diff_duplicate_local_keys_only_one_consumed() {
        let inc = vec![course("new-a", "信息安全", Some("A"))];
        let old = vec![
            course("old-1", "信息安全", Some("A")),
            course("old-2", "信息安全", Some("A")),
        ];
        let r = diff_courses(&old, &inc);
        assert_eq!((r.added, r.removed), (0, 1));
        assert_eq!(r.courses.iter().filter(|c| !c.disabled).count(), 1);
        assert_eq!(r.courses.iter().filter(|c| c.disabled).count(), 1);
    }

    /// weekday_covered：该周该星期有未停开导入课才算已覆盖（2026-09-20 取证口径）。
    #[test]
    fn weekday_covered_matches_import_course_in_week() {
        let mut c = course("default-a", "信息安全", Some("A"));
        c.day = 7;
        c.weeks = vec![2];
        assert!(weekday_covered(&[c.clone()], 2, 7));
        // 周次不含 → 未覆盖
        assert!(!weekday_covered(&[c.clone()], 5, 7));
        // Manual 不算教务覆盖
        let mut m = c.clone();
        m.source = CourseSource::Manual;
        assert!(!weekday_covered(&[m.clone()], 2, 7));
        // 停开不算
        let mut d = c.clone();
        d.disabled = true;
        assert!(!weekday_covered(&[d], 2, 7));
    }

    /// redundant_extra_override_ids：与教务条目完全重合的 extra 判冗余；
    /// 时段不同 / jxb 不同 / Manual 重合 / rescheduled 类型都不算。
    #[test]
    fn redundant_extra_requires_exact_overlap() {
        use crate::model::{CourseOverride, OverrideKind};
        let ov = |id: &str, course_id: &str, day: u8| CourseOverride {
            id: id.into(),
            course_id: course_id.into(),
            weeks: vec![5],
            change_type: OverrideKind::Extra,
            new_day: Some(day),
            new_start_section: Some(3),
            new_end_section: Some(4),
            new_position: None,
            source_notice_id: "n1".into(),
            auto_applied: true,
        };
        let mut origin = course("old-1", "信息安全", Some("A"));
        origin.day = 1; // 原课周一
        let mut swaped = course("new-swap", "信息安全", Some("A"));
        swaped.day = 7; // 教务调休条目：周日 3-4 节第 5 周
        swaped.weeks = vec![5];
        // 教务已排周日 3-4 → extra(周日 3-4) 冗余
        assert_eq!(
            redundant_extra_override_ids(&[origin.clone(), swaped.clone()], &[ov("o1", "old-1", 7)]),
            vec!["o1"]
        );
        // 教务没排周日 → 保留
        assert!(redundant_extra_override_ids(&[origin.clone()], &[ov("o1", "old-1", 7)]).is_empty());
        // 时段不同 → 保留
        let mut other = swaped.clone();
        other.start_section = Some(5);
        other.end_section = Some(6);
        assert!(redundant_extra_override_ids(&[origin.clone(), other], &[ov("o1", "old-1", 7)]).is_empty());
        // jxb 不一致的同名条目 → 不算覆盖（保守保留）
        let mut other_jxb = swaped.clone();
        other_jxb.class_id = Some("B".into());
        assert!(redundant_extra_override_ids(&[origin.clone(), other_jxb], &[ov("o1", "old-1", 7)]).is_empty());
        // 周次未全覆盖（教务只排第 5 周，override 还要第 7 周）→ 保留
        let mut wide = ov("o1", "old-1", 7);
        wide.weeks = vec![5, 7];
        assert!(redundant_extra_override_ids(&[origin, swaped], &[wide]).is_empty());
    }
}
