//! 调课通知 L1/L2 解析内核（M2.5 冻结契约 §2.5，纯函数、无 Tauri / 无网络依赖）。
//!
//! - **L1 规则层提取**：课程名（对本地课程名做精确/包含匹配）、周次
//!   （`第3周` / `3-4周` / `本周`）、星期（`周一`…`周日` / `星期日` / `星期天`）、
//!   节次（`3-4节` / `第3节`）、教室（`D4-207` 形态 / `教室：` 后）、类型关键词
//!   （调课/停课/补课 → [`OverrideKind`]）。全部 std 字符串处理，不引入 regex。
//! - **L2 置信**：课程名精确匹配唯一一门**且**周次、星期、节次齐全 →
//!   [`NoticeConfidence::High`]（自动应用）；缺任一要素、课程名匹配到多门或
//!   匹配不到 → [`NoticeConfidence::Low`]（进「待确认」，`reason` 写清中文降级原因）。
//! - 输出单候选 [`NoticeCandidate`]：一次粘贴对应一条通知（教务通知拆条由用户
//!   分次粘贴，见 decisions 文档）。`noticeId` 由正文哈希生成（非密码学，仅作
//!   去重与撤销键；M5 接公告流时改用公告 id）。

use crate::model::{Course, OverrideKind};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// 周次/节次解析上限（防「2026-09」这类长数字误匹配；学期 20 周、节次 ≤30 已覆盖）。
const MAX_WEEK: u32 = 52;
const MAX_SECTION: u32 = 30;

/// 解析置信度（契约 §2.3 序列化为 `"high"|"low"`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeConfidence {
    /// 自动应用（autoApplied=true）
    High,
    /// 进「待确认」列表，用户一键采纳
    Low,
}

/// 调课通知候选（契约 §2.3 冻结字段，camelCase；`courseId?` 等 Option 字段
/// 为 None 时省略）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticeCandidate {
    pub notice_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub course_id: Option<String>,
    /// 提取/命中的课程名（匹配不到时为书名号提取文本，可能为空串）
    pub course_name: String,
    pub change_type: OverrideKind,
    pub weeks: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_day: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_start_section: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_end_section: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_position: Option<String>,
    pub confidence: NoticeConfidence,
    /// 降级原因（High 时为空串），多个原因以「；」连接
    pub reason: String,
    /// 原文摘录（命中课程所在行，超长截断）
    pub excerpt: String,
}

/// 通知 ID：`manual:<16 位十六进制>`。**非密码学哈希，仅作去重与撤销键**；
/// M5 接公告流时改用公告 id。
pub fn notice_id_for(text: &str) -> String {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    format!("manual:{:016x}", h.finish())
}

// ---------------- L1：类型关键词 ----------------

/// 类型关键词 → OverrideKind。优先级：停课 > 补课 > 调课；无命中按调课处理
/// （返回 explicit=false 供 reason 透明化，但不影响置信——契约 High 条件不含类型）。
fn detect_kind(text: &str) -> (OverrideKind, bool) {
    if text.contains("停课") || text.contains("暂停一次") || text.contains("不上课") {
        return (OverrideKind::Cancelled, true);
    }
    if text.contains("补课") || text.contains("加课") || text.contains("增加一次") {
        return (OverrideKind::Extra, true);
    }
    const RESCHEDULE: [&str; 10] = [
        "调课", "调整", "改到", "改至", "换到", "换至", "移到", "移至", "更换", "变更为",
    ];
    for k in RESCHEDULE {
        if text.contains(k) {
            return (OverrideKind::Rescheduled, true);
        }
    }
    (OverrideKind::Rescheduled, false)
}

// ---------------- L1：数字锚定扫描 helper ----------------

/// 在字节位置 `end`（如「周」/「节」的起始字节）之前收集连续 ASCII 数字，
/// 返回 `(数值, 数字段起始字节)`。仅按 ASCII 字节操作（UTF-8 多字节字符的
/// 字节均 ≥0x80，不会与 ASCII 混淆），切片范围全为 ASCII 数字，安全。
fn digits_before(bytes: &[u8], end: usize) -> Option<(u32, usize)> {
    let mut s = end;
    while s > 0 && bytes[s - 1].is_ascii_digit() {
        s -= 1;
    }
    if s == end {
        return None;
    }
    let text = std::str::from_utf8(&bytes[s..end]).ok()?;
    let v: u32 = text.parse().ok()?;
    Some((v, s))
}

/// 周次提取：区间 `3-4周` / `第3-4周` 优先（避免把「4」误提为单周），
/// 其次单周 `第3周` / `3周`，最后「本周」（需 `current_week`，None = 无法确定）。
pub fn parse_weeks(text: &str, current_week: Option<u32>) -> Option<Vec<u32>> {
    let b = text.as_bytes();
    for (p, _) in text.match_indices('周') {
        if let Some((hi, hs)) = digits_before(b, p) {
            if hs > 0 && b[hs - 1] == b'-' {
                if let Some((lo, _)) = digits_before(b, hs - 1) {
                    if (1..=MAX_WEEK).contains(&lo) && (lo..=MAX_WEEK).contains(&hi) {
                        return Some((lo..=hi).collect());
                    }
                }
            }
        }
    }
    for (p, _) in text.match_indices('周') {
        if let Some((w, _)) = digits_before(b, p) {
            if (1..=MAX_WEEK).contains(&w) {
                return Some(vec![w]);
            }
        }
    }
    if text.contains("本周") {
        return current_week.map(|w| vec![w]);
    }
    None
}

/// 节次提取：区间 `3-4节`（可带「第」）优先，其次单节 `第3节`（单节要求「第」
/// 前缀，避免「共16节课」「3节连上」等节次数表述误提），返回 `(起始小节, 结束小节)`
/// （单节两者相同）。
pub fn parse_sections(text: &str) -> Option<(u8, u8)> {
    let b = text.as_bytes();
    for (p, _) in text.match_indices('节') {
        if let Some((hi, hs)) = digits_before(b, p) {
            if hs > 0 && b[hs - 1] == b'-' {
                if let Some((lo, _)) = digits_before(b, hs - 1) {
                    if (1..=MAX_SECTION).contains(&lo) && lo <= hi && hi <= MAX_SECTION {
                        return Some((lo as u8, hi as u8));
                    }
                }
            }
            if (1..=MAX_SECTION).contains(&hi) && text[..hs].ends_with('第') {
                return Some((hi as u8, hi as u8));
            }
        }
    }
    None
}

/// 「调整到 / 调至 / 改到 / 换到 / 更换为 …」类箭头词：调课通知的新时间/新教室
/// 通常在箭头之后（「由周一3-4节调整到周四5-6节」）。返回最早出现的箭头词之后的
/// 子串，供周次/星期/节次/教室四个提取器 **tail 优先、全文回退** 使用。
fn after_adjust_arrow(text: &str) -> Option<&str> {
    const ARROWS: [&str; 12] = [
        "调整到", "调整至", "调到", "调至", "改到", "改至", "换到", "换至", "移到", "移至",
        "更换为", "变更为",
    ];
    ARROWS
        .iter()
        .filter_map(|a| text.find(a).map(|p| p + a.len()))
        .min()
        .map(|end| &text[end..])
}

/// 星期提取：`周一`…`周日` / `星期一`…`星期日`（含「星期天」），返回 1=周一…7=周日。
/// 「周二至周四」等区间表述取第一个（契约只要求单星期，区间是 L2 降级场景由用户确认）。
pub fn parse_day(text: &str) -> Option<u8> {
    fn day_of(ch: char) -> Option<u8> {
        match ch {
            '一' => Some(1),
            '二' => Some(2),
            '三' => Some(3),
            '四' => Some(4),
            '五' => Some(5),
            '六' => Some(6),
            '日' | '天' => Some(7),
            _ => None,
        }
    }
    let chars: Vec<char> = text.chars().collect();
    for i in 0..chars.len() {
        if chars[i] == '周' {
            if let Some(d) = chars.get(i + 1).copied().and_then(day_of) {
                return Some(d);
            }
        }
        if chars[i] == '星' && chars.get(i + 1) == Some(&'期') {
            if let Some(d) = chars.get(i + 2).copied().and_then(day_of) {
                return Some(d);
            }
        }
    }
    None
}

// ---------------- L1：教室 ----------------

const CLASSROOM_TERMINATORS: [char; 16] = [
    '，', '。', '；', '、', '！', '？', '）', '…', ',', ';', '.', '!', '?', ')', '：', ':',
];

/// 教室提取：先扫 `D4-207` 形态 token（字母开头、`字母数字-数字`），
/// 未命中再看「教室：」/「教室:」后的内容（取到终止符/行尾）。
pub fn parse_classroom(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_alphabetic() {
            let start = i;
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
                j += 1;
            }
            if let Some(tok) = classroom_from_token(&text[start..j]) {
                return Some(tok.to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    let hit = text
        .find("教室：")
        .map(|p| (p, "教室：".len()))
        .or_else(|| text.find("教室:").map(|p| (p, "教室:".len())));
    if let Some((p, skip)) = hit {
        let rest = &text[p + skip..];
        let rest = rest.trim_start();
        let end = rest
            .char_indices()
            .find(|(_, ch)| CLASSROOM_TERMINATORS.contains(ch) || ch.is_whitespace())
            .map(|(k, _)| k)
            .unwrap_or(rest.len());
        let val = rest[..end].trim();
        if !val.is_empty() {
            return Some(val.to_string());
        }
    }
    None
}

/// `D4-207` 形态：token 含 `-`，`-` 前同时含字母与数字，`-` 后全为数字。
fn classroom_from_token(tok: &str) -> Option<&str> {
    let (head, tail) = tok.split_once('-')?;
    if tail.is_empty() || !tail.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let has_letter = head.bytes().any(|c| c.is_ascii_alphabetic());
    let has_digit = head.bytes().any(|c| c.is_ascii_digit());
    (has_letter && has_digit).then_some(tok)
}

// ---------------- L1：课程名匹配 ----------------

/// 对本地课程名做 contains 匹配（`《信息安全》` 等带书名号形态天然命中子串）。
/// 返回 `(course_name 填充值, course_id, 降级原因)`：
/// - 唯一名且该名下唯一门 → course_id 就位、无原因（契约的「精确匹配唯一一门」）；
/// - 唯一名多门同名 → Low「匹配到 N 门同名课程，请手动选择」；
/// - 多名命中 → 裁剪被长名包含的短名（文本含「信息安全实验」必含「信息安全」），
///   仍多名 → Low「通知中出现多门课程」（course_id=None，用户手动选择）；
/// - 0 命中 → 尝试书名号《X》提取名字填充 course_name，Low「未找到本地课表中的同名课程」。
fn match_courses(text: &str, courses: &[Course]) -> (String, Option<String>, Vec<String>) {
    let mut hit_names: Vec<&str> = Vec::new();
    for c in courses {
        if !c.name.is_empty() && text.contains(c.name.as_str()) && !hit_names.contains(&c.name.as_str())
        {
            hit_names.push(c.name.as_str());
        }
    }
    match hit_names.len() {
        0 => {
            let ext = extract_bracketed(text).unwrap_or_default();
            (
                ext,
                None,
                vec!["通知中未找到本地课表中的同名课程，请手动选择课程".to_string()],
            )
        }
        _ => {
            // 裁剪被其他命中名包含的短名（如「信息安全」⊂「信息安全实验」）
            let pruned: Vec<&str> = hit_names
                .iter()
                .filter(|n| !hit_names.iter().any(|m| *m != **n && m.contains(**n)))
                .copied()
                .collect();
            let list: &[&str] = if pruned.is_empty() { &hit_names } else { &pruned };
            if list.len() == 1 {
                let name = list[0];
                let owned: Vec<&Course> = courses.iter().filter(|c| c.name == name).collect();
                if owned.len() == 1 {
                    (name.to_string(), Some(owned[0].id.clone()), Vec::new())
                } else {
                    (
                        name.to_string(),
                        None,
                        vec![format!("匹配到 {} 门同名课程，请手动选择", owned.len())],
                    )
                }
            } else {
                (
                    list.join("；"),
                    None,
                    vec!["通知中出现多门课程，请分次粘贴或手动选择".to_string()],
                )
            }
        }
    }
}

/// 书名号《X》内容提取（0 命中时填充 course_name 用）。
fn extract_bracketed(text: &str) -> Option<String> {
    let s = text.find('《')? + '《'.len_utf8();
    let e = text[s..].find('》')? + s;
    let name = text[s..e].trim();
    (!name.is_empty()).then(|| name.to_string())
}

// ---------------- L2 前置：全校日期置换（2026-09-19 真实公告形态） ----------------

/// 一次「N月N日（星期X）」日期命中。置换型公告（「9月20日（星期日）补
/// 9月28日（星期一）课程」）不含课程名与节次，是课表层置换而非单课调课，
/// 早于 [`parse_notice_text`] 识别（否则书名号《关于…放假安排的通知》会被
/// 误提为课程名，摘录也锚不到有效行）。
#[derive(Debug, Clone)]
struct DateHit {
    month: u32,
    day: u32,
    /// 日期后紧跟的括号内「星期X」提示（可缺，缺失时由学期锚点推算）。
    weekday: Option<u8>,
    /// 日期段起始/结束字节位（置换对之间夹的连接词用它切 gap）。
    start: usize,
    end: usize,
}

/// 「月」字之前 / 之后的连续 ASCII 数字（复用 [`digits_before`] 的字节安全前提：
/// UTF-8 多字节字符各字节 ≥0x80，不会与 ASCII 混淆）。
fn digits_after(bytes: &[u8], start: usize) -> Option<(u32, usize)> {
    let mut e = start;
    while e < bytes.len() && bytes[e].is_ascii_digit() {
        e += 1;
    }
    if e == start {
        return None;
    }
    std::str::from_utf8(&bytes[start..e])
        .ok()?
        .parse()
        .ok()
        .map(|v| (v, e))
}

/// 日期后紧跟的「（…）」/「(…)」括号段：跳过空白后必须是开括号，
/// 返回 `(括号内文本, 闭括号后的字节偏移)`。
fn paren_after(s: &str) -> Option<(&str, usize)> {
    let mut chars = s.char_indices();
    let content_start = loop {
        let (i, ch) = chars.next()?;
        if ch.is_whitespace() {
            continue;
        }
        if ch == '（' || ch == '(' {
            break i + ch.len_utf8();
        }
        return None;
    };
    let rest = &s[content_start..];
    let close = rest.find(['）', ')'])?;
    let close_len = rest[close..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    Some((&rest[..close], content_start + close + close_len))
}

/// 扫描全部「N月N日（星期X）」命中（月 1..=12、日 1..=31 过滤误命中）。
fn scan_dates(text: &str) -> Vec<DateHit> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    for (p, _) in text.match_indices('月') {
        let Some((month, ms)) = digits_before(b, p) else { continue };
        if !(1..=12).contains(&month) {
            continue;
        }
        let Some((day, mut de)) = digits_after(b, p + '月'.len_utf8()) else { continue };
        if !(1..=31).contains(&day) {
            continue;
        }
        // 跳过日期尾缀「日」（「9月20日（星期日）」的括号提示在「日」之后）
        if text[de..].starts_with('日') {
            de += '日'.len_utf8();
        }
        let (weekday, end) = match paren_after(&text[de..]) {
            Some((inner, off)) => (parse_day(inner), de + off),
            None => (None, de),
        };
        out.push(DateHit { month, day, weekday, start: ms, end });
    }
    out
}

/// 置换对：两个相邻日期中间夹置换连接词（补/上/按/换）。优先取 gap 含「补」
/// 的相邻对（放假调休正文常混有多组日期，如「10月1日至8日放假」+ 补课日期），
/// 无「补」时退而取第一组连接词相邻对。
fn find_swap_pair(text: &str) -> Option<(DateHit, DateHit)> {
    let dates = scan_dates(text);
    for w in dates.windows(2) {
        if text[w[0].end..w[1].start].contains('补') {
            return Some((w[0].clone(), w[1].clone()));
        }
    }
    dates
        .windows(2)
        .find(|w| {
            text[w[0].end..w[1].start]
                .chars()
                .any(|c| matches!(c, '上' | '按' | '换'))
        })
        .map(|w| (w[0].clone(), w[1].clone()))
}

/// 学期锚点下的日期换算：年份取学期年；日期早于开学（寒假补课跨年）顺延一年。
fn date_in_semester(month: u32, day: u32, semester_start: NaiveDate) -> Option<NaiveDate> {
    let y = semester_start.year();
    let d = NaiveDate::from_ymd_opt(y, month, day)?;
    Some(if d < semester_start {
        NaiveDate::from_ymd_opt(y + 1, month, day)?
    } else {
        d
    })
}

/// 全校日期置换探测结果（契约 §22）：置换格式命中但要素不全 → `Err`（中文
/// 原因，供「未能解析」徽标）；非置换格式 → `None`。
pub struct DateSwap {
    /// 上课日（「9月20日补9月28日课」的 9月20日）；星期由日期自含
    pub date: NaiveDate,
    /// 被补日的星期（1=周一 … 7=周日）：括号「星期X」提示优先，缺失由
    /// 学期锚点推算；两者皆无 → None（候选 Low，不可采纳）
    pub weekday: Option<u8>,
    pub confidence: NoticeConfidence,
    pub reason: String,
    /// 置换句所在行（含两个日期与「补」的那行），供候选展示
    pub excerpt: String,
}

/// 探测正文中的**全校日期置换**（「9月20日（星期日）补9月28日（星期一）课程」，
/// 2026-09-19 真实公告形态）：置换型通知不含课程名/节次，是课表层置换——
/// 旧逐课解析会把书名号《关于…放假安排的通知》误提为课程名。命中时 tauri 层
/// 产出置换候选（`apply_swap_day` 写 `config.swap_days`），替代逐课 Extra。
pub fn detect_date_swap(
    text: &str,
    semester_start: Option<NaiveDate>,
) -> Option<Result<DateSwap, String>> {
    let text = text.trim();
    let (d1, d2) = find_swap_pair(text)?;
    // 上课日必须锚定学期年（无锚点无法换算年份/星期/周次）
    let Some(anchor) = semester_start else {
        return Some(Err(
            "该通知是全校日期置换型，但未设置学期起始日（请先在设置中同步学期信息）".to_string(),
        ));
    };
    let date = match date_in_semester(d1.month, d1.day, anchor) {
        Some(d) => d,
        None => return Some(Err("置换日期无法解析".to_string())),
    };
    let weekday = d2.weekday.or_else(|| {
        date_in_semester(d2.month, d2.day, anchor).map(|d| d.weekday().number_from_monday() as u8)
    });
    let (confidence, reason) = match weekday {
        Some(_) => (NoticeConfidence::High, String::new()),
        None => (
            NoticeConfidence::Low,
            "缺少被补日的星期信息，无法确定补哪天的课".to_string(),
        ),
    };
    let excerpt = excerpt_of(text, Some(&format!("{}月{}日", d2.month, d2.day)));
    Some(Ok(DateSwap { date, weekday, confidence, reason, excerpt }))
}

// ---------------- L2：主流程 ----------------

/// 解析通知正文 → 单候选（契约 §2.5：输入通知正文 + 本地课程表 + 当前周次）。
pub fn parse_notice_text(
    text: &str,
    courses: &[Course],
    current_week: Option<u32>,
) -> Vec<NoticeCandidate> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let (change_type, _explicit) = detect_kind(text);
    let (course_name, course_id, mut reasons) = match_courses(text, courses);

    // 新时间/新教室在「调整到 …」箭头之后（无箭头则 tail=text）：
    // tail 优先取不到再回退全文（如「周一3-4节 由 D4-207 调整到 D4-305」星期仍在全文）
    let tail = after_adjust_arrow(text).unwrap_or(text);
    let weeks = parse_weeks(tail, current_week).or_else(|| parse_weeks(text, current_week));
    if weeks.is_none() {
        reasons.push(if text.contains("本周") && current_week.is_none() {
            "提到「本周」但无法确定当前教学周（未导入课表或假期），请补充具体周次".to_string()
        } else {
            "缺少周次信息".to_string()
        });
    }
    let new_day = parse_day(tail).or_else(|| parse_day(text));
    if new_day.is_none() {
        reasons.push("缺少星期信息".to_string());
    }
    let sections = parse_sections(tail).or_else(|| parse_sections(text));
    if sections.is_none() {
        reasons.push("缺少节次信息".to_string());
    }
    let new_position = parse_classroom(tail).or_else(|| parse_classroom(text));

    // 置信 = 课程唯一命中 && 周次/星期/节次齐全（reasons 为空 ⇔ 三要素齐 + 课程唯一）
    let confidence = if reasons.is_empty() {
        NoticeConfidence::High
    } else {
        NoticeConfidence::Low
    };

    vec![NoticeCandidate {
        notice_id: notice_id_for(text),
        course_id,
        course_name,
        change_type,
        weeks: weeks.unwrap_or_default(),
        new_day,
        new_start_section: sections.map(|(s, _)| s),
        new_end_section: sections.map(|(_, e)| e),
        new_position,
        confidence,
        reason: reasons.join("；"),
        excerpt: excerpt_of(text, None),
    }]
}

/// 原文摘录：课程名所在行优先，否则首行；超 80 字符截断加省略号。
fn excerpt_of(text: &str, anchor: Option<&str>) -> String {
    let line = anchor
        .and_then(|n| text.lines().find(|l| l.contains(n)))
        .or_else(|| text.lines().next())
        .unwrap_or("");
    let line = line.trim();
    if line.chars().count() > 80 {
        format!("{}…", line.chars().take(80).collect::<String>())
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CourseSource;

    fn course(id: &str, name: &str, day: u8, start: u8, end: u8, weeks: Vec<u32>) -> Course {
        Course {
            id: id.into(),
            course_table_id: "default".into(),
            name: name.into(),
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

    fn fixture() -> Vec<Course> {
        vec![
            course("default-a", "信息安全", 1, 1, 2, vec![1, 3]),
            course("default-b", "密码学", 3, 3, 4, vec![2]),
        ]
    }

    // ---------- 契约要求的 5 类 ----------

    /// 1) 高置信：全要素 + 精确唯一匹配；类型=调课、教室形态命中。
    #[test]
    fn high_confidence_reschedule() {
        let cands = parse_notice_text(
            "【调课通知】第5周 信息安全 由周一3-4节 调整到 周四3-4节 D4-305",
            &fixture(),
            Some(2),
        );
        assert_eq!(cands.len(), 1);
        let c = &cands[0];
        assert_eq!(c.confidence, NoticeConfidence::High);
        assert_eq!(c.reason, "");
        assert_eq!(c.course_id.as_deref(), Some("default-a"));
        assert_eq!(c.course_name, "信息安全");
        assert_eq!(c.change_type, OverrideKind::Rescheduled);
        assert_eq!(c.weeks, vec![5]);
        assert_eq!(c.new_day, Some(4));
        assert_eq!(c.new_start_section, Some(3));
        assert_eq!(c.new_end_section, Some(4));
        assert_eq!(c.new_position.as_deref(), Some("D4-305"));
        assert!(c.notice_id.starts_with("manual:"));
        assert_eq!(c.notice_id.len(), "manual:".len() + 16);
        assert!(!c.excerpt.is_empty());
    }

    /// 2) 低置信（缺要素）：缺星期/节次/周次时 reason 写明；缺「本周」锚点单列文案。
    #[test]
    fn low_confidence_missing_elements() {
        // 缺星期 + 节次
        let c = &parse_notice_text("第5周 信息安全 教室更换为 D4-305", &fixture(), Some(2))[0];
        assert_eq!(c.confidence, NoticeConfidence::Low);
        assert_eq!(c.course_id.as_deref(), Some("default-a"));
        assert!(c.reason.contains("缺少星期"));
        assert!(c.reason.contains("缺少节次"));
        // 缺周次（含星期节次）
        let c = &parse_notice_text("周四3-4节 信息安全 调整到 D4-305", &fixture(), Some(2))[0];
        assert_eq!(c.confidence, NoticeConfidence::Low);
        assert!(c.reason.contains("缺少周次"));
        // 「本周」但无锚点
        let c = &parse_notice_text("本周周四3-4节 信息安全 调整到 D4-305", &fixture(), None)[0];
        assert_eq!(c.confidence, NoticeConfidence::Low);
        assert!(c.reason.contains("无法确定当前教学周"));
    }

    /// 3) 多门匹配：同名两门 → Low 且 course_id=None；多名命中裁剪后单名仍可唯一。
    #[test]
    fn ambiguous_course_matches_low() {
        let courses = vec![
            course("default-a", "信息安全", 1, 1, 2, vec![1]),
            course("default-a2", "信息安全", 5, 1, 2, vec![1]),
        ];
        let c = &parse_notice_text("第5周周三3-4节 信息安全 停课", &courses, Some(2))[0];
        assert_eq!(c.confidence, NoticeConfidence::Low);
        assert_eq!(c.course_id, None);
        assert_eq!(c.course_name, "信息安全");
        assert!(c.reason.contains("同名课程"));

        // 「信息安全」与「信息安全实验」：文本只提长名 → 裁剪后唯一命中（不误判多门）
        let courses = vec![
            course("default-a", "信息安全", 1, 1, 2, vec![1]),
            course("default-l", "信息安全实验", 2, 3, 4, vec![1]),
        ];
        let c = &parse_notice_text("第5周周二3-4节 信息安全实验 调整到 D4-305", &courses, Some(2))[0];
        assert_eq!(c.confidence, NoticeConfidence::High);
        assert_eq!(c.course_id.as_deref(), Some("default-l"));
    }

    /// 4) 匹配不到课程：course_id=None、书名号提取填充 course_name。
    #[test]
    fn no_course_match_low() {
        let c = &parse_notice_text(
            "第5周周三3-4节 《高等数学》 调整到 D4-305",
            &fixture(),
            Some(2),
        )[0];
        assert_eq!(c.confidence, NoticeConfidence::Low);
        assert_eq!(c.course_id, None);
        assert_eq!(c.course_name, "高等数学");
        assert!(c.reason.contains("未找到本地课表"));
    }

    /// 5) 停课：类型=cancelled；停课时间（星期/节次）原样保留供前端定位。
    #[test]
    fn cancelled_notice_keeps_time() {
        let c = &parse_notice_text("第3周周三3-4节 密码学 停课一次", &fixture(), Some(2))[0];
        assert_eq!(c.confidence, NoticeConfidence::High);
        assert_eq!(c.change_type, OverrideKind::Cancelled);
        assert_eq!(c.course_id.as_deref(), Some("default-b"));
        assert_eq!(c.weeks, vec![3]);
        assert_eq!(c.new_day, Some(3));
        assert_eq!(c.new_position, None);
    }

    // ---------- 周次解析专项 ----------

    /// 「本周」解析：带锚点取当前周；「3-4周」区间展开。
    #[test]
    fn week_parsing_current_and_range() {
        let c = &parse_notice_text("本周周四3-4节 信息安全 调整到 D4-305", &fixture(), Some(7))[0];
        assert_eq!(c.confidence, NoticeConfidence::High);
        assert_eq!(c.weeks, vec![7]);

        let c = &parse_notice_text("3-4周 周四3-4节 信息安全 调整到 D4-305", &fixture(), Some(7))[0];
        assert_eq!(c.weeks, vec![3, 4]);
        assert_eq!(c.confidence, NoticeConfidence::High);

        let c = &parse_notice_text("第11-12周 周四3-4节 信息安全 调整到 D4-305", &fixture(), Some(7))[0];
        assert_eq!(c.weeks, vec![11, 12]);
    }

    /// 周次解析器单元形态：「第N周」/「N周」/「本周」/区间，及「周X」星期不干扰。
    #[test]
    fn parse_weeks_forms() {
        assert_eq!(parse_weeks("第3周", Some(9)), Some(vec![3]));
        assert_eq!(parse_weeks("第12周上机", Some(9)), Some(vec![12]));
        assert_eq!(parse_weeks("12周内完成", Some(9)), Some(vec![12]));
        assert_eq!(parse_weeks("本周", Some(9)), Some(vec![9]));
        assert_eq!(parse_weeks("本周", None), None);
        assert_eq!(parse_weeks("周三3-4节", Some(9)), None); // 星期不触发周次
        assert_eq!(parse_weeks("没有周次表述", None), None);
        // 区间优先：不把「3-4周」拆成单周 4
        assert_eq!(parse_weeks("3-4周", None), Some(vec![3, 4]));
        assert_eq!(parse_weeks("第1-16周", None), Some((1..=16).collect::<Vec<u32>>()));
    }

    /// 节次解析形态：区间/单节/「第N节」，「每节课」「共16节课」不误提。
    #[test]
    fn parse_sections_forms() {
        assert_eq!(parse_sections("3-4节"), Some((3, 4)));
        assert_eq!(parse_sections("第3-4节"), Some((3, 4)));
        assert_eq!(parse_sections("第3节"), Some((3, 3)));
        assert_eq!(parse_sections("每节课都要签到"), None);
        assert_eq!(parse_sections("共16节课"), None);
        assert_eq!(parse_sections("3节连上"), None); // 单节必须带「第」
        assert_eq!(parse_sections("11-12节 晚课"), Some((11, 12)));
    }

    /// 星期解析形态：周X / 星期X / 星期天，「本周」「周末」不误提。
    #[test]
    fn parse_day_forms() {
        assert_eq!(parse_day("周三3-4节"), Some(3));
        assert_eq!(parse_day("星期日"), Some(7));
        assert_eq!(parse_day("星期天"), Some(7));
        assert_eq!(parse_day("周日"), Some(7));
        assert_eq!(parse_day("本周"), None);
        assert_eq!(parse_day("周末"), None);
        assert_eq!(parse_day("周二至周四"), Some(2));
    }

    /// 教室解析：D4-207 形态、「教室：」兜底（含中文教学楼名）、无教室为 None。
    #[test]
    fn classroom_forms() {
        assert_eq!(parse_classroom("调至 D4-305 上课"), Some("D4-305".into()));
        assert_eq!(parse_classroom("教室：文科楼B203"), Some("文科楼B203".into()));
        assert_eq!(parse_classroom("教室:D4-305"), Some("D4-305".into()));
        assert_eq!(parse_classroom("另行通知"), None);
        // 非「字母数字-数字」形态不命中
        assert_eq!(parse_classroom("教室在 207"), None);
    }

    /// 类型关键词：补课 → extra；无关键词默认调课（置信不受影响）。
    #[test]
    fn kind_detection() {
        let c = &parse_notice_text("第5周周六3-4节 密码学 补课一次 教室D4-305", &fixture(), Some(2))[0];
        assert_eq!(c.change_type, OverrideKind::Extra);
        assert_eq!(c.confidence, NoticeConfidence::High);
        let c = &parse_notice_text("第5周周六3-4节 密码学 上课地点 D4-305", &fixture(), Some(2))[0];
        assert_eq!(c.change_type, OverrideKind::Rescheduled);
        assert_eq!(c.confidence, NoticeConfidence::High);
    }

    /// noticeId 对同一正文稳定、不同正文不同；空文本不产出候选。
    #[test]
    fn notice_id_stable_and_empty_text() {
        assert_eq!(notice_id_for("abc"), notice_id_for("abc"));
        assert_ne!(notice_id_for("abc"), notice_id_for("abd"));
        assert!(parse_notice_text("   \n ", &fixture(), Some(2)).is_empty());
    }

    // ---------- 全校日期置换（2026-09-19 真实公告） ----------

    /// 真实公告正文（jwc.cwxu.edu.cn/info/1100/5957.htm 取证原文，脱敏前导敬语）。
    const SWAP_NOTICE: &str = "全体师生：\n\n根据学校《关于2026年中秋、国庆放假安排的通知》要求，\
9月20日（星期日）补9月28日（星期一）课程。全校所有学生（含2026级新生）按照本方案执行，请全校所有学生按时上课，不得缺勤。\n\
\n请各教学单位、任课教师、全体学生提前做好上课安排，按时开展教学活动。\n\n教务处\n\n2026年9月17日";

    /// 置换探测：真实公告正文 → Swap {date=2026-09-20, weekday=1（周一）,
    /// High}；书名号《…放假安排的通知》不干扰（置换分支先于课程名提取）。
    #[test]
    fn date_swap_detected_from_real_notice() {
        use chrono::NaiveDate;
        let sw = detect_date_swap(
            SWAP_NOTICE,
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
        )
        .expect("置换格式应命中")
        .expect("要素齐全应 Ok");
        assert_eq!(sw.date, NaiveDate::from_ymd_opt(2026, 9, 20).unwrap());
        assert_eq!(sw.weekday, Some(1));
        assert_eq!(sw.confidence, NoticeConfidence::High);
        assert!(sw.reason.is_empty());
    }

    /// 缺学期锚点：置换格式命中但 Err（原因写明缺学期起始日），不产误候选。
    #[test]
    fn date_swap_without_anchor_is_err() {
        let r = detect_date_swap(SWAP_NOTICE, None).expect("置换格式应命中");
        assert!(r.is_err());
        assert!(r.err().unwrap().contains("学期起始日"));
    }

    /// 常规单课通知不是置换：detect 返回 None，普通解析路径不受影响。
    #[test]
    fn regular_notice_is_not_a_swap() {
        use chrono::NaiveDate;
        assert!(detect_date_swap(
            "第5周周四3-4节 信息安全 调整到 D4-305",
            Some(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap())
        )
        .is_none());
    }
}