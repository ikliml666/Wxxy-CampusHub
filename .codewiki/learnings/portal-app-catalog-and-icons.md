---
title: 应用目录、图标代拉与 appLink 校验分工（受保护资源与两种白名单）
type: learning
source_files:
  - crates/campus-portal/src/client.rs
  - crates/campus-portal/src/parse.rs
  - crates/campus-portal/src/article.rs
  - tauri-app/src-tauri/src/commands/portal.rs
tags:
  - portal
  - apps
  - icon
  - security
  - url-whitelist
---

# 应用目录、图标代拉与 appLink 校验分工（受保护资源与两种白名单）

2026-09-18 M2 批次 3（应用页）的两个实测结论：门户应用图标是**受保护的同源资源**（前端直连必裂图，须后端带会话代拉）；应用 `appLink` 的校验**不能沿用域名白名单**（30 个应用中 16 条在校园域外，会拦掉学校自己的合法应用）。由此定下「打开动作按协议白名单、抓取动作按域名白名单」的分工原则。

## 教训一：同源受保护资源（图标）必须后端代拉，且魔数判 MIME

应用图标走 `<base>/zuul/docrepo/download?attachmentId=<appIcon UUID>`——**门户同源、需会话 Cookie 的受保护资源**。前端（WebView 域）直连该地址拿不到门户会话上下文，得到裂图。

**方案**：后端 `PortalClient::app_icon_data_url`（`client.rs:230-276`）带会话代拉字节 → `guess_image_mime`（`parse.rs:654-668`）按**魔数**判 MIME → base64 编码为 `data:<mime>;base64,…` 填进 `iconUrl`。三个要点：

- **不信任响应 Content-Type**：文档库静态资源常给 `application/octet-stream`；魔数识别不出（如返回 HTML 错误页/登录跳转页）按「无图标」处理，**避免把错误页伪装成 data URL**。
- **缓存语义分级**（`client.rs:110-112`）：`Some(dataUrl)` 正常缓存；`None` = 服务端确认给的不是图片，**同样缓存**防每次刷新重试；传输类 Err 不缓存，下次目录刷新自动重试。附件 id 请求前校验 UUID 字符集。
- **降级不阻塞**：`query_app_catalog`（`client.rs:389-433`）把附件 id 去重后 `buffer_unordered(4)` 并发代拉，单图标失败降级 `iconUrl: null` + 前端占位图标，**目录永不因图标失败阻塞**。

通用教训：**受会话保护的二进制资源，前端跨域拿不到**——凡是「同源 + 需 Cookie」的资源（图标、附件、验证码图外的一切），代拉职责在后端；同时图标这类装饰性资源要与主数据解耦，失败只降级自己、不拖垮页面。

## 教训二：appLink 的校内外分布——域名白名单会拦掉学校自己的应用

2026-09-18 真机实测应用目录 30 条 `appLink`，**16 条在校园域外**：万方 4、虚拟图书馆（flyread）4、**一卡通 `10.3.100.110`（内网 IP）4**、超星泛雅 2、中国知网 2。若沿用批次 2 的 `*.cwxu.edu.cn` 域名白名单，这些学校自己的合法应用全部打不开——真机点「一卡通」在修前正是被「仅支持校园官网链接」拦住的。

修正：新增**协议白名单** `is_http_url`（`article.rs:45-58`，仅 http/https，拒绝 `file:`/`javascript:`/`data:` 等），`open_app`（`portal.rs:264-279`）用它。放行的安全前提（三个条件**同时**成立才敢放宽）：

1. URL 来自**校方应用目录**（服务端下发，非用户输入）；
2. 后端**不抓取**该 URL 的内容（无 SSRF 面）；
3. 只在**系统浏览器**打开（WebView 不承载该域内容，浏览器自带站点隔离与证书校验）。

## 教训三：原则——打开动作按协议白名单，抓取动作按域名白名单

两种校验语义不同、不可互换，源码注释与单测双向钉死：

| 校验 | 规则 | 适用 | 风险面 |
|---|---|---|---|
| `is_allowed_info_url`（`article.rs:23-33`） | `*.cwxu.edu.cn` 域名白名单 | 后端要**抓取内容**的 URL（`fetch_info_detail`）、`open_in_browser` | SSRF / 钓鱼（URL 可能被诱导） |
| `is_http_url`（`article.rs:45-58`） | 仅 http/https 协议白名单 | `open_app` 打开目录下发的 appLink | scheme 滥用（`file:`/`javascript:`） |

- **抓取路径的域名白名单不要放宽**——那是真正的 SSRF 防线；放宽的余地只存在于「不抓取 + 受信来源 + 系统浏览器打开」三条前提齐备的打开路径上。
- 判断放宽与否先问「这条 URL 谁提供、后端碰不碰它的内容、谁来渲染」——三个答案变了，白名单口径就要重估。
- 单测回归钉住：`open_in_browser_domain_whitelist_stays_tight`（`article.rs:380`）确认域名白名单未因 `open_app` 引入而放宽；协议白名单的放行/拒绝用例直接引用实测校内外域名形态。

## 遗留与关联

- `open_app` 的 **WebVPN B 类包装未做**（WebVPN 会话属 M4）：B 类应用（外购数据库、内网 IP 直链）校外访问由浏览器侧报错，届时按设计文档附录 A 的 A/B/C 分类接入打开策略引擎。
- 协议实现：[[modules/campus-portal|门户业务协议核心]]「应用目录与图标代拉」节；接线：[[modules/campus-hub-tauri|接线层]]「应用/日程命令与 open_app」节；前端消费：[[modules/frontend-shell|前端外壳]] AppsPanel 一节。
- 设计定稿：`docs/design/frontend-design.md` 附录 G2/G3。
