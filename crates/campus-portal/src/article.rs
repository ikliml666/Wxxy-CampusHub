//! 资讯正文提取与清洗（官网静态页 → 安全 HTML 片段，纯函数 + 脱敏单测）。
//!
//! 官网正文页为博达 webplus 系统：正文容器 `div.v_news_content`（外层
//! `#vsb_content_*`，tw/jwc/xgc 三站实测分别 `vsb_content_501/6/2`），标题
//! `h2` 优先、`<title>` 兜底。本模块：
//! - [`is_allowed_info_url`]：**域名白名单**（仅 `*.cwxu.edu.cn`，计划红线 3，
//!   防把用户可控 URL 透传给 HTTP 客户端造成 SSRF/钓鱼）；
//! - [`is_http_url`]：**协议白名单**（仅 http/https）——只用于「系统浏览器打开、
//!   后端不抓取内容」的 URL（`open_app` 打开校方目录下发的 appLink）；
//! - [`is_auth_wall`]：判定正文页被站点鉴权开门页拦截（见函数注释）——命中时
//!   命令层正常返回 `needsBrowser=true` 引导浏览器打开，**不进错误态**；
//! - [`extract_article`]：提取标题与正文容器，按**标签/属性白名单**重建 HTML
//!   片段（剔除 script/style/iframe/事件属性，相对地址转绝对）。
//! 清洗在后端完成，前端直接渲染，不再二次处理。

use crate::{InfoDetail, PortalError};
use ego_tree::iter::Edge;
use scraper::{node::Node, ElementRef, Html, Selector};
use std::fmt::Write as _;

/// 正文页域名白名单：仅允许 `cwxu.edu.cn` 及其子域（host 精确后缀匹配，
/// 防 `cwxu.edu.cn.evil.com` 与 `cwxu.edu.cn@evil.com` 形态绕过）。
pub fn is_allowed_info_url(raw: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(raw) else {
        return false;
    };
    let Some(host) = u.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    matches!(u.scheme(), "http" | "https")
        && (host == "cwxu.edu.cn" || host.ends_with(".cwxu.edu.cn"))
}

/// 打开类 URL 的**协议白名单**：仅允许 `http`/`https`，拒绝 `file:` /
/// `javascript:` / `data:` 等任何其他 scheme。
///
/// 适用场景：**只在系统浏览器打开、后端绝不抓取其内容**的 URL（当前唯一调用方
/// 是 `open_app` 打开校方应用目录下发的 `appLink`）。此处不做域名限制的理由：
/// 该 URL 来自校方应用目录（受信来源），且我们的后端不发起对该 URL 的任何请求
/// （无 SSRF 面），限制域名只会拦掉学校自己的合法应用——2026-09-18 真机实测
/// 30 条目录数据中有 16 条为非校园域（一卡通 `10.3.100.110`、知网、万方、超星、
/// 虚拟图书馆），用域名白名单全部打不开。真正的 SSRF 防护留在「后端要抓取的
/// 正文 URL」路径上（[`is_allowed_info_url`] 域名白名单，**不要放宽那条**）。
pub fn is_http_url(raw: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(raw) else {
        return false;
    };
    matches!(u.scheme(), "http" | "https")
}

/// 危险标签：连同整棵子树剔除（可执行代码 / 嵌入对象 / 表单控件）。
fn is_dropped_tag(name: &str) -> bool {
    matches!(
        name,
        "script"
            | "style"
            | "iframe"
            | "frame"
            | "frameset"
            | "object"
            | "embed"
            | "applet"
            | "noscript"
            | "link"
            | "meta"
            | "base"
            | "form"
            | "input"
            | "button"
            | "select"
            | "textarea"
            | "option"
            | "svg"
            | "math"
            | "template"
            | "video"
            | "audio"
            | "source"
            | "track"
            | "canvas"
            | "dialog"
    )
}

/// 保留标签（其余无害标签只解包子节点、不输出标签本身）。
fn is_kept_tag(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "span"
            | "br"
            | "hr"
            | "img"
            | "a"
            | "ul"
            | "ol"
            | "li"
            | "dl"
            | "dt"
            | "dd"
            | "table"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "strong"
            | "b"
            | "em"
            | "i"
            | "u"
            | "s"
            | "sup"
            | "sub"
            | "blockquote"
            | "pre"
            | "code"
            | "section"
            | "article"
            | "center"
            | "font"
    )
}

/// 文本节点转义（scraper 已解码实体，重建时须再编码；`&` 必须最先替换）。
fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 属性值转义（文本转义 + 引号）。
fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

/// `src`/`href` 值清洗：相对地址转绝对；仅保留 http/https（图片额外放行
/// `data:image/`），其余 scheme（javascript:/data:text/html 等）丢弃该属性。
fn sanitize_url(raw: &str, base: &reqwest::Url, is_img_src: bool) -> Option<String> {
    let abs = base.join(raw.trim()).ok()?;
    if matches!(abs.scheme(), "http" | "https") {
        return Some(abs.to_string());
    }
    if is_img_src && abs.as_str().starts_with("data:image/") {
        return Some(abs.to_string());
    }
    None
}

/// 元素属性白名单（事件属性 `on*`、style/class/id 等一律不在表内 → 丢弃）。
fn keep_attr(tag: &str, attr: &str) -> bool {
    match tag {
        "img" => matches!(attr, "src" | "alt" | "width" | "height" | "align"),
        "a" => matches!(attr, "href" | "title" | "name"),
        "td" | "th" => matches!(attr, "colspan" | "rowspan" | "align"),
        _ => matches!(attr, "align" | "width" | "height"),
    }
}

/// 判定正文页被站点鉴权开门页拦截（纯函数供离线单测，命中 → 命令层正常
/// 返回 `needsBrowser=true` 引导浏览器打开，不进错误态）。判定依据（2026-09-18
/// 主智能体实机 curl 验证，`content.jsp` 形态文章——通知公告/规章制度两栏——
/// 服务端无论如何重放都停在鉴权页）：
/// ① HTTP 非 2xx；② 重定向**最终 URL** 命中 `/system/resource/code/auth/auth.htm`；
/// ③ 页面标题精确为「系统提示」。
///
/// ③ 用 `<title>` 解析而非子串搜索——正文文本偶含这四个字时不误判。
pub fn is_auth_wall(status_success: bool, final_url: &str, page_html: &str) -> bool {
    if !status_success || final_url.contains("/system/resource/code/auth/auth.htm") {
        return true;
    }
    let doc = Html::parse_document(page_html);
    let Ok(sel) = Selector::parse("title") else {
        return false;
    };
    doc.select(&sel)
        .next()
        .map(|t| t.text().collect::<String>().trim() == "系统提示")
        .unwrap_or(false)
}

/// 从官网静态页提取正文并清洗为安全 HTML 片段（纯函数，供离线单测）。
///
/// 标题：`h2` 优先，`<title>` 兜底；正文容器：`div.v_news_content` 优先，
/// `[id^=vsb_content]` 兜底；未命中 / 正文为空 → [`PortalError::Parse`]。
pub fn extract_article(page_html: &str, page_url: &str) -> Result<InfoDetail, PortalError> {
    let base = reqwest::Url::parse(page_url)
        .map_err(|_| PortalError::Parse("正文链接无法解析".to_string()))?;
    let doc = Html::parse_document(page_html);

    // 标题：h2 优先、<title> 兜底（文章页实测 h2 唯一）
    let text_of = |sel: Selector| {
        doc.select(&sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .filter(|t| !t.is_empty())
    };
    let h2 = Selector::parse("h2").expect("固定选择器");
    let title_tag = Selector::parse("title").expect("固定选择器");
    let title = text_of(h2)
        .or_else(|| text_of(title_tag))
        .ok_or_else(|| PortalError::Parse("正文页缺少标题".to_string()))?;

    let content = doc
        .select(&Selector::parse("div.v_news_content, [id^=vsb_content]").expect("固定选择器"))
        .next()
        .ok_or_else(|| PortalError::Parse("未找到正文内容".to_string()))?;

    let mut html = String::new();
    render_subtree(&content, &base, &mut html);
    if html.trim().is_empty() {
        return Err(PortalError::Parse("正文内容为空".to_string()));
    }
    Ok(InfoDetail {
        title,
        html: Some(html),
        needs_browser: false,
        url: page_url.to_string(),
    })
}

/// 深度优先重建正文容器子树为安全 HTML 片段。
///
/// 遍历用 DFS Open/Close 配对（`ElementRef::traverse`）；危险子树用抑制计数
/// 整段剔除（Open +1 / Close -1，DFS 配对保证归零）；白名单外无害标签解包
/// （保留子内容不输出标签）；void 元素（img/br/hr）无 Close 边，天然自闭合。
fn render_subtree(root: &ElementRef, base: &reqwest::Url, out: &mut String) {
    let root_id = root.id();
    let mut suppress = 0usize;
    for edge in root.traverse() {
        match edge {
            Edge::Open(node) => {
                // 容器自身标签不输出（前端自行包裹样式容器），只重建其内容
                if node.id() == root_id {
                    continue;
                }
                if suppress > 0 {
                    suppress += 1;
                    continue;
                }
                match node.value() {
                    Node::Text(t) => out.push_str(&escape_text(&t.text)),
                    Node::Element(el) => {
                        let name = el.name();
                        if is_dropped_tag(name) {
                            suppress = 1;
                        } else if is_kept_tag(name) {
                            out.push('<');
                            out.push_str(name);
                            for (an, av) in el.attrs() {
                                if !keep_attr(name, an) {
                                    continue;
                                }
                                match an {
                                    "src" | "href" => {
                                        if let Some(v) =
                                            sanitize_url(av, base, name == "img" && an == "src")
                                        {
                                            let _ = write!(out, " {an}=\"{}\"", escape_attr(&v));
                                        }
                                    }
                                    _ => {
                                        let _ = write!(out, " {an}=\"{}\"", escape_attr(av));
                                    }
                                }
                            }
                            out.push('>');
                        }
                        // 白名单外且无害：解包，子节点照常遍历
                    }
                    _ => {} // Comment / Doctype 等跳过
                }
            }
            Edge::Close(node) => {
                if node.id() == root_id {
                    continue;
                }
                if suppress > 0 {
                    suppress -= 1;
                    continue;
                }
                if let Some(el) = node.value().as_element() {
                    let name = el.name();
                    if is_kept_tag(name) {
                        let _ = write!(out, "</{name}>");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- fixture（全部脱敏：占位标题/正文，无真实数据） ----------

    /// 博达 webplus 文章页同构迷你页（容器/标题结构与实测一致，内容虚构）。
    const PAGE_FIXTURE: &str = r#"<!DOCTYPE html>
<html><head><title>示例文章标题 - 无锡学院</title></head>
<body>
<div class="main_contit"><h2>示例通知标题</h2></div>
<div id="vsb_content_501"><div class="v_news_content">
  <p class="vsbcontent_start">正文第一段，含<a href="/info/1033/1.htm">内部链接</a>。</p>
  <script>alert('xss')</script>
  <style>.evil{}</style>
  <p onclick="evil()" style="color:red">正文第二段，含<a href="javascript:void(0)">坏链接</a>。</p>
  <img src="../pic/pic1.jpg" alt="示例图" onerror="evil()">
  <iframe src="//evil.example/frame"></iframe>
  <form action="/steal"><input name="q"><p>表单内段落</p></form>
  <p>末段<a href="https://other.example/page">外站链接</a></p>
</div></div>
</body></html>"#;

    const PAGE_URL: &str = "https://tw.cwxu.edu.cn/info/1164/2564.htm";

    // ---------- is_allowed_info_url ----------

    #[test]
    fn url_whitelist_allows_campus_hosts_only() {
        assert!(is_allowed_info_url("https://www.cwxu.edu.cn/content.jsp?urltype=news.NewsContentUrl&wbtreeid=1039&wbnewsid=1"));
        assert!(is_allowed_info_url(
            "https://tw.cwxu.edu.cn/info/1164/2564.htm"
        ));
        assert!(is_allowed_info_url(
            "http://jwc.cwxu.edu.cn/info/1100/1.htm"
        ));
        assert!(is_allowed_info_url("https://cwxu.edu.cn/"));
        // 混淆形态必须拒绝
        assert!(!is_allowed_info_url("https://cwxu.edu.cn.evil.com/x.htm"));
        assert!(!is_allowed_info_url("https://cwxu.edu.cn@evil.com/x.htm"));
        assert!(!is_allowed_info_url(
            "https://evil.com/?u=https://cwxu.edu.cn/"
        ));
        assert!(!is_allowed_info_url("ftp://cwxu.edu.cn/x.htm"));
        assert!(!is_allowed_info_url("file:///c:/x.htm"));
        assert!(!is_allowed_info_url("javascript:alert(1)"));
        assert!(!is_allowed_info_url("not a url"));
        assert!(!is_allowed_info_url(""));
    }

    // ---------- is_http_url（open_app 协议白名单；域名不限） ----------

    #[test]
    fn open_app_protocol_guard_allows_http_https_from_trusted_catalog() {
        // 校方应用目录实测存在的外部域 / 内网 IP 应用（受信来源、系统浏览器打开）
        assert!(is_http_url("https://www.cnki.net/"));
        assert!(is_http_url("http://10.3.100.110/"));
        assert!(is_http_url("https://www.wanfangdata.com.cn/"));
        assert!(is_http_url("https://fysso.chaoxing.com/login"));
        assert!(is_http_url("https://cwxu.flyread.com.cn/"));
        // 校园域与大写 scheme 形态
        assert!(is_http_url("https://jwgl.cwxu.edu.cn/"));
        assert!(is_http_url("HTTPS://Lib.CWXU.EDU.CN/"));
    }

    #[test]
    fn open_app_protocol_guard_rejects_non_http_schemes() {
        assert!(!is_http_url("file:///C:/Windows/System32/calc.exe"));
        assert!(!is_http_url("javascript:alert(1)"));
        assert!(!is_http_url("data:text/html;base64,PHNjcmlwdD4="));
        assert!(!is_http_url("ftp://example.com/x"));
        assert!(!is_http_url("vbscript:msgbox(1)"));
        assert!(!is_http_url("not a url"));
        assert!(!is_http_url(""));
    }

    // ---------- open_in_browser 域名白名单回归（修复后不得放宽） ----------

    #[test]
    fn open_in_browser_domain_whitelist_stays_tight() {
        // 资讯正文降级打开仍走 is_allowed_info_url：校园域放行不变
        assert!(is_allowed_info_url(
            "https://www.cwxu.edu.cn/info/1033/9001.htm"
        ));
        // 校方目录里合法、但正文抓取路径仍不放行的外部域 / 内网 IP（回归保护）
        assert!(!is_allowed_info_url("https://www.cnki.net/"));
        assert!(!is_allowed_info_url("http://10.3.100.110/"));
        assert!(!is_allowed_info_url("https://www.wanfangdata.com.cn/"));
        assert!(!is_allowed_info_url("https://cwxu.edu.cn.evil.com/"));
    }

    // ---------- extract_article ----------

    #[test]
    fn article_extracts_title_and_clean_html() {
        let d = extract_article(PAGE_FIXTURE, PAGE_URL).unwrap();
        assert_eq!(d.title, "示例通知标题");
        // 正常路径：needsBrowser=false、url 回填、html 为 Some
        assert!(!d.needs_browser);
        assert_eq!(d.url, PAGE_URL);
        let h = d.html.as_deref().expect("正常页 html 为 Some");
        // 保留正文文本与白名单结构
        assert!(h.contains("正文第一段"));
        assert!(h.contains("<p"));
        // script/style/iframe/form 整棵剔除（连同子内容）
        assert!(!h.contains("alert"));
        assert!(!h.contains("evil{"));
        assert!(!h.contains("iframe"));
        assert!(!h.contains("<form"));
        assert!(!h.contains("<input"));
        // ponytail: form 内段落一并丢弃是既定取舍——换取最简单的整棵剔除
        assert!(!h.contains("表单内段落"));
        // 事件属性与 style/class 丢弃
        assert!(!h.contains("onclick"));
        assert!(!h.contains("onerror"));
        assert!(!h.contains("style="));
        assert!(!h.contains("vsbcontent_start"));
        // 相对地址 → 绝对（../pic/pic1.jpg 相对 /info/1164/ → /info/pic/pic1.jpg）
        assert!(
            h.contains(r#"src="https://tw.cwxu.edu.cn/info/pic/pic1.jpg""#),
            "实际: {h}"
        );
        assert!(h.contains(r#"href="https://tw.cwxu.edu.cn/info/1033/1.htm""#));
        // 危险 scheme 链接地址被丢，锚文本保留
        assert!(!h.contains("javascript:"));
        assert!(h.contains("坏链接"));
        // 外站 http(s) 链接按 scheme 白名单保留（正文可含合法外链）；
        // 点击是否导航由前端容器拦截（InfoPanel 点击委托），后端不剥语义
        assert!(h.contains(r#"href="https://other.example/page""#));
        assert!(h.contains("外站链接"));
        // 实体转义回写正确（text 节点重新编码）
        let esc = extract_article(
            r#"<html><head><title>转义测试</title></head><body><div class="v_news_content"><p>a&lt;b & c</p></div></body></html>"#,
            PAGE_URL,
        )
        .unwrap();
        assert!(esc.html.as_deref().unwrap().contains("a&lt;b &amp; c"));
    }

    #[test]
    fn article_falls_back_to_title_tag_and_vsb_container() {
        // 无 h2、无 v_news_content：title 兜底 + [id^=vsb_content] 兜底
        let page = r#"<html><head><title>兜底标题_无锡学院</title></head>
<body><div id="vsb_content_99"><p>兜底正文</p></div></body></html>"#;
        let d = extract_article(page, PAGE_URL).unwrap();
        assert_eq!(d.title, "兜底标题_无锡学院");
        assert!(d.html.as_deref().unwrap().contains("兜底正文"));
    }

    #[test]
    fn article_error_paths() {
        // 无正文容器
        assert!(extract_article("<html><body><p>孤立段落</p></body></html>", PAGE_URL).is_err());
        // 容器存在但剔除后为空
        let empty = r#"<html><head><title>示例标题</title></head><body><div class="v_news_content"><script>x</script></div></body></html>"#;
        let e = extract_article(empty, PAGE_URL).unwrap_err();
        assert!(e.to_string().contains("正文内容为空"));
        // 无标题（h2 与 title 全缺）
        let notitle = r#"<html><body><div class="v_news_content"><p>正文</p></div></body></html>"#;
        assert!(extract_article(notitle, PAGE_URL).is_err());
        // page_url 非法
        assert!(extract_article(PAGE_FIXTURE, "::bad url::").is_err());
    }

    // ---------- is_auth_wall（依据主智能体 2026-09-18 实机 curl 结论构造，脱敏） ----------

    /// 鉴权开门页同构迷你页：标题「系统提示」（非敏感站点文案）。
    const AUTH_WALL_FIXTURE: &str = r#"<html><head><title>系统提示</title></head>
<body><div class="msg"><p>请登录后访问。</p></div></body></html>"#;

    #[test]
    fn auth_wall_detected_by_final_url_and_title() {
        // ① 重定向最终 URL 命中 auth 开门页路径（title 为何都判中）
        assert!(is_auth_wall(
            true,
            "https://www.cwxu.edu.cn/system/resource/code/auth/auth.htm",
            AUTH_WALL_FIXTURE
        ));
        assert!(is_auth_wall(
            true,
            "https://www.cwxu.edu.cn/system/resource/code/auth/auth.htm",
            "<html><head><title>任意标题</title></head><body></body></html>"
        ));
        // ② 页面标题精确「系统提示」（最终 URL 未命中时仍判中）
        assert!(is_auth_wall(
            true,
            "https://tw.cwxu.edu.cn/info/1164/2564.htm",
            AUTH_WALL_FIXTURE
        ));
        // ③ HTTP 非 2xx（body 未下载 → 空 html）
        assert!(is_auth_wall(
            false,
            "https://www.cwxu.edu.cn/info/1039/1.htm",
            ""
        ));
    }

    #[test]
    fn auth_wall_not_triggered_for_normal_pages() {
        // 正常文章页不误判
        assert!(!is_auth_wall(
            true,
            "https://tw.cwxu.edu.cn/info/1164/2564.htm",
            PAGE_FIXTURE
        ));
        // 正文文本偶含「系统提示」四字不误判（判定解析 <title> 而非子串搜索）
        let body_contains_words = r#"<html><head><title>示例公告标题</title></head>
<body><div class="v_news_content"><p>如遇系统提示页面打不开，请联系管理员。</p></div></body></html>"#;
        assert!(!is_auth_wall(
            true,
            "https://tw.cwxu.edu.cn/info/1164/2564.htm",
            body_contains_words
        ));
        // 空文档 / 无 title 不误判
        assert!(!is_auth_wall(true, "https://tw.cwxu.edu.cn/x.htm", ""));
    }
}
