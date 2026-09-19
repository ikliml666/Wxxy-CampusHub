//! 电费余额历史（**客户端自采的日快照**）存储：%APPDATA%/campushub/electricity_history.json。
//!
//! # 为什么必须自采（M4 探针实测，2026-09-19）
//!
//! 学校侧**没有**宿舍电费的每日余额时间序列：`/charge/turnover/*` 的 `balance_amount` 恒 `null`、
//! `mouthAccount` 只给本月合计（参数被忽略）、`threeExpen_account` 恒空。
//! 能拿到的只有「缴了多少」的账单流水与「某个瞬间」的余额文本 ⇒ 日余额只能由客户端自己采。
//! 应用没开的日子就是空档，**图表如实显示为空**（不插值、不补零——补零会把「没采」画成「余额 0」）。
//!
//! # 记录形状与去重键
//!
//! - **`id = roomKey@date`（确定性推导，不是随机数）**：同房间同一天恒同 id ⇒ 跨端合并只需按 id 去重，
//!   无需任何时钟同步或向量时钟。`device` 只记「哪台机器观察到的」，**不参与 id**（同一天两台机器采同一房间
//!   得到的是同一个观测点，取 `collectedAt` 较新的一条即更接近当天最终余额）。
//! - **去重键 = `roomKey` + 日期**：同日重复采集**覆盖**当天那条（新记录 `collectedAt` 必然更新，见 [`normalize`]）。
//! - `roomKey` 由片区 id + 级联路径拼出（[`room_key_of`]），与 `SavedRoom.id`（本机自增时间戳）无关——
//!   后者删掉重建就变，做不了跨端主键。
//!
//! # 多端同步（本轮只落数据结构，不做网络层）
//!
//! append 语义 + 纯函数 [`merge_history`]：将来 Android 端各自导出本文件 JSON，在校园网内由一台常开设备
//! 合并回写。合并是**可交换、幂等**的（同 id 取 `collectedAt` 新者），故重复合并、乱序合并结果一致。
//! 局限见 [`merge_history`] 的注。
//!
//! # 读取宽容语义
//!
//! 文件缺失/损坏/反序列化失败 → **空历史**，不报错、**不删除坏文件**（保留现场便于诊断，
//! 与 `infra/timetable.rs`、`commands/electricity.rs` 的 `load_rooms` 同款纪律）。
//!
//! # 敏感纪律
//!
//! 本文件只承载 `map.showData` 那句自由文本（余额句）与房间路径，**不含** token / 账号 / cookie /
//! 户号（`map.data` 的 `account` 等 PII 在 crate 层就不透出，见 `campus_synjones::charge` 头注）。

use campus_synjones::charge::RoomStep;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// 采集来源：应用启动补采（**不弹窗、失败只记日志**）。
pub const SOURCE_AUTO: &str = "auto";
/// 采集来源：用户在应用内手动点「立即采集」。
pub const SOURCE_MANUAL: &str = "manual";

/// 历史保留上限（条）。
///
/// 单房间一天一条 ⇒ 2000 条 ≈ 5.5 年单房间历史（或 20 个房间各 100 天）。
/// 超出后按时间**丢最旧**（[`normalize`]）。文件量级：每条约 300 字节 ⇒ 上限约 600KB，
/// 整体读写（无数据库）仍然廉价。
pub const MAX_HISTORY: usize = 2000;

/// 一次余额采集快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// 稳定主键 `roomKey@date`（见模块头注；跨端合并的唯一依据）。
    pub id: String,
    /// 观察到本次快照的设备名（`COMPUTERNAME`/`HOSTNAME`，取不到为 None）。**不参与 id**。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// 房间稳定键（片区 id + 路径，[`room_key_of`]）。
    pub room_key: String,
    /// 房间显示名（`SavedRoom.label`，保存时已由后端补默认名）。
    #[serde(default)]
    pub room_name: String,
    /// 片区 id（`FeeItem::id`）。
    #[serde(default)]
    pub feeitem_id: String,
    /// 片区名（列表展示不必再拉目录）。
    #[serde(default)]
    pub feeitem_name: String,
    /// 采集时刻（ISO8601 本地带偏移，如 `2026-09-19T17:20:31+08:00`）。
    pub collected_at: String,
    /// 采集日期（`YYYY-MM-DD`，本地时区）——**去重键的一半**。
    pub date: String,
    /// 余额（元）。学校的末级文本提不到数字时是 `None` ⇒ UI 显示「无数据」，
    /// **绝不臆造 0**（0 元与「没采到」在曲线图上语义完全不同）。
    pub balance: Option<f64>,
    /// 学校侧余额句原文（`map.showData` 各字段值拼接；留证据，不解析）。
    #[serde(default)]
    pub raw: String,
    /// 来源：`auto` / `manual`（见 [`SOURCE_AUTO`] / [`SOURCE_MANUAL`]）。
    pub source: String,
}

fn history_path(dir: &Path) -> PathBuf {
    dir.join("electricity_history.json")
}

/// 本条快照的主键：`roomKey@date`（确定性 ⇒ 跨端一致）。
pub fn make_id(room_key: &str, date: &str) -> String {
    format!("{}@{}", room_key.trim(), date.trim())
}

/// 房间稳定键：`<片区 id>::<level=value>><level=value>...`。
///
/// 用路径的 `level` + `value`（`value` 形如 `1&无锡学院`，自带服务端数字 id）而**不是** `code`
/// （`campus`/`building` 这类参数名属另一维度，且服务端可改）——同样的物理房间在任一端、任何时候
/// 选出来都算出同一个键。已知局限：学校把楼栋**改名**会改 `value` 的文本部分 ⇒ 键变化、历史断成两段
/// （无法从现有数据里区分「改名」与「换楼」；这是接受的风险，不做模糊匹配）。
pub fn room_key_of(feeitem_id: &str, path: &[RoomStep]) -> String {
    let steps: Vec<String> = path
        .iter()
        .map(|s| format!("{}={}", s.level, s.value.trim()))
        .collect();
    format!("{}::{}", feeitem_id.trim(), steps.join(">"))
}

/// 本机设备名（多端合并时区分「谁的观测」；取不到返回 `None`，不作为合并依据）。
///
/// 只读环境变量、不落任何硬件指纹：Windows 取 `COMPUTERNAME`，其它平台取 `HOSTNAME`。
pub fn device_name() -> Option<String> {
    for key in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(key) {
            let t = v.trim();
            if !t.is_empty() {
                return Some(t.chars().take(64).collect());
            }
        }
    }
    None
}

/// 读取本地历史。文件缺失/损坏 → 空列表（不报错、不删坏文件）；读出后一律走 [`normalize`]
/// 收敛（排序 + 同 id 去重 + 上限），故调用方拿到的永远是规范形态。
pub fn load_history(dir: &Path) -> Vec<HistoryEntry> {
    let Ok(raw) = fs::read_to_string(history_path(dir)) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<HistoryEntry>>(&raw)
        .map(normalize)
        .unwrap_or_else(|e| {
            log::warn!("electricity_history.json 损坏，回退空历史: {e}");
            Vec::new()
        })
}

/// 整体写入历史（`normalize` 后落盘，保证文件形态与内存一致）。
pub fn save_history(dir: &Path, entries: &[HistoryEntry]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let sorted = normalize(entries.to_vec());
    let json = serde_json::to_string_pretty(&sorted).map_err(|e| e.to_string())?;
    fs::write(history_path(dir), json).map_err(|e| format!("写 electricity_history.json 失败：{e}"))
}

/// 同 id 取舍键：`(collectedAt, source, 分制余额, 原文)`，**直接比较、不构造中间值**。
///
/// **全字段参与**是刻意的：合并的取舍必须只依赖记录本身、不依赖输入顺序，才能保证
/// [`merge_history`] 可交换且幂等（只比 `collectedAt` 时，两条 `collectedAt` 完全相同的记录
/// 会随「谁先被遍历到」而变，破坏这两条性质）。实践中到 `collectedAt` 这一层就已经分开了。
/// 余额 `None` 排在任何数值之前（宁取有数据的观测）。
fn cmp_keep_key(a: &HistoryEntry, b: &HistoryEntry) -> std::cmp::Ordering {
    /// 余额的分制整数（`None` → `i64::MIN`，排在所有数值之前）。
    fn fen(e: &HistoryEntry) -> i64 {
        e.balance.map(|b| (b * 100.0).round() as i64).unwrap_or(i64::MIN)
    }
    a.collected_at
        .cmp(&b.collected_at)
        .then_with(|| a.source.cmp(&b.source))
        .then_with(|| fen(a).cmp(&fen(b)))
        .then_with(|| a.raw.cmp(&b.raw))
}

/// 规范形态：**同 id（= 同房间同一天）先去重**（保留 [`cmp_keep_key`] 最大者，即「同日重复采集覆盖
/// 当天那条」）→ 再按 `(date, collectedAt, id)` **升序**排列 → 超 [`MAX_HISTORY`] 丢最旧。
///
/// ⚠️ 去重不能只做「排序后看相邻」：按时间排序时，同 id 的两条会被**别的房间的同期记录插在中间**
/// （如 `101@9-19 09:00`、`102@9-19 09:05`、`101@9-19 21:00`）⇒ 必须先按 id 分组去重。
/// 升序是刻意的：曲线图的 x 轴天然是时间正序，前端不需要再排一次。
pub fn normalize(mut entries: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    entries.sort_by(|a, b| {
        a.id.cmp(&b.id)
            .then_with(|| cmp_keep_key(a, b))
    });
    // 同 id 的条目按上序必然相邻 ⇒ 每组只留最后一条（取舍键最大者）
    let mut out: Vec<HistoryEntry> = Vec::with_capacity(entries.len());
    for e in entries {
        if out.last().is_some_and(|last| last.id == e.id) {
            out.pop();
        }
        out.push(e);
    }
    out.sort_by(|a, b| {
        (a.date.as_str(), a.collected_at.as_str(), a.id.as_str()).cmp(&(
            b.date.as_str(),
            b.collected_at.as_str(),
            b.id.as_str(),
        ))
    });
    if out.len() > MAX_HISTORY {
        let drop_n = out.len() - MAX_HISTORY;
        out.drain(0..drop_n);
    }
    out
}

/// 追加一条快照（同日同房间则覆盖），返回规范形态的新列表。
pub fn upsert(mut entries: Vec<HistoryEntry>, entry: HistoryEntry) -> Vec<HistoryEntry> {
    entries.push(entry);
    normalize(entries)
}

/// **多端合并**：按 `id` 去重（冲突取 `collectedAt` 新者），再排序 + 上限。
///
/// 语义与局限：
/// 1. **可交换、幂等**——`merge(a,b) == merge(b,a)`、`merge(x,x) == normalize(x)`，故「谁先导谁后导」
///    「重复导入同一份」都不改变结果（多端同步的硬前提）；
/// 2. **只解决「同一天同房间有两条」**，不解决「两台机器在同一天的不同时刻采到不同余额」的**取舍**问题——
///    那由 `collectedAt` 决定（取更晚的，通常更接近当天最终值），**不做平均、不做差分**；
/// 3. **不做设备级冲突检测**（没有向量时钟/版本号）：两端同时改同一天的数据、且各自 `collectedAt` 都晚于
///    对方时，合并只保留一条 ⇒ 另一条的观测被丢弃。日快照是「每天一个点」的形态，丢一条不影响趋势；
///    真需要逐次观测精度时应改成 append-only 事件流（本轮不做，见任务书 §2「只落数据结构」）。
pub fn merge_history(a: Vec<HistoryEntry>, b: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    let mut all = a;
    all.extend(b);
    normalize(all)
}

/// 该房间今天是否已采过（启动补采的判据；只比 `room_key` + `date`，不看 `source`）。
pub fn has_entry_today(entries: &[HistoryEntry], room_key: &str, today: &str) -> bool {
    entries
        .iter()
        .any(|e| e.room_key == room_key && e.date == today)
}

/// 过滤窗口的天数上限（10 年）。`days` 由前端给：不做上限时 `days = u32::MAX` 会让
/// `NaiveDate - Duration` 直接越过 chrono 的可表示范围而 **panic**（命令层不许 panic）。
pub const MAX_FILTER_DAYS: u32 = 3650;

/// 过滤 + 时间升序返回：`room_key` 为 None 表示不限房间；`days` 为 None 表示不限时间。
///
/// `days` 含今天（`days=7` ⇒ `[today-6, today]`，`0`/`1` 都按「只看今天」处理，超过
/// [`MAX_FILTER_DAYS`] 按上限处理）。日期无法解析的条目在启用 `days` 时被丢弃
/// （无法判定新旧时宁可不出现在窗口里，不猜测）。
pub fn filter(
    entries: &[HistoryEntry],
    room_key: Option<&str>,
    days: Option<u32>,
    today: NaiveDate,
) -> Vec<HistoryEntry> {
    let from = days.map(|d| today - chrono::Duration::days(d.clamp(1, MAX_FILTER_DAYS) as i64 - 1));
    entries
        .iter()
        .filter(|e| room_key.is_none_or(|k| e.room_key == k))
        .filter(|e| match from {
            None => true,
            Some(from) => NaiveDate::parse_from_str(e.date.trim(), "%Y-%m-%d")
                .map(|d| d >= from && d <= today)
                .unwrap_or(false),
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("campushub-ehist-{tag}-{n}"))
    }

    fn path_of(room: &str) -> Vec<RoomStep> {
        vec![
            RoomStep {
                level: 1,
                code: "campus".to_string(),
                value: "1&无锡学院".to_string(),
                name: "无锡学院".to_string(),
            },
            RoomStep {
                level: 2,
                code: "building".to_string(),
                value: "2309&1号楼".to_string(),
                name: "1号楼".to_string(),
            },
            RoomStep {
                level: 3,
                code: "room".to_string(),
                value: room.to_string(),
                name: room.to_string(),
            },
        ]
    }

    fn entry(room: &str, date: &str, at: &str, balance: Option<f64>) -> HistoryEntry {
        let key = room_key_of("450", &path_of(room));
        HistoryEntry {
            id: make_id(&key, date),
            device: Some("PC-A".to_string()),
            room_key: key,
            room_name: format!("1号楼 {room}"),
            feeitem_id: "450".to_string(),
            feeitem_name: "梅园1号-梅园3号".to_string(),
            collected_at: at.to_string(),
            date: date.to_string(),
            balance,
            raw: "房间当前剩余电费625.35".to_string(),
            source: SOURCE_MANUAL.to_string(),
        }
    }

    /// 落盘往返：save → load 相等；文件是 camelCase 明文，且按时间升序。
    #[test]
    fn history_roundtrip_is_camel_case_and_ascending() {
        let dir = temp_dir("rt");
        let mut later = entry("101", "2026-09-19", "2026-09-19T18:00:00+08:00", Some(12.5));
        later.source = SOURCE_AUTO.to_string();
        let entries = vec![
            later.clone(),
            entry("101", "2026-09-17", "2026-09-17T08:00:00+08:00", None),
        ];
        save_history(&dir, &entries).unwrap();

        let raw = fs::read_to_string(history_path(&dir)).unwrap();
        assert!(raw.contains("\"roomKey\""), "落盘应 camelCase：{raw}");
        assert!(raw.contains("\"balance\""), "balance 为 null 也要在场：{raw}");

        let loaded = load_history(&dir);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].date, "2026-09-17", "升序：最早的在前");
        assert_eq!(loaded[1], later);
        assert_eq!(loaded[0].balance, None, "提不到余额存 None（不是 0）");
        fs::remove_dir_all(&dir).ok();
    }

    /// 缺失 / 损坏 → 空历史且**不删坏文件**（保留现场）。
    #[test]
    fn history_missing_or_corrupt_falls_back_to_empty() {
        let dir = temp_dir("missing");
        assert!(load_history(&dir).is_empty());

        let corrupt = temp_dir("corrupt");
        fs::create_dir_all(&corrupt).unwrap();
        fs::write(history_path(&corrupt), "{ not json").unwrap();
        assert!(load_history(&corrupt).is_empty());
        assert!(history_path(&corrupt).exists(), "坏文件必须保留");
        fs::remove_dir_all(&corrupt).ok();
    }

    /// 去重（核心规则）：同房间同日期 → 覆盖（`collectedAt` 新者胜），条数不增；换日期 → 追加。
    #[test]
    fn upsert_overwrites_same_room_and_date() {
        let e1 = entry("101", "2026-09-19", "2026-09-19T08:00:00+08:00", Some(10.0));
        let mut e2 = entry("101", "2026-09-19", "2026-09-19T20:00:00+08:00", Some(3.0));
        e2.source = SOURCE_AUTO.to_string();
        let list = upsert(upsert(Vec::new(), e1), e2.clone());
        assert_eq!(list.len(), 1, "同房间同日期只保留一条");
        assert_eq!(list[0], e2, "晚采的那条胜出");

        // 乱序写入（先写晚的）同样收敛到同一条
        let e1_again = entry("101", "2026-09-19", "2026-09-19T08:00:00+08:00", Some(10.0));
        let list2 = upsert(upsert(Vec::new(), e2), e1_again);
        assert_eq!(list2.len(), 1);
        assert_eq!(list2[0].balance, Some(3.0), "旧观测不得覆盖新观测");

        let next_day = entry("101", "2026-09-20", "2026-09-20T08:00:00+08:00", Some(1.0));
        assert_eq!(upsert(list2, next_day).len(), 2, "换一天应追加");

        // 同一天不同房间 → 两条（房间号是 key 的一部分）
        let other = entry("102", "2026-09-19", "2026-09-19T20:00:00+08:00", Some(9.0));
        assert_eq!(upsert(list, other).len(), 2);
    }

    /// 保留上限：超出 [`MAX_HISTORY`] 丢最旧，长度恒等于上限（期望值同法计算，不硬编码日期）。
    #[test]
    fn normalize_caps_and_drops_oldest() {
        let base = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        let day = |i: i64| (base + chrono::Duration::days(i)).format("%Y-%m-%d").to_string();
        let total = MAX_HISTORY + 3;
        let entries: Vec<HistoryEntry> = (0..total)
            .map(|i| {
                let date = day(i as i64);
                let at = format!("{date}T08:00:00+08:00");
                entry("101", &date, &at, Some(i as f64))
            })
            .collect();
        let out = normalize(entries);
        assert_eq!(out.len(), MAX_HISTORY, "超出上限应裁剪");
        assert_eq!(out[0].date, day(3), "最旧的 3 条被丢弃（丢最旧、留最新）");
        let last_date = day(total as i64 - 1);
        assert_eq!(
            out.last().map(|e| e.date.as_str()),
            Some(last_date.as_str()),
            "最新的那条保留"
        );
        assert_eq!(out[0].balance, Some(3.0), "留下的第一条正是第 4 天的观测");
    }

    /// 跨端合并：按 id 去重取新者、顺序无关、幂等（多端同步的三条硬性质）。
    ///
    /// 用例刻意让**另一个房间**的同期记录夹在同房间的两条之间（09:00 / 09:05 / 21:00）——
    /// 这是「排序后看相邻来去重」会漏合并的形态（回归防线）。
    #[test]
    fn merge_is_deduped_order_insensitive_and_idempotent() {
        let key101 = room_key_of("450", &path_of("101"));
        let dev_a = vec![
            entry("101", "2026-09-18", "2026-09-18T09:00:00+08:00", Some(50.0)),
            entry("101", "2026-09-19", "2026-09-19T09:00:00+08:00", Some(40.0)),
        ];
        let dev_b = vec![
            entry("101", "2026-09-19", "2026-09-19T21:00:00+08:00", Some(20.0)),
            entry("102", "2026-09-19", "2026-09-19T09:05:00+08:00", Some(7.0)),
        ];
        let ab = merge_history(dev_a.clone(), dev_b.clone());
        let ba = merge_history(dev_b, dev_a);
        assert_eq!(ab, ba, "合并必须与顺序无关");
        assert_eq!(ab.len(), 3, "同房间同日期的两条必须合成一条（哪怕中间夹着别的房间）");

        let merged = ab
            .iter()
            .find(|e| e.room_key == key101 && e.date == "2026-09-19")
            .expect("合并后应有 101 在 9-19 的那条");
        assert_eq!(merged.balance, Some(20.0), "冲突取 collectedAt 新者（21:00 > 09:00）");
        assert_eq!(ab[0].date, "2026-09-18", "升序");
        assert_eq!(ab.last().map(|e| e.date.as_str()), Some("2026-09-19"));
        assert_eq!(
            ab.iter()
                .filter(|e| e.room_key == room_key_of("450", &path_of("102")))
                .count(),
            1,
            "不同房间各自保留"
        );

        assert_eq!(merge_history(ab.clone(), ab.clone()), ab, "重复合并幂等");
    }

    /// 今天是否采过：只看 room_key + date（不看来源、不看余额是否为 None）。
    #[test]
    fn has_entry_today_matches_room_and_date() {
        let key = room_key_of("450", &path_of("101"));
        let other = room_key_of("450", &path_of("102"));
        let list = vec![entry("101", "2026-09-19", "2026-09-19T08:00:00+08:00", None)];
        assert!(has_entry_today(&list, &key, "2026-09-19"));
        assert!(!has_entry_today(&list, &key, "2026-09-20"), "换一天 = 还没采");
        assert!(!has_entry_today(&list, &other, "2026-09-19"), "换房间 = 还没采");
        assert!(!has_entry_today(&[], &key, "2026-09-19"));
    }

    /// 房间键：稳定（同输入同输出）、与 `SavedRoom.id` 无关、片区 id 参与键、`code` 漂移不影响。
    #[test]
    fn room_key_is_stable_and_scoped() {
        let a = room_key_of("450", &path_of("101"));
        assert_eq!(a, room_key_of("450", &path_of("101")), "同输入必同键");
        assert!(a.starts_with("450::"), "片区 id 参与键：{a}");
        assert!(a.contains("3=101"), "末级房间号在键里：{a}");

        // 另一片区（同名楼栋房间）→ 不同键（不同缴费系统的同名房间不是同一个电表）
        assert_ne!(a, room_key_of("449", &path_of("101")));
        // `code`（参数名）漂移不改变键
        let mut renamed_code = path_of("101");
        for s in &mut renamed_code {
            s.code = "x".to_string();
        }
        assert_eq!(a, room_key_of("450", &renamed_code));
        // 楼栋改名会改变键（已知局限，如实钉住）
        let mut renamed_building = path_of("101");
        renamed_building[1].value = "2309&1号楼(新)".to_string();
        assert_ne!(a, room_key_of("450", &renamed_building));
    }

    /// `id` 确定性：跨端同房间同日 => 同 id（合并去重的唯一依据）。
    #[test]
    fn id_is_deterministic_across_devices() {
        let key = room_key_of("450", &path_of("101"));
        assert_eq!(make_id(&key, "2026-09-19"), make_id(&key, "2026-09-19"));
        assert_ne!(make_id(&key, "2026-09-19"), make_id(&key, "2026-09-20"));
        let mut a = entry("101", "2026-09-19", "2026-09-19T08:00:00+08:00", Some(1.0));
        let mut b = a.clone();
        a.device = Some("PC-A".to_string());
        b.device = Some("PHONE-B".to_string());
        a.collected_at = "2026-09-19T08:00:00+08:00".to_string();
        b.collected_at = "2026-09-19T23:00:00+08:00".to_string();
        assert_eq!(a.id, b.id, "设备不同不改变 id（同一天同房间是同一个观测点）");
    }

    /// 过滤：房间 / 天数（含今天、无数据日不补点）、日期不可解析时启用 days 则丢弃。
    #[test]
    fn filter_by_room_and_days() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 19).unwrap();
        let list = vec![
            entry("101", "2026-09-01", "2026-09-01T08:00:00+08:00", Some(1.0)),
            entry("101", "2026-09-18", "2026-09-18T08:00:00+08:00", Some(2.0)),
            entry("101", "2026-09-19", "2026-09-19T08:00:00+08:00", Some(3.0)),
            entry("102", "2026-09-19", "2026-09-19T09:00:00+08:00", Some(4.0)),
        ];
        let key101 = room_key_of("450", &path_of("101"));

        assert_eq!(filter(&list, None, None, today).len(), 4);
        assert_eq!(filter(&list, Some(&key101), None, today).len(), 3, "按房间过滤");

        let week = filter(&list, None, Some(7), today);
        assert_eq!(week.len(), 3, "近 7 天含今天（9/13 起）");
        assert!(
            week.iter().all(|e| e.date.as_str() >= "2026-09-13"),
            "窗口下界 9/13：{:?}",
            week.iter().map(|e| &e.date).collect::<Vec<_>>()
        );

        assert_eq!(filter(&list, Some(&key101), Some(1), today).len(), 1, "days=1 只看今天");
        assert_eq!(filter(&list, None, Some(0), today).len(), 2, "days=0 按 1 处理（只看今天）");

        // 日期不可解析：不限时间时保留，启用 days 时丢弃（不猜测新旧）
        let mut broken = list.clone();
        broken.push(entry("101", "not-a-date", "not-a-date", Some(0.5)));
        assert_eq!(filter(&broken, None, None, today).len(), 5);
        assert_eq!(filter(&broken, None, Some(30), today).len(), 4);

        // 极值 days 不得 panic（`NaiveDate - Duration` 会越过可表示范围）
        assert_eq!(
            filter(&list, None, Some(u32::MAX), today).len(),
            4,
            "超大 days 按上限处理，不 panic"
        );
        assert_eq!(MAX_FILTER_DAYS, 3650);
    }

    /// 设备名：取得到就是非空短串（不测试具体机器名，避免依赖环境）。
    #[test]
    fn device_name_is_optional_and_short() {
        if let Some(d) = device_name() {
            assert!(!d.trim().is_empty());
            assert!(d.chars().count() <= 64);
        }
    }
}
