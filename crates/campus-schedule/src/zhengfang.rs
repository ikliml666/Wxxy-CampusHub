// Wxxy-CampusHub original code (no upstream counterpart):
// Zhengfang (正方) academic system course-table response parser.
// Protocol verified against https://jwgl.cwxu.edu.cn on 2026-09-17:
//   POST /jwglxt/kbcx/xskbcx_cxXsKb.html?gnmkdm=N2151   body: xnm=<学年>&xqm=<学期>

//! 正方教务课表响应解析：`kbList` → [`Course`]（周次位掩码展开、节次解析）。

use crate::model::{expand_week_mask, Course, CourseSource};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ZhengfangError {
    #[error("HTTP 状态 {0}")]
    Status(u16),
    #[error("响应缺少 kbList")]
    MissingKbList,
}

/// 正方 `kbList` 条目（只声明用到的字段，其余忽略；全部宽容缺省）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KbEntry {
    #[serde(default)]
    kcmc: String,
    #[serde(default)]
    cdmc: String,
    #[serde(default)]
    jxbmc: String,
    #[serde(default, rename = "jxb_id")]
    jxb_id: String,
    /// 星期几 "1".."7"（1=周一）
    #[serde(default)]
    xqj: String,
    /// 节次 "3-4"
    #[serde(default)]
    jcs: String,
    /// 周次十进制位掩码（bit0=第 1 周）
    #[serde(default)]
    oldzc: String,
    /// 授课教师姓名
    #[serde(default)]
    xm: String,
    #[serde(default)]
    kcxz: String,
    #[serde(default)]
    khfsmc: String,
}

/// 学期代码：第 1 学期=3，第 2 学期=12，暑期=16（正方约定，实测确认 3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Semester {
    First,
    Second,
    Summer,
}

impl Semester {
    pub fn xqm(self) -> u32 {
        match self {
            Semester::First => 3,
            Semester::Second => 12,
            Semester::Summer => 16,
        }
    }
}

/// 构造课表查询表单体：`xnm=<学年起始年>&xqm=<学期代码>`。
pub fn query_body(academic_year: u32, semester: Semester) -> String {
    format!("xnm={}&xqm={}", academic_year, semester.xqm())
}

fn parse_sections(jcs: &str) -> Option<(u8, u8)> {
    let (a, b) = jcs.trim().split_once('-')?;
    let start: u8 = a.trim().parse().ok()?;
    let end: u8 = b.trim().parse().ok()?;
    if start == 0 || end < start {
        return None;
    }
    Some((start, end))
}

fn parse_mask(s: &str) -> u64 {
    u64::from_str_radix(s.trim(), 10).unwrap_or(0)
}

/// 解析正方课表响应 JSON，输出领域课程列表（source=Import，颜色按课程名稳定散列）。
pub fn parse_kb_response(json: &str, course_table_id: &str) -> Result<Vec<Course>, ZhengfangError> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|_| ZhengfangError::MissingKbList)?;
    let entries = v
        .get("kbList")
        .and_then(|k| k.as_array())
        .ok_or(ZhengfangError::MissingKbList)?;

    let mut courses = Vec::with_capacity(entries.len());
    for (i, e) in entries.iter().enumerate() {
        let kb: KbEntry = match serde_json::from_value(e.clone()) {
            Ok(k) => k,
            Err(_) => continue,
        };
        let Some((start, end)) = parse_sections(&kb.jcs) else {
            continue;
        };
        let day: u8 = match kb.xqj.trim().parse() {
            Ok(d @ 1..=7) => d,
            _ => continue,
        };
        let weeks = expand_week_mask(parse_mask(&kb.oldzc));
        if weeks.is_empty() {
            continue;
        }
        courses.push(Course {
            id: format!("{course_table_id}-{}", kb.jxb_id),
            course_table_id: course_table_id.to_string(),
            name: kb.kcmc.clone(),
            teacher: kb.xm.clone(),
            position: kb.cdmc.clone(),
            day,
            start_section: Some(start),
            end_section: Some(end),
            is_custom_time: false,
            custom_start_time: None,
            custom_end_time: None,
            color_index: stable_color(&kb.kcmc),
            remark: Some(format!(
                "{}{}",
                if kb.kcxz.is_empty() { "" } else { &kb.kcxz },
                if kb.khfsmc.is_empty() {
                    String::new()
                } else {
                    format!("·{}", kb.khfsmc)
                }
            )),
            source: CourseSource::Import,
            weeks,
            class_id: if kb.jxb_id.is_empty() { None } else { Some(kb.jxb_id.clone()) },
            disabled: false,
        });
    }
    Ok(courses)
}

/// 课程名 → 稳定颜色索引（同一门课每次导入颜色一致）。
fn stable_color(name: &str) -> u16 {
    name.bytes().fold(0u32, |acc, b| (acc * 31 + b as u32) & 0xFFFF) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "kbList": [
        {
          "kcmc": "信息隐藏与取证技术",
          "cdmc": "C5科教中心109",
          "jxbmc": "信息隐藏与取证技术-0001",
          "jxb_id": "542DE6D999B66D47E0632150010AFBE7",
          "xqj": "1",
          "jcs": "3-4",
          "oldzc": "4095",
          "xm": "林博乐",
          "kcxz": "必修",
          "khfsmc": "考试"
        },
        {
          "kcmc": "坏数据",
          "xqj": "9",
          "jcs": "x-y",
          "oldzc": "abc"
        }
      ],
      "xsxx": { "XM": "林博乐", "BJMC": "24信安2" }
    }"#;

    #[test]
    fn parses_real_shape() {
        let out = parse_kb_response(SAMPLE, "t1").unwrap();
        assert_eq!(out.len(), 1); // 坏数据被跳过
        let c = &out[0];
        assert_eq!(c.name, "信息隐藏与取证技术");
        assert_eq!(c.position, "C5科教中心109");
        assert_eq!(c.teacher, "林博乐");
        assert_eq!(c.day, 1);
        assert_eq!(c.start_section, Some(3));
        assert_eq!(c.end_section, Some(4));
        assert_eq!(c.weeks, (1..=12).collect::<Vec<_>>());
        assert_eq!(c.class_id.as_deref(), Some("542DE6D999B66D47E0632150010AFBE7"));
        assert_eq!(c.source, CourseSource::Import);
        // 同名课程颜色稳定
        assert_eq!(c.color_index, stable_color("信息隐藏与取证技术"));
    }

    #[test]
    fn query_body_matches_protocol() {
        assert_eq!(query_body(2026, Semester::First), "xnm=2026&xqm=3");
        assert_eq!(Semester::Second.xqm(), 12);
    }
}
