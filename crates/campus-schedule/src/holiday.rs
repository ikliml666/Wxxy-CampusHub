//! 法定节假日(timor.tech API)解析与跳过日期合并。
//!
//! 数据源 `GET https://timor.tech/api/holiday/year/<年>`(免费、无鉴权;需浏览器
//! UA,否则被 Cloudflare 拦截——UA 由 tauri 层请求时附带,本 crate 只管解析):
//! `{"code":0,"holiday":{"01-01":{"holiday":true,"name":"元旦","date":"2026-01-01",…},…}}`,
//! `holiday=true` 放假日、`false` 调休补班日(不跳过,照常上课)。网络拉取在
//! tauri 层(与校园会话无关,自建 client),本模块只做纯解析与合并(离线可测)。
//!
//! 合并语义:法定假日并入 [`CourseTableConfig::skipped_dates`](渲染「休」列、
//! ICS 剔除、今日页 skipped 均由既有消费方生效),`holiday_names` 整体替换为
//! 本次拉取结果——它是「自动假日」的唯一记录,刷新时先把旧自动假日从
//! skipped_dates 移除再并入新集,用户手动加的跳过日期不受影响。

use crate::model::NamedDate;
use chrono::NaiveDate;
use serde::Deserialize;

/// timor year API 单条假日信息(只声明用到的字段,其余宽容缺省)。
#[derive(Debug, Deserialize)]
pub struct TimorHolidayInfo {
    /// true = 放假日;false = 调休补班日(照常上课,不进跳过集)
    pub holiday: bool,
    /// 节假日名(如「国庆节」「元旦」)
    pub name: String,
    /// 完整日期 "YYYY-MM-DD"(带年,优先于顶层 key 的 "MM-DD")
    #[serde(default)]
    pub date: Option<String>,
}

/// timor year API 响应外壳。
#[derive(Debug, Deserialize)]
pub struct TimorYearResponse {
    pub holiday: std::collections::BTreeMap<String, TimorHolidayInfo>,
}

/// 解析 timor 年度假日响应,输出放假日(`holiday=true`)列表,升序去重。
/// 解析失败(非 JSON / 缺 holiday 键)返回 Err 中文消息;单条日期坏数据跳过。
pub fn parse_timor_year(json: &str) -> Result<Vec<NamedDate>, String> {
    let resp: TimorYearResponse = serde_json::from_str(json)
        .map_err(|e| format!("节假日接口返回解析失败:{e}"))?;
    let mut out: Vec<NamedDate> = Vec::new();
    for v in resp.holiday.values() {
        if !v.holiday {
            continue;
        }
        let Some(d) = v
            .date
            .as_deref()
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        else {
            continue;
        };
        out.push(NamedDate {
            date: d,
            name: v.name.clone(),
        });
    }
    out.sort_by_key(|n| n.date);
    out.dedup_by_key(|n| n.date);
    Ok(out)
}

/// 把新拉取的法定假日合并进配置:先剔除旧自动假日(`named` 记录的日期),
/// 再并入 `fresh` 的日期。返回更新后的 `(skipped_dates, holiday_names)`。
/// 两个列表升序去重;同名日期以 fresh 为准(name 更新)。
pub fn merge_holidays(
    skipped: Vec<NaiveDate>,
    named: &[NamedDate],
    fresh: Vec<NamedDate>,
) -> (Vec<NaiveDate>, Vec<NamedDate>) {
    let old_auto: std::collections::HashSet<NaiveDate> =
        named.iter().map(|n| n.date).collect();
    let mut skipped: Vec<NaiveDate> = skipped
        .into_iter()
        .filter(|d| !old_auto.contains(d))
        .collect();
    for n in &fresh {
        skipped.push(n.date);
    }
    skipped.sort();
    skipped.dedup();
    (skipped, fresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"code":0,"holiday":{
        "10-01":{"holiday":true,"name":"国庆节","date":"2026-10-01","wage":3},
        "09-20":{"holiday":false,"name":"中秋节前补班","date":"2026-09-20","wage":1},
        "09-25":{"holiday":true,"name":"中秋节","date":"2026-09-25","wage":2},
        "10-02":{"holiday":true,"name":"国庆节","date":"2026-10-02","wage":2}
    }}"#;

    #[test]
    fn parse_keeps_only_holiday_true_sorted() {
        let out = parse_timor_year(SAMPLE).expect("解析成功");
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].date.to_string(), "2026-09-25");
        assert_eq!(out[0].name, "中秋节");
        assert_eq!(out[2].date.to_string(), "2026-10-02");
        // 补班日不进集
        assert!(out.iter().all(|n| n.date.to_string() != "2026-09-20"));
    }

    #[test]
    fn parse_bad_payload_is_err() {
        assert!(parse_timor_year("<html>blocked</html>").is_err());
        assert!(parse_timor_year("{}").is_err());
    }

    #[test]
    fn merge_replaces_auto_keeps_manual() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("合法日期");
        let named = vec![NamedDate {
            date: d("2026-01-01"),
            name: "元旦".into(),
        }];
        // 手动日 3-01 + 旧自动日 1-01
        let skipped = vec![d("2026-01-01"), d("2026-03-01")];
        let fresh = vec![NamedDate {
            date: d("2026-10-01"),
            name: "国庆节".into(),
        }];
        let (skipped, named) = merge_holidays(skipped, &named, fresh);
        // 旧自动日被剔除、新自动日并入、手动日保留
        assert_eq!(
            skipped,
            vec![d("2026-03-01"), d("2026-10-01")]
        );
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].name, "国庆节");
    }

    #[test]
    fn merge_sorts_and_dedups() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("合法日期");
        let fresh = vec![
            NamedDate {
                date: d("2026-10-02"),
                name: "国庆节".into(),
            },
            NamedDate {
                date: d("2026-10-01"),
                name: "国庆节".into(),
            },
        ];
        let (skipped, _) = merge_holidays(vec![], &[], fresh);
        assert_eq!(skipped, vec![d("2026-10-01"), d("2026-10-02")]);
    }
}
