//! 锡院助手门户业务协议核心（无 Tauri 依赖，安卓可复用）。
//!
//! 职责：复用 [`campus_auth::cas::CasClient`] 的已登录会话（同 Cookie jar），以门户
//! 前端同款请求头组调用门户业务接口（学期 / 钱包卡 / 周课表 / 资讯 / 待办 / 应用 /
//! 日程），并把响应解析成供 IPC 层透传的 DTO。解析全部为纯函数（[`parse`]），单测
//! 用脱敏 fixture；资讯正文抓取与清洗见 [`article`]（域名白名单 + 标签/属性白名单重建）。
//!
//! 敏感纪律：网关 JWT 与邮箱 `loginUrl`（内含 authkey）只在内存中使用，
//! 不落盘、不写日志、不进文档、不返回给前端（解析结构体直接不定义该字段）。

pub mod access;
pub mod article;
pub mod client;
pub mod parse;

pub use access::{classify_app_access, AppAccess};
pub use article::{
    extract_article, html_text, is_allowed_attachment_url, is_allowed_info_url, is_auth_wall,
    is_http_url,
};
pub use client::PortalClient;
pub use parse::{
    block_time_slots, collect_schedule_notices, elapsed_slot_count, guess_image_mime,
    next_course, next_course_from_now, notice_keyword_hits, parse_app_groups, parse_app_items,
    parse_info_columns, parse_info_list, parse_schedule_classify, parse_schedule_day_counts,
    parse_schedule_events, parse_semester_info, parse_todo_list, parse_todo_tabs,
    parse_wallet_summary, parse_week_schedule, section_time_slots, WeekSchedule,
};

/// campus-portal 协议层错误。
#[derive(Debug, thiserror::Error)]
pub enum PortalError {
    #[error("HTTP 请求失败: {0}")]
    Http(#[from] reqwest::Error),
    /// 会话内取不到网关 JWT / 学号（未登录或登录信息不完整），提示重新登录。
    #[error("请先登录")]
    NotLogin,
    #[error("{0}")]
    Parse(String),
}

/// 学期与当前周（`api/upp/config/querySemesterInfo`；服务端字段均为字符串）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemesterInfo {
    /// 入学年级，如 `"2026"`。
    pub grade: String,
    /// 学期序号，如 `"1"`。
    pub semester: String,
    /// 当前教学周，如 `"2"`。
    pub current_week: String,
    /// 学期总周数，如 `"19"`。
    pub week_count: String,
    /// 开学日 `"YYYYMMDD"`。
    pub start_date: String,
    /// 结束日 `"YYYYMMDD"`。
    pub end_date: String,
    /// 今天星期几的中文（如 `"星期五"`）。
    pub current_week_day: String,
}

/// 钱包三卡摘要（钱包卡 `data.data` 内嵌 JSON 数组首项，2026-09-18 实测）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletSummary {
    /// 一卡通余额（元；`YE` 字段，`ZHYE`/`SSYE` 实测同值）。
    pub card_balance: Option<f64>,
    /// 在借图书数（`SL` 字段）。
    pub book_borrowed: Option<u32>,
    /// 未读邮件数（`mailNewCount` 字段）。
    pub mail_unread: Option<u32>,
}

/// 「下一节课」简报（周课表矩阵单格拆分；格子第 4 段任课教师名按契约丢弃）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseBrief {
    pub name: String,
    pub room: String,
    pub teaching_class: String,
    /// 1-based 大节号（矩阵 10 列 = 5 大节 × 2 小节，列对映射 col/2+1；校本
    /// 大节作息表见 campus-portal::parse 的 block_time_slots）。
    pub slot: u32,
    /// 该节开始时刻 `"HH:MM"`（默认节次表查不到该节为 None）。
    pub start_time: Option<String>,
}

/// 资讯栏目（`queryUserSubscribeColumn` + 实测全量兜底，见 parse::KNOWN_COLUMNS）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoColumn {
    pub id: String,
    pub name: String,
    /// 订阅接口的 sortNum；未订阅栏目无该值（排序时按实测全量顺序垫底）。
    pub sort_num: u32,
}

/// 资讯条目（`querySimpleInfoCenter` list[]，仅取契约字段）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoItem {
    pub id: String,
    pub title: String,
    pub column_title: String,
    /// `"YYYY-MM-DD HH:MM:SS"` 原样透传。
    pub publish_time: String,
    /// 来源部门（服务端可能为 null）。
    pub dept: Option<String>,
    /// 官网正文页 URL（抓取时经 [`is_allowed_info_url`] 白名单校验）。
    pub url: String,
}

/// 资讯分页（`total`/`pageCount` 实测不可靠——pageSize=1 时返回 0——原样透传，
/// **前端分页只能以 items.length == pageSize 判断可能有下一页**）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoPage {
    pub page: u32,
    pub page_size: u32,
    pub page_count: u32,
    pub total: u32,
    pub items: Vec<InfoItem>,
}

/// 调课通知简报（重设计轮批 A，tauri 层契约 §18）：资讯栏目扫描按标题关键词
/// 命中的候选通知，**只进候选确认流，不自动改数据**（解析结果由用户确认采纳）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleNoticeBrief {
    pub title: String,
    /// 发布时间 `"YYYY-MM-DD HH:MM:SS"`（列表原样透传；倒序排序键）。
    pub date: String,
    /// 官网正文页 URL（`parse_notice_from_url` 的入参，抓取时仍过域名白名单）。
    pub url: String,
    /// 所属栏目名（服务端 `columnTitle`，缺失回落扫描常量表名）。
    pub column: String,
    /// 标题命中的关键词（强词在前）。
    pub matched_keywords: Vec<String>,
}

/// 资讯正文附件（正文容器内指向 pdf/doc/zip 等常见文件后缀的 `<a href>`，
/// 抽取规则见 `article::extract_attachments`；下载走 `download_attachment`
/// 命令，白名单 `is_allowed_attachment_url`）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoAttachment {
    /// 展示名：链接文本优先，空文本回落 URL 尾段文件名。
    pub name: String,
    /// 绝对 URL（相对地址已按详情页 base 补全）。
    pub url: String,
}

/// 资讯正文（计划 §2.1 `InfoDetail { title, html }` 的兼容扩展：正常返回
/// 清洗后的 HTML；正文被站点鉴权保护时 `needsBrowser=true` 正常返回（非错误），
/// 前端引导在浏览器中打开 `url`）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoDetail {
    pub title: String,
    /// 安全 HTML 片段（前端直接渲染，不再二次清洗）；`needsBrowser` 时为 null。
    pub html: Option<String>,
    /// true = 正文受站点鉴权开门页保护（auth.htm / 「系统提示」/ 非 2xx，
    /// 见 article::is_auth_wall），`html` 为空，前端引导浏览器打开 `url`。
    pub needs_browser: bool,
    /// 原始正文页 URL（`needsBrowser` 时用于浏览器打开；打开前仍强制域名白名单）。
    pub url: String,
    /// 正文附件列表（无附件 / `needsBrowser` 时为空数组；serde default 保持
    /// 旧数据兼容——序列化恒有键、值为数组）。
    #[serde(default)]
    pub attachments: Vec<InfoAttachment>,
}

/// 待办分栏（`queryTabItems`；接口实际返回 6 个 tab，前端按契约只展示三个）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoTab {
    pub id: String,
    pub name: String,
    pub desc: String,
    pub count: u32,
}

/// 待办条目。⚠️ 实测账号无待办数据（queryFlowItems 返回空数组），字段名无法
/// 与真实响应核对——解析按多候选键宽松映射（见 parse::todo_item_field），
/// 真机出现数据后需校准。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub applicant: String,
    pub apply_time: String,
    pub source: String,
    pub node: String,
    pub urgency: String,
}

/// 待办分页（分页字段与 [`InfoPage`] 同一实测口径：total/pageCount 不可靠）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoPage {
    pub page: u32,
    pub page_size: u32,
    pub page_count: u32,
    pub total: u32,
    pub items: Vec<TodoItem>,
}

// ---------------- M2 批次 3：应用 / 日程 ----------------

/// 门户应用条目（`v2/queryApp` 组内与 `queryMyStore` 共用同构，仅取契约字段）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppItem {
    pub id: String,
    pub name: String,
    /// 图标 data URL（后端带会话代拉，见 client::app_icon_data_url）；
    /// 拉取失败 / 无图标为 None，前端显示占位图标。
    pub icon_url: Option<String>,
    /// 服务端下发的应用链接（打开时仍经 [`is_allowed_info_url`] 域名白名单强制校验）。
    pub link: String,
    /// `isCas == "1"`（CAS 单点登录类应用；**不可全信**——可达性以此项为准）。
    pub is_cas: bool,
    pub show_type: String,
    /// 可达性分类（按 [`access::classify_app_access`] 的附录 A 实测表推导；
    /// 不信任门户 `isCas`，表未命中回落 `External`）。
    pub access: AppAccess,
    /// 图标附件 id（`appIcon` UUID），仅协议层拉取图标用；`#[serde(skip)]`
    /// 不透传 IPC（前端只需要拼好的 data URL）。
    #[serde(skip)]
    pub icon_id: Option<String>,
}

/// 应用分组（`v2/queryApp` data[].depName + appList；id 即部门名，实测 8 组互异）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppGroup {
    pub id: String,
    pub name: String,
    pub apps: Vec<AppItem>,
}

/// 应用目录（计划 §2.1 `AppCatalog { groups }` 的兼容扩展：追加 `pinned`
/// —— `queryMyStore` 的收藏/常用条目，用于钉选区；groups 为部门分组全量）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCatalog {
    pub groups: Vec<AppGroup>,
    pub pinned: Vec<AppItem>,
}

/// 日程分类（`findScheduleClassifyList`，实测 5 类，code 形如 `Default-person`）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleClassify {
    pub name: String,
    pub code: String,
    /// 服务端给的色值（如 `#ff9ee1`），前端过滤 chip 与日程块色标直接使用。
    pub color: String,
}

/// 日程条目（`findScheduleBetweenTime` data[]，仅取契约字段；startTime/endTime
/// 为毫秒时间戳）。classifyName/color 由分类列表按 code 映射补全——明细里的
/// `scheduleClassifyName` 实测可为 null，不能依赖。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleEvent {
    pub id: String,
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// 地点（`address`，服务端可能为 null → 空串）。
    pub place: String,
    pub classify_code: String,
    pub classify_name: String,
    pub color: String,
    /// 附加信息（会议条目的主持人/参会人员/承办单位拼接文本；课表等其余来源
    /// 为 None——契约的兼容扩展，前端详情卡有则展示）。
    pub extra: Option<String>,
}

/// 每日日程计数（`getCountBetweenTime`，day 形如 `"2026-09-01"`；月视图角标用，
/// 当前批次前端未消费，能力先落协议层）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleDayCount {
    pub day: String,
    pub count: u32,
}
