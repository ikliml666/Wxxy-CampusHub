//! 本地课表存储（%APPDATA%/campushub/timetable.json）。
//!
//! 与 session.json 不同，课表是**非凭据明文**（与 profile.json 同级），
//! 承载 `campus_schedule::Timetable`（冻结契约 §2.2：单文件、无数据库，
//! 原子性由调用方保证——本模块只做整体 read/write）。
//!
//! 读取宽容语义（冻结契约 §2.3 `get_timetable`）：文件**缺失或损坏**一律返回
//! 空课表（`courses: []`），不报错——课表页首屏不能因存储异常白屏。

use campus_portal::section_time_slots;
use campus_schedule::model::{CourseTableConfig, Timetable};
use std::fs;
use std::path::{Path, PathBuf};

/// 默认课表 ID（单课表阶段唯一取值；多课表是后续演进项，结构已预留
/// `course_table_id` 字段隔离）。与导入时 `parse_kb_response(json, <id>)` 保持一致。
pub const DEFAULT_TABLE_ID: &str = "default";

fn timetable_path(dir: &Path) -> PathBuf {
    dir.join("timetable.json")
}

/// 空课表（文件缺失/损坏时的读取结果，也是导入前的初始状态）。
/// `updated_at` 空串 = 从未更新。
pub fn empty_timetable() -> Timetable {
    Timetable {
        config: CourseTableConfig {
            course_table_id: DEFAULT_TABLE_ID.to_string(),
            // 契约 §17：默认显示周末（与 serde default true 同口径，新用户首屏即 7 列）
            show_weekends: true,
            semester_start_date: None,
            semester_total_weeks: 20,
            first_day_of_week: 1,
            slots: None,
            skipped_dates: Vec::new(),
            swap_days: vec![],
            holiday_names: vec![],
            slot_rules: Vec::new(),
            show_non_current_week: false,
            last_auto_import: None,
            last_holiday_fetch: None,
        },
        courses: Vec::new(),
        overrides: Vec::new(),
        updated_at: String::new(),
    }
}

/// 小节口径迁移（重设计轮批 A，tauri 层契约 §18）：`config.slots` / `slot_rules`
/// 语义已从「大节表（5 行）」改为「小节表（恒 11 行，[`section_time_slots`]）」。
/// 非 11 行的 slots / 规则在 load 时**一次性丢弃**回落内置小节表——取舍：现存
/// 用户数据 slots 均为 None（旧大节自定义作息一次性弃用，零实际影响），保留
/// 旧大节数据反而会让小节号查表语义错乱。
///
/// ponytail: 迁移判据 = 「行数 != 11」这一近似而非字节比对大节表；后端校验
/// （validate_time_slots）不锁死行数，故手改 JSON 的非 11 行表同样会被丢——
/// 前端批 B 小节编辑器恒定产出 11 行后，该张力自然消失；需要放宽时升级为
/// 「仅丢弃与旧大节表逐字节相等的 slots」。
fn migrate_section_slots(tt: &mut Timetable) {
    let section_len = section_time_slots().len(); // 恒 11（parse.rs 内置表钉死）
    if tt.config.slots.as_ref().is_some_and(|s| s.len() != section_len) {
        tt.config.slots = None;
    }
    tt.config.slot_rules.retain(|r| r.slots.len() == section_len);
}

/// 读取本地课表。缺失/损坏/反序列化失败 → 空课表（不报错，不删除坏文件——
/// 保留现场便于诊断，下次整体写入时自然覆盖）；读出成功后套小节口径迁移。
pub fn load_timetable(dir: &Path) -> Timetable {
    let raw = match fs::read_to_string(timetable_path(dir)) {
        Ok(raw) => raw,
        Err(_) => return empty_timetable(),
    };
    match serde_json::from_str(&raw) {
        Ok(mut tt) => {
            migrate_section_slots(&mut tt);
            tt
        }
        Err(e) => {
            log::warn!("timetable.json 损坏，回退空课表: {e}");
            empty_timetable()
        }
    }
}

/// 整体写入本地课表（上次内容的原子性/备份由调用方决策；本模块不追加命名约定）。
pub fn save_timetable(dir: &Path, timetable: &Timetable) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let json = serde_json::to_string_pretty(timetable).map_err(|e| e.to_string())?;
    fs::write(timetable_path(dir), json).map_err(|e| format!("写 timetable.json 失败: {e}"))
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use campus_schedule::model::{Course, CourseOverride, CourseSource, OverrideKind};
    use std::{fs, time::{SystemTime, UNIX_EPOCH}};

    fn temp_dir(tag: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("campushub-tt-{tag}-{n}"))
    }

    fn sample() -> Timetable {
        let mut tt = empty_timetable();
        tt.updated_at = "2026-09-18T10:00:00+08:00".into();
        tt.courses.push(Course {
            id: "default-abc".into(),
            course_table_id: DEFAULT_TABLE_ID.into(),
            name: "信息安全".into(),
            teacher: "某老师".into(),
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
            class_id: Some("abc".into()),
            disabled: false,
        });
        tt.overrides.push(CourseOverride {
            id: "ov1".into(),
            course_id: "default-abc".into(),
            weeks: vec![3],
            change_type: OverrideKind::Rescheduled,
            new_day: Some(5),
            new_start_section: Some(6),
            new_end_section: Some(7),
            new_position: Some("D4-305".into()),
            source_notice_id: "manual:deadbeef".into(),
            auto_applied: false,
        });
        tt
    }

    /// 往返：save → load 与原值相等；落盘为 JSON 明文且顶层键 camelCase。
    #[test]
    fn timetable_roundtrip_preserves_all_sections() {
        let dir = temp_dir("rt");
        let tt = sample();
        save_timetable(&dir, &tt).unwrap();

        let raw = fs::read_to_string(timetable_path(&dir)).unwrap();
        assert!(raw.contains("\"updatedAt\""));
        assert!(raw.contains("\"overrides\""));

        let loaded = load_timetable(&dir);
        assert_eq!(loaded, tt);

        fs::remove_dir_all(&dir).ok();
    }

    /// 缺失 → 空课表（courses 为空、不报错）；损坏 JSON → 同样回空且不删坏文件。
    #[test]
    fn timetable_missing_or_corrupt_falls_back_to_empty() {
        let dir = temp_dir("missing");
        let tt = load_timetable(&dir);
        assert!(tt.courses.is_empty());
        assert_eq!(tt.config.course_table_id, DEFAULT_TABLE_ID);
        assert_eq!(tt.updated_at, "");

        // 损坏文件：回空 + 文件保留（不静默清除现场）
        let corrupt = temp_dir("corrupt");
        fs::create_dir_all(&corrupt).unwrap();
        fs::write(timetable_path(&corrupt), "{ not json").unwrap();
        let tt = load_timetable(&corrupt);
        assert!(tt.courses.is_empty());
        assert!(timetable_path(&corrupt).exists());

        fs::remove_dir_all(&corrupt).ok();
    }

    /// 小节口径迁移（重设计轮批 A 契约 §18）：load 时非 11 行的 config.slots /
    /// slot_rules 一次性丢弃回落内置（旧大节自定义作息弃用），11 行规则保留。
    #[test]
    fn timetable_load_migrates_block_slots_to_section() {
        use campus_portal::section_time_slots as builtin_sections;
        use campus_schedule::model::{SlotRule, TimeSlot};

        let dir = temp_dir("migrate");
        let mut tt = sample();
        let old_block = TimeSlot {
            number: 1,
            start_time: "08:00".into(),
            end_time: "09:40".into(),
            alias: None,
        };
        tt.config.slots = Some(vec![old_block.clone()]); // 旧大节形态（len != 11）
        tt.config.slot_rules = vec![
            SlotRule {
                start_date: chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
                end_date: chrono::NaiveDate::from_ymd_opt(2027, 2, 28).unwrap(),
                slots: vec![old_block], // 旧大节规则 → 丢弃
            },
            SlotRule {
                start_date: chrono::NaiveDate::from_ymd_opt(2027, 3, 1).unwrap(),
                end_date: chrono::NaiveDate::from_ymd_opt(2027, 4, 30).unwrap(),
                slots: builtin_sections(), // 11 行小节规则 → 保留
            },
        ];
        save_timetable(&dir, &tt).unwrap();

        let loaded = load_timetable(&dir);
        assert!(loaded.config.slots.is_none(), "非 11 行主作息应被丢弃回落内置");
        assert_eq!(loaded.config.slot_rules.len(), 1, "非 11 行规则应被丢弃");
        assert_eq!(loaded.config.slot_rules[0].slots.len(), 11);
        // 保留的规则经 effective 口径回落正确（此处仅确认内置表恒 11 行）
        assert_eq!(builtin_sections().len(), 11);

        fs::remove_dir_all(&dir).ok();
    }
}
