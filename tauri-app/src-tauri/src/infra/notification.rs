//! 通知中心本地存储与去重纯函数（M5 批 1）：`notification_state.json` +
//! `notification_settings.json`，落 `%APPDATA%/campushub/`。
//!
//! # 读写纪律（与 `infra/electricity_history.rs` 同款）
//!
//! 文件缺失/损坏/反序列化失败 → 回退默认值（state=空、settings=默认设置），
//! **不报错、不删除坏文件**（保留现场便于诊断）；`serde(default)` 全字段兜底，
//! 旧版本文件缺字段也能读。
//!
//! # 去重 / 游标设计（一句话版）
//!
//! 每个轮询源（`info:{栏目id}` / `todo`）一个**已见 id 队列**（插入序，上限
//! [`CURSOR_CAP`] 丢最旧）+ 一条**显式基线标志**（[`NotificationState::baselined_sources`]）：
//! 本轮首页条目 id 与队列比对，**不在队列的即新通知**；源**首次拉取**（无基线标志）
//! 无论有没有条目都只建游标、**不通知**（否则装好应用第一轮会轰炸式弹出 7 栏 ×10
//! 条历史公告）。基线必须显式记录而非以「游标为空」推断——待办源实测常空页起步，
//! 若以空游标当未基线，第一条真待办会被静默吞进基线（P1 修复）。旧版本文件无标志
//! 字段：游标非空的源迁移时视作已基线（[`diff_seen`]），游标为空的源重建基线。
//! 通知进未读列表（同 id 覆盖原位、新 id 追加尾部、超 [`MAX_UNREAD`] 丢最旧），
//! 游标与通知列表互不回收——已读只清未读列表，游标保证「读过/删掉的服务端条目」
//! 不会在下一轮轮询里复活。
//!
//! # 电费低余额节流
//!
//! 提醒成功后记 `lastElecAlertAt`（epoch 秒），**24h 内不重复**（[`DAY_SECS`]
//! 边界见 [`elec_should_alert`] 单测）；余额 `None`（学校文本里提不到数字）绝不
//! 触发提醒，与电费历史「不臆造 0」同一纪律。
//!
//! # 敏感纪律
//!
//! 两份文件只含通知文本、栏目 id、间隔与余额数值，**不含** token / 账号 /
//! cookie / 户号（电费通知只用房间显示名）。

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------- 常量 ----------------

/// 通知种类：门户资讯。
pub const KIND_INFO: &str = "info";
/// 通知种类：门户待办。
pub const KIND_TODO: &str = "todo";
/// 通知种类：电费低余额。
pub const KIND_ELECTRICITY: &str = "electricity";

/// 未读通知上限（条），超出丢最旧（[`push_unread`]）。每条约 200 字节 ⇒ 文件量级 40KB。
pub const MAX_UNREAD: usize = 200;
/// 单源已见 id 队列上限（条），超出丢最旧（[`diff_seen`]）。
/// 轮询每源每轮取 10 条 ⇒ 100 条 ≈ 10 轮首页缓冲，条目「掉出首页又回来」
/// 的误报窗口足够小，同时防队列无限膨胀。
pub const CURSOR_CAP: usize = 100;
/// 一天的秒数（电费提醒节流窗口）。
pub const DAY_SECS: i64 = 24 * 3600;

/// 门户订阅栏目默认值 = 实测全量 7 栏 id（与 `campus-portal` `parse::KNOWN_COLUMNS`
/// 同一份实测数据；该常量 `pub(crate)` 不可跨 crate 引用，故在此落默认设置值，
/// 修改栏目表时两处需同步）。
pub const DEFAULT_INFO_COLUMNS: &[&str] = &[
    "9",
    "f382fddd843b4058a486a9375ecf422d",
    "a8bc1e5a9225475b9841b5a237c690df",
    "ea0a5b2158bf48b3afeb026477c626e4",
    "4f5a7ccbc5704a6690f0d3ac429c2201",
    "5d2c45d23866497cb2bfe93e9f136bb2",
    "d4901da2e5df4db9b6b551df4d5b85dd",
];

// ---------------- 数据形状 ----------------

/// 一条通知（未读列表项，camelCase 落盘 / 透出前端）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationItem {
    /// 确定性主键：`info:{栏目id}:{源条目id}` / `todo:{源条目id}` / `elec:{YYYY-MM-DD}`。
    pub id: String,
    /// 种类：[`KIND_INFO`] / [`KIND_TODO`] / [`KIND_ELECTRICITY`]。
    pub kind: String,
    /// 通知标题（公告标题 / 待办标题 / 「{房间} 余额不足」）。
    pub title: String,
    /// 副文案（栏目名·发布时间 / 申请人·申请时间 / 余额与阈值明细）。
    pub body: String,
    /// 产生时刻（ISO8601 本地带偏移，`SecondsFormat::Secs`）。
    pub created_at: String,
    /// 门户资讯的官网原文 URL（可跳转）；待办 / 电费为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// 通知状态（`notification_state.json`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationState {
    /// 未读通知列表（时间升序追加；同 id 覆盖原位，超 [`MAX_UNREAD`] 丢最旧）。
    pub unread: Vec<NotificationItem>,
    /// 各源已见 id 队列：key = `info:{栏目id}` / `todo`，value = 插入序（新的在后）。
    pub cursors: BTreeMap<String, Vec<String>>,
    /// 已建基线的源 key（显式基线标志）：源**首次拉取**无论有没有条目都建基线、
    /// 不通知；此后才正常比对报新。缺字段（旧版本文件）→ serde default 空集，
    /// 迁移语义见 [`diff_seen`]。
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub baselined_sources: BTreeSet<String>,
    /// 电费低余额上次提醒时刻（epoch 秒）。None = 从未提醒过。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_elec_alert_at: Option<i64>,
}

/// 通知设置（`notification_settings.json`）。全字段 `default`：旧文件缺字段
/// 自动补默认值（serde 结构体级 `default` + 每字段默认见 [`Default`]）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationSettings {
    /// 门户订阅栏目 id 列表（空 = 资讯通知关；默认全 7 栏）。
    pub info_columns: Vec<String>,
    /// 待办通知开关。
    pub todo_enabled: bool,
    /// 电费低余额提醒开关。
    pub electricity_enabled: bool,
    /// 电费提醒阈值（元）：余额 < 阈值触发。
    pub electricity_threshold_yuan: f64,
    /// 门户资讯轮询间隔（分钟）。
    pub info_interval_min: u32,
    /// 待办轮询间隔（分钟）。
    pub todo_interval_min: u32,
    /// 电费轮询间隔（分钟）。
    pub electricity_interval_min: u32,
    /// 静音：true = 只进通知中心，不发系统通知。
    pub mute_system_notify: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            info_columns: DEFAULT_INFO_COLUMNS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            todo_enabled: true,
            electricity_enabled: true,
            electricity_threshold_yuan: 10.0,
            info_interval_min: 10,
            todo_interval_min: 10,
            electricity_interval_min: 30,
            mute_system_notify: false,
        }
    }
}

// ---------------- 落盘（读宽容、坏文件保留） ----------------

fn state_path(dir: &Path) -> PathBuf {
    dir.join("notification_state.json")
}

fn settings_path(dir: &Path) -> PathBuf {
    dir.join("notification_settings.json")
}

/// 读通知状态。文件缺失/损坏 → 默认空状态，不报错、不删坏文件。
pub fn load_state(dir: &Path) -> NotificationState {
    let Ok(raw) = fs::read_to_string(state_path(dir)) else {
        return NotificationState::default();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        log::warn!("notification_state.json 损坏，回退空通知状态: {e}");
        NotificationState::default()
    })
}

/// 写通知状态（整体覆盖落盘）。
pub fn save_state(dir: &Path, st: &NotificationState) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let json = serde_json::to_string_pretty(st).map_err(|e| e.to_string())?;
    fs::write(state_path(dir), json).map_err(|e| format!("写 notification_state.json 失败: {e}"))
}

/// 读通知设置。文件缺失/损坏 → 默认设置，不报错、不删坏文件。
///
/// 读宽容兜底：阈值 < 0 或非有限（旧版本文件/手改；save 侧 [`validate_settings`]
/// 已挡，load 不走它）按默认 10 元处理——与轮询间隔 clamp（[`effective_interval_secs`]）
/// 同款纪律，读出的原始值不直接信任。
pub fn load_settings(dir: &Path) -> NotificationSettings {
    let fallback = || NotificationSettings::default();
    let Ok(raw) = fs::read_to_string(settings_path(dir)) else {
        return fallback();
    };
    serde_json::from_str::<NotificationSettings>(&raw)
        .map(|mut s| {
            if !(s.electricity_threshold_yuan.is_finite() && s.electricity_threshold_yuan >= 0.0) {
                s.electricity_threshold_yuan = NotificationSettings::default().electricity_threshold_yuan;
            }
            s
        })
        .unwrap_or_else(|e| {
            log::warn!("notification_settings.json 损坏，回退默认设置: {e}");
            fallback()
        })
}

/// 写通知设置（整体覆盖落盘；调用方负责先 [`validate_settings`]）。
pub fn save_settings(dir: &Path, s: &NotificationSettings) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    fs::write(settings_path(dir), json)
        .map_err(|e| format!("写 notification_settings.json 失败: {e}"))
}

// ---------------- 纯函数：游标 / 未读列表 / 节流 / 校验 ----------------

/// `diff_seen` 结果：新增条目 id（保持 fetched 顺序）与推进后的游标。
#[derive(Debug, Clone, PartialEq)]
pub struct Diff {
    /// fetched 中「不在游标里」的 id（本轮的新通知）。
    pub new_ids: Vec<String>,
    /// 推进后的完整游标（旧队列 + fetched 全部 id，超 [`CURSOR_CAP`] 丢最旧）。
    pub next: Vec<String>,
    /// true = 本轮是「首次建基线」（该源此前无基线标志）：只记游标、`new_ids`
    /// 恒空，调用方应把源 key 写进 [`NotificationState::baselined_sources`]。
    pub baselined: bool,
}

/// 已见比对（纯函数）：fetched 里不在 cursor 的 id 即新通知；游标推进为
/// 「旧队列 + fetched 全量」（**新条目与已见条目都进游标**，保证首页抖动时
/// 已见过的条目不会因「本轮又出现」被重复计数），超 [`CURSOR_CAP`] 丢最旧。
///
/// **首次建基线**：`already_baselined == false` 时无论 fetched 有无都只建游标、
/// 不通知（应用装好第一轮不把历史公告当新通知轰炸；待办源常空页起步，故基线
/// 用显式标志判定、不能用「游标为空」推断——空页起步后第一条真待办会被当
/// 基线吞掉）。旧版本文件无基线标志：调用方以「游标非空」视作已基线迁移。
pub fn diff_seen(cursor: &[String], already_baselined: bool, fetched: &[String]) -> Diff {
    let mut next = cursor.to_vec();
    let mut new_ids = Vec::new();
    let mut baselined = false;
    if !already_baselined {
        // 首次拉取：建基线（游标 = fetched 全量，本轮一律不通知）
        next = fetched.to_vec();
        baselined = true;
    } else {
        for id in fetched {
            if !next.contains(id) {
                new_ids.push(id.clone());
                next.push(id.clone());
            }
        }
    }
    if next.len() > CURSOR_CAP {
        let drop_n = next.len() - CURSOR_CAP;
        next.drain(0..drop_n);
    }
    Diff {
        new_ids,
        next,
        baselined,
    }
}

/// 未读列表追加（纯函数）：**同 id 覆盖原位**（条数不增、顺序不动），新 id
/// 追加尾部（列表保持时间升序），超 [`MAX_UNREAD`] 丢最旧（头部）。
pub fn push_unread(list: Vec<NotificationItem>, item: NotificationItem) -> Vec<NotificationItem> {
    let mut out = list;
    if let Some(slot) = out.iter_mut().find(|e| e.id == item.id) {
        *slot = item;
    } else {
        out.push(item);
    }
    if out.len() > MAX_UNREAD {
        let drop_n = out.len() - MAX_UNREAD;
        out.drain(0..drop_n);
    }
    out
}

/// 电费低余额提醒判定（纯函数）：余额提得到、低于阈值、且距上次提醒 ≥ 24h。
///
/// `balance: None`（学校文本里提不到数字）**绝不提醒**——与电费历史「不臆造 0」
/// 同一纪律；`last_alert_at: None` = 从未提醒过，立即允许。
pub fn elec_should_alert(
    balance: Option<f64>,
    threshold: f64,
    last_alert_at: Option<i64>,
    now_secs: i64,
) -> bool {
    let Some(bal) = balance else {
        return false;
    };
    if bal >= threshold {
        return false;
    }
    match last_alert_at {
        None => true,
        Some(last) => now_secs - last >= DAY_SECS,
    }
}

/// 设置校验（save 命令层用）：三个轮询间隔必须在 5..=720 分钟，阈值 ≥ 0 且有限。
/// 返回中文原因（非法即拒绝落盘）；load 不走这里（读宽容，tick 侧另行 clamp）。
pub fn validate_settings(s: &NotificationSettings) -> Result<(), String> {
    for (name, v) in [
        ("资讯轮询间隔", s.info_interval_min),
        ("待办轮询间隔", s.todo_interval_min),
        ("电费轮询间隔", s.electricity_interval_min),
    ] {
        if !(5..=720).contains(&v) {
            return Err(format!("{name}必须在 5～720 分钟之间"));
        }
    }
    if !s.electricity_threshold_yuan.is_finite() || s.electricity_threshold_yuan < 0.0 {
        return Err("电费提醒阈值不能为负数".to_string());
    }
    Ok(())
}

/// 单源有效轮询间隔（秒，纯函数）：`base 分钟 × 退避倍数`。base 统一 clamp 到
/// 5..=720（load 宽容读出的原始值不可直接信任）；**退避封顶 = max(基础间隔, 2
/// 小时)**——2h 上限只约束退避放大，绝不把用户设置的大基础间隔（如 720 分钟）
/// 压短（失败只能让轮询变稀、不能变密）。
pub fn effective_interval_secs(base_min: u32, backoff_mult: u32) -> u64 {
    let base_secs = base_min.clamp(5, 720) as u64 * 60;
    (base_secs * backoff_mult.max(1) as u64).min(base_secs.max(2 * 3600))
}

// ---------------- 通知构建（确定性 id） ----------------

/// 本机时刻的 ISO8601（带偏移、秒精度）；与电费历史 `collected_at` 同款。
pub fn now_iso() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// 当前 epoch 秒（时钟回拨等异常时返回 0，仅用于节流比较，不 panic）。
pub fn now_epoch_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// 门户资讯通知：id = `info:{栏目id}:{源条目id}`（栏目前缀防跨栏目 id 撞车）。
pub fn info_notification(
    column_id: &str,
    item_id: &str,
    title: &str,
    column_title: &str,
    publish_time: &str,
    url: &str,
) -> NotificationItem {
    NotificationItem {
        id: format!("info:{}:{}", column_id.trim(), item_id.trim()),
        kind: KIND_INFO.to_string(),
        title: title.to_string(),
        body: format!("{} · {}", column_title, publish_time),
        created_at: now_iso(),
        url: Some(url.to_string()),
    }
}

/// 门户待办通知：id = `todo:{源条目id}`。
pub fn todo_notification(item_id: &str, title: &str, applicant: &str, apply_time: &str) -> NotificationItem {
    NotificationItem {
        id: format!("todo:{}", item_id.trim()),
        kind: KIND_TODO.to_string(),
        title: title.to_string(),
        body: format!("{} · {}", applicant, apply_time),
        created_at: now_iso(),
        url: None,
    }
}

/// 电费低余额通知：id = `elec:{YYYY-MM-DD}`（一天一条，与 24h 节流双保险）。
pub fn elec_notification(room_name: &str, balance: f64, threshold: f64, today: &str) -> NotificationItem {
    NotificationItem {
        id: format!("elec:{}", today),
        kind: KIND_ELECTRICITY.to_string(),
        title: format!("{} 余额不足", room_name),
        body: format!("当前余额 {balance:.2} 元，低于提醒阈值 {threshold:.2} 元"),
        created_at: now_iso(),
        url: None,
    }
}

/// 通知种类的中文标签（系统通知的分类前缀）。
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        KIND_INFO => "校园资讯",
        KIND_TODO => "待办提醒",
        KIND_ELECTRICITY => "电费提醒",
        _ => "通知",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("campushub-notif-{tag}-{n}"))
    }

    fn item(id: &str) -> NotificationItem {
        NotificationItem {
            id: id.to_string(),
            kind: KIND_INFO.to_string(),
            title: format!("标题-{id}"),
            body: "校园要闻 · 2026-09-20 08:00:00".to_string(),
            created_at: "2026-09-20T08:00:00+08:00".to_string(),
            url: Some("https://www.cwxu.edu.cn/a.htm".to_string()),
        }
    }

    // ---------------- 设置：roundtrip / 宽容读 / serde default ----------------

    /// 设置落盘往返：save → load 相等，文件 camelCase。
    #[test]
    fn settings_roundtrip_is_camel_case() {
        let dir = temp_dir("srt");
        let mut s = NotificationSettings::default();
        s.info_columns = vec!["9".to_string()];
        s.todo_enabled = false;
        s.electricity_threshold_yuan = 3.5;
        s.electricity_interval_min = 45;
        s.mute_system_notify = true;
        save_settings(&dir, &s).unwrap();

        let raw = fs::read_to_string(settings_path(&dir)).unwrap();
        assert!(raw.contains("\"infoColumns\""), "落盘应 camelCase：{raw}");
        assert!(raw.contains("\"electricityThresholdYuan\""), "{raw}");

        assert_eq!(load_settings(&dir), s);
        fs::remove_dir_all(&dir).ok();
    }

    /// 缺失 / 损坏 → 默认设置，且**不删坏文件**（保留现场）。
    #[test]
    fn settings_missing_or_corrupt_falls_back_to_default() {
        let dir = temp_dir("smiss");
        assert_eq!(load_settings(&dir), NotificationSettings::default());

        let corrupt = temp_dir("scorrupt");
        fs::create_dir_all(&corrupt).unwrap();
        fs::write(settings_path(&corrupt), "{ not json").unwrap();
        assert_eq!(load_settings(&corrupt), NotificationSettings::default());
        assert!(settings_path(&corrupt).exists(), "坏文件必须保留");
        fs::remove_dir_all(&corrupt).ok();
    }

    /// 旧版文件缺字段 → serde default 逐字段兜底，出现的字段照读。
    #[test]
    fn settings_serde_default_fills_missing_fields() {
        let dir = temp_dir("slegacy");
        fs::create_dir_all(&dir).unwrap();
        fs::write(settings_path(&dir), r#"{"todoEnabled":false}"#).unwrap();
        let s = load_settings(&dir);
        assert!(!s.todo_enabled, "出现的字段照读");
        assert_eq!(
            s.info_columns,
            NotificationSettings::default().info_columns,
            "缺失字段回落默认（全 7 栏）"
        );
        assert_eq!(s.info_interval_min, 10);
        assert_eq!(s.electricity_threshold_yuan, 10.0);
        assert!(!s.mute_system_notify);
        fs::remove_dir_all(&dir).ok();
    }

    /// 阈值读宽容 clamp：负数 / NaN → 默认 10 元（save 侧 validate 挡不住旧文件，
    /// load 侧兜底，与间隔 clamp 同款）；0 元与正数照读。
    #[test]
    fn load_settings_clamps_bad_threshold_to_default() {
        let mk = |tag: &str, raw: &str| {
            let dir = temp_dir(tag);
            fs::create_dir_all(&dir).unwrap();
            fs::write(settings_path(&dir), raw).unwrap();
            let s = load_settings(&dir);
            fs::remove_dir_all(&dir).ok();
            s
        };
        assert_eq!(
            mk("sneg", r#"{"electricityThresholdYuan":-0.1}"#).electricity_threshold_yuan,
            10.0,
            "负阈值按默认处理"
        );
        assert_eq!(
            mk("snan", r#"{"electricityThresholdYuan":"NaN"}"#).electricity_threshold_yuan,
            10.0,
            "NaN 视为坏值按默认处理（serde 对 f64 的 NaN 走字符串，读失败整体回落同效）"
        );
        assert_eq!(
            mk("szero", r#"{"electricityThresholdYuan":0}"#).electricity_threshold_yuan,
            0.0,
            "0 元合法照读（等效关闭）"
        );
        assert_eq!(
            mk("spos", r#"{"electricityThresholdYuan":3.5}"#).electricity_threshold_yuan,
            3.5,
            "正数照读"
        );
    }

    /// 默认设置：全 7 栏、门户/待办 10 分钟、电费 30 分钟、非静音。
    #[test]
    fn settings_default_matches_contract() {
        let s = NotificationSettings::default();
        assert_eq!(s.info_columns.len(), 7);
        assert_eq!(s.info_interval_min, 10);
        assert_eq!(s.todo_interval_min, 10);
        assert_eq!(s.electricity_interval_min, 30);
        assert!(s.todo_enabled && s.electricity_enabled);
        assert!(!s.mute_system_notify);
    }

    // ---------------- 状态：roundtrip / 宽容读 ----------------

    /// 状态落盘往返：未读列表 / 游标 / 基线标志 / 节流时刻都要原样回来。
    #[test]
    fn state_roundtrip_keeps_all_fields() {
        let dir = temp_dir("strt");
        let mut st = NotificationState::default();
        st.unread = vec![item("info:9:101"), item("elec:2026-09-20")];
        st.unread[1].kind = KIND_ELECTRICITY.to_string();
        st.unread[1].url = None;
        st.cursors.insert("info:9".to_string(), vec!["101".into(), "102".into()]);
        st.cursors.insert("todo".to_string(), vec!["t1".into()]);
        st.baselined_sources.insert("info:9".to_string());
        st.baselined_sources.insert("todo".to_string());
        st.last_elec_alert_at = Some(1_758_000_000);
        save_state(&dir, &st).unwrap();

        let raw = fs::read_to_string(state_path(&dir)).unwrap();
        assert!(raw.contains("\"lastElecAlertAt\""), "{raw}");
        assert!(raw.contains("\"baselinedSources\""), "{raw}");
        assert_eq!(load_state(&dir), st);

        // 旧版本文件无 baselinedSources 字段 → serde default 空集，游标照读
        let legacy = temp_dir("stlegacy");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            state_path(&legacy),
            r#"{"cursors":{"todo":["t1"]},"unread":[]}"#,
        )
        .unwrap();
        let old = load_state(&legacy);
        assert!(old.baselined_sources.is_empty(), "旧文件无标志 → 空集（迁移语义见 diff_seen）");
        assert_eq!(old.cursors.get("todo").map(|v| v.as_slice()), Some(&["t1".to_string()][..]));
        fs::remove_dir_all(&dir).ok();
        fs::remove_dir_all(&legacy).ok();
    }

    /// 缺失 / 损坏 → 空状态且坏文件保留（与 electricity_history 同纪律）。
    #[test]
    fn state_missing_or_corrupt_falls_back_to_empty() {
        let dir = temp_dir("stmiss");
        assert_eq!(load_state(&dir), NotificationState::default());

        let corrupt = temp_dir("stcorrupt");
        fs::create_dir_all(&corrupt).unwrap();
        fs::write(state_path(&corrupt), "not-json{").unwrap();
        assert_eq!(load_state(&corrupt), NotificationState::default());
        assert!(state_path(&corrupt).exists(), "坏文件必须保留");
        fs::remove_dir_all(&corrupt).ok();
    }

    // ---------------- 游标 / 去重（核心规则） ----------------

    /// diff：新 id 追进通知与游标、旧 id 不再报、游标含 fetched 全量（防首页抖动重复计数）。
    #[test]
    fn diff_seen_reports_only_new_and_advances_cursor() {
        let first = diff_seen(&[], false, &["a".into(), "b".into()]);
        assert!(first.baselined, "首次拉取建基线");
        assert!(first.new_ids.is_empty(), "基线轮不产生通知");
        assert_eq!(first.next, vec!["a", "b"]);

        let second = diff_seen(&first.next, true, &["b".into(), "c".into(), "d".into()]);
        assert!(!second.baselined);
        assert_eq!(second.new_ids, vec!["c", "d"], "只有新条目产生通知");
        assert_eq!(second.next, vec!["a", "b", "c", "d"], "已见条目留在游标里");

        // 首页抖动：c 掉出又回来 → 不算新（游标保留全量已见）
        let third = diff_seen(&second.next, true, &["a".into(), "b".into(), "c".into()]);
        assert!(third.new_ids.is_empty());
    }

    /// 基线判定是显式标志而非「游标为空」：首次拉取无论 fetched 有无都建基线
    /// （待办源常空页起步——若以空游标当未基线，第一条真待办会被静默吞掉，P1）。
    #[test]
    fn diff_seen_baselines_first_pull_even_if_fetch_empty() {
        let d = diff_seen(&[], false, &[]);
        assert!(d.baselined, "空页首轮也建基线（显式标志，不靠游标推断）");
        assert!(d.next.is_empty());
        assert!(d.new_ids.is_empty());
        // 基线后（already_baselined = true）首条真条目正常报新
        let d2 = diff_seen(&d.next, true, &["x".into()]);
        assert!(!d2.baselined);
        assert_eq!(d2.new_ids, vec!["x"], "基线后的首条新条目必须通知");
    }

    /// 游标上限：超 [`CURSOR_CAP`] 丢最旧，长度封顶。
    #[test]
    fn diff_seen_caps_cursor() {
        let old: Vec<String> = (0..CURSOR_CAP).map(|i| format!("o{i}")).collect();
        let mut fetched: Vec<String> = (0..CURSOR_CAP).map(|i| format!("n{i}")).collect();
        // fetched 里掺 5 个旧 id（应算已见，不重复上报）
        for i in 0..5 {
            fetched.push(format!("o{i}"));
        }
        let d = diff_seen(&old, true, &fetched);
        assert_eq!(d.next.len(), CURSOR_CAP, "游标封顶");
        assert_eq!(d.new_ids.len(), CURSOR_CAP, "新 id 全部上报（n0..n99；掺入的旧 id 不算新）");
        // 旧 100 条 + 新 100 条 = 200 → 丢最旧的 100 条（o0..o99 全被挤出）
        assert_eq!(d.next[0], "n0", "最旧的整段被挤出（丢最旧）");
        assert_eq!(d.next.last().unwrap(), "n99");
        assert!(!d.next.iter().any(|id| id.starts_with('o')), "旧 id 已全部出窗");
    }

    // ---------------- 未读列表 ----------------

    /// 未读列表：新 id 追加尾部、同 id 覆盖原位（条数不增、位置不动）、超上限丢最旧。
    #[test]
    fn push_unread_appends_overwrites_and_caps() {
        let list = vec![item("a"), item("b")];
        // 新 id 追加尾部
        let l2 = push_unread(list.clone(), item("c"));
        assert_eq!(l2.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c"]);

        // 同 id 覆盖原位：条数不增、位置不动、内容更新
        let mut updated = item("b");
        updated.title = "覆盖后的标题".to_string();
        let l3 = push_unread(l2, updated);
        assert_eq!(l3.len(), 3);
        assert_eq!(l3[1].title, "覆盖后的标题");
        assert_eq!(l3[2].id, "c", "后加入的 c 不受影响");

        // 超 MAX_UNREAD 丢最旧（头部）：a/b/c 共 3 条最旧被挤出
        let mut many = l3;
        for i in 0..MAX_UNREAD {
            many = push_unread(many, item(&format!("x{i}")));
        }
        assert_eq!(many.len(), MAX_UNREAD, "封顶");
        assert_eq!(many[0].id, "x0", "a/b/c 已被挤出，x0 成为最旧");
        assert_eq!(many.last().unwrap().id, format!("x{}", MAX_UNREAD - 1));
    }

    // ---------------- 电费节流（24h 边界） ----------------

    /// 节流边界：23:59:59 拒、24h 整过；None 余额与未低于阈值都不提醒；首提醒放行。
    #[test]
    fn elec_alert_throttle_boundary() {
        let now = 1_758_000_000_i64;
        // 首次（从未提醒）：低于阈值 → 提醒
        assert!(elec_should_alert(Some(3.0), 10.0, None, now));
        // 余额提不到 → 绝不提醒（不臆造 0）
        assert!(!elec_should_alert(None, 10.0, None, now));
        // 未低于阈值 → 不提醒
        assert!(!elec_should_alert(Some(10.0), 10.0, None, now));
        assert!(!elec_should_alert(Some(11.0), 10.0, None, now));

        let last = now - DAY_SECS;
        // 24h 整：允许（>= 边界取「提醒」侧——次日同刻即可再提醒）
        assert!(elec_should_alert(Some(3.0), 10.0, Some(last), now), "恰好 24h 应放行");
        // 23:59:59：拒绝
        assert!(!elec_should_alert(Some(3.0), 10.0, Some(last + 1), now));
    }

    // ---------------- 设置校验 ----------------

    /// 校验：间隔 5..=720、阈值 ≥ 0 且有限；非法给中文原因。
    #[test]
    fn validate_settings_rejects_out_of_range_with_chinese_reason() {
        let mut s = NotificationSettings::default();
        assert!(validate_settings(&s).is_ok(), "默认设置合法");

        s.info_interval_min = 4;
        let e = validate_settings(&s).unwrap_err();
        assert!(e.contains("资讯轮询间隔"), "实际 {e}");
        s.info_interval_min = 721;
        assert!(validate_settings(&s).is_err());
        s.info_interval_min = 720;
        assert!(validate_settings(&s).is_ok(), "720 恰好合法");
        s.info_interval_min = 5;
        assert!(validate_settings(&s).is_ok(), "5 恰好合法");

        s.todo_interval_min = 0;
        assert!(validate_settings(&s).is_err());
        s.todo_interval_min = 10;

        s.electricity_threshold_yuan = -0.1;
        let e = validate_settings(&s).unwrap_err();
        assert!(e.contains("阈值"), "实际 {e}");
        s.electricity_threshold_yuan = 0.0;
        assert!(validate_settings(&s).is_ok(), "0 元阈值合法（余额 < 0 不可能，等效关闭）");
        s.electricity_threshold_yuan = f64::NAN;
        assert!(validate_settings(&s).is_err(), "NaN 必须拒绝");
    }

    // ---------------- 有效间隔（退避封顶） ----------------

    /// 有效间隔：base clamp 到 5..=720、退避封顶 max(基础间隔, 2h)（只放大不压短）。
    #[test]
    fn effective_interval_clamps_base_and_caps_backoff() {
        assert_eq!(effective_interval_secs(10, 1), 600, "正常态 = 10 分钟");
        assert_eq!(effective_interval_secs(3, 1), 300, "base 过小 clamp 到 5 分钟");
        assert_eq!(
            effective_interval_secs(1000, 1),
            720 * 60,
            "base 过大 clamp 到 720 分钟且不被 2h 压短"
        );
        assert_eq!(effective_interval_secs(60, 100), 2 * 3600, "退避封顶 2 小时");
        assert_eq!(
            effective_interval_secs(720, 9),
            720 * 60,
            "基础间隔已是 12h：退避不超基础间隔（只稀不密）"
        );
        assert_eq!(effective_interval_secs(10, 0), 600, "倍数 0 视为 1（防呆）");
    }

    // ---------------- 通知构建（确定性 id） ----------------

    /// id 规则：`info:{栏目id}:{条目id}`、`todo:{条目id}`、`elec:{日期}`，同输入同 id。
    #[test]
    fn notification_ids_are_deterministic() {
        let a = info_notification("9", "101", "关于调整的通知", "通知公告", "2026-09-20 08:00:00", "https://x");
        assert_eq!(a.id, "info:9:101");
        assert_eq!(a.kind, KIND_INFO);
        assert_eq!(a.url.as_deref(), Some("https://x"));
        assert_eq!(info_notification("9", "101", "t", "c", "d", "u").id, a.id, "确定性");

        let t = todo_notification("t1", "请假审批", "张三", "2026-09-20 09:00:00");
        assert_eq!(t.id, "todo:t1");
        assert_eq!(t.kind, KIND_TODO);
        assert_eq!(t.url, None);

        let e = elec_notification("1号楼 101", 3.2, 10.0, "2026-09-20");
        assert_eq!(e.id, "elec:2026-09-20");
        assert_eq!(e.kind, KIND_ELECTRICITY);
        assert!(e.title.contains("101"));
        assert!(e.body.contains("3.20"), "余额两位小数：{}", e.body);
        assert!(e.body.contains("10.00"), "阈值进文案：{}", e.body);

        // 不同栏目同条目 id → 通知 id 不撞车
        assert_ne!(info_notification("9", "1", "t", "c", "d", "u").id, info_notification("f382fddd843b4058a486a9375ecf422d", "1", "t", "c", "d", "u").id);
    }
}
