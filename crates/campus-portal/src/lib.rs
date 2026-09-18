//! 锡院助手门户业务协议核心（无 Tauri 依赖，安卓可复用）。
//!
//! 职责：复用 [`campus_auth::cas::CasClient`] 的已登录会话（同 Cookie jar），以门户
//! 前端同款请求头组调用门户业务接口（学期 / 钱包卡 / 周课表），并把响应解析成
//! 供 IPC 层透传的 DTO。解析全部为纯函数（[`parse`]），单测用脱敏 fixture。
//!
//! 敏感纪律：网关 JWT 与邮箱 `loginUrl`（内含 authkey）只在内存中使用，
//! 不落盘、不写日志、不进文档、不返回给前端（解析结构体直接不定义该字段）。

pub mod client;
pub mod parse;

pub use client::PortalClient;
pub use parse::{
    elapsed_slot_count, next_course, next_course_from_now, parse_semester_info,
    parse_wallet_summary, parse_week_schedule, WeekSchedule,
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
