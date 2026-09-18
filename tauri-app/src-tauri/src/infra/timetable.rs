//! 本地课表存储（%APPDATA%/campushub/timetable.json）。
//!
//! 与 session.json 不同，课表是**非凭据明文**（与 profile.json 同级），
//! 承载 `campus_schedule::Timetable`（冻结契约 §2.2：单文件、无数据库，
//! 原子性由调用方保证——本模块只做整体 read/write）。
//!
//! 读取宽容语义（冻结契约 §2.3 `get_timetable`）：文件**缺失或损坏**一律返回
//! 空课表（`courses: []`），不报错——课表页首屏不能因存储异常白屏。

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
            show_weekends: false,
            semester_start_date: None,
            semester_total_weeks: 20,
            first_day_of_week: 1,
        },
        courses: Vec::new(),
        overrides: Vec::new(),
        updated_at: String::new(),
    }
}

/// 读取本地课表。缺失/损坏/反序列化失败 → 空课表（不报错，不删除坏文件——
/// 保留现场便于诊断，下次整体写入时自然覆盖）。
pub fn load_timetable(dir: &Path) -> Timetable {
    let raw = match fs::read_to_string(timetable_path(dir)) {
        Ok(raw) => raw,
        Err(_) => return empty_timetable(),
    };
    match serde_json::from_str(&raw) {
        Ok(tt) => tt,
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
}
