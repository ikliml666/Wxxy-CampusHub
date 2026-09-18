//! 门户响应解析纯函数（`parse_*(&str) -> Result<T>`），离线单测用脱敏 fixture。
//!
//! 两种响应信封，解析器不混用（计划 §1.1）：
//! - 门户自身服务：`{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":...}`
//!   —— 本模块三个接口全部属此信封；
//! - 日程服务 `bs-schedule/*`（`code=="0"`）：M2 批次 3 再支持。

use crate::{
    CourseBrief, InfoColumn, InfoItem, InfoPage, PortalError, SemesterInfo, TodoItem, TodoPage,
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
/// 时间锚定 2026-09-18 主智能体日程服务实测（`findScheduleBetweenTime` 的
/// `Default-class` 事件真实上课时间）：大节2 10:10-11:50、大节3 13:45-15:25 为
/// 实测值；其余档位为推算，M2.5 校本化作息时校准。**不改动
/// `campus-schedule::default_time_slots()`**（上游默认值，被金标测试钉住）。
fn block_time_slots() -> Vec<TimeSlot> {
    const RAW: [(u8, &str, &str); 5] = [
        // ponytail: 未实测——由大节2 10:10 开始反推（30 分钟大课间）；M2.5 校准
        (1, "08:00", "09:40"),
        // 2026-09-18 日程服务实测（信息隐藏与取证技术 10:10→11:50）
        (2, "10:10", "11:50"),
        // 2026-09-18 日程服务实测（信息安全等 13:45→15:25）
        (3, "13:45", "15:25"),
        // ponytail: 未实测——由大节3 结束 15:25 + 10 分钟课间推出；M2.5 校准
        (4, "15:35", "17:15"),
        // ponytail: 未实测——晚上档位占位；M2.5 校准
        (5, "18:30", "20:10"),
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
/// 查默认 13 节表会把大节4 的课标成 14:50，正确为 15:35）。
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
        // 大节4（15:35-17:15）进行中 → 4
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
    /// 「信息安全」，当前时间在上午 → 起点必须是大节4 的 15:35（此前按小节号
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
        assert_eq!(c.start_time.as_deref(), Some("15:35"));
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
        // → 当天从 col2 起 → 下一节是大节4 15:35
        let c = next_course(&grid, 5, 1, &s).unwrap();
        assert_eq!(c.slot, 4);
        assert_eq!(c.start_time.as_deref(), Some("15:35"));
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
}
