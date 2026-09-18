//! 门户业务客户端：复用 CAS 会话的 [`CasClient`]（同 Cookie jar），按门户前端
//! 同款请求头组调用业务接口；JWT 与用户资料按会话内存缓存（绝不落盘）。
//!
//! 缓存生命周期 = 会话生命周期：[`PortalClient`] 挂在 `CasSession` 上，登录/登出/
//! 会话替换时整体随会话丢弃，无需单独失效逻辑；会话失效导致取不到 JWT 时返回
//! [`PortalError::NotLogin`]，上层提示重新登录。

use crate::article::{extract_article, is_allowed_info_url, is_auth_wall};
use crate::parse::{
    parse_info_columns, parse_info_list, parse_semester_info, parse_todo_list, parse_todo_tabs,
    parse_wallet_summary, parse_week_schedule,
};
use crate::{
    InfoColumn, InfoDetail, InfoPage, PortalError, SemesterInfo, TodoPage, TodoTab, WalletSummary,
    WeekSchedule,
};
use campus_auth::cas::{csrf_token, CasClient, PORTAL_PROBE};
use campus_auth::CampusAuthError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 单请求超时（reqwest per-request；与门户页面 XHR 体感对齐，避免命令悬挂）。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 学期信息端点。
const EP_SEMESTER: &str = "api/upp/config/querySemesterInfo";
/// 钱包卡端点（cardId 为门户首页布局对该卡的固定分配，2026-09-18 实机捕获）。
const EP_WALLET_CARD: &str =
    "api/upp/contentDisplay/queryAppointCard/bd80fdbde8ed4f9abf9cef936bd907a8";
/// 本周课表端点（`results=` 空参数与官方前端一致）。
const EP_WEEK_SCHEDULE: &str =
    "api/uppcard/kbsz/queryAWeekSchedule?cardId=53dfb6845c3048e8bb0325672444a51a&results=";
/// 资讯栏目端点（只返回**用户订阅的**栏目，全量兜底见 parse::KNOWN_COLUMNS）。
const EP_INFO_COLUMNS: &str = "api/uppinfo/userSetting/queryUserSubscribeColumn";
/// 资讯列表端点（`total`/`pageCount` 实测不可靠，分页以 items.length 为准）。
const EP_INFO_LIST: &str = "api/uppinfo/infoCenter/querySimpleInfoCenter";
/// 待办分栏端点。
const EP_TODO_TABS: &str = "api/uppflow/affairCenter/queryTabItems?isCount=1";
/// 待办列表端点（`tabId` ∈ todo|done|apply|unread|read|focus，见 query_todo_list）。
const EP_TODO_LIST: &str = "api/uppflow/process/queryFlowItems";

/// 栏目 id 只允许 ASCII 字母数字（服务端下发为数字或十六进制串；拼接进查询串
/// 前校验，防异常输入破坏 URL 结构）。
fn is_safe_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// 门户网关鉴权头三元组（来自 `tryLoginUserInfo`）。
/// **敏感**：`token_id` 为网关 JWT，只在内存使用——不落盘、不写日志，故不实现 Debug。
#[derive(Clone)]
struct AuthHead {
    token_id: String,
    user_id: String,
    org_id: String,
}

/// 门户业务客户端。Clone 廉价（`CasClient` 与缓存均为 Arc 包装，Cookie jar 共享）。
#[derive(Clone)]
pub struct PortalClient {
    /// 已登录 CAS 会话 client（含 Cookie jar；clone 与原实例共享 jar）。
    cas: CasClient,
    /// JWT/资料缓存（std Mutex：guard 在 await 前 drop，不跨 await 持锁；
    /// 锁中毒按缓存未命中处理，不影响正确性）。
    auth: Arc<Mutex<Option<AuthHead>>>,
}

/// 当前 epoch 毫秒（系统时钟回拨时为 0，仅影响 csrf 头，服务端会拒绝并报错）。
fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// tryLoginUserInfo 错误 → 协议层错误（HTTP 原样透传，其余给可读中文）。
fn profile_err(e: CampusAuthError) -> PortalError {
    match e {
        CampusAuthError::Http(e) => PortalError::Http(e),
        other => PortalError::Parse(format!("获取登录信息失败: {other}")),
    }
}

impl PortalClient {
    pub fn new(cas: CasClient) -> Self {
        Self {
            cas,
            auth: Arc::new(Mutex::new(None)),
        }
    }

    /// 清空鉴权缓存（同步方法，锁内无 await；当前缓存随会话销毁，仅防御性提供）。
    pub fn clear_auth(&self) {
        if let Ok(mut g) = self.auth.lock() {
            *g = None;
        }
    }

    /// 取鉴权头三元组：缓存命中直接用，未命中调一次 `tryLoginUserInfo` 并入缓存。
    async fn auth_head(&self) -> Result<AuthHead, PortalError> {
        {
            let g = self.auth.lock().ok();
            if let Some(h) = g.as_ref().and_then(|g| g.as_ref()) {
                return Ok(h.clone());
            }
        } // guard 在 await 前 drop
        let profile = self.cas.portal_user_profile().await.map_err(profile_err)?;
        let head = AuthHead {
            token_id: profile.token_id.ok_or(PortalError::NotLogin)?,
            user_id: profile.user_id.ok_or(PortalError::NotLogin)?,
            org_id: profile.org_id.unwrap_or_else(|| "-1".to_string()),
        };
        if let Some(mut g) = self.auth.lock().ok() {
            *g = Some(head.clone());
        }
        Ok(head)
    }

    /// 门户同源 GET（base 复用 [`PORTAL_PROBE`]，不另设第二事实来源）。
    /// 统一注入计划 §1.1 全部请求头（JWT 无 Bearer 前缀、csrf 现算、`_t` 防缓存）。
    async fn get(&self, endpoint: &str) -> Result<String, PortalError> {
        let head = self.auth_head().await?;
        let now = now_ms();
        let sep = if endpoint.contains('?') { '&' } else { '?' };
        let url = format!(
            "{}/{}{sep}_t={now}",
            PORTAL_PROBE.trim_end_matches('/'),
            endpoint
        );
        let resp = self
            .cas
            .http_client()
            .get(url)
            .timeout(REQUEST_TIMEOUT)
            // JWT 无 Bearer 前缀；loginUserId/loginUserName 同为学号（门户前端同款）
            .header("Authorization", &head.token_id)
            .header("loginUserId", &head.user_id)
            .header("loginUserName", &head.user_id)
            .header("loginUserOrgId", &head.org_id)
            .header("appid", "ly-upp")
            .header("csrfTimestamp", now.to_string())
            .header("csrfToken", csrf_token(now))
            .header("X-Requested-With", "XMLHttpRequest")
            .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(PortalError::Parse(format!("门户接口 HTTP {status}")));
        }
        Ok(text)
    }

    /// 学期与当前周。
    pub async fn query_semester_info(&self) -> Result<SemesterInfo, PortalError> {
        parse_semester_info(&self.get(EP_SEMESTER).await?)
    }

    /// 钱包三卡摘要（余额 / 在借图书 / 未读邮件）。
    pub async fn query_wallet_summary(&self) -> Result<WalletSummary, PortalError> {
        parse_wallet_summary(&self.get(EP_WALLET_CARD).await?)
    }

    /// 本周课表（供「下一节课」计算）。
    pub async fn query_week_schedule(&self) -> Result<WeekSchedule, PortalError> {
        parse_week_schedule(&self.get(EP_WEEK_SCHEDULE).await?)
    }

    /// 资讯栏目列表（订阅接口 + 实测全量兜底，见 parse::KNOWN_COLUMNS）。
    pub async fn query_info_columns(&self) -> Result<Vec<InfoColumn>, PortalError> {
        parse_info_columns(&self.get(EP_INFO_COLUMNS).await?)
    }

    /// 资讯列表（`total`/`pageCount` 实测不可靠，见 parse::parse_info_list）。
    pub async fn query_info_list(
        &self,
        column_id: &str,
        page: u32,
        page_size: u32,
    ) -> Result<InfoPage, PortalError> {
        if !is_safe_id(column_id) {
            return Err(PortalError::Parse("无效的资讯栏目".to_string()));
        }
        // 参数形态与官方前端一致（columnType/showNewDate 留空）
        let ep = format!(
            "{EP_INFO_LIST}?pageNum={page}&pageSize={page_size}&columnIds={column_id}&columnType=&showNewDate=0"
        );
        parse_info_list(&self.get(&ep).await?)
    }

    /// 待办分栏（接口实际返回 6 个 tab，全量透传；前端按契约展示三个）。
    pub async fn query_todo_tabs(&self) -> Result<Vec<TodoTab>, PortalError> {
        parse_todo_tabs(&self.get(EP_TODO_TABS).await?)
    }

    /// 待办列表（tabId 白名单校验，防异常输入破坏查询串结构）。
    pub async fn query_todo_list(
        &self,
        tab_id: &str,
        page: u32,
        page_size: u32,
    ) -> Result<TodoPage, PortalError> {
        if !matches!(
            tab_id,
            "todo" | "done" | "apply" | "unread" | "read" | "focus"
        ) {
            return Err(PortalError::Parse("无效的待办分栏".to_string()));
        }
        // 空参数（isUrge/isRead/isDone/时间区间）与官方前端一致
        let ep = format!(
            "{EP_TODO_LIST}?groupType=time&isUrge=&isRead=&isDone=&applyStartime=&applyEndtime=&arriveStartime=&arriveEndtime=&pageSize={page_size}&pageNum={page}&tabId={tab_id}&sorter=DESC"
        );
        parse_todo_list(&self.get(&ep).await?)
    }

    /// 资讯正文：抓官网静态页并提取清洗（见 article::extract_article）。
    ///
    /// 三种结果（真机验收结论：`content.jsp` 形态文章——通知公告/规章制度——
    /// 被站点鉴权开门页拦截，服务端重放无效，不是网络抖动）：
    /// - 正常抓到 → [`extract_article`] 清洗后的 HTML；
    /// - 被鉴权开门页拦截（`article::is_auth_wall`）→ 正常返回
    ///   `needsBrowser=true`（**不是错误**），前端引导浏览器打开原文；
    /// - 网络/解析异常 → Err（前端错误态可重试）。
    ///
    /// 安全：`url` 必须过 [`is_allowed_info_url`] 域名白名单（计划红线 3，防
    /// SSRF/钓鱼）；且用裸 `http_client` **不附带任何门户鉴权头**——正文页是
    /// 公开静态页无需登录，JWT 只发给门户同源，绝不随正文抓取泄漏到其他域名。
    pub async fn fetch_info_detail(&self, url: &str) -> Result<InfoDetail, PortalError> {
        if !is_allowed_info_url(url) {
            return Err(PortalError::Parse("仅支持校园官网正文链接".to_string()));
        }
        let parsed = reqwest::Url::parse(url)
            .map_err(|_| PortalError::Parse("正文链接无法解析".to_string()))?;
        let resp = self
            .cas
            .http_client()
            .get(parsed)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        let status_success = resp.status().is_success();
        // reqwest 已跟随重定向：url() 为最终 URL（鉴权开门页判定依据之一）
        let final_url = resp.url().to_string();
        let text = if status_success {
            resp.text().await?
        } else {
            String::new()
        };
        if is_auth_wall(status_success, &final_url, &text) {
            return Ok(InfoDetail {
                title: String::new(),
                html: None,
                needs_browser: true,
                url: url.to_string(),
            });
        }
        extract_article(&text, url)
    }
}
