//! 门户业务客户端：复用 CAS 会话的 [`CasClient`]（同 Cookie jar），按门户前端
//! 同款请求头组调用业务接口；JWT 与用户资料按会话内存缓存（绝不落盘）。
//!
//! 缓存生命周期 = 会话生命周期：[`PortalClient`] 挂在 `CasSession` 上，登录/登出/
//! 会话替换时整体随会话丢弃，无需单独失效逻辑；会话失效导致取不到 JWT 时返回
//! [`PortalError::NotLogin`]，上层提示重新登录。

use crate::parse::{parse_semester_info, parse_wallet_summary, parse_week_schedule};
use crate::{PortalError, SemesterInfo, WalletSummary, WeekSchedule};
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
}
