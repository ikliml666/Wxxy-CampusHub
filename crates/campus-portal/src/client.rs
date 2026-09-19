//! 门户业务客户端：复用 CAS 会话的 [`CasClient`]（同 Cookie jar），按门户前端
//! 同款请求头组调用业务接口；JWT 与用户资料按会话内存缓存（绝不落盘）。
//!
//! 缓存生命周期 = 会话生命周期：[`PortalClient`] 挂在 `CasSession` 上，登录/登出/
//! 会话替换时整体随会话丢弃，无需单独失效逻辑；会话失效导致取不到 JWT 时返回
//! [`PortalError::NotLogin`]，上层提示重新登录。

use crate::article::{extract_article, is_allowed_info_url, is_auth_wall};
use crate::parse::{
    collect_schedule_notices, guess_image_mime, meeting_query_title, parse_app_groups,
    parse_app_items, parse_info_columns, parse_info_list, parse_meeting_events,
    parse_schedule_classify, parse_schedule_day_counts, parse_schedule_events,
    parse_semester_info, parse_todo_list, parse_todo_tabs, parse_wallet_summary,
    parse_week_schedule, teaching_week_of,
};
use crate::{
    AppCatalog, AppItem, InfoColumn, InfoDetail, InfoPage, PortalError, ScheduleClassify,
    ScheduleDayCount, ScheduleEvent, ScheduleNoticeBrief, SemesterInfo, TodoPage, TodoTab,
    WalletSummary, WeekSchedule,
};
use base64::Engine as _;
use campus_auth::cas::{csrf_token, CasClient, PORTAL_PROBE};
use campus_auth::CampusAuthError;
use futures_util::StreamExt;
use std::collections::HashMap;
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
/// 应用分组端点（v2 形态每组自带 depName+appList，实测 8 组 30 应用全量覆盖；
/// 参数与官方前端一致）。
const EP_APP_GROUPS: &str =
    "api/upp/appStore/v2/queryApp?pageNum=1&pageSize=999&appLabel=&appType=&appName=&isStore=&appSort=0&pageId=";
/// 我的收藏/常用端点（官方前端同参数形态）。
const EP_MY_STORE: &str = "api/upp/appStore/queryMyStore?excludeMobile=1";
/// 日程分类端点（bs-schedule POST；**必须带完整头组**，缺头返回 500 系统错误）。
const EP_SCHEDULE_CLASSIFY: &str =
    "api/bs-schedule/innerPlaintext/scheduleRpcManage/findScheduleClassifyList";
/// 日程区间明细端点（POST）。
const EP_SCHEDULE_BETWEEN: &str =
    "api/bs-schedule/innerPlaintext/scheduleRpcManage/findScheduleBetweenTime";
/// 每日日程计数端点（POST；月视图角标用）。
const EP_SCHEDULE_COUNT: &str =
    "api/bs-schedule/innerPlaintext/scheduleRpcManage/getCountBetweenTime";
/// 校级会议卡端点（`DJZ` 参数实测按周过滤：`第<中文数字>周会议日程安排表`；
/// `10.1.90.34` 是门户代理的内部主机。2026-09-18 实测第二周 6 条/第一周 3 条）。
const EP_MEETING: &str = "api/uppexcard/ext/dynamicData/10.1.90.34/ZCHY";
/// 会议并入日程使用的分类 code（与 5 类过滤的「会议日程」对齐）。
const MEETING_CODE: &str = "Default-Meeting";
/// 门户文档库图标下载（相对 `<base>/`；`appIcon` 字段即 attachmentId UUID，
/// 2026-09-18 实测形态）。图标是同源受保护资源，需带会话 Cookie 由后端代拉。
const EP_DOCREPO: &str = "zuul/docrepo/download?attachmentId=";

/// 图标代拉并发上限（内网 RTT 短，4 路足够；更高并发对网关不友好）。
const ICON_CONCURRENCY: usize = 4;

/// 调课通知扫描栏目（重设计轮批 A，tauri 层契约 §18）：id 取自实测全量栏目表
/// [`crate::parse::KNOWN_COLUMNS`]——「通知公告」（校级综合公告，数字短 id）与
/// 「教务处」（调课/停课通知的发布主体）。2 栏够覆盖主来源，扩栏只改本表。
const NOTICE_SCAN_COLUMNS: &[(&str, &str)] = &[
    ("9", "通知公告"),
    ("ea0a5b2158bf48b3afeb026477c626e4", "教务处"),
];
/// 每栏目扫描条数（首页 ~50 条，覆盖近一两个月的公告量）。
const NOTICE_SCAN_LIMIT: u32 = 50;
/// 返回简报上限（按日期倒序截断）。
const NOTICE_MAX_ITEMS: usize = 20;

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

/// 门户同源请求统一头组（计划 §1.1 全表；GET/POST/图标下载共用同一事实来源）。
/// JWT 无 Bearer 前缀；csrf 现算毫秒时间戳。`now` 由调用方传入（三处调用同一毫秒）。
fn with_portal_headers(
    rb: reqwest::RequestBuilder,
    head: &AuthHead,
    now: u128,
) -> reqwest::RequestBuilder {
    rb.header("Authorization", &head.token_id)
        .header("loginUserId", &head.user_id)
        .header("loginUserName", &head.user_id)
        .header("loginUserOrgId", &head.org_id)
        .header("appid", "ly-upp")
        .header("csrfTimestamp", now.to_string())
        .header("csrfToken", csrf_token(now))
        .header("X-Requested-With", "XMLHttpRequest")
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
}

/// 门户业务客户端。Clone 廉价（`CasClient` 与缓存均为 Arc 包装，Cookie jar 共享）。
#[derive(Clone)]
pub struct PortalClient {
    /// 已登录 CAS 会话 client（含 Cookie jar；clone 与原实例共享 jar）。
    cas: CasClient,
    /// JWT/资料缓存（std Mutex：guard 在 await 前 drop，不跨 await 持锁；
    /// 锁中毒按缓存未命中处理，不影响正确性）。
    auth: Arc<Mutex<Option<AuthHead>>>,
    /// 应用图标 data URL 缓存，键 = appIcon UUID；值 None = 服务端确认无图
    ///（非图片字节），下次不再重拉。传输失败不缓存（下次重试）。
    icons: Arc<Mutex<HashMap<String, Option<String>>>>,
    /// 日程分类缓存（实测 5 类静态数据，会话内拉一次）。
    classify: Arc<Mutex<Option<Vec<ScheduleClassify>>>>,
    /// 学期信息缓存（会话内恒定：教学周次推算与今日页共用，省重复请求）。
    semester: Arc<Mutex<Option<SemesterInfo>>>,
    /// 会议日程缓存，键 = 教学周次；值 None = 服务端确认 0 条（不再重试）。
    /// 传输/解析失败不缓存（下次刷新重试）——与图标缓存同一模式。
    meetings: Arc<Mutex<HashMap<u32, Option<Vec<ScheduleEvent>>>>>,
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
            icons: Arc::new(Mutex::new(HashMap::new())),
            classify: Arc::new(Mutex::new(None)),
            semester: Arc::new(Mutex::new(None)),
            meetings: Arc::new(Mutex::new(HashMap::new())),
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
        let resp = with_portal_headers(
            self.cas.http_client().get(url).timeout(REQUEST_TIMEOUT),
            &head,
            now,
        )
        .send()
        .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(PortalError::Parse(format!("门户接口 HTTP {status}")));
        }
        Ok(text)
    }

    /// 门户同源 POST JSON（bs-schedule RPC 接口用；头组与 GET 同源同组，另带
    /// `Content-Type: application/json`）。⚠️ bs-schedule 的
    /// `findScheduleBetweenTime`/`getCountBetweenTime` 缺头会返回 500「系统错误」
    ///（2026-09-18 实测），故与 GET 共用 [`with_portal_headers`] 全量头组。
    async fn post_json(
        &self,
        endpoint: &str,
        body: serde_json::Value,
    ) -> Result<String, PortalError> {
        let head = self.auth_head().await?;
        let now = now_ms();
        let url = format!("{}/{endpoint}", PORTAL_PROBE.trim_end_matches('/'));
        let resp = with_portal_headers(
            self.cas.http_client().post(url).timeout(REQUEST_TIMEOUT),
            &head,
            now,
        )
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(PortalError::Parse(format!("日程接口 HTTP {status}")));
        }
        Ok(text)
    }

    /// 应用图标代拉：`<base>/zuul/docrepo/download?attachmentId=<appIcon>` →
    /// data URL（魔数校验非图片字节；base64 编码随响应返回，**不落盘不写日志**）。
    ///
    /// 结果缓存（键 = appIcon UUID）：Some(dataUrl) 命中不重拉；`None` = 服务端
    /// 明确给的不是图片（错误页/非图片格式），同样缓存避免每次刷新重试；
    /// 传输类 Err 不缓存，下次目录刷新自动重试。失败最终降级为 `icon_url: None`
    ///（前端显示占位图标，不阻塞目录）。
    async fn app_icon_data_url(&self, attachment_id: &str) -> Result<Option<String>, PortalError> {
        if let Some(hit) = self
            .icons
            .lock()
            .ok()
            .and_then(|g| g.get(attachment_id).cloned())
        {
            return Ok(hit);
        }
        // 附件 id 只允许 UUID 字符集（服务端下发，防御性校验防破坏 URL 结构）
        if attachment_id.is_empty()
            || !attachment_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Ok(None);
        }
        let head = self.auth_head().await?;
        let now = now_ms();
        let url = format!(
            "{}/{EP_DOCREPO}{attachment_id}&_t={now}",
            PORTAL_PROBE.trim_end_matches('/')
        );
        let resp = with_portal_headers(
            self.cas.http_client().get(url).timeout(REQUEST_TIMEOUT),
            &head,
            now,
        )
        .send()
        .await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        let outcome = if !status.is_success() {
            None
        } else {
            guess_image_mime(&bytes).map(|mime| {
                format!(
                    "data:{mime};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                )
            })
        };
        if let Some(mut g) = self.icons.lock().ok() {
            g.insert(attachment_id.to_string(), outcome.clone());
        }
        Ok(outcome)
    }

    /// 学期与当前周（**会话内缓存**：教学周次推算与今日页共用，数据会话内恒定）。
    pub async fn query_semester_info(&self) -> Result<SemesterInfo, PortalError> {
        if let Some(hit) = self.semester.lock().ok().and_then(|g| g.clone()) {
            return Ok(hit);
        }
        let info = parse_semester_info(&self.get(EP_SEMESTER).await?)?;
        if let Some(mut g) = self.semester.lock().ok() {
            *g = Some(info.clone());
        }
        Ok(info)
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

    /// 调课通知自动发现（重设计轮批 A，tauri 层契约 §18）：逐个扫描栏目
    ///（[`NOTICE_SCAN_COLUMNS`]）拉首页 [`NOTICE_SCAN_LIMIT`] 条，标题按
    /// [`crate::parse::notice_keyword_hits`] 检测（强词或 ≥1 弱词命中即纳入），
    /// 过滤/去重/倒序/截断收敛在纯函数 [`collect_schedule_notices`]。
    ///
    /// **只发现不解析**：返回的简报交由前端展示，用户确认后才经
    /// `parse_notice_from_url` 走候选确认流（不静默改数据）。
    /// 任一栏目取数失败（未登录/网络）→ Err 上抛（`NotLogin` 文案「请先登录」，
    /// 其余 `PortalError` Display 均为中文）。
    pub async fn query_schedule_notices(&self) -> Result<Vec<ScheduleNoticeBrief>, PortalError> {
        let mut pages = Vec::with_capacity(NOTICE_SCAN_COLUMNS.len());
        for (id, _name) in NOTICE_SCAN_COLUMNS {
            pages.push(self.query_info_list(id, 1, NOTICE_SCAN_LIMIT).await?);
        }
        Ok(collect_schedule_notices(
            NOTICE_SCAN_COLUMNS,
            &pages,
            NOTICE_MAX_ITEMS,
        ))
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

    /// 应用目录：v2 部门分组（8 组 30 应用全量覆盖）+ 我的收藏/常用钉选，
    /// 并**带会话代拉全部图标**（唯一 id 去重后限并发 [`ICON_CONCURRENCY`]）。
    ///
    /// 图标失败的条目 `icon_url: None`（前端占位图），不阻塞目录返回；
    /// 两个 JSON 接口任一失败整体 Err（前端错误态可重试）。
    pub async fn query_app_catalog(&self) -> Result<AppCatalog, PortalError> {
        let mut groups = parse_app_groups(&self.get(EP_APP_GROUPS).await?)?;
        let mut pinned = parse_app_items(&self.get(EP_MY_STORE).await?)?;

        // 唯一附件 id（分组与钉选图标可能重叠，去重后统一拉）
        let mut ids: Vec<String> = Vec::new();
        {
            let mut push = |it: &AppItem| {
                if let Some(icon) = &it.icon_id {
                    if !ids.iter().any(|x| x == icon) {
                        ids.push(icon.clone());
                    }
                }
            };
            for g in &groups {
                for it in &g.apps {
                    push(it);
                }
            }
            for it in &pinned {
                push(it);
            }
        }
        let fetched: HashMap<String, Option<String>> = futures_util::stream::iter(ids)
            .map(|id| async move {
                // 单图标失败（传输错）→ None 降级，不影响其余图标与目录整体
                let url = self.app_icon_data_url(&id).await.ok().flatten();
                (id, url)
            })
            .buffer_unordered(ICON_CONCURRENCY)
            .collect()
            .await;
        let fill = |items: &mut Vec<AppItem>| {
            for it in items {
                if let Some(icon) = &it.icon_id {
                    it.icon_url = fetched.get(icon).cloned().flatten();
                }
            }
        };
        for g in &mut groups {
            fill(&mut g.apps);
        }
        fill(&mut pinned);
        Ok(AppCatalog { groups, pinned })
    }

    /// 日程分类（实测 5 类静态数据；**会话内缓存**，仅首次真正发请求）。
    pub async fn query_schedule_classify(&self) -> Result<Vec<ScheduleClassify>, PortalError> {
        if let Some(hit) = self.classify.lock().ok().and_then(|g| g.clone()) {
            return Ok(hit);
        }
        let body = self
            .post_json(
                EP_SCHEDULE_CLASSIFY,
                serde_json::json!({"pageSize": 0, "pageNum": 1}),
            )
            .await?;
        let list = parse_schedule_classify(&body)?;
        if let Some(mut g) = self.classify.lock().ok() {
            *g = Some(list.clone());
        }
        Ok(list)
    }

    /// 日程区间明细（`startMs`/`endMs` 毫秒时间戳；`codes` 为选中的分类 code，
    /// 由前端过滤 chips 决定；空切片原样传 `[]`——服务端语义未实测，前端保证
    /// 全不选时不发请求）。classifyName/color 由 [`query_schedule_classify`]
    /// 的分类列表按 code 映射补全（明细里 `scheduleClassifyName` 可为 null）。
    pub async fn query_schedule_events(
        &self,
        start_ms: u64,
        end_ms: u64,
        codes: &[String],
    ) -> Result<Vec<ScheduleEvent>, PortalError> {
        let classify = self.query_schedule_classify().await?;
        // body 字段与官方前端逐字一致（scheduleName=null、publishStatus=1、
        // collaborative 空串），多余字段一律不带
        let body = serde_json::json!({
            "startTime": start_ms,
            "endTime": end_ms,
            "scheduleClassifyCodeList": codes,
            "scheduleName": null,
            "publishStatus": 1,
            "collaborativeId": "",
            "collaborativeType": "",
        });
        parse_schedule_events(&self.post_json(EP_SCHEDULE_BETWEEN, body).await?, &classify)
    }

    /// 每日日程计数（月视图角标用；bs-schedule 接口无分类过滤参数，计数为当日全量）。
    pub async fn query_schedule_day_counts(
        &self,
        start_ms: u64,
        end_ms: u64,
    ) -> Result<Vec<ScheduleDayCount>, PortalError> {
        let body = self
            .post_json(
                EP_SCHEDULE_COUNT,
                serde_json::json!({"startTime": start_ms, "endTime": end_ms}),
            )
            .await?;
        parse_schedule_day_counts(&body)
    }

    /// 校级会议日程并入（M2 遗留 A2）。
    ///
    /// 由 `start_ms`（前端所取周/月区间起点）推算教学周次，以标题
    /// `第<中文数字>周会议日程安排表` 调会议卡接口（DJZ 实测按周过滤，无周次
    /// 前缀返回 0 条）。
    ///
    /// **降级承诺（绝不影响课表日程与日历本身）**：codes 未选「会议」分类 /
    /// 学期信息不可用 / 周次推算不出（开学前）/ 拉取或解析失败 / 0 条 → 一律
    /// 返回空切片，本方法永不 Err。仅保留起点落在 `[start_ms, end_ms]` 内的
    /// 条目（防御性；同周会议按构造必在区间内）。
    ///
    /// 可观测性：**只在降级/失败时**输出一行 `[meeting-diag]` 到 stderr（环节 +
    /// 周次/HTTP 状态/错误类别；不含 JWT/cookie/响应体——错误消息仅含 URL、
    /// HTTP 状态与服务端 message，均无敏感字段），供真机 `tauri dev` 排障；
    /// 成功路径静默（正常刷新零输出）。
    pub async fn query_meetings_for_range(
        &self,
        start_ms: u64,
        end_ms: u64,
        codes: &[String],
    ) -> Vec<ScheduleEvent> {
        if !codes.iter().any(|c| c == MEETING_CODE) {
            return Vec::new();
        }
        let sem = match self.query_semester_info().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[meeting-diag] 学期信息失败: {e}");
                return Vec::new();
            }
        };
        let Some(week) = teaching_week_of(start_ms, &sem.start_date) else {
            eprintln!(
                "[meeting-diag] 周次推算失败: start_date={} startMs={start_ms}",
                sem.start_date
            );
            return Vec::new();
        };
        match self.query_meeting_events(week).await {
            // 请求失败/解析失败已在 query_meeting_events 内打点，此处静默降级
            Ok(events) => events
                .into_iter()
                .filter(|e| e.start_ms >= start_ms && e.start_ms < end_ms)
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// 按教学周次拉会议日程（**会话内缓存**：确认 0 条同样缓存；传输/解析失败
    /// 不缓存，下次刷新重试）。URL 中文 query 由 url 层自动 percent-encode
    ///（离线已验证与官方请求逐字节一致），与官方前端请求形态一致。
    async fn query_meeting_events(&self, week: u32) -> Result<Vec<ScheduleEvent>, PortalError> {
        if let Some(hit) = self.meetings.lock().ok().and_then(|g| g.get(&week).cloned()) {
            return Ok(hit.unwrap_or_default());
        }
        let Some(title) = meeting_query_title(week) else {
            return Err(PortalError::Parse(format!(
                "教学周次 {week} 无法构造查询标题"
            )));
        };
        let classify = self.query_schedule_classify().await?;
        let endpoint = format!("{EP_MEETING}?DJZ={title}&pageNum=1&pageSize=20");
        let body = match self.get(&endpoint).await {
            Ok(b) => b,
            Err(e) => {
                // {e} 含 URL（DJZ 为周次会议标题，公开信息；无凭据参数）、
                // HTTP 状态或传输错误类别
                eprintln!("[meeting-diag] 第{week}周 请求失败: {e}");
                return Err(e);
            }
        };
        match parse_meeting_events(&body, &classify) {
            Ok(events) => {
                if let Some(mut g) = self.meetings.lock().ok() {
                    g.insert(week, Some(events.clone()));
                }
                Ok(events)
            }
            // 信封失败形态含服务端 message（如「请重新登录」）、JSON 形态错误
            // 为 serde 类别信息，均不含响应体内容
            Err(e) => {
                eprintln!("[meeting-diag] 第{week}周 解析失败: {e}");
                Err(e)
            }
        }
    }
}
