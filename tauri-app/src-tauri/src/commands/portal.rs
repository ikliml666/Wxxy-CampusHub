//! 门户数据命令（M2 批次 1：今日页总览 + 批次 2：资讯页/待办页 + 批次 3：
//! 应用页/日程页；IPC 契约见计划 §2.1，camelCase）。
//!
//! `get_portal_overview` 聚合三个门户接口（学期 / 钱包卡 / 本周课表），**每个
//! 子字段取失败不阻塞其余字段**（失败项为 null，前端回落空态，不整页报错）。
//! 批次 2/3 的其余命令为单接口透传（资讯、待办、应用目录、日程、打开应用），
//! 统一经 [`portal_of`]：未登录 → `success:false, message:"请先登录"`；协议错误
//! 映射为可读中文 message，不 panic。
//!
//! 敏感纪律：JWT 与邮箱 `loginUrl` 在 campus-portal 内部消化，本命令只透出
//! 钱包数字、课程简报、资讯条目与清洗后的正文 HTML，不含任何凭据字段。

use super::auth::{session_client, CommandResult};
use crate::infra::state::AppState;
use base64::Engine as _;
use campus_portal::{
    is_allowed_attachment_url, is_allowed_info_url, is_http_url, next_course_from_now, AppCatalog,
    CourseBrief, InfoColumn, InfoDetail, InfoPage, PortalClient, ScheduleClassify, ScheduleDayCount,
    ScheduleEvent, SemesterInfo, TodoPage, TodoTab, WalletSummary,
};
use serde::Serialize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::State;
use tauri_plugin_opener::OpenerExt;

/// 无会话时的约定文案（与 profile.rs 同口径，前端据此引导登录）。
const ERR_NO_SESSION: &str = "请先登录";

/// 附件响应体上限 15 MB：base64 后约 20 MB，经 IPC 序列化已是前端内存压力
/// 上限，超出引导用户从原文页浏览器下载。
const ATTACHMENT_MAX_BYTES: usize = 15 * 1024 * 1024;

/// 附件下载超时（15 MB 慢链路预留，与 10s 的 HTML 正文抓取不同量级）。
const ATTACHMENT_TIMEOUT: Duration = Duration::from_secs(60);

/// get_portal_overview → data。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalOverview {
    pub semester: Option<SemesterInfo>,
    pub wallet: Option<WalletSummary>,
    /// 无课 / 课表取失败为 null（前端隐藏横幅）。
    pub next_course: Option<CourseBrief>,
    /// 聚合完成时刻 epoch 毫秒。
    pub fetched_at: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 今日页总览：学期信息 + 钱包三卡 + 下一节课（子字段各自尽力而为）。
#[tauri::command]
pub async fn get_portal_overview(
    state: State<'_, AppState>,
) -> Result<CommandResult<PortalOverview>, String> {
    // 锁内只 clone portal（Arc 包装，廉价），drop guard 后再 await
    let portal = {
        let guard = state.session.lock().await;
        guard.as_ref().map(|s| s.portal.clone())
    };
    let Some(portal) = portal else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };

    // 子字段独立取数：任一失败置 None，不影响其余（计划 §2.1）
    let semester = portal.query_semester_info().await.ok();
    let wallet = portal.query_wallet_summary().await.ok();
    let next_course = portal
        .query_week_schedule()
        .await
        .ok()
        .and_then(|ws| next_course_from_now(&ws));

    Ok(CommandResult::ok(PortalOverview {
        semester,
        wallet,
        next_course,
        fetched_at: now_ms(),
    }))
}

/// 会话内门户客户端（None = 未登录）。锁内只 clone portal（Arc 廉价），
/// drop guard 后调用方再 await。协议错误由各命令统一映射为可读中文 message。
async fn portal_of(state: &State<'_, AppState>) -> Option<PortalClient> {
    let guard = state.session.lock().await;
    guard.as_ref().map(|s| s.portal.clone())
}

/// 资讯栏目（订阅接口 + 实测全量兜底，7 栏）。
#[tauri::command]
pub async fn get_info_columns(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<InfoColumn>>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_info_columns()
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 资讯列表（total/pageCount 实测不可靠，前端以 items.length 判断分页）。
#[tauri::command]
pub async fn get_info_list(
    state: State<'_, AppState>,
    column_id: String,
    page: u32,
    page_size: u32,
) -> Result<CommandResult<InfoPage>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_info_list(&column_id, page, page_size)
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 资讯正文（后端已清洗：域名白名单 + 标签/属性白名单，前端直接渲染）。
#[tauri::command]
pub async fn get_info_detail(
    state: State<'_, AppState>,
    url: String,
) -> Result<CommandResult<InfoDetail>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .fetch_info_detail(&url)
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 待办分栏（接口实际返回 6 个 tab，全量透传，前端展示 todo/done/apply）。
#[tauri::command]
pub async fn get_todo_tabs(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<TodoTab>>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_todo_tabs()
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 待办列表（tabId 白名单校验见 client::query_todo_list）。
#[tauri::command]
pub async fn get_todo_list(
    state: State<'_, AppState>,
    tab_id: String,
    page: u32,
    page_size: u32,
) -> Result<CommandResult<TodoPage>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_todo_list(&tab_id, page, page_size)
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 系统浏览器打开 URL（可复用 helper，批次 3 `open_app` 复用；实现为官方
/// `tauri-plugin-opener` 的 Rust API，不手写 Win32 调用）。
///
/// 安全：**域名白名单强制**——只允许 `*.cwxu.edu.cn`（复用
/// `campus_portal::is_allowed_info_url`，与正文抓取同一事实来源），非法域名
/// 一律拒绝，防被诱导打开任意 URL。
pub(crate) fn open_url_in_browser(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    if !is_allowed_info_url(url) {
        return Err("仅支持校园官网链接".to_string());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| format!("打开浏览器失败: {e}"))
}

/// 在系统浏览器中打开原文（needsBrowser 的正文 / 官网页面）。
#[tauri::command]
pub async fn open_in_browser(
    app: tauri::AppHandle,
    url: String,
) -> Result<CommandResult<()>, String> {
    Ok(match open_url_in_browser(&app, &url) {
        Ok(()) => CommandResult::empty(),
        Err(e) => CommandResult::err(&e),
    })
}

// ---------------- M2 批次 3：应用页 / 日程页 ----------------

/// 应用目录（部门分组 + 收藏/常用钉选；图标已由后端代拉为 data URL，失败的
/// 条目 iconUrl 为 null，前端显示占位图标）。
#[tauri::command]
pub async fn get_app_catalog(
    state: State<'_, AppState>,
) -> Result<CommandResult<AppCatalog>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_app_catalog()
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 日程分类（实测 5 类，会话内后端已缓存）。
#[tauri::command]
pub async fn get_schedule_classify(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<ScheduleClassify>>, String> {
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_schedule_classify()
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 日程区间明细（周/月视图取数；startMs/endMs 为本地周（月）首尾毫秒时间戳，
/// codes 为选中的分类 code 列表）。
///
/// **校级会议并入**（M2 遗留 A2）：codes 含「会议」分类时，按区间起点所在教学
/// 周次并入会议卡日程（classifyCode=Default-Meeting，主持人/参会人员等在
/// `extra`）。会议拉取失败/0 条 → 空贡献，不影响课表日程；课表明细失败才报错。
#[tauri::command]
pub async fn get_schedule_month(
    state: State<'_, AppState>,
    start_ms: u64,
    end_ms: u64,
    codes: Vec<String>,
) -> Result<CommandResult<Vec<ScheduleEvent>>, String> {
    // 区间倒挂视为非法入参（前端 bug 防御），不透传给服务端
    if end_ms <= start_ms {
        return Ok(CommandResult::err("日程区间无效"));
    }
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    match portal.query_schedule_events(start_ms, end_ms, &codes).await {
        Ok(mut list) => {
            // 校级会议并入（M2 遗留 A2）：codes 含「会议」分类时按区间起点所在
            // 教学周次并入会议卡日程；拉取失败/0 条为空贡献，不影响课表日程
            //（失败时 stderr 有 [meeting-diag] 打点，成功路径静默）
            list.extend(portal.query_meetings_for_range(start_ms, end_ms, &codes).await);
            Ok(CommandResult::ok(list))
        }
        Err(e) => Ok(CommandResult::err(&e.to_string())),
    }
}

/// 每日日程计数（月视图角标）。⚠️ bs-schedule 的 getCountBetweenTime 无分类
/// 过滤参数——角标为当日**全量**日程数，如实呈现服务端计数（不伪造过滤后
/// 的计数）；「5 类过滤」作用于周视图明细与从月视图跳转后的展示。
#[tauri::command]
pub async fn get_schedule_day_counts(
    state: State<'_, AppState>,
    start_ms: u64,
    end_ms: u64,
) -> Result<CommandResult<Vec<ScheduleDayCount>>, String> {
    if end_ms <= start_ms {
        return Ok(CommandResult::err("日程区间无效"));
    }
    let Some(portal) = portal_of(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(portal
        .query_schedule_day_counts(start_ms, end_ms)
        .await
        .map(CommandResult::ok)
        .unwrap_or_else(|e| CommandResult::err(&e.to_string())))
}

/// 打开应用（`isCas` 契约保留字段）。
///
/// 校验用**协议白名单**（[`campus_portal::is_http_url`]，仅 http/https），而非
/// [`open_url_in_browser`] 的域名白名单：该 URL 来自校方应用目录（受信来源），
/// 且只在**系统浏览器**打开、我们的后端不抓取它（无 SSRF 面）——实测 30 条目录
/// 数据中 16 条为非校园域（一卡通 `10.3.100.110`、知网、万方、超星、虚拟图书馆），
/// 域名白名单会把学校自己的合法应用全部拦掉。域名白名单（SSRF 面）保留在
/// 「后端抓取的正文 URL」路径（`fetch_info_detail`）与 `open_in_browser` 上。
///
/// ⚠️ 残留（计划附录 A / 批次 3 范围）：**B 类应用的 WebVPN 会话包装未实现**
/// ——WebVPN 会话尚未打通，需 WebVPN 环境的应用（外购数据库、内网 IP 直链）在
/// 校外网络下由浏览器侧自行报错。待 WebVPN 会话打通后在此处按 isCas/link
/// 分类包装，届时一并处理打开前的会话预置。
#[tauri::command]
pub async fn open_app(
    app: tauri::AppHandle,
    url: String,
    is_cas: bool,
) -> Result<CommandResult<()>, String> {
    // isCas 当前不影响打开策略（协议直开，见 doc comment），保留入参以冻结 IPC 契约
    let _ = is_cas;
    if !is_http_url(&url) {
        return Ok(CommandResult::err("仅支持 http/https 链接"));
    }
    // 官方 tauri-plugin-opener 的 Rust API（不手写 Win32 调用）
    Ok(match app.opener().open_url(&url, None::<&str>) {
        Ok(()) => CommandResult::empty(),
        Err(e) => CommandResult::err(&format!("打开浏览器失败: {e}")),
    })
}

// ---------------- 资讯附件（pdf 查看器铺路：下载命令 + DTO） ----------------
// 配套 CSP 放行在 tauri.conf.json（该文件为严格 JSON 且 security 段拒绝未知
// 键，注释无法落在原处，故记于此）：script-src 加 'wasm-unsafe-eval'（pdf.js
// wasm 解码）、新增 worker-src 'self' blob:（pdf.js worker）、img-src 加
// blob: data:（内嵌位图）；connect-src 未动。

/// download_attachment → data（base64 为裸标准编码，前端自行拼 data: / 喂
/// pdf.js；fileName 已尽量还原原始名，展示兜底由前端处理）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentData {
    pub file_name: String,
    pub base64: String,
}

/// 最小 percent-decode（`%XX` → 字节，非法 `%` 序列原样保留），仅用于附件
/// 文件名展示层；手写不引 percent-encoding 依赖（reqwest/url 的内部依赖
/// 不直接可用）。
fn percent_decode(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(hi), Some(lo)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 附件文件名：Content-Disposition 的 `filename*=`（RFC 5987，取 `''` 后段并
/// percent-decode）优先，其次 `filename=`（剥引号），都无回落 URL 尾段；尾段
/// 也取不到时给「附件」。
fn file_name_of(disposition: Option<&str>, url: &reqwest::Url) -> String {
    if let Some(cd) = disposition {
        if let Some(v) = cd
            .split(';')
            .find_map(|p| p.trim().strip_prefix("filename*="))
        {
            let raw = v.trim_matches('"').rsplit("''").next().unwrap_or(v);
            let name = percent_decode(raw.trim());
            if !name.is_empty() {
                return name;
            }
        }
        if let Some(v) = cd
            .split(';')
            .find_map(|p| p.trim().strip_prefix("filename="))
        {
            let name = percent_decode(v.trim().trim_matches('"'));
            if !name.is_empty() {
                return name;
            }
        }
    }
    url.path_segments()
        .and_then(|mut segs| segs.next_back().map(str::to_string))
        .filter(|s| !s.is_empty())
        .map(|s| percent_decode(&s))
        .unwrap_or_else(|| "附件".to_string())
}

/// 下载资讯附件（`download_attachment`）。
///
/// 安全链路：① URL 白名单（[`is_allowed_attachment_url`]：校园域 + 一卡通
/// `10.3.100.110`，防 SSRF）；② 必须有会话（门户域 cookie 在 RecordingJar，
/// GET 时自动携带，复用 [`session_client`]）；③ 响应体**流式**累计、超
/// [`ATTACHMENT_MAX_BYTES`] 即断开（content_length 可缺失/谎报，不能只看头）。
/// 成功返回文件名 + base64，由前端落盘 / 渲染，后端不写盘。
#[tauri::command]
pub async fn download_attachment(
    state: State<'_, AppState>,
    url: String,
) -> Result<CommandResult<AttachmentData>, String> {
    if !is_allowed_attachment_url(&url) {
        return Ok(CommandResult::err("仅支持校园官网附件链接"));
    }
    let Some(client) = session_client(&state).await else {
        return Ok(CommandResult::err("需要登录后才能下载附件"));
    };
    let Ok(parsed) = reqwest::Url::parse(&url) else {
        return Ok(CommandResult::err("附件链接无法解析"));
    };
    let mut resp = match client
        .http_client()
        .get(parsed.clone())
        .timeout(ATTACHMENT_TIMEOUT)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return Ok(CommandResult::err(&format!("下载附件失败: {e}"))),
    };
    if !resp.status().is_success() {
        return Ok(CommandResult::err(&format!(
            "下载附件失败 (HTTP {})",
            resp.status().as_u16()
        )));
    }
    // 头里直接报得清的超限快断（省一次无效下载）；缺失/谎报由流式循环兜底
    if resp
        .content_length()
        .is_some_and(|n| n as usize > ATTACHMENT_MAX_BYTES)
    {
        return Ok(CommandResult::err("附件过大，请从原文页下载"));
    }
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len() + chunk.len() > ATTACHMENT_MAX_BYTES {
                    return Ok(CommandResult::err("附件过大，请从原文页下载"));
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => return Ok(CommandResult::err(&format!("下载附件失败: {e}"))),
        }
    }
    let file_name = file_name_of(
        resp.headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok()),
        &parsed,
    );
    Ok(CommandResult::ok(AttachmentData {
        file_name,
        base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_url_whitelist() {
        assert!(is_allowed_attachment_url(
            "https://www.cwxu.edu.cn/__local/A/B/12/x.pdf"
        ));
        // 一卡通内网服务器放行（附件直链实测会挂在这里）
        assert!(is_allowed_attachment_url("http://10.3.100.110/upload/x.pdf"));
        assert!(is_allowed_attachment_url("https://jwc.cwxu.edu.cn/a.docx"));
        // 混淆 / 非 http 形态拒绝
        assert!(!is_allowed_attachment_url("https://cwxu.edu.cn.evil.com/x.pdf"));
        assert!(!is_allowed_attachment_url(
            "https://evil.com/?u=http://10.3.100.110/x.pdf"
        ));
        assert!(!is_allowed_attachment_url("ftp://10.3.100.110/x.pdf"));
        assert!(!is_allowed_attachment_url("not a url"));
    }

    #[test]
    fn file_name_prefer_disposition_then_url_tail() {
        let u =
            reqwest::Url::parse("https://www.cwxu.edu.cn/__local/A/B/12/report.PDF").unwrap();
        // filename= 带引号
        assert_eq!(
            file_name_of(Some(r#"attachment; filename="期末安排.pdf""#), &u),
            "期末安排.pdf"
        );
        // filename* RFC 5987 形态（percent-decode）
        assert_eq!(
            file_name_of(Some("attachment; filename*=UTF-8''%E6%8A%A5%E8%A1%A8.pdf"), &u),
            "报表.pdf"
        );
        // CD 无 filename / 无 CD → URL 尾段
        assert_eq!(file_name_of(Some("attachment"), &u), "report.PDF");
        assert_eq!(file_name_of(None, &u), "report.PDF");
    }

    #[test]
    fn percent_decode_minimal() {
        assert_eq!(percent_decode("%E4%B8%AD.pdf"), "中.pdf");
        assert_eq!(percent_decode("plain.pdf"), "plain.pdf");
        assert_eq!(percent_decode("bad%2"), "bad%2");
        assert_eq!(percent_decode("bad%zz"), "bad%zz");
    }
}
