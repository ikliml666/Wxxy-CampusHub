---
title: 门户业务协议核心（campus-portal）
type: module
source_files:
  - crates/campus-portal/src/lib.rs
  - crates/campus-portal/src/client.rs
  - crates/campus-portal/src/parse.rs
  - crates/campus-portal/src/article.rs
tags:
  - portal
  - http
  - jwt
  - parse
  - timetable
  - scraper
---

# 门户业务协议核心（campus-portal）

`crates/campus-portal` 是协议单点第四员（与 [[modules/campus-auth|CAS 协议核心]] 平级，**无 Tauri 依赖**，安卓可复用）：以门户前端同款请求头组调用门户业务接口（学期 / 钱包卡 / 周课表 / 资讯 / 待办），把响应解析成供 IPC 层透传的 DTO；官网资讯正文的抓取与清洗在独立 article 模块完成。协议细节（接口全表、请求头来源、栏目 id↔名称、校本作息实测）见 `docs/superpowers/plans/2026-09-18-m2-portal-pages.md` §1.1–§1.3。模块文档明示敏感纪律：网关 JWT 与邮箱 `loginUrl`（内含 authkey）只在内存使用，不落盘、不写日志、不进文档、不返回前端（`lib.rs:1-9`）。

## 会话复用方式（不重建登录）

- `PortalClient { cas: CasClient, auth: Arc<Mutex<Option<AuthHead>>> }`（`client.rs:58-65`）：内部持已登录 `CasClient`，clone 共享同一 Cookie jar——**不复制会话/登录管理**，这是计划 §2.2 的冻结约定。
- 挂载点：`CasSession.portal`（`tauri-app/src-tauri/src/infra/state.rs:16-21`），`restore_session` 与会话重建时 `PortalClient::new(client.clone())` 构造（`state.rs:109-121`）——缓存生命周期 = 会话生命周期，登录/登出/会话替换时整体随会话丢弃，无需单独失效逻辑；防御性 `clear_auth()`（`client.rs:92-96`）。
- JWT/资料内存缓存 `AuthHead { token_id, user_id, org_id }`（`client.rs:50-55`）：缓存命中直接用，未命中调一次 `portal_user_profile()`（tryLoginUserInfo）入缓存（`client.rs:99-116`）。**`AuthHead` 故意不派生 Debug**（JWT 字段防误记日志）；std Mutex guard 在 await 前 drop、不跨 await 持锁，锁中毒按缓存未命中处理。取不到 JWT（会话失效）→ `PortalError::NotLogin`（"请先登录"）。
- `org_id` 缺省回填 `"-1"`（学生实测值，`client.rs:110`）。

## 请求头方案（统一 helper）

私有 `get(endpoint)`（`client.rs:120-152`）统一注入计划 §1.1 全部头：`Authorization`（JWT，**无 Bearer 前缀**）、`loginUserId`/`loginUserName`（同为学号）、`loginUserOrgId`、`appid: ly-upp`、`csrfTimestamp`（毫秒时间戳）+ `csrfToken`（**直接复用 `campus_auth::cas::csrf_token`，不重写**）、`X-Requested-With`、`Accept`；每请求 10s 超时，URL 附 `_t` 时间戳防缓存。门户 base 复用 `campus_auth::cas::PORTAL_PROBE`（`cas.rs:23`，批次 1 提升为 pub），不设第二事实来源。为此 `CasClient` 新增 `http_client()` getter（`cas.rs:134`）。计划约定 POST 需另加 `Content-Type`；批次 1 三接口与批次 2 五接口均为 GET，POST helper 待批次 3 日程接口（`bs-schedule/*`）引入。

## 公开 API

`lib.rs` 导出（全部 camelCase serde）：

- **`PortalError`**：`Http(reqwest)` / `NotLogin("请先登录")` / `Parse(String)`——HTTP 失败以状态码文本归入 Parse 通道（`client.rs:146-150`），错误风格与 campus-auth 一致（中文 message、不 panic）。
- **`SemesterInfo`** / **`WalletSummary`** / **`CourseBrief`** / **`WeekSchedule`**（批次 1，字段见 [[learnings/portal-block-periods-and-school-timetable|门户大节语义与校本作息]] 与源码）。
- **批次 2 资讯**：`InfoColumn { id, name, sortNum }` / `InfoItem { id, title, columnTitle, publishTime, dept: Option, url }` / `InfoPage { page, pageSize, pageCount, total, items }`（`lib.rs:82-116`）。⚠️ `total`/`pageCount` **实测不可靠**（pageSize=1 时返回 0），原样透传，**前端分页以 items.length == pageSize 判断**（`lib.rs:106-107` 注释冻结口径）；`InfoItem.url` 为官网正文页 URL（接口 `extLink`）。
- **批次 2 `InfoDetail { title, html: Option, needsBrowser, url }`**（`lib.rs:121-132`）：计划 §2.1 `{title, html}` 的兼容扩展，三分类结果——正常返回清洗后 HTML；`needsBrowser=true` 表示正文受官网鉴权保护（**正常返回，不是错误**），前端引导浏览器打开 `url`；网络/解析异常才走 Err。
- **批次 2 待办**：`TodoTab { id, name, desc, count }` / `TodoItem { id, title, applicant, applyTime, source, node, urgency }` / `TodoPage`（`lib.rs:135-168`）。⚠️ 接口实际返回 6 个 tab（todo/done/apply/unread/read/focus），全量透传，前端按契约展示三个；`TodoItem` 字段形态**未实测**（账号无待办数据，`lib.rs:144-146` 注释），按多候选键宽松映射，真机出现数据后需校准。
- 客户端方法：批次 1 三查询 `query_semester_info` / `query_wallet_summary` / `query_week_schedule`（`client.rs:155-167`）；批次 2 五查询 `query_info_columns`（`client.rs:170-172`）/ `query_info_list`（`client.rs:175-189`，`columnId` 拼接查询串前经 `is_safe_id` ASCII 字母数字校验，`client.rs:44-46`）/ `query_todo_tabs`（`client.rs:192-194`）/ `query_todo_list`（`client.rs:197-214`，`tabId` 白名单六值校验）/ `fetch_info_detail`（`client.rs:228-258`）。端点常量 `EP_*`（`client.rs:26-40`）。

## 正文抓取与清洗（article.rs，批次 2）

官网正文页是公开静态页，抓取**不走统一 `get()` helper**：`fetch_info_detail`（`client.rs:228-258`）用裸 `http_client` 直接请求 `extLink`，**不带任何门户鉴权头**——JWT 只发门户同源，绝不随正文抓取发往其他域名（`client.rs:226-227` 注释冻结此口径）。链路三步：

1. **域名白名单** `is_allowed_info_url`（`article.rs:21-31`）：scheme 仅 http/https + host 精确后缀匹配 `cwxu.edu.cn` 及子域，防把用户可控 URL 透传给 HTTP 客户端（SSRF/钓鱼，计划红线 3）；拒绝 `cwxu.edu.cn.evil.com`、`cwxu.edu.cn@evil.com`、query 参数藏白名单域名等混淆形态。接线层的 `open_url_in_browser` helper 复用同一函数——浏览器打开与正文抓取同一事实来源。背景与绕过形态分析见 [[learnings/cwxu-official-site-content-extraction|官网正文抓取与鉴权门降级]]。
2. **鉴权门判定** `is_auth_wall(status_success, final_url, page_html)`（`article.rs:157-169`）：三依据——① HTTP 非 2xx；② 重定向**最终 URL**（reqwest 自动跟随后的 `resp.url()`，`client.rs:243`）命中 `/system/resource/code/auth/auth.htm`；③ 页面 `<title>` **精确等于**「系统提示」。③ 刻意解析 `<title>` 而非全文子串搜索——正文文本偶含这四个字不误判（有回归单测）。命中 → 正常返回 `needs_browser=true`（**不是错误**），`content.jsp` 系栏目为何如此见 [[learnings/cwxu-official-site-content-extraction|官网正文抓取与鉴权门降级]]。
3. **提取与清洗** `extract_article`（`article.rs:175-209`）：标题 `h2` 优先、`<title>` 兜底；容器 `div.v_news_content` 优先、`[id^=vsb_content]` 兜底（博达 webplus 标准结构）；`render_subtree`（`article.rs:216-280`）按**标签/属性白名单重建** HTML 片段——危险标签（script/style/iframe/form/svg/math 等，`is_dropped_tag`）以 DFS Open/Close 配对的抑制计数整棵剔除；白名单外无害标签解包（保留子内容、不输出标签本身）；属性只留白名单表（`keep_attr`，`article.rs:140-147`），`on*`/style/class 天然不在表内；`src`/`href` 相对地址转绝对且仅保留 http/https（图片额外放行 `data:image/`，`sanitize_url`，`article.rs:128-137`）；文本节点与属性值重建时重新转义（scraper 已解码实体，`&` 最先替换）。无标题/无容器/清洗后为空 → `PortalError::Parse`。清洗在后端完成，前端直接渲染不再二次处理。

新增后端依赖：`scraper` 0.27 + `ego-tree` 0.11（HTML 解析与树遍历；手写 tokenizer 不可靠，弃）。

## 解析纯函数（parse.rs）

解析全部为 `parse_*(body: &str) -> Result<T, PortalError>` 纯函数（网络与解析彻底分离，单测不碰网络）：

- `parse_semester_info`（`parse.rs:52`）：门户信封 `meta.success` 判定 + 逐字段校验。
- `parse_wallet_summary`（`parse.rs:81`）：钱包卡 `data.data` 是**内嵌 JSON 字符串**，二次 parse 后取数组首项；`loginUrl`（邮箱免密链接，内含 authkey，**等同凭据**）以「结构体不定义该字段」方式直接丢弃。
- `parse_week_schedule`（`parse.rs:126`）→ `WeekSchedule`。
- 「下一节课」链路：`elapsed_slot_count`（`parse.rs:190`）→ `next_course`（`parse.rs:231`）→ `next_course_from_now`（含跨天与周末守卫，`parse.rs:260`）；单格拆分 `course_from_cell`（四段拆分，**第 4 段任课教师名按契约丢弃**）；校本大节表 `block_time_slots()`——完整口径见 [[learnings/portal-block-periods-and-school-timetable|门户大节语义与校本作息]]。
- **批次 2 追加**：
  - `parse_info_columns`（`parse.rs:314-348`）：订阅接口只返回当前账号订阅的栏目（实测 3 个），以实测全量常量 **`KNOWN_COLUMNS`**（7 栏 id↔名称，`parse.rs:275-283`）兜底补全——未订阅项按实测全量顺序垫底、`sortNum` 用 1000+序号保序占位；`titleLocale` 实测为 JSON 字符串 `{"zh_CN":...}`，`locale_zh`（`parse.rs:287-301`）宽松兼容字符串/对象两种形态、zh_CN 缺失取任意非空值。
  - `parse_info_list`（`parse.rs:355-386`）：`infoId` 或 `extLink` 缺失的条目跳过（无 id 无法标记已读、无 url 无法打开正文），其余字段缺失降级空串；分页字段经 `jnum_u32` 宽松 u32、原样透传。
  - `parse_todo_tabs`（`parse.rs:392-411`）：6 tab 全量透传；`selected`（筛选项定义）不在契约内，忽略。
  - `parse_todo_list`（`parse.rs:426-459`）：信封 `data` 内层又是 `data:[]` 条目数组；字段经 `todo_item_field`（`parse.rs:416-420`）按候选键序取第一个非空值（候选键为门户系统常见命名，真实字段形态未实测）；id 映射不出的条目跳过（前端列表需要稳定 key）。

## 测试（27 个，全离线脱敏 fixture）

批次 1 的 13 个（`parse.rs`，含回归用例 `next_course_maps_column_pair_to_block_start` 钉「列对→大节起始时刻」口径）+ 批次 2 新增 14 个：`parse.rs` 8 个（栏目兜底补全与排序、`titleLocale` 宽松形态、列表过滤与降级、待办多候选键与空数据等）；`article.rs:282-456` 6 个——白名单放行与绕过形态（`cwxu.edu.cn.evil.com` / `cwxu.edu.cn@evil.com` / `ftp:` / `file:` / `javascript:` / query 藏域名）、正文提取与清洗断言（危险标签整棵剔除、事件属性/style 丢弃、相对转绝对、实体回写、外站 http(s) 链接保留而导航由前端容器拦截）、兜底选择器（无 h2 用 title、无 v_news_content 用 `[id^=vsb_content]`）、错误路径、auth wall 三依据判定与正常页不误判（含「正文含『系统提示』四字不误判」回归）。fixture 全部用占位值，真实姓名/学号/邮箱/authkey/JWT 不进测试样本。

接线方式见 [[modules/campus-hub-tauri|接线层]] 的 `get_portal_overview` 与批次 2 的资讯/待办/浏览器打开命令；前端消费见 [[modules/frontend-shell|前端外壳]] InfoPanel/TodoPanel 两节。
