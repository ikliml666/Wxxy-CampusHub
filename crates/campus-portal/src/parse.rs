//! 门户响应解析纯函数（`parse_*(&str) -> Result<T>`），离线单测用脱敏 fixture。
//!
//! 两种响应信封，解析器不混用（计划 §1.1）：
//! - 门户自身服务：`{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":...}`
//!   —— 见 [`envelope_data`]；
//! - 日程服务 `bs-schedule/*`：`{"code":"0","msg":"ok","data":...}`，成功判据
//!   `code=="0"` —— 见 [`schedule_data`]（失败形态实测 code 为数字、message 键名
//!   会变成 `message`，两者都兼容）。

use crate::access::classify_app_access;
use crate::{
    AppGroup, AppItem, CourseBrief, InfoColumn, InfoItem, InfoPage, PortalError, ScheduleClassify,
    ScheduleDayCount, ScheduleEvent, ScheduleNoticeBrief, SemesterInfo, TodoItem, TodoPage,
    TodoTab, WalletSummary,
};
use campus_schedule::TimeSlot;

/// 门户统一信封校验：`meta.success==true` 时返回 `data` 引用，否则 Err（保留服务端 message）。
fn envelope_data<'a>(
    v: &'a serde_json::Value,
    api: &str,
) -> Result<&'a serde_json::Value, PortalError> {
    let success = v
        .get("meta")
        .and_then(|m| m.get("success"))
        .and_then(|s| s.as_bool());
    if success != Some(true) {
        let msg = v
            .get("meta")
            .and_then(|m| m.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("服务端未返回原因");
        return Err(PortalError::Parse(format!("{api} 失败: {msg}")));
    }
    v.get("data")
        .ok_or_else(|| PortalError::Parse(format!("{api} 响应缺少 data")))
}

/// 宽松取字符串：字符串原样，整数转字符串（实测字段以字符串为主，容数字形态）。
fn jstr(v: &serde_json::Value) -> Option<String> {
    v.as_str()
        .map(str::to_string)
        .or_else(|| v.as_i64().map(|n| n.to_string()))
}

/// 宽松取数字：数字原样，字符串数字 trim 后解析。
fn jnum(v: &serde_json::Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
}

/// 解析 `api/upp/config/querySemesterInfo` 响应（纯函数供离线单测）。
///
/// `data` 七个字段（grade/semester/currentWeek/weekCount/startDate/endDate/
/// currentWeekDay）任一缺失或类型异常 → [`PortalError::Parse`]（上层把 semester
/// 置 None，不阻塞其余字段）；`currentDate` 不在 IPC 契约内，忽略。
pub fn parse_semester_info(body: &str) -> Result<SemesterInfo, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("querySemesterInfo 响应解析失败: {e}")))?;
    let d = envelope_data(&v, "querySemesterInfo")?;
    let s = |key: &str| -> Result<String, PortalError> {
        let raw = d
            .get(key)
            .ok_or_else(|| PortalError::Parse(format!("querySemesterInfo 缺少 {key}")))?;
        jstr(raw)
            .ok_or_else(|| PortalError::Parse(format!("querySemesterInfo 字段 {key} 类型异常")))
    };
    Ok(SemesterInfo {
        grade: s("grade")?,
        semester: s("semester")?,
        current_week: s("currentWeek")?,
        week_count: s("weekCount")?,
        start_date: s("startDate")?,
        end_date: s("endDate")?,
        current_week_day: s("currentWeekDay")?,
    })
}

/// 解析钱包卡 `api/upp/contentDisplay/queryAppointCard/<cardId>` 响应（纯函数供离线单测）。
///
/// 实测形态：`data.data` 是 **JSON 字符串**，二次解析后为数组，取首项
/// `YE`（余额）/`SL`（在借图书）/`mailNewCount`（未读邮件）。`loginUrl`
/// （邮箱免密链接，内含 authkey）**直接丢弃**——结构体不定义该字段。
/// 单字段缺失/类型异常 → 该项 `None`（部分成功）；信封失败 / `data.data` 非字符串 /
/// 内层数组为空 → [`PortalError::Parse`]。
pub fn parse_wallet_summary(body: &str) -> Result<WalletSummary, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("钱包卡响应解析失败: {e}")))?;
    let d = envelope_data(&v, "钱包卡")?;
    let inner = d
        .get("data")
        .and_then(|x| x.as_str())
        .ok_or_else(|| PortalError::Parse("钱包卡响应缺少 data.data 字符串".to_string()))?;
    let items: serde_json::Value = serde_json::from_str(inner)
        .map_err(|e| PortalError::Parse(format!("钱包卡内层 JSON 解析失败: {e}")))?;
    let first = items
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| PortalError::Parse("钱包卡无数据".to_string()))?;
    // 字段级宽松：缺失/类型异常只影响自身项，不影响其余
    let balance = first.get("YE").and_then(jnum);
    let books = first.get("SL").and_then(jnum).map(|n| n.max(0.0) as u32);
    let mail = first
        .get("mailNewCount")
        .and_then(jnum)
        .map(|n| n.max(0.0) as u32);
    Ok(WalletSummary {
        card_balance: balance,
        book_borrowed: books,
        mail_unread: mail,
    })
}

/// `queryAWeekSchedule` 原始形态（解析结果供「下一节课」计算）。
#[derive(Debug, Clone, PartialEq)]
pub struct WeekSchedule {
    /// 服务端已按当前周返回的矩阵：行=周一..周日；10 列 = **5 大节 × 2 小节**
    ///（列对 (0,1)→大节1 … (8,9)→大节5，与 swskjc+xwskjc+wsskjc=4+4+2 吻合，
    /// 实测四门课逐条验证列对→大节映射），一门课占同一大节的相邻两列；
    /// 空串=无课。
    pub grid: Vec<Vec<String>>,
    /// 今天星期几（服务端 `xqj`，1=周一 … 7=周日）。
    pub weekday: u8,
}

/// 解析 `api/uppcard/kbsz/queryAWeekSchedule` 响应（纯函数供离线单测）。
///
/// 只取「下一节课」所需最小字段：`resultsJsonArr` 矩阵与 `xqj`（周次/学期等
/// 其余字段用不上，忽略）。格子为 `"课程名,教室,教学班,姓名"`，此处不拆分
/// （拆分见 [`next_course`]）。
pub fn parse_week_schedule(body: &str) -> Result<WeekSchedule, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("周课表响应解析失败: {e}")))?;
    let d = envelope_data(&v, "周课表")?;
    let rows = d
        .get("resultsJsonArr")
        .and_then(|x| x.as_array())
        .ok_or_else(|| PortalError::Parse("周课表响应缺少 resultsJsonArr".to_string()))?;
    if rows.is_empty() {
        return Err(PortalError::Parse("周课表矩阵为空".to_string()));
    }
    let grid = rows
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or_else(|| PortalError::Parse("周课表矩阵行结构异常".to_string()))
                .map(|cells| {
                    cells
                        .iter()
                        .map(|c| c.as_str().unwrap_or_default().to_string())
                        .collect::<Vec<_>>()
                })
        })
        .collect::<Result<Vec<_>, PortalError>>()?;
    let weekday = d
        .get("xqj")
        .and_then(|x| x.as_u64().or_else(|| x.as_str()?.trim().parse().ok()))
        .unwrap_or(0) as u8;
    Ok(WeekSchedule { grid, weekday })
}

/// 校本「大节」作息表（100 分钟/大节；矩阵 10 列 = 5 大节 × 2 小节）。
///
/// ponytail：**仅门户聚合兜底用**（`next_course`/「下一节课」横幅按矩阵大节
/// 列对查时刻）——课表链路（网格/ICS/今日页）已统一为小节口径
///（[`section_time_slots`]，tauri 层契约 §18），大节表不再参与课表取值。
///
/// 时间锚定：大节2 10:10-11:50、大节3 13:45-15:25 为 2026-09-18 日程服务实测
///（`findScheduleBetweenTime` 的 `Default-class` 事件真实上课时间）；大节1 由
/// 大节2 起点反推（30 分钟大课间）；大节4/5 为 **2026-09-19 用户截图校准**
///（15:55-17:35、18:45-21:20）。**不改动 `campus-schedule::default_time_slots()`**
///（上游默认值，被金标测试钉住）。
pub fn block_time_slots() -> Vec<TimeSlot> {
    const RAW: [(u8, &str, &str); 5] = [
        // 由大节2 10:10 起反推（30 分钟大课间）；2026-09-19 随小节表校准确认
        (1, "08:00", "09:40"),
        // 2026-09-18 日程服务实测（信息隐藏与取证技术 10:10→11:50）
        (2, "10:10", "11:50"),
        // 2026-09-18 日程服务实测（信息安全等 13:45→15:25）
        (3, "13:45", "15:25"),
        // 2026-09-19 用户截图校准
        (4, "15:55", "17:35"),
        // 2026-09-19 用户截图校准
        (5, "18:45", "21:20"),
    ];
    RAW.into_iter()
        .map(|(number, start_time, end_time)| TimeSlot {
            number,
            start_time: start_time.into(),
            end_time: end_time.into(),
            alias: None,
        })
        .collect()
}

/// 校本「小节」作息表（11 小节，45 分钟/节；2026-09-19 用户截图校准，上游参考
/// 截图同为 11 节口径）。
///
/// `pub`：**课表全链路的内置默认小节表**（重设计轮批 A，tauri 层契约 §18）——
/// 网格时间列、ICS 时刻展开、今日页节次课取值统一经 `effective_slots_at` 的
/// 第三段回落用它；与大节表 [`block_time_slots`]（仅门户聚合兜底）的分工见
/// tauri 层契约 §18。单点事实来源，调用方不得复制常量。
pub fn section_time_slots() -> Vec<TimeSlot> {
    const RAW: [(u8, &str, &str); 11] = [
        (1, "08:00", "08:45"),
        (2, "08:55", "09:40"),
        (3, "10:10", "10:55"),
        (4, "11:05", "11:50"),
        (5, "13:45", "14:30"),
        (6, "14:40", "15:25"),
        (7, "15:55", "16:40"),
        (8, "16:50", "17:35"),
        (9, "18:45", "19:30"),
        (10, "19:40", "20:25"),
        (11, "20:35", "21:20"),
    ];
    RAW.into_iter()
        .map(|(number, start_time, end_time)| TimeSlot {
            number,
            start_time: start_time.into(),
            end_time: end_time.into(),
            alias: None,
        })
        .collect()
}

/// 当前时刻「已开始」的节数（按传入节次表口径；传 [`block_time_slots`] 即大节数，
/// 0..=slots.len()，早于第一节为 0，晚于末节为全部）。
///
/// `"HH:MM"` 等长字符串字典序即时间序。
pub fn elapsed_slot_count(now_hm: &str, slots: &[TimeSlot]) -> usize {
    slots
        .iter()
        .filter(|s| s.start_time.as_str() <= now_hm)
        .count()
}

/// 周课表矩阵单格 → [`CourseBrief`]（空格 None；第 4 段任课教师名丢弃）。
///
/// 列号 → 大节号：10 列 = 5 大节 × 2 小节，`(0,1)→大节1 … (8,9)→大节5`，
/// 即 `col / 2 + 1`；课程起始时间取该大节开始时刻（真机缺陷教训：按小节号
/// 查默认 13 节表会把大节4 的课标成 14:50，正确为 15:55）。
fn course_from_cell(cell: &str, col: usize, slots: &[TimeSlot]) -> Option<CourseBrief> {
    let cell = cell.trim();
    if cell.is_empty() {
        return None;
    }
    let mut seg = cell.split(',').map(str::trim);
    let name = seg.next().filter(|s| !s.is_empty())?.to_string();
    let room = seg.next().unwrap_or_default().to_string();
    let teaching_class = seg.next().unwrap_or_default().to_string();
    let slot = col as u32 / 2 + 1;
    let start_time = slots
        .iter()
        .find(|s| s.number as u32 == slot)
        .map(|s| s.start_time.clone());
    Some(CourseBrief {
        name,
        room,
        teaching_class,
        slot,
        start_time,
    })
}

/// 「下一节课」：从（今天 `weekday_now`，已开始 `elapsed` 个大节）起在矩阵内向后
/// 线性扫第一个非空格——当天从大节 `elapsed+1` 的首列（= `elapsed*2`）起，之后
/// 各天从第 0 列起；**只在本周矩阵内**（服务端矩阵即当前周，跨周单双周课表
/// 可能不同，返回 None 隐藏横幅）。
///
/// `weekday_now`：1=周一…7=周日，越界 → None；`grid` 与 `weekday_now` 行列不符 → None。
pub fn next_course(
    grid: &[Vec<String>],
    weekday_now: u8,
    elapsed: usize,
    slots: &[TimeSlot],
) -> Option<CourseBrief> {
    let day_idx = (weekday_now as usize).checked_sub(1)?;
    if day_idx >= grid.len() {
        return None;
    }
    // 今天从 elapsed*2 列起（大节 elapsed 进行/已过，其列对 (elapsed-1)*2 起跳过），
    // 后续天从 0 列起（(row_offset, start_col) 对）
    let scans = std::iter::once((0usize, elapsed * 2))
        .chain((1..grid.len() - day_idx).map(|off| (off, 0usize)));
    for (row_off, start_col) in scans {
        let Some(row) = grid.get(day_idx + row_off) else {
            continue;
        };
        for (col, cell) in row.iter().enumerate().skip(start_col) {
            if let Some(c) = course_from_cell(cell, col, slots) {
                return Some(c);
            }
        }
    }
    None
}

/// 由本地当前时刻计算「下一节课」（[`next_course`] 的命令层薄包装；
/// 星期与 HH:MM 取本机时钟，节次表用校本大节表 [`block_time_slots`]）。
pub fn next_course_from_now(ws: &WeekSchedule) -> Option<CourseBrief> {
    use chrono::Datelike;
    let now = chrono::Local::now();
    let weekday = now.weekday().number_from_monday() as u8;
    let now_hm = now.format("%H:%M").to_string();
    let slots = block_time_slots();
    let elapsed = elapsed_slot_count(&now_hm, &slots);
    next_course(&ws.grid, weekday, elapsed, &slots)
}

/// 实测全量资讯栏目 id ↔ 名称（2026-09-18 主智能体实机侦察，计划 §1.2）。
///
/// 依据：`queryUserSubscribeColumn` 只返回**当前账号订阅的栏目**（实测 3 个），
/// 而资讯页栏目 rail 需要完整 7 栏——未订阅栏目与名称以此表兜底补全。
/// 正文均在官网静态页（`*.cwxu.edu.cn`）。
pub(crate) const KNOWN_COLUMNS: &[(&str, &str)] = &[
    ("9", "通知公告"),
    ("f382fddd843b4058a486a9375ecf422d", "校园要闻"),
    ("a8bc1e5a9225475b9841b5a237c690df", "校园快讯"),
    ("ea0a5b2158bf48b3afeb026477c626e4", "教务处"),
    ("4f5a7ccbc5704a6690f0d3ac429c2201", "学工处"),
    ("5d2c45d23866497cb2bfe93e9f136bb2", "规章制度"),
    ("d4901da2e5df4db9b6b551df4d5b85dd", "团委"),
];

/// `titleLocale` → 中文名：实测为 JSON 字符串 `{"zh_CN":...,"en_US":...}`，
/// 宽松兼容直接给对象/字符串的形态；zh_CN 缺失时取任意一个非空值。
fn locale_zh(v: Option<&serde_json::Value>) -> Option<String> {
    let obj: serde_json::Value = match v? {
        serde_json::Value::String(s) => serde_json::from_str(s).ok()?,
        other => other.clone(),
    };
    obj.get("zh_CN")
        .and_then(jstr)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            obj.as_object()?
                .values()
                .find_map(jstr)
                .filter(|s| !s.is_empty())
        })
}

/// 分页字段宽松 u32（缺失/异常按 0；total/pageCount 实测不可靠，见 InfoPage）。
fn jnum_u32(v: &serde_json::Value, key: &str) -> u32 {
    v.get(key).and_then(jnum).unwrap_or(0.0).max(0.0) as u32
}

/// 解析 `api/uppinfo/userSetting/queryUserSubscribeColumn`（纯函数供离线单测）。
///
/// 输出 = 订阅项（按接口 sortNum 升序，名称取 `titleLocale.zh_CN`，解析失败
/// 回落 [`KNOWN_COLUMNS`] 常量名）+ 未订阅项（按实测全量顺序垫底补全）。
/// 未订阅项无接口 sortNum，用 1000+序号保序占位（实测订阅 sortNum 为个位数量级，
/// 前端直接消费数组顺序，该值仅排序用）。
pub fn parse_info_columns(body: &str) -> Result<Vec<InfoColumn>, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("资讯栏目响应解析失败: {e}")))?;
    let d = envelope_data(&v, "资讯栏目")?;
    let arr = d
        .as_array()
        .ok_or_else(|| PortalError::Parse("资讯栏目响应结构异常".to_string()))?;
    let mut out: Vec<InfoColumn> = Vec::with_capacity(KNOWN_COLUMNS.len());
    for item in arr {
        let Some(id) = item
            .get("columnId")
            .and_then(jstr)
            .filter(|s| !s.is_empty())
        else {
            continue; // 缺 id 的条目无意义，跳过
        };
        let known = KNOWN_COLUMNS.iter().find(|(kid, _)| *kid == id);
        let name = locale_zh(item.get("titleLocale"))
            .or_else(|| known.map(|(_, n)| (*n).to_string()))
            .unwrap_or_else(|| "未知栏目".to_string());
        let sort_num = jnum_u32(item, "sortNum");
        out.push(InfoColumn { id, name, sort_num });
    }
    for (i, (id, name)) in KNOWN_COLUMNS.iter().enumerate() {
        if !out.iter().any(|c| c.id == *id) {
            out.push(InfoColumn {
                id: (*id).to_string(),
                name: (*name).to_string(),
                sort_num: 1000 + i as u32,
            });
        }
    }
    out.sort_by_key(|c| c.sort_num);
    Ok(out)
}

/// 解析 `api/uppinfo/infoCenter/querySimpleInfoCenter`（纯函数供离线单测）。
///
/// ⚠️ 实测 `total`/`pageCount` 均不可靠（pageSize=1 时返回 0），原样透传；
/// **前端分页以 items.length 与 pageSize 判断**。`infoId` 或 `extLink` 缺失的
/// 条目跳过（无 id 无法标记已读、无 url 无法打开正文），其余字段缺失降级空串。
pub fn parse_info_list(body: &str) -> Result<InfoPage, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("资讯列表响应解析失败: {e}")))?;
    let d = envelope_data(&v, "资讯列表")?;
    let empty = Vec::new();
    let list = d.get("list").and_then(|x| x.as_array()).unwrap_or(&empty);
    let items = list
        .iter()
        .filter_map(|it| {
            let id = it.get("infoId").and_then(jstr).filter(|s| !s.is_empty())?;
            let url = it.get("extLink").and_then(jstr).filter(|s| !s.is_empty())?;
            Some(InfoItem {
                id,
                title: it.get("infoTitle").and_then(jstr).unwrap_or_default(),
                column_title: it.get("columnTitle").and_then(jstr).unwrap_or_default(),
                publish_time: it.get("publishTime").and_then(jstr).unwrap_or_default(),
                dept: it
                    .get("publishDeptName")
                    .and_then(jstr)
                    .filter(|s| !s.is_empty()),
                url,
            })
        })
        .collect();
    Ok(InfoPage {
        page: jnum_u32(d, "pageNum"),
        page_size: jnum_u32(d, "pageSize"),
        page_count: jnum_u32(d, "pageCount"),
        total: jnum_u32(d, "total"),
        items,
    })
}

// ---------------- 调课通知自动发现（重设计轮批 A，tauri 层契约 §18） ----------------

/// 调课通知标题关键词表。
///
/// ponytail: 固定词表（强 = 明确调课动词，弱 = 常见调整表述）；误报/漏报
/// 反馈后再调，升级路径 = 可配置词表（用户自定义关键词）。
pub(crate) const NOTICE_KEYWORDS_STRONG: &[&str] = &["调课", "停课", "补课"];
pub(crate) const NOTICE_KEYWORDS_WEAK: &[&str] = &["教学调整", "课程调整", "上课时间", "课程变更"];

/// 标题关键词命中（纯函数供离线单测）：强命中任一或弱命中 ≥1 → 返回命中的
/// 关键词列表（强在前，供前端展示与后续排序）；无命中 → 空表。
pub fn notice_keyword_hits(title: &str) -> Vec<&'static str> {
    let mut hits: Vec<&'static str> = Vec::new();
    let mut push_hits = |words: &'static [&'static str]| {
        for w in words {
            if title.contains(w) && !hits.contains(w) {
                hits.push(w);
            }
        }
    };
    push_hits(NOTICE_KEYWORDS_STRONG);
    push_hits(NOTICE_KEYWORDS_WEAK);
    hits
}

/// 各栏目资讯页 → 调课通知简报（纯函数供离线单测）：逐条按 [`notice_keyword_hits`]
/// 过滤标题（任一命中即纳入），按 `url` 去重（同一公告多栏目转载），按日期
/// 倒序（`publish_time` 为 `"YYYY-MM-DD HH:MM:SS"`，字典序即时间序），上限
/// 20 条。`columns` 与 `pages` 按序对应（client 层逐栏目拉取后 zip 传入）。
pub fn collect_schedule_notices(
    columns: &[(&str, &str)],
    pages: &[InfoPage],
    max_items: usize,
) -> Vec<ScheduleNoticeBrief> {
    let mut out: Vec<ScheduleNoticeBrief> = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for ((_id, name), page) in columns.iter().zip(pages) {
        for it in &page.items {
            let hits = notice_keyword_hits(&it.title);
            if hits.is_empty() || !seen.insert(it.url.as_str()) {
                continue;
            }
            out.push(ScheduleNoticeBrief {
                title: it.title.clone(),
                date: it.publish_time.clone(),
                url: it.url.clone(),
                column: if it.column_title.is_empty() {
                    (*name).to_string()
                } else {
                    it.column_title.clone()
                },
                matched_keywords: hits.into_iter().map(str::to_string).collect(),
            });
        }
    }
    out.sort_by(|a, b| b.date.cmp(&a.date));
    out.truncate(max_items);
    out
}

/// 解析 `api/uppflow/affairCenter/queryTabItems?isCount=1`（纯函数供离线单测）。
///
/// 接口实际返回 6 个 tab（todo/done/apply/unread/read/focus），全量透传；
/// 契约前端只展示 todo/done/apply 三栏。`selected`（筛选项定义）不在契约内，忽略。
pub fn parse_todo_tabs(body: &str) -> Result<Vec<TodoTab>, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("待办分栏响应解析失败: {e}")))?;
    let d = envelope_data(&v, "待办分栏")?;
    let arr = d
        .as_array()
        .ok_or_else(|| PortalError::Parse("待办分栏响应结构异常".to_string()))?;
    Ok(arr
        .iter()
        .filter_map(|t| {
            let id = t.get("tabId").and_then(jstr).filter(|s| !s.is_empty())?;
            Some(TodoTab {
                id,
                name: t.get("tabName").and_then(jstr).unwrap_or_default(),
                desc: t.get("tabDesc").and_then(jstr).unwrap_or_default(),
                count: jnum_u32(t, "count"),
            })
        })
        .collect())
}

/// 待办条目字段宽松映射：按候选键序取第一个非空值。⚠️ 真实字段形态未实测
/// （账号无待办数据，`queryFlowItems` 返回空数组），候选键为门户系统常见命名，
/// 真机出现数据后需校准（见 `lib::TodoItem` 注释）。
fn todo_item_field(obj: &serde_json::Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| obj.get(*k).and_then(jstr).filter(|s| !s.is_empty()))
        .unwrap_or_default()
}

/// 解析 `api/uppflow/process/queryFlowItems`（纯函数供离线单测）。
///
/// 信封 `data` 内层又是 `data:[]` 条目数组（与列表页同构）；id 无法映射出的
/// 条目跳过（前端列表需要稳定 key）。
pub fn parse_todo_list(body: &str) -> Result<TodoPage, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("待办列表响应解析失败: {e}")))?;
    let d = envelope_data(&v, "待办列表")?;
    let empty = Vec::new();
    let list = d.get("data").and_then(|x| x.as_array()).unwrap_or(&empty);
    let items = list
        .iter()
        .filter_map(|it| {
            let id = todo_item_field(it, &["processId", "itemId", "id"]);
            if id.is_empty() {
                return None;
            }
            Some(TodoItem {
                id,
                title: todo_item_field(it, &["title", "processName", "itemName", "workName"]),
                applicant: todo_item_field(
                    it,
                    &["applicant", "applyUserName", "creatorName", "senderName"],
                ),
                apply_time: todo_item_field(it, &["applyTime", "createTime", "sendTime"]),
                source: todo_item_field(it, &["source", "appName", "deptName"]),
                node: todo_item_field(it, &["node", "nodeName", "currentNode", "stepName"]),
                urgency: todo_item_field(it, &["urgency", "urgencyName", "urgencyCode"]),
            })
        })
        .collect();
    Ok(TodoPage {
        page: jnum_u32(d, "pageNum"),
        page_size: jnum_u32(d, "pageSize"),
        page_count: jnum_u32(d, "pageCount"),
        total: jnum_u32(d, "total"),
        items,
    })
}

// ---------------- M2 批次 3：应用 / 日程 ----------------

/// bs-schedule 日程服务信封校验：`code == "0"`（字符串 "0"，成功样本形态）返回
/// `data` 引用（键缺失 → None，调用方按空集合处理——成功信封下「无数据」是
/// 合法形态）；失败（实测 code 为数字、说明键为 `message`）→ [`PortalError::Parse`]。
fn schedule_data<'a>(
    v: &'a serde_json::Value,
    api: &str,
) -> Result<Option<&'a serde_json::Value>, PortalError> {
    let ok = v
        .get("code")
        .and_then(jstr)
        .is_some_and(|c| c.trim() == "0");
    if !ok {
        let msg = v
            .get("msg")
            .or_else(|| v.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("服务端未返回原因");
        return Err(PortalError::Parse(format!("{api} 失败: {msg}")));
    }
    Ok(v.get("data"))
}

/// `"0"/"1"`（实测字符串形态）或 0/1 数字 → bool（其他值按 false）。
fn jbool01(v: &serde_json::Value) -> bool {
    v.as_i64()
        .map(|n| n == 1)
        .unwrap_or_else(|| v.as_str().is_some_and(|s| s.trim() == "1"))
}

/// 单个门户应用条目 → [`AppItem`]（缺 appId 的条目无稳定 key，由调用方跳过）。
fn app_item_from(it: &serde_json::Value) -> Option<AppItem> {
    let id = it.get("appId").and_then(jstr).filter(|s| !s.is_empty())?;
    let link = it.get("appLink").and_then(jstr).unwrap_or_default();
    Some(AppItem {
        id,
        name: it.get("appName").and_then(jstr).unwrap_or_default(),
        // data URL 由 client 层带会话代拉后填充，解析阶段恒 None
        icon_url: None,
        // 可达性按附录 A 实测表推导（不信任 isCas——附录 A 有标 cas 实则
        // 停自家登录页/仅 WebVPN 可达的）；表未命中回落 External
        access: classify_app_access(&link),
        link,
        is_cas: it.get("isCas").map(jbool01).unwrap_or(false),
        show_type: it.get("showType").and_then(jstr).unwrap_or_default(),
        icon_id: it.get("appIcon").and_then(jstr).filter(|s| !s.is_empty()),
    })
}

/// 解析 `api/upp/appStore/v2/queryApp`（部门分组形态：data[].{depName,appList,count}，
/// 实测 8 组 30 应用全量覆盖）。
///
/// depName 兼容字符串/null（宽松 jstr）；空 depName 的组按「未分组」保留（应用
/// 不因分组字段异常而丢失）；组内条目缺 appId 跳过。组顺序按接口原样（官方
/// 即按部门排序），不再二次排序。
pub fn parse_app_groups(body: &str) -> Result<Vec<AppGroup>, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("应用分组响应解析失败: {e}")))?;
    let d = envelope_data(&v, "应用分组")?;
    let arr = d
        .as_array()
        .ok_or_else(|| PortalError::Parse("应用分组响应结构异常".to_string()))?;
    Ok(arr
        .iter()
        .map(|g| {
            let name = g
                .get("depName")
                .and_then(jstr)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "未分组".to_string());
            let apps = g
                .get("appList")
                .and_then(|x| x.as_array())
                .map(|items| items.iter().filter_map(app_item_from).collect::<Vec<_>>())
                .unwrap_or_default();
            AppGroup {
                id: name.clone(),
                name,
                apps,
            }
        })
        .collect())
}

/// 解析 `api/upp/appStore/queryMyStore`（我的收藏/常用，data 为应用条目数组）。
pub fn parse_app_items(body: &str) -> Result<Vec<AppItem>, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("我的收藏响应解析失败: {e}")))?;
    let d = envelope_data(&v, "我的收藏")?;
    let arr = d
        .as_array()
        .ok_or_else(|| PortalError::Parse("我的收藏响应结构异常".to_string()))?;
    Ok(arr.iter().filter_map(app_item_from).collect())
}

/// 解析 `api/bs-schedule/innerPlaintext/scheduleRpcManage/findScheduleClassifyList`
/// （日程分类，实测 5 类）。缺 classifyCode 的条目跳过（过滤 key），名称/颜色
/// 缺省空串。
pub fn parse_schedule_classify(body: &str) -> Result<Vec<ScheduleClassify>, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("日程分类响应解析失败: {e}")))?;
    let d = schedule_data(&v, "日程分类")?;
    let empty = Vec::new();
    let arr = d.and_then(|x| x.as_array()).unwrap_or(&empty);
    Ok(arr
        .iter()
        .filter_map(|c| {
            let code = c
                .get("classifyCode")
                .and_then(jstr)
                .filter(|s| !s.is_empty())?;
            Some(ScheduleClassify {
                name: c.get("classifyName").and_then(jstr).unwrap_or_default(),
                code,
                color: c.get("classifyColor").and_then(jstr).unwrap_or_default(),
            })
        })
        .collect())
}

/// 解析 `api/bs-schedule/innerPlaintext/scheduleRpcManage/findScheduleBetweenTime`
/// （日程区间明细；`classify` 为分类列表，按 code 映射补全名称与颜色）。
///
/// 实测字段：`scheduleName`（标题）、`startTime/endTime`（**毫秒时间戳**）、
/// `address`（地点，可为 null）、`typeCode`（分类 code——明细无
/// `scheduleClassifyCode` 字段，`scheduleClassifyName` 实测可为 null 不可依赖，
/// 候选键宽松映射，真机数据异常时便于校准）。缺 id / 时间非数字的条目跳过
/// （无 key / 无法按时间轴展示）。
pub fn parse_schedule_events(
    body: &str,
    classify: &[ScheduleClassify],
) -> Result<Vec<ScheduleEvent>, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("日程明细响应解析失败: {e}")))?;
    let d = schedule_data(&v, "日程明细")?;
    let empty = Vec::new();
    let arr = d.and_then(|x| x.as_array()).unwrap_or(&empty);
    Ok(arr
        .iter()
        .filter_map(|it| {
            let id = it.get("id").and_then(jstr).filter(|s| !s.is_empty())?;
            let start_ms = it.get("startTime").and_then(jnum)? as u64;
            let end_ms = it.get("endTime").and_then(jnum)? as u64;
            let code = it
                .get("typeCode")
                .or_else(|| it.get("scheduleClassifyCode"))
                .and_then(jstr)
                .unwrap_or_default();
            let known = classify.iter().find(|c| c.code == code);
            Some(ScheduleEvent {
                id,
                title: it.get("scheduleName").and_then(jstr).unwrap_or_default(),
                start_ms,
                end_ms,
                place: it.get("address").and_then(jstr).unwrap_or_default(),
                classify_name: known
                    .map(|c| c.name.clone())
                    .or_else(|| {
                        it.get("scheduleClassifyName")
                            .and_then(jstr)
                            .filter(|s| !s.is_empty())
                    })
                    .unwrap_or_default(),
                color: known.map(|c| c.color.clone()).unwrap_or_default(),
                classify_code: code,
                extra: None,
            })
        })
        .collect())
}

/// 解析 `api/bs-schedule/innerPlaintext/scheduleRpcManage/getCountBetweenTime`
/// （每日日程计数，day 形如 `"2026-09-01"`）。缺 day 的条目跳过。
pub fn parse_schedule_day_counts(body: &str) -> Result<Vec<ScheduleDayCount>, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("日程计数响应解析失败: {e}")))?;
    let d = schedule_data(&v, "日程计数")?;
    let empty = Vec::new();
    let arr = d.and_then(|x| x.as_array()).unwrap_or(&empty);
    Ok(arr
        .iter()
        .filter_map(|it| {
            let day = it.get("day").and_then(jstr).filter(|s| !s.is_empty())?;
            Some(ScheduleDayCount {
                day,
                count: jnum_u32(it, "count"),
            })
        })
        .collect())
}

// ---------------- M2 遗留 A2：校级会议日程并入 ----------------

/// 教学周次 → 中文数字（1..=99；常规学期 1-19，越界留余量）。0 或 >99 → None。
///
/// 用于构造会议卡查询标题 `第<中文数字>周会议日程安排表`（DJZ 参数实测按周
/// 过滤，2026-09-18 四组对照：第二周→6 条、第一周→3 条、第九周→0 条、无周次
/// 前缀→0 条）。示例：1→一、10→十、12→十二、20→二十、21→二十一。
pub fn week_to_chinese(week: u32) -> Option<String> {
    const DIGITS: [&str; 10] = ["", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    if week == 0 || week > 99 {
        return None;
    }
    let tens = week / 10;
    let ones = (week % 10) as usize;
    let tens_part = match tens {
        0 => String::new(),
        1 => "十".to_string(),
        _ => format!("{}十", DIGITS[tens as usize]),
    };
    Some(format!("{tens_part}{}", DIGITS[ones]))
}

/// 会议条目自然语言时刻（`SJ`，实测如「下午3:00」）→ 24h `(h, m)`。
///
/// 全角冒号归一；「下午/晚上」且 h<12 → +12（「下午12:00」保持中午 12 点），
/// 「上午/凌晨/中午」保持原值。解析不出（缺冒号 / 越界 / 非数字）→ None，
/// 调用方按全天处理，**不伪造时刻**。
pub fn parse_meeting_time(sj: &str) -> Option<(u8, u8)> {
    // 跳过前导非数字（「下午3」→ 3）后取段首连续数字（「00-4」→ 0，
    // 容忍「3:00-4:30」区间形态取开始时刻）
    let leading = |s: &str| -> Option<u8> {
        let digits = s.trim_start_matches(|c: char| !c.is_ascii_digit());
        let n = digits.chars().take_while(|c| c.is_ascii_digit()).count();
        if n == 0 {
            return None;
        }
        digits[..n].parse().ok()
    };
    let s = sj.trim().replace('：', ":");
    let (h, m) = s.split_once(':')?;
    let h: u8 = leading(h.trim())?;
    let m: u8 = leading(m.trim())?;
    if h > 23 || m > 59 {
        return None;
    }
    let h = if (sj.contains("下午") || sj.contains("晚上")) && h < 12 {
        h + 12
    } else {
        h
    };
    Some((h, m))
}

/// 会议日期（`NF` 年份 + `RQ` 实测如「9月15日」）→ 本地当日
/// `[00:00:00.000, 23:59:59.999]` 毫秒区间。解析不出 → None（调用方跳过条目：
/// 无日期锚点无法落时间轴）。
pub fn parse_meeting_day_range(nf: &str, rq: &str) -> Option<(u64, u64)> {
    use chrono::TimeZone;
    let (m, d) = rq.split_once('月')?;
    let month: u32 = m.trim().parse().ok()?;
    let day: u32 = d.trim().trim_end_matches('日').trim().parse().ok()?;
    let year: i32 = nf.trim().parse().ok()?;
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let start = chrono::Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .single()?;
    let end = chrono::Local
        .from_local_datetime(&date.and_hms_opt(23, 59, 59)?)
        .single()?;
    Some((start.timestamp_millis() as u64, end.timestamp_millis() as u64 + 999))
}

/// 教学周次推算：`start_ms`（前端所取周/月区间的起点毫秒）落在第几教学周。
/// `start_date` 为学期开学日 `"YYYYMMDD"`（开学当周 = 第 1 周）；开学前 → None。
pub fn teaching_week_of(start_ms: u64, start_date: &str) -> Option<u32> {
    use chrono::TimeZone;
    let y: i32 = start_date.get(0..4)?.parse().ok()?;
    let m: u32 = start_date.get(4..6)?.parse().ok()?;
    let d: u32 = start_date.get(6..8)?.parse().ok()?;
    let semester_start = chrono::NaiveDate::from_ymd_opt(y, m, d)?;
    let local = chrono::Local
        .timestamp_millis_opt(start_ms as i64)
        .single()?;
    let diff = (local.date_naive() - semester_start).num_days();
    (diff >= 0).then_some(diff as u32 / 7 + 1)
}

/// 解析会议卡接口 `api/uppexcard/ext/dynamicData/<内部主机>/ZCHY?DJZ=<周次会议
/// 日程标题>`（门户信封；条目字段 HYMC/ZCR/CBDW/DD/RQ/SJ/ZJ/CXRY/NF/ZC，
/// 2026-09-18 实测 6 条/页）。
///
/// 映射为 [`ScheduleEvent`]：title=HYMC、place=DD、classifyCode 固定
/// `Default-Meeting`（名称/颜色由 `classify` 列表映射）；日期由 NF+RQ 推出，
/// 时间由 SJ 转 24h——SJ 解析不出按**全天**（00:00-23:59），解析出则
/// endMs=startMs（服务端只给开始时刻，不伪造结束时间，前端对等值显示单时刻）。
/// NF/RQ 缺失或推不出日期的条目跳过。ZCR/CXRY/CBDW 非空段拼进 `extra`。
/// id 用序号合成 `meeting-<i>`（服务端条目无稳定 id，仅当次渲染 key 用）。
pub fn parse_meeting_events(
    body: &str,
    classify: &[ScheduleClassify],
) -> Result<Vec<ScheduleEvent>, PortalError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| PortalError::Parse(format!("会议日程响应解析失败: {e}")))?;
    let d = envelope_data(&v, "会议日程")?;
    let empty = Vec::new();
    let arr = d.as_array().unwrap_or(&empty);
    let known = classify.iter().find(|c| c.code == "Default-Meeting");
    let mut out = Vec::new();
    for (i, it) in arr.iter().enumerate() {
        let Some(title) = it.get("HYMC").and_then(jstr).filter(|s| !s.is_empty()) else {
            continue;
        };
        let (Some(nf), Some(rq)) = (
            it.get("NF").and_then(jstr),
            it.get("RQ").and_then(jstr),
        ) else {
            continue;
        };
        let Some((day_start, day_end)) = parse_meeting_day_range(&nf, &rq) else {
            continue;
        };
        let (start_ms, end_ms) =
            match it.get("SJ").and_then(jstr).as_deref().and_then(parse_meeting_time) {
                Some((h, m)) => {
                    let start = day_start + (u64::from(h) * 3600 + u64::from(m) * 60) * 1000;
                    (start, start)
                }
                None => (day_start, day_end),
            };
        let extra: Vec<String> = [
            ("主持人", it.get("ZCR").and_then(jstr)),
            ("参会人员", it.get("CXRY").and_then(jstr)),
            ("承办单位", it.get("CBDW").and_then(jstr)),
        ]
        .into_iter()
        .filter_map(|(label, v)| {
            v.filter(|s| !s.is_empty()).map(|s| format!("{label}：{s}"))
        })
        .collect();
        out.push(ScheduleEvent {
            id: format!("meeting-{i}"),
            title,
            start_ms,
            end_ms,
            place: it.get("DD").and_then(jstr).unwrap_or_default(),
            classify_code: "Default-Meeting".to_string(),
            classify_name: known
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "会议".to_string()),
            color: known.map(|c| c.color.clone()).unwrap_or_default(),
            extra: (!extra.is_empty()).then(|| extra.join(" · ")),
        });
    }
    Ok(out)
}

/// 会议卡查询标题（`DJZ` 参数）：`第<中文数字>周会议日程安排表`。
///
/// 实测该参数**按周过滤**（2026-09-18 四组对照：第二周→6 条、第一周→3 条、
/// 第九周→0 条、无周次前缀→0 条），周次取教学周（`teaching_week_of` 推算）；
/// 0 / >99（`week_to_chinese` 表达不了）→ None（调用方按拉取失败降级）。
pub fn meeting_query_title(week: u32) -> Option<String> {
    Some(format!("第{}周会议日程安排表", week_to_chinese(week)?))
}

/// 图标字节 → MIME 猜测（魔数判断，不信任响应 Content-Type——文档库静态资源
/// 常给 `application/octet-stream`）。识别不出（如 HTML 错误页/登录跳转页）
/// 返回 None，调用方按「无图标」降级，避免把错误页伪装成 data URL。
pub fn guess_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"<?xml") || bytes.starts_with(b"<svg") {
        Some("image/svg+xml")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- fixture（全部脱敏：占位学号/姓名/课程，无真实数据） ----------

    /// 学期信息实测形态（学期字段非敏感，与计划 §1.2 同构）。
    const SEMESTER_FIXTURE: &str = r#"{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":{"grade":"2026","semester":"1","currentWeek":"2","weekCount":"19","startDate":"20260907","endDate":"20270117","currentDate":"2026-09-18","currentWeekDay":"星期五"}}"#;

    /// 钱包卡实测形态：`data.data` 为内嵌 JSON 字符串数组；`loginUrl` 用
    /// TEST-PLACEHOLDER 占位（解析后必须被丢弃，结构体无该字段）。
    const WALLET_FIXTURE: &str = r#"{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":{"contentDisplay":"（略）","data":"[{\"ZJ\":\"暂无\",\"ZHYE\":102.51,\"JYE\":0.45,\"YE\":102.51,\"SSYE\":102.51,\"SL\":8,\"mailNewCount\":\"1\",\"mailFolderNewCount\":\"0\",\"totalCount\":\"1\",\"pageTotalCount\":1,\"loginUrl\":\"https://mail.example.edu/redirect?k=TEST-PLACEHOLDER\"}]"}}"#;

    /// 7×10 周课表（形状与实测同构：周五列 1,2（大节1）与列 7,8（大节4）有课、
    /// 周末全空；内容全脱敏）。
    fn fixture_grid() -> Vec<Vec<String>> {
        let mut g = vec![vec![String::new(); 10]; 7];
        let put = |g: &mut Vec<Vec<String>>, day: usize, slots: &[usize], name: &str| {
            for &s in slots {
                g[day][s] = format!("{name},教A-101,2023级1班,李四");
            }
        };
        put(&mut g, 0, &[2, 3, 4, 5], "高等数学");
        put(&mut g, 1, &[4, 5], "大学英语");
        put(&mut g, 2, &[2, 3], "数据结构");
        put(&mut g, 3, &[4, 5], "大学英语");
        put(&mut g, 4, &[0, 1, 6, 7], "高等数学");
        g
    }

    fn week_schedule_fixture(weekday: u8) -> String {
        let grid: Vec<Vec<String>> = fixture_grid();
        let rows: Vec<String> = grid
            .iter()
            .map(|row| {
                let cells: Vec<String> = row
                    .iter()
                    .map(|c| serde_json::to_string(c).unwrap())
                    .collect();
                format!("[{}]", cells.join(","))
            })
            .collect();
        format!(
            r#"{{"meta":{{"success":true,"statusCode":200,"message":"ok"}},"data":{{"resultsJsonArr":[{}],"zs":"2","xn":"2026","xq":"1","xqj":{weekday},"swskjc":4,"xwskjc":4,"wsskjc":2,"weekcount":"19"}}}}"#,
            rows.join(",")
        )
    }

    // ---------- parse_semester_info ----------

    #[test]
    fn semester_info_parses_all_fields() {
        let info = parse_semester_info(SEMESTER_FIXTURE).unwrap();
        assert_eq!(info.grade, "2026");
        assert_eq!(info.semester, "1");
        assert_eq!(info.current_week, "2");
        assert_eq!(info.week_count, "19");
        assert_eq!(info.start_date, "20260907");
        assert_eq!(info.end_date, "20270117");
        assert_eq!(info.current_week_day, "星期五");
    }

    #[test]
    fn semester_info_missing_field_is_error() {
        let body = r#"{"meta":{"success":true},"data":{"grade":"2026"}}"#;
        assert!(parse_semester_info(body).is_err());
        assert!(parse_semester_info("not json").is_err());
        let bad_meta = r#"{"meta":{"success":false,"message":"请重新登录"},"data":{}}"#;
        let err = parse_semester_info(bad_meta).unwrap_err();
        assert!(err.to_string().contains("请重新登录"));
    }

    // ---------- parse_wallet_summary ----------

    #[test]
    fn wallet_summary_parses_first_item_and_tolerant_types() {
        let w = parse_wallet_summary(WALLET_FIXTURE).unwrap();
        assert_eq!(w.card_balance, Some(102.51));
        assert_eq!(w.book_borrowed, Some(8));
        // mailNewCount 实测为字符串 "1"
        assert_eq!(w.mail_unread, Some(1));
    }

    #[test]
    fn wallet_summary_partial_success_on_missing_fields() {
        // YE/SL 缺失 → 对应项 None，mailNewCount 正常（部分成功不整体报错）
        let body = r#"{"meta":{"success":true},"data":{"data":"[{\"mailNewCount\":\"2\"}]"}}"#;
        let w = parse_wallet_summary(body).unwrap();
        assert_eq!(w.card_balance, None);
        assert_eq!(w.book_borrowed, None);
        assert_eq!(w.mail_unread, Some(2));
    }

    #[test]
    fn wallet_summary_error_paths() {
        // data.data 非字符串
        assert!(parse_wallet_summary(r#"{"meta":{"success":true},"data":{"data":123}}"#).is_err());
        // 内层非数组 / 空数组
        assert!(parse_wallet_summary(r#"{"meta":{"success":true},"data":{"data":"{}"}}"#).is_err());
        let empty =
            parse_wallet_summary(r#"{"meta":{"success":true},"data":{"data":"[]"}}"#).unwrap_err();
        assert!(empty.to_string().contains("无数据"));
        // 信封失败 / 非法 JSON
        assert!(parse_wallet_summary(r#"{"meta":{"success":false}}"#).is_err());
        assert!(parse_wallet_summary("not json").is_err());
    }

    // ---------- parse_week_schedule ----------

    #[test]
    fn week_schedule_parses_grid_and_weekday_number() {
        let ws = parse_week_schedule(&week_schedule_fixture(5)).unwrap();
        assert_eq!(ws.weekday, 5);
        assert_eq!(ws.grid.len(), 7);
        assert_eq!(ws.grid[0].len(), 10);
        assert!(ws.grid[4][0].contains("高等数学"));
        assert!(ws.grid[5][0].is_empty());
    }

    #[test]
    fn week_schedule_tolerates_string_weekday_and_errors_on_missing_grid() {
        let body = week_schedule_fixture(5).replace("\"xqj\":5", "\"xqj\":\"5\"");
        assert_eq!(parse_week_schedule(&body).unwrap().weekday, 5);
        assert!(parse_week_schedule(r#"{"meta":{"success":true},"data":{}}"#).is_err());
    }

    // ---------- next_course / elapsed_slot_count ----------

    fn slots() -> Vec<TimeSlot> {
        block_time_slots()
    }

    /// 内置 11 小节表（契约 §17，2026-09-19 用户截图校准）：条数、号连续
    /// 1..=11、关键档位时刻（上午止/午休后/与大节 4、5 校准值同锚/全天止）。
    #[test]
    fn section_time_slots_covers_eleven_sections() {
        let s = section_time_slots();
        assert_eq!(s.len(), 11);
        for (i, slot) in s.iter().enumerate() {
            assert_eq!(slot.number as usize, i + 1, "小节号应连续 1..=11");
        }
        assert_eq!(s[0].start_time, "08:00");
        assert_eq!(s[0].end_time, "08:45");
        assert_eq!(s[3].end_time, "11:50", "上午止（小节 4）");
        assert_eq!(s[4].start_time, "13:45", "午休后（小节 5）");
        assert_eq!(s[6].start_time, "15:55", "与大节 4 校准值同锚");
        assert_eq!(s[8].start_time, "18:45", "与大节 5 校准值同锚");
        assert_eq!(s[10].end_time, "21:20", "全天止（小节 11）");
    }

    #[test]
    fn elapsed_block_count_by_time() {
        let s = slots();
        assert_eq!(elapsed_slot_count("07:00", &s), 0);
        // 大节1（08:00-09:40）进行中 → 已开始 1 大节
        assert_eq!(elapsed_slot_count("08:30", &s), 1);
        // 上午大课间（09:40-10:10）→ 大节2 未开始，仍为 1
        assert_eq!(elapsed_slot_count("10:00", &s), 1);
        // 大节3（13:45-15:25）进行中 → 3
        assert_eq!(elapsed_slot_count("14:00", &s), 3);
        // 大节4（15:55-17:35）进行中 → 4
        assert_eq!(elapsed_slot_count("16:00", &s), 4);
        // 深夜 → 全部 5 大节
        assert_eq!(elapsed_slot_count("22:00", &s), 5);
    }

    #[test]
    fn next_course_splits_cell_and_drops_fourth_segment() {
        let grid = fixture_grid();
        let c = next_course(&grid, 5, 0, &slots()).unwrap();
        assert_eq!(c.name, "高等数学");
        assert_eq!(c.room, "教A-101");
        assert_eq!(c.teaching_class, "2023级1班");
        assert_eq!(c.slot, 1);
        assert_eq!(c.start_time.as_deref(), Some("08:00"));
    }

    /// 主智能体验收缺陷回归：周五列 7,8（1-based；0-based 6,7 = 大节4）有课
    /// 「信息安全」，当前时间在上午 → 起点必须是大节4 的 15:55（此前按小节号
    /// 查默认 13 节表误报 14:50）。
    #[test]
    fn next_course_maps_column_pair_to_block_start() {
        let mut grid = vec![vec![String::new(); 10]; 7];
        grid[4][6] = "信息安全,C5科教中心313,2023级1班,李四".into();
        grid[4][7] = "信息安全,C5科教中心313,2023级1班,李四".into();
        // 上午（大节1 已开始，elapsed=1）→ 命中大节4
        let c = next_course(&grid, 5, 1, &slots()).unwrap();
        assert_eq!(c.name, "信息安全");
        assert_eq!(c.room, "C5科教中心313");
        assert_eq!(c.slot, 4);
        assert_eq!(c.start_time.as_deref(), Some("15:55"));
        // 边界：大节4 进行中（elapsed=4）→ 本周无更多课 → None
        assert!(next_course(&grid, 5, 4, &slots()).is_none());
        // 边界：全部大节已过（elapsed=5）→ None
        assert!(next_course(&grid, 5, 5, &slots()).is_none());
    }

    #[test]
    fn next_course_skips_ongoing_block_and_crosses_days() {
        let grid = fixture_grid();
        let s = slots();
        // 周五列 1,2（大节1）与 7,8（大节4）有课：大节1 进行中（elapsed=1）
        // → 当天从 col2 起 → 下一节是大节4 15:55
        let c = next_course(&grid, 5, 1, &s).unwrap();
        assert_eq!(c.slot, 4);
        assert_eq!(c.start_time.as_deref(), Some("15:55"));
        // 周四大节4 进行中（elapsed=4）→ 周四无更多课 → 周五大节1
        let c = next_course(&grid, 4, 4, &s).unwrap();
        assert_eq!(c.slot, 1);
        assert_eq!(c.start_time.as_deref(), Some("08:00"));
        // 周日（第 7 行）无课 → None
        assert!(next_course(&grid, 7, 0, &s).is_none());
        // 全空矩阵 → None
        let empty = vec![vec![String::new(); 10]; 7];
        assert!(next_course(&empty, 5, 0, &s).is_none());
        // weekday 越界防御
        assert!(next_course(&grid, 0, 0, &s).is_none());
        assert!(next_course(&grid, 8, 0, &s).is_none());
    }

    #[test]
    fn course_from_cell_partial_segments_and_unknown_block() {
        let s = slots();
        // 只有两段：教学班为空串，仍有效（col4 → 大节3，13:45 开始）
        let c = course_from_cell("体育,操场,", 4, &s).unwrap();
        assert_eq!(c.name, "体育");
        assert_eq!(c.room, "操场");
        assert_eq!(c.teaching_class, "");
        assert_eq!(c.slot, 3);
        assert_eq!(c.start_time, Some("13:45".to_string()));
        // 列号超出 5 大节（异常长行）→ 大节6 无常量 → start_time None
        let c = course_from_cell("晚课,教B-202,x,李四", 10, &s).unwrap();
        assert_eq!(c.slot, 6);
        assert_eq!(c.start_time, None);
        // 空格 / 纯逗号 → None
        assert!(course_from_cell("", 0, &s).is_none());
        assert!(course_from_cell("  ", 0, &s).is_none());
        assert!(course_from_cell(",教室,班,李四", 0, &s).is_none());
    }

    #[test]
    fn next_course_from_now_weekend_guard() {
        // 直接验证薄包装在「本地周末」场景的防御不 panic（结果取决于运行日）
        let ws = parse_week_schedule(&week_schedule_fixture(5)).unwrap();
        let _ = next_course_from_now(&ws);
    }

    // ---------- parse_info_columns（fixture 全脱敏：仅栏目 id 与通用名称） ----------

    /// 订阅接口形态：只含当前账号订阅的栏目（fixture 取 2 个 + 1 个
    /// titleLocale 损坏的条目验证常量名兜底）。
    const COLUMNS_FIXTURE: &str = r#"{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":[
        {"columnId":"f382fddd843b4058a486a9375ecf422d","titleLocale":"{\"zh_CN\":\"校园要闻\",\"en_US\":\"Campus News\"}","sortNum":2,"columnType":1,"isRemind":1},
        {"columnId":"9","titleLocale":"{\"zh_CN\":\"通知公告\"}","sortNum":1,"columnType":1,"isRemind":0},
        {"columnId":"a8bc1e5a9225475b9841b5a237c690df","titleLocale":"broken","sortNum":3,"columnType":1,"isRemind":0}
    ]}"#;

    #[test]
    fn info_columns_merge_subscribed_with_known_fallback() {
        let cols = parse_info_columns(COLUMNS_FIXTURE).unwrap();
        // 订阅 3 个 + 未订阅 4 个 = 实测全量 7 栏
        assert_eq!(cols.len(), 7);
        // 订阅项按 sortNum 升序在前；titleLocale 损坏的栏目回落常量名
        assert_eq!(cols[0].id, "9");
        assert_eq!(cols[0].name, "通知公告");
        assert_eq!(cols[0].sort_num, 1);
        assert_eq!(cols[1].name, "校园要闻");
        assert_eq!(cols[2].name, "校园快讯"); // 常量名兜底
        assert_eq!(cols[2].sort_num, 3);
        // 未订阅项按实测全量顺序垫底（sortNum 1000+ 保序占位）
        assert_eq!(cols[3].id, "ea0a5b2158bf48b3afeb026477c626e4");
        assert_eq!(cols[3].name, "教务处");
        assert_eq!(cols[6].name, "团委");
        // KNOWN_COLUMNS 前 3 项已被订阅，团委在全量表中排第 7（索引 6）
        assert_eq!(cols[6].sort_num, 1006);
    }

    #[test]
    fn info_columns_error_paths() {
        // data 非数组 / 信封失败 / 非法 JSON
        assert!(parse_info_columns(r#"{"meta":{"success":true},"data":{}}"#).is_err());
        assert!(parse_info_columns(r#"{"meta":{"success":false}}"#).is_err());
        assert!(parse_info_columns("not json").is_err());
        // 缺 columnId 的条目被跳过（不报错）
        let cols = parse_info_columns(r#"{"meta":{"success":true},"data":[{"titleLocale":"{}"}]}"#)
            .unwrap();
        assert_eq!(cols.len(), 7); // 仅剩常量兜底
        assert_eq!(cols[0].name, "通知公告");
    }

    // ---------- parse_info_list ----------

    /// 列表形态复刻（字段与计划 §1.2 同构，内容全脱敏；total/pageCount=0
    /// 复刻「pageSize=1 时分页字段不可靠」的实测形态）。
    const INFO_LIST_FIXTURE: &str = r#"{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":{
        "pageNum":1,"pageSize":10,"total":0,"pageCount":0,
        "list":[
            {"infoId":"9001","infoTitle":"示例通知标题一","extLink":"https://www.cwxu.edu.cn/content.jsp?urltype=news.NewsContentUrl&wbtreeid=1039&wbnewsid=9001","publishTime":"2026-09-01 10:00:00","columnTitle":"通知公告","publishDeptName":"示例部门","hitCount":123,"detailType":"link","top":0},
            {"infoId":"9002","infoTitle":"示例通知标题二","extLink":"https://jwc.cwxu.edu.cn/info/1100/9002.htm","publishTime":"2026-09-02 11:00:00","columnTitle":"教务处","publishDeptName":null,"hitCount":null,"detailType":"link"},
            {"infoId":"9003","infoTitle":"缺正文链接的条目"}
        ]}}"#;

    #[test]
    fn info_list_maps_items_and_passes_unreliable_paging_through() {
        let p = parse_info_list(INFO_LIST_FIXTURE).unwrap();
        // total/pageCount 实测不可靠，原样透传（前端以 items.length 判断分页）
        assert_eq!((p.page, p.page_size, p.page_count, p.total), (1, 10, 0, 0));
        // 缺 extLink 的条目被跳过
        assert_eq!(p.items.len(), 2);
        let first = &p.items[0];
        assert_eq!(first.id, "9001");
        assert_eq!(first.title, "示例通知标题一");
        assert_eq!(first.column_title, "通知公告");
        assert_eq!(first.publish_time, "2026-09-01 10:00:00");
        assert_eq!(first.dept.as_deref(), Some("示例部门"));
        assert_eq!(
            first.url,
            "https://www.cwxu.edu.cn/content.jsp?urltype=news.NewsContentUrl&wbtreeid=1039&wbnewsid=9001"
        );
        // publishDeptName null → dept None
        assert_eq!(p.items[1].dept, None);
    }

    #[test]
    fn info_list_empty_and_error_paths() {
        // 空列表（当前账号无订阅栏目数据时的实测形态）
        let empty = parse_info_list(
            r#"{"meta":{"success":true},"data":{"pageNum":1,"pageSize":10,"total":0,"pageCount":0,"list":[]}}"#,
        )
        .unwrap();
        assert!(empty.items.is_empty());
        // list 缺失 → 空列表不报错；信封失败 / 非法 JSON → Err
        assert!(parse_info_list(r#"{"meta":{"success":true},"data":{}}"#)
            .unwrap()
            .items
            .is_empty());
        assert!(parse_info_list(r#"{"meta":{"success":false}}"#).is_err());
        assert!(parse_info_list("not json").is_err());
    }

    // ---------- 调课通知自动发现（批 A：关键词检测 + 收集/去重/倒序/截断） ----------

    #[test]
    fn notice_keywords_strong_weak_and_miss() {
        // 强命中
        assert_eq!(notice_keyword_hits("关于第5周周一调课的通知"), vec!["调课"]);
        assert_eq!(notice_keyword_hits("某课程停课通知"), vec!["停课"]);
        // 弱命中（无强词也纳入）
        assert_eq!(notice_keyword_hits("关于课程调整的说明"), vec!["课程调整"]);
        // 强弱同时命中：强在前；多命中不重复
        assert_eq!(
            notice_keyword_hits("调课安排与课程调整说明"),
            vec!["调课", "课程调整"]
        );
        assert_eq!(notice_keyword_hits("停课补课通知"), vec!["停课", "补课"]);
        // 无命中
        assert!(notice_keyword_hits("关于运动会场地安排的通知").is_empty());
        assert!(notice_keyword_hits("").is_empty());
    }

    #[test]
    fn collect_schedule_notices_filters_dedups_sorts_and_truncates() {
        let mk_item = |id: &str, title: &str, time: &str, url: &str, col: &str| InfoItem {
            id: id.into(),
            title: title.into(),
            column_title: col.into(),
            publish_time: time.into(),
            dept: None,
            url: url.into(),
        };
        let page = |items: Vec<InfoItem>| InfoPage {
            page: 1,
            page_size: 50,
            page_count: 0,
            total: 0,
            items,
        };
        let columns = [("9", "通知公告"), ("ea0a5b2158bf48b3afeb026477c626e4", "教务处")];
        let pages = [
            page(vec![
                mk_item("1", "示例运动会通知", "2026-09-01 10:00:00", "u1", "通知公告"),
                mk_item("2", "关于《信息安全》调课的通知", "2026-09-03 09:00:00", "u2", "通知公告"),
                mk_item("3", "期中考试安排", "2026-09-05 08:00:00", "u3", "通知公告"),
            ]),
            page(vec![
                mk_item("4", "某课程补课通知", "2026-09-02 14:00:00", "u4", "教务处"),
                // 同 url 跨栏目转载 → 去重（后拉到的丢弃）
                mk_item("5", "关于《信息安全》调课的通知（转载）", "2026-09-04 09:00:00", "u2", "教务处"),
                // columnTitle 缺失 → 回落扫描常量名
                mk_item("6", "课程调整说明", "2026-09-06 09:00:00", "u6", ""),
            ]),
        ];
        let out = collect_schedule_notices(&columns, &pages, 20);
        // 无命中 2 条被滤、u2 跨栏目转载去重（保留先出现的原条目）→ 3 条；按日期倒序
        let dates: Vec<&str> = out.iter().map(|b| b.date.as_str()).collect();
        assert_eq!(
            dates,
            ["2026-09-06 09:00:00", "2026-09-03 09:00:00", "2026-09-02 14:00:00"]
        );
        assert_eq!(out[0].column, "教务处", "columnTitle 空 → 回落扫描常量名");
        assert_eq!(out[1].column, "通知公告", "转载去重保留先出现的原条目");
        assert_eq!(out[2].matched_keywords, vec!["补课".to_string()]);
        // 上限截断（倒序后取前 N）
        assert_eq!(collect_schedule_notices(&columns, &pages, 2).len(), 2);
    }

    // ---------- parse_todo_tabs ----------

    /// 分栏形态（与实测同构：tabId/tabName/tabDesc/count/selected；值脱敏）。
    const TODO_TABS_FIXTURE: &str = r#"{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":[
        {"tabId":"todo","tabName":"我的待办","tabDesc":"待办的事务","count":2,"selected":[{"fieldId":"f1","fieldCode":"urgency","fieldName":"紧急程度"}]},
        {"tabId":"done","tabName":"我的已办","tabDesc":"已完成的事务","count":0,"selected":[]},
        {"tabId":"apply","tabName":"我的申请","tabDesc":"申请的事务","count":0,"selected":[]}
    ]}"#;

    #[test]
    fn todo_tabs_map_id_name_desc_count() {
        let tabs = parse_todo_tabs(TODO_TABS_FIXTURE).unwrap();
        assert_eq!(tabs.len(), 3);
        assert_eq!(tabs[0].id, "todo");
        assert_eq!(tabs[0].name, "我的待办");
        assert_eq!(tabs[0].desc, "待办的事务");
        assert_eq!(tabs[0].count, 2);
        assert_eq!(tabs[1].count, 0);
    }

    #[test]
    fn todo_tabs_error_paths() {
        assert!(parse_todo_tabs(r#"{"meta":{"success":true},"data":{}}"#).is_err());
        assert!(parse_todo_tabs(r#"{"meta":{"success":false}}"#).is_err());
        assert!(parse_todo_tabs("not json").is_err());
    }

    // ---------- parse_todo_list ----------

    /// 列表形态：信封 data 内层 `data:[]`；两条分别用主候选键与备选候选键
    /// （真实条目字段未实测，见 todo_item_field 注释；内容全脱敏占位）。
    const TODO_LIST_FIXTURE: &str = r#"{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":{
        "pageNum":1,"pageSize":10,"pageCount":0,"total":0,
        "data":[
            {"processId":"p1","processName":"示例申请事项","applicant":"张三","applyTime":"2026-09-10 09:00:00","appName":"示例应用","nodeName":"学院审批","urgency":"一般"},
            {"id":"p2","title":"备选键名条目","senderName":"李四","createTime":"2026-09-11 10:00:00","deptName":"示例部门","stepName":"待审核","urgencyCode":"0"},
            {"title":"缺 id 的条目"}
        ]}}"#;

    #[test]
    fn todo_list_maps_candidate_keys_and_skips_idless() {
        let p = parse_todo_list(TODO_LIST_FIXTURE).unwrap();
        assert_eq!((p.page, p.page_size), (1, 10));
        // 缺 id 的条目跳过
        assert_eq!(p.items.len(), 2);
        let a = &p.items[0];
        assert_eq!(a.id, "p1");
        assert_eq!(a.title, "示例申请事项");
        assert_eq!(a.applicant, "张三");
        assert_eq!(a.apply_time, "2026-09-10 09:00:00");
        assert_eq!(a.source, "示例应用");
        assert_eq!(a.node, "学院审批");
        assert_eq!(a.urgency, "一般");
        let b = &p.items[1];
        // 备选候选键（id/title/senderName/createTime/deptName/stepName/urgencyCode）
        assert_eq!(b.id, "p2");
        assert_eq!(b.title, "备选键名条目");
        assert_eq!(b.applicant, "李四");
        assert_eq!(b.source, "示例部门");
        assert_eq!(b.node, "待审核");
        assert_eq!(b.urgency, "0");
    }

    #[test]
    fn todo_list_empty_and_error_paths() {
        // 当前账号实测形态：三栏均空数组
        let empty = parse_todo_list(
            r#"{"meta":{"success":true},"data":{"pageNum":1,"pageSize":10,"pageCount":0,"total":0,"data":[]}}"#,
        )
        .unwrap();
        assert!(empty.items.is_empty());
        // data.data 缺失 → 空列表不报错；信封失败 / 非法 JSON → Err
        assert!(parse_todo_list(r#"{"meta":{"success":true},"data":{}}"#)
            .unwrap()
            .items
            .is_empty());
        assert!(parse_todo_list(r#"{"meta":{"success":false}}"#).is_err());
        assert!(parse_todo_list("not json").is_err());
    }

    // ---------- parse_app_groups / parse_app_items（fixture 全脱敏） ----------

    /// v2 分组形态：data[].{depName,appList,count}；字段值形态复刻实测
    ///（isCas/showType 为字符串 "0"/"1"，orderId 数字与字符串两种都见）。
    /// 内容占位：部门=示例部门，应用=示例应用。
    const APP_GROUPS_FIXTURE: &str = r#"{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":[
        {"depName":"示例部门一","count":2,"appList":[
            {"appId":"app-001","appName":"示例应用一","appIcon":"00000000-0000-0000-0000-000000000001","appLink":"https://app1.cwxu.edu.cn/","isCas":"1","showType":"1","orderId":1,"isPersonal":"0","isRecommend":"1"},
            {"appId":"app-002","appName":"示例应用二","appIcon":"","appLink":"https://app2.cwxu.edu.cn/","isCas":"0","showType":"2","orderId":"2","isPersonal":"0","isRecommend":"0"},
            {"appName":"缺 id 的条目","appLink":"https://x.cwxu.edu.cn/"}
        ]},
        {"depName":null,"count":1,"appList":[
            {"appId":"app-003","appName":"示例应用三","appIcon":"00000000-0000-0000-0000-000000000003","appLink":"https://app3.cwxu.edu.cn/","isCas":0,"showType":"1","orderId":3,"isPersonal":"0","isRecommend":"0"}
        ]}
    ]}"#;

    #[test]
    fn app_groups_maps_dep_name_and_tolerant_field_types() {
        let groups = parse_app_groups(APP_GROUPS_FIXTURE).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "示例部门一");
        // 缺 appId 的条目跳过
        assert_eq!(groups[0].apps.len(), 2);
        let a = &groups[0].apps[0];
        assert_eq!(a.id, "app-001");
        assert_eq!(a.name, "示例应用一");
        assert!(a.is_cas);
        assert_eq!(a.icon_url, None); // data URL 由 client 层填充
        assert_eq!(
            a.icon_id.as_deref(),
            Some("00000000-0000-0000-0000-000000000001")
        );
        // orderId 字符串形态不影响其余字段；appIcon 空串 → icon_id None
        let b = &groups[0].apps[1];
        assert_eq!(b.icon_id, None);
        assert!(!b.is_cas);
        // depName null → 「未分组」兜底；isCas 数字 0 → false
        assert_eq!(groups[1].name, "未分组");
        assert!(!groups[1].apps[0].is_cas);
    }

    #[test]
    fn app_groups_error_paths() {
        assert!(parse_app_groups(r#"{"meta":{"success":true},"data":{}}"#).is_err());
        assert!(parse_app_groups(r#"{"meta":{"success":false}}"#).is_err());
        assert!(parse_app_groups("not json").is_err());
        // appList 缺失 → 空组保留
        let g = parse_app_groups(r#"{"meta":{"success":true},"data":[{"depName":"示例部门"}]}"#)
            .unwrap();
        assert_eq!(g.len(), 1);
        assert!(g[0].apps.is_empty());
    }

    /// queryMyStore 形态：data 为应用数组（收藏/常用）；isCas 用数字 1 形态覆盖。
    const APP_ITEMS_FIXTURE: &str = r#"{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":[
        {"appId":"app-001","appName":"示例应用一","appIcon":"00000000-0000-0000-0000-000000000001","appLink":"https://app1.cwxu.edu.cn/","isCas":1,"showType":"1","orderId":1},
        {"appId":"app-009","appName":"示例个人应用","appIcon":"00000000-0000-0000-0000-000000000009","appLink":"http://10.0.0.1/app","isCas":"0","showType":"2","orderId":9}
    ]}"#;

    #[test]
    fn app_items_maps_mystore_entries() {
        let items = parse_app_items(APP_ITEMS_FIXTURE).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[0].is_cas); // 数字 1 形态
        assert_eq!(items[1].name, "示例个人应用");
        assert!(!items[1].is_cas);
        assert!(parse_app_items(r#"{"meta":{"success":true},"data":[]}"#)
            .unwrap()
            .is_empty());
    }

    // ---------- 日程服务（bs-schedule 信封 code=="0"） ----------

    /// 分类形态：实测 5 类（code 固定为 Default-*，名称为通用分类词，非个人数据）。
    ///（raw string 用 ## 定界——内容含 `"#ff9ee1"` 色值，单个 # 会被提前终止）
    const SCHEDULE_CLASSIFY_FIXTURE: &str = r##"{"code":"0","msg":"ok","data":[
        {"classifyName":"个人日程","classifyCode":"Default-person","classifyColor":"#ff9ee1"},
        {"classifyName":"活动","classifyCode":"Default-Activity","classifyColor":"#95ec93"},
        {"classifyName":"会议","classifyCode":"Default-Meeting","classifyColor":"#62b7fd"},
        {"classifyName":"值班","classifyCode":"Default-duty","classifyColor":"#ffb37c"},
        {"classifyName":"课程","classifyCode":"Default-class","classifyColor":"#c0a1fd"}
    ]}"##;

    #[test]
    fn schedule_classify_maps_code_name_color() {
        let cs = parse_schedule_classify(SCHEDULE_CLASSIFY_FIXTURE).unwrap();
        assert_eq!(cs.len(), 5);
        assert_eq!(cs[0].code, "Default-person");
        assert_eq!(cs[0].color, "#ff9ee1");
        // 失败信封：code 为数字（没带头时的实测形态），说明键为 message
        let err = parse_schedule_classify(r#"{"code":500,"message":"系统错误","data":null}"#)
            .unwrap_err();
        assert!(err.to_string().contains("系统错误"));
        assert!(parse_schedule_classify(r#"{"code":"1","msg":"fail"}"#).is_err());
        assert!(parse_schedule_classify("not json").is_err());
    }

    /// 明细形态复刻：typeCode 存分类 code；scheduleClassifyName/address 可为
    /// null；startTime/endTime 为毫秒数字；一条缺时间戳的坏条目。
    const SCHEDULE_EVENTS_FIXTURE: &str = r#"{"code":"0","msg":"ok","data":[
        {"id":"evt-001","scheduleName":"示例会议一","typeCode":"Default-Meeting","scheduleClassifyName":null,"address":"示例楼 101","startTime":1788192000000,"endTime":1788195600000,"publishStatus":"1","status":"1"},
        {"id":"evt-002","scheduleName":"示例课程","typeCode":"Default-class","scheduleClassifyName":null,"address":null,"startTime":1788747000000,"endTime":1788753000000},
        {"id":"evt-003","scheduleName":"无时间的坏条目","typeCode":"Default-person","startTime":null,"endTime":null},
        {"id":"evt-004","scheduleName":"未知分类条目","typeCode":"Default-other","scheduleClassifyName":"自定义分类","address":"示例地点","startTime":1788280000000,"endTime":1788283600000}
    ]}"#;

    #[test]
    fn schedule_events_map_classify_and_skip_timeless() {
        let cs = parse_schedule_classify(SCHEDULE_CLASSIFY_FIXTURE).unwrap();
        let evs = parse_schedule_events(SCHEDULE_EVENTS_FIXTURE, &cs).unwrap();
        // 缺时间戳的条目跳过
        assert_eq!(evs.len(), 3);
        let a = &evs[0];
        assert_eq!(a.id, "evt-001");
        assert_eq!(a.title, "示例会议一");
        assert_eq!(a.start_ms, 1788192000000);
        assert_eq!(a.end_ms, 1788195600000);
        assert_eq!(a.place, "示例楼 101");
        assert_eq!(a.classify_code, "Default-Meeting");
        assert_eq!(a.classify_name, "会议"); // 由分类列表映射（明细里该字段为 null）
        assert_eq!(a.color, "#62b7fd");
        // address null → 空串
        assert_eq!(evs[1].place, "");
        assert_eq!(evs[1].classify_name, "课程");
        // 未知分类：code 原样保留，名称回落明细 scheduleClassifyName，颜色空串
        let c = &evs[2];
        assert_eq!(c.classify_code, "Default-other");
        assert_eq!(c.classify_name, "自定义分类");
        assert_eq!(c.color, "");
        // 空区间 / data 缺失 → 空列表不报错
        assert!(
            parse_schedule_events(r#"{"code":"0","msg":"ok","data":[]}"#, &cs)
                .unwrap()
                .is_empty()
        );
        assert!(parse_schedule_events(r#"{"code":"0","msg":"ok"}"#, &cs)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn schedule_day_counts_maps_day_and_count() {
        let body = r#"{"code":"0","msg":"ok","data":[{"day":"2026-09-01","count":0},{"day":"2026-09-02","count":3},{"count":9}]}"#;
        let dc = parse_schedule_day_counts(body).unwrap();
        // 缺 day 的条目跳过
        assert_eq!(dc.len(), 2);
        assert_eq!(dc[0].day, "2026-09-01");
        assert_eq!(dc[0].count, 0);
        assert_eq!(dc[1].count, 3);
        assert!(parse_schedule_day_counts(r#"{"code":"0","msg":"ok"}"#)
            .unwrap()
            .is_empty());
        assert!(parse_schedule_day_counts(r#"{"code":500,"message":"系统错误"}"#).is_err());
    }

    // ---------- M2 遗留 A2：会议并入 ----------

    #[test]
    fn week_number_to_chinese_covers_regular_semester() {
        assert_eq!(week_to_chinese(1).as_deref(), Some("一"));
        assert_eq!(week_to_chinese(2).as_deref(), Some("二"));
        assert_eq!(week_to_chinese(9).as_deref(), Some("九"));
        assert_eq!(week_to_chinese(10).as_deref(), Some("十"));
        assert_eq!(week_to_chinese(11).as_deref(), Some("十一"));
        assert_eq!(week_to_chinese(19).as_deref(), Some("十九"));
        assert_eq!(week_to_chinese(20).as_deref(), Some("二十"));
        assert_eq!(week_to_chinese(21).as_deref(), Some("二十一"));
        assert_eq!(week_to_chinese(99).as_deref(), Some("九十九"));
        assert_eq!(week_to_chinese(0), None);
        assert_eq!(week_to_chinese(100), None);
    }

    #[test]
    fn meeting_time_parses_natural_language_to_24h() {
        // 实测形态「下午3:00」→ 15:00
        assert_eq!(parse_meeting_time("下午3:00"), Some((15, 0)));
        assert_eq!(parse_meeting_time("上午9:30"), Some((9, 30)));
        assert_eq!(parse_meeting_time("晚上7:30"), Some((19, 30)));
        // 24h 原样；全角冒号归一；「下午12:00」保持中午
        assert_eq!(parse_meeting_time("14:00"), Some((14, 0)));
        assert_eq!(parse_meeting_time("9：15"), Some((9, 15)));
        assert_eq!(parse_meeting_time("下午12:00"), Some((12, 0)));
        assert_eq!(parse_meeting_time("下午15:05"), Some((15, 5)));
        // 解析不出 → None（调用方按全天，不伪造）
        assert_eq!(parse_meeting_time("下午三点"), None);
        assert_eq!(parse_meeting_time("待定"), None);
        assert_eq!(parse_meeting_time("25:00"), None);
        assert_eq!(parse_meeting_time("12:60"), None);
        assert_eq!(parse_meeting_time(""), None);
    }

    #[test]
    fn meeting_day_range_roundtrips_local_date() {
        use chrono::{Datelike, TimeZone};
        let (s, e) = parse_meeting_day_range("2026", "9月15日").unwrap();
        assert_eq!(e - s, 86_399_999);
        let d = chrono::Local.timestamp_millis_opt(s as i64).single().unwrap();
        assert_eq!((d.year(), d.month(), d.day()), (2026, 9, 15));
        // 解析不出 → None
        assert_eq!(parse_meeting_day_range("2026", "待通知"), None);
        assert_eq!(parse_meeting_day_range("xxxx", "9月15日"), None);
        assert_eq!(parse_meeting_day_range("2026", "13月40日"), None);
    }

    #[test]
    fn teaching_week_counts_from_semester_start() {
        use chrono::{NaiveDate, TimeZone};
        let ms = |(y, m, d): (i32, u32, u32)| {
            chrono::Local
                .from_local_datetime(&NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(0, 0, 0).unwrap())
                .single()
                .unwrap()
                .timestamp_millis() as u64
        };
        // 开学日 2026-09-07（周一）为第 1 周
        assert_eq!(teaching_week_of(ms((2026, 9, 7)), "20260907"), Some(1));
        assert_eq!(teaching_week_of(ms((2026, 9, 13)), "20260907"), Some(1));
        assert_eq!(teaching_week_of(ms((2026, 9, 14)), "20260907"), Some(2));
        assert_eq!(teaching_week_of(ms((2026, 9, 20)), "20260907"), Some(2));
        // 开学前 / 坏参数
        assert_eq!(teaching_week_of(ms((2026, 9, 6)), "20260907"), None);
        assert_eq!(teaching_week_of(ms((2026, 9, 14)), "2026090"), None);
    }

    /// 会议卡 fixture（门户信封；内容全脱敏：示例会议/示例地点/张三，
    /// 不含真实会议名/人名/单位）。
    const MEETING_FIXTURE: &str = r#"{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":[
        {"HYMC":"示例会议一","ZCR":"张三","CBDW":"示例单位","DD":"示例楼 201","RQ":"9月15日","SJ":"下午3:00","ZJ":"星期二","CXRY":"示例人员","NF":"2026","ZC":"第二周会议","DJZ":"第二周会议日程安排表"},
        {"HYMC":"示例会议二","ZCR":"","CBDW":"","DD":"","RQ":"9月16日","SJ":"时间待定","ZJ":"星期三","CXRY":"","NF":"2026","ZC":"第二周会议","DJZ":"第二周会议日程安排表"},
        {"HYMC":"缺日期的坏条目","ZCR":"张三","DD":"示例楼","RQ":"","SJ":"下午3:00","NF":"2026"}
    ]}"#;

    #[test]
    fn meeting_events_map_fields_and_degrade_gracefully() {
        let cs = parse_schedule_classify(SCHEDULE_CLASSIFY_FIXTURE).unwrap();
        let evs = parse_meeting_events(MEETING_FIXTURE, &cs).unwrap();
        // 缺日期（RQ 空）的条目跳过
        assert_eq!(evs.len(), 2);
        let a = &evs[0];
        assert_eq!(a.id, "meeting-0");
        assert_eq!(a.title, "示例会议一");
        assert_eq!(a.place, "示例楼 201");
        assert_eq!(a.classify_code, "Default-Meeting");
        assert_eq!(a.classify_name, "会议");
        assert_eq!(a.color, "#62b7fd");
        // 「下午3:00」→ 当日 15:00 开始；服务端无结束时刻 → endMs=startMs
        let (ds, _de) = parse_meeting_day_range("2026", "9月15日").unwrap();
        assert_eq!(a.start_ms, ds + 15 * 3_600_000);
        assert_eq!(a.end_ms, a.start_ms);
        // 附加信息拼接（非空段）
        assert_eq!(a.extra.as_deref(), Some("主持人：张三 · 参会人员：示例人员 · 承办单位：示例单位"));
        // SJ 解析不出 → 全天（当日 00:00-23:59:59.999）；附加信息全空 → extra None
        let b = &evs[1];
        let (ds2, de2) = parse_meeting_day_range("2026", "9月16日").unwrap();
        assert_eq!(b.start_ms, ds2);
        assert_eq!(b.end_ms, de2);
        assert_eq!(b.extra, None);
        // 空数据 / 信封失败 / 非法 JSON
        assert!(parse_meeting_events(r#"{"meta":{"success":true},"data":[]}"#, &cs)
            .unwrap()
            .is_empty());
        assert!(parse_meeting_events(r#"{"meta":{"success":false}}"#, &cs).is_err());
        assert!(parse_meeting_events("not json", &cs).is_err());
    }

    /// 「不信任 isCas」：门户标 cas（"1"）但可达性表归 webvpn → 以表为准。
    #[test]
    fn access_overrides_portal_iscas() {
        let webvpn = app_item_from(&serde_json::json!({
            "appId": "app-tsg", "appName": "示例镜像库",
            "appLink": "https://tsgcnki.cwxu.edu.cn/", "isCas": "1"
        }))
        .unwrap();
        assert_eq!(webvpn.access, crate::access::AppAccess::Webvpn);
        let cas = app_item_from(&serde_json::json!({
            "appId": "app-wf", "appName": "示例文献库",
            "appLink": "https://www.wanfangdata.com.cn/", "isCas": 1
        }))
        .unwrap();
        assert_eq!(cas.access, crate::access::AppAccess::Cas);
        // 链接解析失败 → 保守默认 external
        let bad = app_item_from(&serde_json::json!({
            "appId": "app-bad", "appLink": "", "isCas": "0"
        }))
        .unwrap();
        assert_eq!(bad.access, crate::access::AppAccess::External);
    }

    /// 回归（离线钉死 DJZ 标题构造）：第 N 周 → `第<中文数字>周会议日程安排表`。
    #[test]
    fn meeting_query_title_builds_week_headline() {
        assert_eq!(
            meeting_query_title(2).as_deref(),
            Some("第二周会议日程安排表")
        );
        assert_eq!(
            meeting_query_title(1).as_deref(),
            Some("第一周会议日程安排表")
        );
        assert_eq!(
            meeting_query_title(9).as_deref(),
            Some("第九周会议日程安排表")
        );
        assert_eq!(meeting_query_title(0), None);
        assert_eq!(meeting_query_title(100), None);
    }

    /// 回归（真机「会议 0 条」排查）：NF=2026、RQ=9月15日、SJ=下午3:00 的
    /// 会议条目，其开始毫秒必须落在本地 [2026-09-14, 2026-09-21) 周区间内
    ///——否则命令层的区间过滤会把本该显示的会议误丢（表现为静默 0 条）。
    #[test]
    fn meeting_event_falls_inside_week_range() {
        use chrono::{NaiveDate, TimeZone};
        let cs = parse_schedule_classify(SCHEDULE_CLASSIFY_FIXTURE).unwrap();
        let local_ms = |(y, m, d): (i32, u32, u32)| {
            chrono::Local
                .from_local_datetime(
                    &NaiveDate::from_ymd_opt(y, m, d)
                        .unwrap()
                        .and_hms_opt(0, 0, 0)
                        .unwrap(),
                )
                .single()
                .unwrap()
                .timestamp_millis() as u64
        };
        // 本周（2026-09-18 实测周次 2）：周一 9/14 00:00 起、7 天（左闭右开）
        let week_start = local_ms((2026, 9, 14));
        let week_end = week_start + 7 * 86_400_000;
        for (rq, sj) in [("9月15日", "下午3:00"), ("9月18日", "上午10:00")] {
            let body = format!(
                r#"{{"meta":{{"success":true,"statusCode":200,"message":"ok"}},"data":[
                    {{"HYMC":"示例会议","ZCR":"张三","CBDW":"示例单位","DD":"示例楼 201","RQ":"{rq}","SJ":"{sj}","ZJ":"星期二","CXRY":"示例人员","NF":"2026","ZC":"第二周会议"}}
                ]}}"#
            );
            let evs = parse_meeting_events(&body, &cs).unwrap();
            assert_eq!(evs.len(), 1, "RQ={rq} 应解析出 1 条");
            assert!(
                evs[0].start_ms >= week_start && evs[0].start_ms < week_end,
                "RQ={rq} 的开始毫秒 {} 应落在 [{week_start}, {week_end})",
                evs[0].start_ms
            );
        }
    }

    // ---------- guess_image_mime（图标代拉的魔数判断） ----------

    #[test]
    fn image_mime_guessed_by_magic_bytes() {
        assert_eq!(
            guess_image_mime(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
            Some("image/png")
        );
        assert_eq!(
            guess_image_mime(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some("image/jpeg")
        );
        assert_eq!(guess_image_mime(b"GIF89a"), Some("image/gif"));
        let webp = b"RIFF\x00\x00\x00\x00WEBPVP8 ".to_vec();
        assert_eq!(guess_image_mime(&webp), Some("image/webp"));
        assert_eq!(guess_image_mime(b"<svg xmlns="), Some("image/svg+xml"));
        // HTML 错误页 / 空字节 / 截断的 RIFF → None（按无图标降级）
        assert_eq!(guess_image_mime(b"<!DOCTYPE html>"), None);
        assert_eq!(guess_image_mime(b""), None);
        assert_eq!(guess_image_mime(b"RIFF"), None);
    }
}
