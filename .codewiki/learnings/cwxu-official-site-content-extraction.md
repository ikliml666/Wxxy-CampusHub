---
title: 官网正文抓取与鉴权门降级设计（content.jsp 系不可抓）
type: learning
source_files:
  - crates/campus-portal/src/article.rs
  - crates/campus-portal/src/client.rs
  - crates/campus-portal/src/parse.rs
  - tauri-app/src-tauri/src/commands/portal.rs
tags:
  - portal
  - scraper
  - security
  - pitfall
---

# 官网正文抓取与鉴权门降级设计（content.jsp 系不可抓）

2026-09-18 M2 批次 2 侦察与实现结论：门户资讯的正文都在学校官网静态页（接口字段 `extLink`），但官网**两类正文 URL 形态抓取能力不同**，据此定下 `InfoDetail` 三分类契约。

## 两类正文 URL 形态

- **`/info/<栏目>/<id>.htm`**（校园要闻/校园快讯/教务处/学工处/团委五栏）：公开静态页，可正常抓取。正文页为博达 webplus 系统，容器命名在三站实测一致：外层 `#vsb_content_*`（tw/jwc/xgc 实测 `vsb_content_501/6/2`）、内层 `div.v_news_content`；标题 `h2` 唯一、`<title>` 兜底可用。
- **`content.jsp?urltype=news.NewsContentUrl&wbtreeid=...&wbnewsid=...`**（通知公告 columnId 9、规章制度 5d2c45d23866497cb2bfe93e9f136bb2 两栏）：**被站点鉴权开门页拦截**——无论带不带 Cookie/UA/Referer 重放，最终都停在 `/system/resource/code/auth/auth.htm`（页面标题「系统提示」）；且该系文章**没有 `/info/` 替代形式**（404）。这不是登录态问题，是站点侧对这类栏目的访问控制，客户端无解——所以不能把它当错误处理，只能降级。

## 由此得出的三分类降级设计

抓正文的结果不该是「成功/失败」二分，而是三分类（`InfoDetail { title, html, needsBrowser, url }`）：

1. **正常 HTML**：清洗后内嵌渲染。
2. **`needsBrowser=true`（正常返回，不是错误）**：`is_auth_wall` 三依据判定命中——HTTP 非 2xx / 重定向**最终 URL** 命中 auth.htm（reqwest 自动跟随后的 `resp.url()`）/ `<title>` 精确等于「系统提示」。前端引导「在浏览器打开原文」，**不进错误态不重试**（站点侧拦截与网络无关，重试只会白转）。第三个依据刻意解析 `<title>` 而非全文子串搜索——正文文本偶含「系统提示」四字不误判（有回归单测 `auth_wall_not_triggered_for_normal_pages`）。
3. **真错误**：网络/解析异常，错误态可重试。

## 安全要点（正文抓取特有的三条）

- **正文抓取用裸 HTTP client，不带门户鉴权头**：正文页是公开静态页无需登录；若复用统一 `get()` helper，会把网关 JWT、学号等头随请求发到 `extLink` 所在域名——凭据泄漏面。JWT 只发门户同源（`client.rs::fetch_info_detail` 注释冻结此口径）。
- **域名白名单必须 host 精确后缀匹配**：`host == "cwxu.edu.cn" || host.ends_with(".cwxu.edu.cn")` 能同时拒绝 `cwxu.edu.cn.evil.com`（前缀混淆）与 `cwxu.edu.cn@evil.com`（userinfo 混淆）两种绕过形态；scheme 白名单拒绝 `ftp:` / `file:` / `javascript:`；仅看 URL 字符串含不含域名会漏掉这两种混淆。同一函数被浏览器打开 helper（`open_url_in_browser`）复用——正文抓取与浏览器打开同一事实来源。
- **HTML 清洗选按白名单重建而非黑名单过滤**：scraper（HTML 解析）+ ego-tree（树遍历）按保留标签/属性白名单重建片段，白名单外内容一律不输出，`on*`/style/class 天然不在表内；危险标签（script/iframe/form 等）连同子树整段剔除。手写 tokenizer 不可靠，弃；`data.data` 式「内嵌 JSON 字符串」之外，官网还常见「HTML 实体已解码的 DOM 树」，重建时文本/属性值必须重新转义（`&` 最先替换）。

## 关联

- 实现与单测：[[modules/campus-portal|门户业务协议核心]] 「正文抓取与清洗」一节
- 前端三分类消费与 needsBrowser 分支：[[modules/frontend-shell|前端外壳]] InfoPanel 一节
- 设计定稿：`docs/design/frontend-design.md` 附录 F2/F3
- 接口全表与栏目 id↔名称：`docs/superpowers/plans/2026-09-18-m2-portal-pages.md` §1.2
