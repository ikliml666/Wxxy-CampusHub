//! 一卡通账单统计（计划 §1.3 实测修订版）：收支汇总 / 日期序列 / 分类聚合 / 分类字典。
//!
//! # 端点与实测事实（2026-09-19 live，探针 `tests/ecard_features_probe_live.rs` B 组）
//!
//! | 用途 | 方法 | 路径 | 参数 |
//! |---|---|---|---|
//! | 收支汇总 | GET | [`EP_COUNT`] | `timeFrom` / `timeTo`（**实测生效**：2026 全年 241540 分 ≠ 无参 533160 分） |
//! | 日期序列 | GET | [`EP_SERIES`] | `dateStr` + `dateType`(month/year) + `statisticsDateStr`(day/month) + `type`(1 收入/2 支出) |
//! | 分类聚合 | GET | [`EP_ASSORT`] | `type` + `timeFrom` + `timeTo` |
//! | 分类字典 | GET | [`EP_TURNOVER_TYPES`] | 无（id 语义：1 消费 / 2 充值 / 3 退款 / 4 扫码付 / 5 补贴） |
//!
//! 旧 wiki「sum/user 参数被忽略、恒空」的结论**已推翻**（计划 §1.3）：实测 `data` 是
//! `{"2026-09-01": 0, "2026-09-03": 1800.0, …}` 的日期→金额 map，月视图键 `YYYY-MM-DD`、
//! 年视图键 `YYYY-MM`。
//!
//! # 单位
//!
//! 全部端点在 `/berserker-search/` 前缀下 ⇒ [`Envelope::Search`]；金额单位一律**分**
//! （实测值形如 `241540.0`），对外换算成**元**（复用 [`crate::ecard`] 的整数分口径）。
//!
//! # 序列口径
//!
//! [`parse_series`] 把 map 转成**按 key 升序**的数组，**零值保留**——服务端对无消费的日期
//! 也回 `"2026-09-20": 0`（实测整月 30 键齐），折线需要连续 x 轴，绝不能过滤。

use crate::client::{Envelope, SynjonesClient};
use crate::ecard::{int_of, yuan};
use crate::CampusSynjonesError;
use serde::Serialize;
use serde_json::Value;

/// 收支汇总（`timeFrom`/`timeTo` 过滤，实测生效）。
pub const EP_COUNT: &str = "/berserker-search/statistics/turnover/count";
/// 日期序列（月/年两种粒度，见模块头注）。
pub const EP_SERIES: &str = "/berserker-search/statistics/turnover/sum/user";
/// 分类聚合（饼图）。
pub const EP_ASSORT: &str = "/berserker-search/statistics/turnover";
/// 分类字典（turnoverType）。
pub const EP_TURNOVER_TYPES: &str = "/berserker-search/search/turnoverType";

/// 收支汇总（单位元）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSummary {
    /// 支出合计（元，`data.expenses` 分→元）。
    pub expenses_yuan: f64,
    /// 收入合计（元，`data.income` 分→元）。
    pub income_yuan: f64,
}

/// 序列上的一点（单位元；`label` 为服务端日期键原文）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsPoint {
    /// 月视图 `YYYY-MM-DD` / 年视图 `YYYY-MM`。
    pub label: String,
    /// 该日/该月金额（元，分→元；**零值保留**）。
    pub amount_yuan: f64,
}

/// 一条分类聚合（饼图扇区，单位元）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsAssortItem {
    /// 分类 id（`typeId`，实测字符串 `"1"`）。
    pub type_id: String,
    /// 分类名（`turnoverType`，如 `"消费"`）。
    pub turnover_type: String,
    /// 英文标识（`nameEn`，实测可能为 null → 空串）。
    pub name_en: String,
    /// 该分类金额（元，`amount` 分→元）。
    pub amount_yuan: f64,
}

/// 分类字典项（`turnoverType`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnoverType {
    pub id: i64,
    /// 分类名（服务端 `name`；是「消费/充值」这类枚举标签，非人名，可透出）。
    pub name: String,
    pub name_en: String,
    pub icon: String,
    pub show_order: u32,
}

// ---------------- 纯解析（单测覆盖，无网络） ----------------

/// 解析收支汇总（`data.expenses` / `data.income`，分→元）。
pub fn parse_summary(v: &Value) -> StatsSummary {
    let data = &v["data"];
    StatsSummary {
        expenses_yuan: yuan(int_of(data.get("expenses")).unwrap_or(0)),
        income_yuan: yuan(int_of(data.get("income")).unwrap_or(0)),
    }
}

/// 解析日期序列：`data` 是 `{日期: 金额分}` 的 map → 按 key 升序数组，**零值保留**。
pub fn parse_series(v: &Value) -> Vec<StatsPoint> {
    let Some(map) = v["data"].as_object() else {
        return Vec::new();
    };
    let mut points: Vec<StatsPoint> = map
        .iter()
        .map(|(k, val)| StatsPoint {
            label: k.clone(),
            amount_yuan: yuan(int_of(Some(val)).unwrap_or(0)),
        })
        .collect();
    points.sort_by(|a, b| a.label.cmp(&b.label));
    points
}

/// 解析分类聚合（`data[]`）。
pub fn parse_assort(v: &Value) -> Vec<StatsAssortItem> {
    v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|it| StatsAssortItem {
                    type_id: crate::ecard::text_of(it.get("typeId")),
                    turnover_type: crate::ecard::text_of(it.get("turnoverType")),
                    name_en: crate::ecard::text_of(it.get("nameEn")),
                    amount_yuan: yuan(int_of(it.get("amount")).unwrap_or(0)),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 解析分类字典（`data[]`）。
pub fn parse_turnover_types(v: &Value) -> Vec<TurnoverType> {
    v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|it| TurnoverType {
                    id: int_of(it.get("id")).unwrap_or(0),
                    name: crate::ecard::text_of(it.get("name")),
                    name_en: crate::ecard::text_of(it.get("nameEn")),
                    icon: crate::ecard::text_of(it.get("icon")),
                    show_order: int_of(it.get("showorder")).unwrap_or(0).max(0) as u32,
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------- 取数 ----------------

/// 收支汇总。`time_from` / `time_to` 原样透传（实测空参 = 全量合计，语义由调用方定）。
pub async fn fetch_stats_summary(
    client: &SynjonesClient,
    time_from: &str,
    time_to: &str,
) -> Result<StatsSummary, CampusSynjonesError> {
    let v = client
        .get(
            EP_COUNT,
            &[("timeFrom", time_from.trim()), ("timeTo", time_to.trim())],
            Envelope::Search,
        )
        .await?;
    Ok(parse_summary(&v))
}

/// 日期序列（月视图：`date_str="2026-09"`, `date_type="month"`, `statistics_date_str="day"`；
/// 年视图：`date_str="2026"`, `date_type="year"`, `statistics_date_str="month"`）。
/// `ty`：`"1"` 收入 / `"2"` 支出（实测口径）。
pub async fn fetch_stats_series(
    client: &SynjonesClient,
    date_str: &str,
    date_type: &str,
    statistics_date_str: &str,
    ty: &str,
) -> Result<Vec<StatsPoint>, CampusSynjonesError> {
    let v = client
        .get(
            EP_SERIES,
            &[
                ("dateStr", date_str.trim()),
                ("dateType", date_type.trim()),
                ("statisticsDateStr", statistics_date_str.trim()),
                ("type", ty.trim()),
            ],
            Envelope::Search,
        )
        .await?;
    Ok(parse_series(&v))
}

/// 分类聚合（`ty`：`"1"` 收入 / `"2"` 支出）。
pub async fn fetch_stats_assort(
    client: &SynjonesClient,
    ty: &str,
    time_from: &str,
    time_to: &str,
) -> Result<Vec<StatsAssortItem>, CampusSynjonesError> {
    let v = client
        .get(
            EP_ASSORT,
            &[
                ("type", ty.trim()),
                ("timeFrom", time_from.trim()),
                ("timeTo", time_to.trim()),
            ],
            Envelope::Search,
        )
        .await?;
    Ok(parse_assort(&v))
}

/// 分类字典（无参）。
pub async fn fetch_turnover_types(
    client: &SynjonesClient,
) -> Result<Vec<TurnoverType>, CampusSynjonesError> {
    let v = client.get(EP_TURNOVER_TYPES, &[], Envelope::Search).await?;
    Ok(parse_turnover_types(&v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// **实测样本**（2026-09-19 live）：无参 533160/498600，2026 全年 241540/251100（分）。
    #[test]
    fn summary_reads_live_counts() {
        let no_params = parse_summary(&json!({
            "code": 200,
            "data": {"expenses": 533160.0, "income": 498600.0},
            "msg": "操作成功"
        }));
        assert_eq!(no_params.expenses_yuan, 5331.60, "533160 分 → 5331.60 元");
        assert_eq!(no_params.income_yuan, 4986.0);

        let y2026 = parse_summary(&json!({"data": {"expenses": 241540, "income": 251100}}));
        assert_eq!(y2026.expenses_yuan, 2415.40, "整数形态也吃得下");
        assert_eq!(y2026.income_yuan, 2511.0);

        // 缺字段 → 0，不 panic
        let empty = parse_summary(&json!({"data": {}}));
        assert_eq!(empty.expenses_yuan, 0.0);
        assert_eq!(empty.income_yuan, 0.0);
    }

    /// **实测样本**（月视图 day 粒度 + 年视图 month 粒度）：map → **按 key 升序**数组，零值保留。
    #[test]
    fn series_sorts_keys_and_keeps_zeros() {
        // 月视图（实测 30 键，此处节选含零值与乱序）
        let month = parse_series(&json!({"data": {
            "2026-09-03": 1800.0, "2026-09-01": 0, "2026-09-20": 0, "2026-09-13": 95
        }}));
        let labels: Vec<&str> = month.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, vec!["2026-09-01", "2026-09-03", "2026-09-13", "2026-09-20"], "升序");
        assert_eq!(month[0].amount_yuan, 0.0, "零值保留（折线要连续）");
        assert_eq!(month[1].amount_yuan, 18.0, "1800 分 → 18 元");
        assert_eq!(month[2].amount_yuan, 0.95);

        // 年视图（键为 YYYY-MM）
        let year = parse_series(&json!({"data": {
            "2026-09": 19555.0, "2026-01": 1040.0, "2026-08": 0, "2026-12": 0
        }}));
        let labels: Vec<&str> = year.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, vec!["2026-01", "2026-08", "2026-09", "2026-12"]);
        assert_eq!(year[0].amount_yuan, 10.40);
        assert_eq!(year[1].amount_yuan, 0.0);
    }

    /// 序列容错：`data` 缺失/非对象 → 空数组；垃圾金额 → 0。
    #[test]
    fn series_tolerates_missing_data() {
        assert!(parse_series(&json!({"code": 200})).is_empty());
        assert!(parse_series(&json!({"data": []})).is_empty());
        let odd = parse_series(&json!({"data": {"2026-09-01": "abc"}}));
        assert_eq!(odd.len(), 1);
        assert_eq!(odd[0].amount_yuan, 0.0);
    }

    /// **实测样本**（B7 分类聚合）：单分类 241540 分，`nameEn` 为 null，`typeId` 是字符串。
    #[test]
    fn assort_reads_live_sample() {
        let v = json!({"code": 200, "data": [
            {"amount": 241540.0, "icon": "consume", "nameEn": null, "turnoverType": "消费", "typeId": "1"}
        ]});
        let items = parse_assort(&v);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].type_id, "1");
        assert_eq!(items[0].turnover_type, "消费");
        assert_eq!(items[0].name_en, "", "null → 空串");
        assert_eq!(items[0].amount_yuan, 2415.40);
        assert!(parse_assort(&json!({"code": 200})).is_empty());
    }

    /// **实测样本**（B8 分类字典）：5 项，id 数字、showorder 数字、nameEn 常在。
    #[test]
    fn turnover_types_read_live_sample() {
        let v = json!({"code": 200, "data": [
            {"createTime": "2024-11-19T12:22:33.000+0000", "flag": "1", "icon": "consume",
             "id": 1, "name": "消费", "nameEn": "consume", "showorder": 1, "status": 1},
            {"icon": "subsidy", "id": 5, "name": "补贴", "nameEn": "subsidy", "showorder": 5, "status": 1}
        ]});
        let types = parse_turnover_types(&v);
        assert_eq!(types.len(), 2);
        assert_eq!(types[0].id, 1);
        assert_eq!(types[0].name, "消费");
        assert_eq!(types[0].name_en, "consume");
        assert_eq!(types[0].icon, "consume");
        assert_eq!(types[0].show_order, 1);
        assert_eq!(types[1].id, 5);
        // 无关字段（createTime/flag/status）不透出
        let text = serde_json::to_string(&types).unwrap();
        assert!(!text.contains("createTime"));
        assert!(parse_turnover_types(&json!({"code": 200})).is_empty());
    }

    /// 命令面 IPC 契约：全 camelCase（前端 `types.ts` 按这些键名取值）。
    #[test]
    fn dtos_are_camel_case() {
        let s = serde_json::to_value(StatsSummary { expenses_yuan: 1.0, income_yuan: 2.0 }).unwrap();
        assert!(s.get("expensesYuan").is_some(), "实际 {s}");
        assert!(s.get("incomeYuan").is_some());

        let p = serde_json::to_value(StatsPoint { label: "2026-09".into(), amount_yuan: 1.0 }).unwrap();
        assert!(p.get("amountYuan").is_some(), "实际 {p}");
        assert!(p.get("label").is_some());

        let a = serde_json::to_value(StatsAssortItem {
            type_id: "1".into(),
            turnover_type: "消费".into(),
            name_en: "consume".into(),
            amount_yuan: 1.0,
        })
        .unwrap();
        for k in ["typeId", "turnoverType", "nameEn", "amountYuan"] {
            assert!(a.get(k).is_some(), "缺 {k}：{a}");
        }

        let t = serde_json::to_value(TurnoverType {
            id: 1,
            name: "消费".into(),
            name_en: "consume".into(),
            icon: "consume".into(),
            show_order: 1,
        })
        .unwrap();
        assert!(t.get("showOrder").is_some(), "实际 {t}");
    }
}
