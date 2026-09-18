//! 锡院助手门户业务协议核心（无 Tauri 依赖，安卓可复用）。
//!
//! 职责：复用 [`campus_auth::cas::CasClient`] 的已登录会话（同 Cookie jar），以门户
//! 前端同款请求头组调用门户业务接口（学期 / 钱包卡 / 周课表 / 资讯 / 待办），并把
//! 响应解析成供 IPC 层透传的 DTO。解析全部为纯函数（[`parse`]），单测用脱敏
//! fixture；资讯正文抓取与清洗见 [`article`]（域名白名单 + 标签/属性白名单重建）。
//!
//! 敏感纪律：网关 JWT 与邮箱 `loginUrl`（内含 authkey）只在内存中使用，
//! 不落盘、不写日志、不进文档、不返回给前端（解析结构体直接不定义该字段）。

pub mod article;
pub mod client;
pub mod parse;

pub use article::{extract_article, is_allowed_info_url, is_auth_wall};
pub use client::PortalClient;
pub use parse::{
    elapsed_slot_count, next_course, next_course_from_now, parse_info_columns, parse_info_list,
    parse_semester_info, parse_todo_list, parse_todo_tabs, parse_wallet_summary,
    parse_week_schedule, WeekSchedule,
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
