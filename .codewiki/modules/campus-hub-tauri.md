---
title: 接线层（campus-hub src-tauri）
type: module
source_files:
  - tauri-app/src-tauri/src/lib.rs
  - tauri-app/src-tauri/src/commands/auth.rs
  - tauri-app/src-tauri/src/commands/profile.rs
  - tauri-app/src-tauri/src/commands/portal.rs
  - tauri-app/src-tauri/src/commands/mod.rs
  - tauri-app/src-tauri/src/infra/state.rs
  - tauri-app/src-tauri/src/infra/mod.rs
  - tauri-app/src-tauri/src/account/crypto.rs
  - tauri-app/src-tauri/src/account/store.rs
  - tauri-app/src-tauri/tauri.conf.json
  - tauri-app/src-tauri/capabilities/default.json
tags:
  - tauri
  - ipc
  - dpapi
  - session
  - login
  - avatar
  - portal
  - opener
  - apps
  - schedule
---

# 接线层（campus-hub src-tauri）

`tauri-app/src-tauri`（crate 名 `campus-hub`）是协议核心与前端之间的 IPC 接线层：命令面、AppState、DPAPI 持久化。协议逻辑零实现——「协议核心全部在 campus-auth crate，本 crate 只做 IPC 接线与本地持久化」（`src/lib.rs:2`）。登录/账号命令定义在 `src/commands/auth.rs`，头像/资料命令定义在 `src/commands/profile.rs`，门户数据命令定义在 `src/commands/portal.rs`，全部注册于 `lib.rs:19-45`。

## 25 条命令面

| 命令 | 参数（camelCase） | data 形态 | 位置 |
|---|---|---|---|
| `get_captcha` | — | `{ uid, pngBase64 }`（pngBase64 为裸 base64，前端自行拼 data URI） | `auth.rs` |
| `login` | `account: { username, password }` | 成功 `{ username, displayName }`；三次识别穷尽 `{ uid, pngBase64 }` + message=`CAPTCHA_MANUAL` | `auth.rs` |
| `login_manual` | `account: { username, password, captchaUid, captchaCode }` | 同 login（手动单次提交不重试） | `auth.rs` |
| `login_saved` | `account: { username }` | 同 login（DPAPI 解密已存密码后复用自动流程） | `auth.rs` |
| `check_session` | — | `{ loggedIn: bool }` | `auth.rs` |
| `logout` | — | 无（`CommandResult::empty()`） | `auth.rs` |
| `list_accounts` | — | `{ accounts: [{ username, lastLogin, displayName? }] }`（密码绝不出现在返回里） | `auth.rs:479-494` |
| `remove_account` | `username: String` | 无（复用 `store::remove_account`，错误原文中文透传，`auth.rs:498-504`） | `auth.rs:498-504` |
| `get_avatar` | — | `{ imageBase64: string\|null, source: "local"\|"official"\|null }` | `profile.rs:162-167` |
| `set_avatar` | `imageBase64: String` | 同 AvatarData（空串/超 2MB → err） | `profile.rs:169-180` |
| `clear_avatar` | — | 同 AvatarData（只清本地，官方保留） | `profile.rs:182-191` |
| `sync_official_avatar` | — | 同 AvatarData（无会话 → err「请先登录」） | `profile.rs:193-209` |
| `upload_official_avatar` | `imageDataUrl: String` | 同 AvatarData（无会话 → err「请先登录」；data URL 非法/超 200KB → err；上传成功后重拉官方头像落盘，2026-09-18 新增） | `profile.rs:219-262` |
| `get_portal_overview` | — | `PortalOverview{ semester, wallet, nextCourse, fetchedAt }`，三个子项均可 null（无会话 → err「请先登录」；2026-09-18 M2 批次 1 新增） | `portal.rs:23-79` |
| `get_info_columns` | — | `InfoColumn[]`（后端固定 7 栏：订阅接口 + 实测全量兜底） | `portal.rs:86-97` |
| `get_info_list` | `columnId, page, pageSize` | `InfoPage`（total/pageCount 不可靠原样透传，前端满页判断分页） | `portal.rs:101-115` |
| `get_info_detail` | `url` | `InfoDetail{ title, html?, needsBrowser, url }` 三分类（正常 HTML / 鉴权门 `needsBrowser=true` 非错误 / 真错误；正文已由协议层白名单清洗，命令层不二次处理） | `portal.rs:119-131` |
| `get_todo_tabs` | — | `TodoTab[]`（接口 6 tab 全量透传，前端按契约展示三个） | `portal.rs:135-146` |
| `get_todo_list` | `tabId, page, pageSize` | `TodoPage`（tabId 白名单校验在协议层 `query_todo_list`） | `portal.rs:150-164` |
| `open_in_browser` | `url` | 无（白名单强制 `*.cwxu.edu.cn`，非法域名 err「仅支持校园官网链接」；2026-09-18 M2 批次 2 新增） | `portal.rs:183-192` |
| `get_app_catalog` | — | `AppCatalog{ groups, pinned }`（图标已由后端代拉为 data URL，失败条目 iconUrl 为 null；2026-09-18 M2 批次 3 新增） | `portal.rs:199-210` |
| `get_schedule_classify` | — | `ScheduleClassify[]`（5 类，会话内后端已缓存） | `portal.rs:214-225` |
| `get_schedule_month` | `startMs, endMs, codes` | `ScheduleEvent[]`（区间倒挂 err「日程区间无效」，前端 bug 防御；**M2 遗留项起 codes 含 `Default-Meeting` 时并入会议卡日程**——失败空贡献不影响课表，失败时 stderr 有 `[meeting-diag]` 打点） | `portal.rs:229-261` |
| `get_schedule_day_counts` | `startMs, endMs` | `ScheduleDayCount[]`（月视图角标；bs-schedule 计数接口无分类参数，计数为当日全量日程数；2026-09-18 M2 遗留项新增，命令数 24 → 25） | `portal.rs:264-281` |
| `open_app` | `url, isCas` | 无（**协议白名单** `is_http_url` 仅 http/https，非法 err「仅支持 http/https 链接」；`isCas` 契约保留字段、当前不影响打开策略——可达性提示由前端按 `AppItem.access` 分级给出） | `portal.rs:284-301` |

约定：业务失败一律 `Ok(CommandResult::err(中文消息))`，`Err(String)` 仅限 IPC 框架层错误（`auth.rs` 注释冻结此口径）。头像五命令统一返回 `AvatarData`（键恒在、值可 null，`profile.rs:33-38`）。

## 门户总览命令（`commands/portal.rs`，M2 批次 1）

`get_portal_overview`（`portal.rs:44-79`）聚合 [[modules/campus-portal|门户业务协议核心]] 的三个接口（学期 / 钱包卡 / 本周课表），协议细节全部下沉 campus-portal，本命令只做接线：

- **子字段失败互不阻塞**：三个查询各自 `.ok()` 置 null，任一失败不影响其余（前端回落空态/"—"，不整页报错）；`nextCourse` 由 `next_course_from_now` 从周课表推算，无课/失败为 null（前端隐藏横幅）。
- **无会话守卫**：锁内 clone `session.portal`（Arc 包装廉价，guard 在 await 前 drop）后取数，`None` → err「请先登录」（`ERR_NO_SESSION`，与 profile.rs 同口径）。
- 敏感纪律：JWT 与邮箱 `loginUrl` 在 campus-portal 内部消化，本命令只透出钱包数字与课程简报，不含任何凭据字段（`portal.rs:1-6` 模块文档）。

## 资讯/待办命令与浏览器打开（M2 批次 2，2026-09-18）

六个新命令与 `get_portal_overview` 的聚合模式不同，全部**单接口透传**：统一经 `portal_of` helper（`portal.rs:79-82`，锁内 clone portal 后取数）+ 协议错误 `e.to_string()` 映射为中文 message，无会话 → err「请先登录」（`ERR_NO_SESSION`，与 profile.rs 同口径）。

- **`open_in_browser`**：系统浏览器打开 URL，走官方 `tauri-plugin-opener` 的 Rust API（`lib.rs:16-18` 注册插件；只在 Rust 侧调用，**不开放前端直接 invoke 插件命令，无需额外 capability**）。白名单校验收敛在 `pub(crate) open_url_in_browser` helper 内部第一行（`portal.rs:173-180`，复用 [[modules/campus-portal|门户业务协议核心]] 的 `is_allowed_info_url`——**与正文抓取同一事实来源**），非法域名 err「仅支持校园官网链接」；批次 3 `open_app` 复用其打开方式但校验换成协议白名单（见下节，两种校验语义不同）。
- **`get_info_detail`**：透传协议层三分类（正常 HTML / `needsBrowser=true` 引导浏览器 / 真错误），`needsBrowser` 是正常返回非错误；正文已由 campus-portal 白名单清洗，命令层不做二次处理。背景见 [[learnings/cwxu-official-site-content-extraction|官网正文抓取与鉴权门降级]]。
- **CSP 配套**：`tauri.conf.json:26` 的 `img-src` 在 `'self' data:` 基础上增加 `https://*.cwxu.edu.cn http://*.cwxu.edu.cn`——内嵌正文官网图片显示的必要配套，域名仍限校园官网。

## 应用/日程命令与 open_app（M2 批次 3 + 遗留项会议并入）

四个新命令（`portal.rs:194-301`）延续批次 2 的单接口透传模式（`portal_of` helper + 无会话 err「请先登录」）；遗留项批次补 `get_schedule_day_counts`（命令数 24 → 25）并扩展 `get_schedule_month`：

- **`get_app_catalog`**：透传协议层 `AppCatalog{groups, pinned}`，图标 data URL 已在协议层代拉拼好，命令层零处理；`AppItem.access` 可达性分类由协议层按附录 A 实测表推导，命令层透传。
- **`get_schedule_classify` / `get_schedule_month`**：日程分类与区间明细透传；`get_schedule_month` 对 `endMs <= startMs` 直接 err「日程区间无效」（前端 bug 防御，不透传服务端）。**会议并入（M2 遗留项）**：课表明细 Ok 时 `extend(query_meetings_for_range(...))`——codes 含 `Default-Meeting` 才并入；会议链路任一环节失败为空贡献（**降级承诺：不影响课表日程与日历**），失败环节在协议层留 `[meeting-diag]` stderr 打点（只打失败，成功静默），命令层零打点。
- **`get_schedule_day_counts`**（遗留项新增）：月视图角标取数，区间倒挂防御同上；透传 bs-schedule `getCountBetweenTime`（**无分类参数，计数为当日全量**）。
- **`open_app(url, isCas)`**：校验用**协议白名单** `campus_portal::is_http_url`（仅 http/https），而非 `open_url_in_browser` 的域名白名单——该 URL 来自校方应用目录（受信来源）、后端不抓取它（无 SSRF 面）、只在系统浏览器打开；实测 30 条目录数据中 16 条为非校园域，域名白名单会把学校自己的合法应用全部拦掉。打开动作复用官方 `tauri-plugin-opener` Rust API。**`isCas` 是契约保留字段，当前不影响打开策略**（`let _ = is_cas`）——可达性提示由前端按 `AppItem.access` 分级给出（webvpn 提示后仍打开 / unavailable 只提示不打开）。⚠️ WebVPN B 类包装未实现（实测网关对未登录请求一律回落、明文包装无法验证，会话打通 + 包装 + A 类 CAS 直达签发归 M4）。分工原则与数据分布见 [[learnings/portal-app-catalog-and-icons|应用目录、图标代拉与 appLink 校验分工]]。

## 登录重试状态机

`run_login` 内核三命令共享（`auth.rs:220-297`），登录用**全新 CasClient**（干净 jar，避免旧会话 cookie 干扰，`auth.rs:228`），密码仅在内存中存续、立即 RSA 加密（`auth.rs:230`）。

- **自动模式**（`auth.rs:240-296`）：`kaptcha → captcha::solve → login` 循环，总提交 ≤`MAX_LOGIN_ATTEMPTS=3`（`auth.rs:27`）。重试决策纯函数 `should_retry`（`auth.rs:206-208`）：**仅 `WrongCaptcha` 重试**；`WrongUserOrPwd`（防 CAS 连续错误计数锁号）、`UserLocked`、`NeedTwoVerify`、`Unknown`、`Network` 一律立即终止（单测 `auth.rs:550-570`）。识别失败（solve 返回 None）未提交到 CAS、不消耗连续错误计数，刷新重试不计提交（`auth.rs:252-259`）。
- **穷尽转手动**（`auth.rs:275-294`）：返回 `success:false + message="CAPTCHA_MANUAL" + data={uid, pngBase64}`，前端据此切手动模式；穷尽前最后一次提交的 uid 可能已被 CAS 消耗，故先尽力刷新一张新验证码图、失败用最后一张兜底。
- **手动模式**（`auth.rs:233-239`）：单次提交不自动重试，验证码错误由用户刷新重输。
- **接近阈值保护**：`parse_error_count_hint` 从 TWOVERIFY 的 data 原文解析「已连续错误N次，阈值M」（`auth.rs:173-190`）；`n+1 >= m` 时错误消息明确提示已停止自动重试。

## LoginResultData：untagged 双形态

`#[serde(untagged)] enum LoginResultData { LoggedIn(LoginData), ManualNeeded(CaptchaData) }`（`auth.rs:101-105`）——序列化时内联，前端拿到裸对象：成功 = `{ username, displayName }`，穷尽 = `{ uid, pngBase64 }`。前端按字段收窄：`isLoginOk`（`"username" in data`）/ `isCaptchaPayload`（`"uid" in data`）类型守卫（`authStore.ts:36-43`）。

## 登录成功收尾（`finish_login` `auth.rs:300-351`）

`sso_follow(PORTAL_SERVICE)` 建门户会话 → **尽力取门户真实姓名**（`portal_user_profile`，失败只 warn 并回退上次已存 displayName、再回退学号——save_account 是整条 upsert，直接传 None 会把旧 displayName 抹掉，`auth.rs:312-330`）→ `save_account`(DPAPI，失败不阻断会话、降级日志 `auth.rs:332-334`) → `jar.snapshot()` 逐条 DPAPI 加密写 session.json（`auth.rs:336-339`）→ AppState 锁内同步赋值（guard 语句末 drop、此后无 await，`auth.rs:341-344`）。`displayName` 无来源时用 username（`auth.rs:349`）。姓名不进日志（敏感纪律）。

`check_session` 返回 false（无会话/已过期/探测失败）时清 AppState 会话与 session.json（`auth.rs:450-455`）。`logout`：`GET {CAS_BASE}/logout` 尽力而为（5s 超时，复用 `sso_follow` 发 GET；REST 登录无 CASTGC，服务端可能本就无全局会话），随后本地清理必然执行（`auth.rs:461-475`）。`session_client` 是取会话 client 的唯一入口（锁内 clone，`auth.rs:357-364`），profile 模块的 `sync_official_avatar` 同样取用。

## DPAPI 裸 FFI（`account/crypto.rs`）

Windows `CryptProtectData` / `CryptUnprotectData`（CurrentUser 作用域，跨用户不可解密）以 `extern "system"` + `#[link(name = "crypt32")]` 裸 FFI 实现（`crypto.rs:16-37`），**零新依赖**（不引入 windows crate），拷贝自参考项目 Wxxy-CampusLogin 同名模块（`crypto.rs:3-4`）。要点：

- `call_dpapi` 通用 helper 统一 DataBlob 构造、结果检查与 `LocalFree` 释放；失败路径判空后同样释放输出缓冲（`crypto.rs:46-78`）。
- 对外只暴露两个字符串接口：`dpapi_protect(明文) -> DPAPI 密文 base64`、`dpapi_unprotect(密文 base64) -> 明文`（`crypto.rs:121-139`）。
- 非 Windows 编译期桩返回 Err「加密存储仅桌面端支持」（`crypto.rs:142-150`；安卓阶段 2 换 Android Keystore）。

## 存储位置与格式（`%APPDATA%/campushub/`，`infra/state.rs:34-38`）

| 文件 | 结构 | 写入方 |
|---|---|---|
| `session.json` | `{ username, cookies: [{ name, valueB64(DPAPI) }] }`（`state.rs:44-57`） | `persist_session`（`state.rs:60-80`）；读 `load_session`（单个 cookie 解密失败跳过，`state.rs:83-96`）；删 `clear_session` |
| `accounts.json` | `{ accounts: [{ username, passwordB64(DPAPI), lastLogin(epoch 毫秒串), displayName? }] }`（`store.rs:13-29`） | `save_account`（同 username upsert 覆盖，`store.rs:43-62`）、`remove_account`（不存在报错，`store.rs:81-89`） |
| `profile.json` | `{ localBase64?, officialBase64?, officialFetchedAt? }`（camelCase，字段缺省即不存在，`profile.rs:41-52`）——**明文 base64，不走 DPAPI** | `store_local_avatar` / `clear_local_avatar` / `store_official_avatar`（`profile.rs:127-153`）；读 `read_profile`（文件缺失/损坏按空档处理，`profile.rs:62-67`） |

启动回填 `restore_session()`（`state.rs:107-122`）：`run()` 在 `manage` 之前调用（避免 setup 内碰 tokio Mutex，`lib.rs:11-13`），读 session.json → 解密 → `jar.restore` 回填；文件缺失/损坏/cookies 空 → None。落盘内容不含明文凭据有单测断言（`state.rs:141-143`）。`CasSession` 自 2026-09-18 起挂 `portal: PortalClient`（`state.rs:16-21`，M2 批次 1）——`finish_login`（`auth.rs:341-345`）与 `restore_session`（`state.rs:117-121`）两处构造均 `PortalClient::new(client.clone())` 共享同一 jar，缓存生命周期 = 会话生命周期（详见 [[modules/campus-portal|门户业务协议核心]]）。

## 头像存取、官方同步与上传学校（`commands/profile.rs`）

头像命令全部只做接线与本地存取，协议拉取/上传复用 `CasClient` 的门户资料接口（见 [[modules/campus-auth|CAS 协议核心]]「门户资料接口」）：

- **生效优先级：本地 > 官方 > 无**，纯函数 `current_avatar` 统一裁决（`profile.rs:78-97`）；`clear_avatar` 只清本地、官方保留（回落展示，`profile.rs:182-191`）。
- **落盘即明文**：头像不是凭据，base64 明文写 `profile.json`，不经 DPAPI（`profile.rs:5-6` 注释；取舍见 [[decisions/guest-mode-account-shell|游客优先与账号外壳决策]]）。
- **本机体积守卫**：`set_avatar` 空串/超 `AVATAR_MAX_B64=2MB` 拒绝，冻结文案「本机头像过大（上限 2MB）」（`profile.rs:23,98-105`；2026-09-18 由 512KB 放宽到 2MB，前端裁切器按同阈值预检）。
- **`sync_official_avatar`**：无会话 → 约定错误文案「请先登录」（`ERR_NO_SESSION`，前端据此引导登录，`profile.rs:28,195-199`）；有会话 → 锁纪律 `session_client` clone 出 client 后发请求，成功落盘 `officialBase64 + officialFetchedAt`，网络/解析失败不落盘、不清已有头像（`profile.rs:193-209`）。
- **`upload_official_avatar(imageDataUrl)`**（2026-09-18 新增，`profile.rs:219-262`）：把裁切后的头像上传回学校系统。链路：无会话直接 err「请先登录」→ `validate_official_data_url` 校验（须 `data:image/` 开头、剥前缀后裸 base64 ≤ `OFFICIAL_AVATAR_MAX_B64=200KB`，`profile.rs:26,108-122`——服务端**原样存储不压缩**，守卫只能本端做）→ `CasClient::portal_change_portrait` 上传（内部现取 JWT/ids 并现算 csrf，见 [[learnings/portal-avatar-upload-protocol|门户头像上传协议]]）→ 成功后重新 `portal_login_info` 拉官方头像落盘 → 返回最新 `AvatarData`（以服务端回读为准）。日志只打码用户名（`profile.rs:264-273`，与 `auth.rs::mask_username` 同款），**绝不打印 data URL**（体积可达数百 KB）。
- 单测覆盖优先级轮转（官方 → 本地覆盖 → 清本地回落官方 → 全空双 null）、2MB/200KB 校验与冻结文案、data URL 非法形态、无会话文案契约（`profile.rs:275-389`）。

## 最小权限

`capabilities/default.json` 仅声明 `permissions: ["core:default"]`、仅 `main` 窗口——不给多余能力。opener 插件（`tauri-plugin-opener = "2"`）只在 Rust 侧经 `OpenerExt` 调用（`portal.rs:176-178`），不开放前端直接 invoke 插件命令，故 capabilities 无需追加条目。CSP（`tauri.conf.json:26`）：`connect-src 'self' ipc://localhost`；`img-src 'self' data:` 之上，M2 批次 2 为内嵌正文官网图片增加 `https://*.cwxu.edu.cn http://*.cwxu.edu.cn`（域名仍限校园官网）。

## 离线单测（`commands/auth.rs:506-626` + `commands/profile.rs:275-389`）

错误码→中文消息映射、重试状态机、计数提示解析与接近阈值文案、`login_saved` 解密失败路径（坏密文/账号不存在，不发起网络请求）、用户名打码（`mask_username`：前 2 位 + 末位，`auth.rs:157-165`）；profile 侧头像优先级轮转、2MB/200KB 体积校验（含 data URL 非法形态）、无会话文案契约。`cargo test --workspace` 全量 **102 passed / 3 ignored**（2026-09-18 M2 遗留项批次校验；campus-auth 26 + campus-hub 15 + campus-schedule 12 + campus-portal 49，另有 3 个 ignored 待真机样本/凭据）。
