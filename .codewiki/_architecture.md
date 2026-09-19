---
title: 系统架构总览
type: architecture
source_files:
  - tauri-app/src-tauri/src/lib.rs
  - tauri-app/src-tauri/src/commands/auth.rs
  - tauri-app/src-tauri/src/commands/portal.rs
  - tauri-app/src-tauri/src/infra/state.rs
  - tauri-app/src-tauri/src/account/crypto.rs
  - tauri-app/src-tauri/src/account/store.rs
  - crates/campus-auth/src/lib.rs
  - crates/campus-schedule/src/lib.rs
  - crates/campus-portal/src/lib.rs
  - crates/campus-portal/src/article.rs
  - crates/campus-portal/src/access.rs
  - crates/campus-synjones/src/lib.rs
  - crates/campus-synjones/src/sso.rs
  - crates/campus-synjones/src/ecard.rs
  - crates/campus-synjones/src/charge.rs
  - tauri-app/src-tauri/src/commands/synjones.rs
  - tauri-app/src-tauri/src/commands/electricity.rs
  - tauri-app/frontend/src/shared/tauriApi.ts
  - tauri-app/frontend/src/shared/types.ts
  - tauri-app/frontend/src/stores/authStore.ts
  - tauri-app/frontend/src/App.tsx
tags:
  - architecture
  - tauri
  - ipc
  - session
  - portal
---

# 系统架构总览

锡院助手（Wxxy-CampusHub）= 自研客户端重写学校融合门户（my.cwxu.edu.cn）与内网慧新E校（10.3.100.110）核心能力，Windows 桌面先行、安卓跟进。技术栈与参考项目 Wxxy-CampusLogin 完全同款：Tauri 2（Rust 后端 + React 19/TypeScript 前端），沿用其「**协议单点 + 平台外壳**」双端同构架构（`PLAN.md:11`）。

## 双端同构：三层结构

```
┌─ tauri-app/frontend/src（React 19 WebView）
│    UI 壳 + zustand store；唯一 IPC 出口 shared/tauriApi.ts
│         │  invoke(CommandResult{success,message?,data?})
├─ tauri-app/src-tauri（平台外壳，crate 名 campus-hub）
│    命令面 / AppState / DPAPI 持久化；不含协议逻辑
│         │  直接函数调用
└─ crates/（协议单点，无 Tauri 依赖）
     campus-auth：CAS 登录协议  campus-schedule：课表领域核心  campus-portal：门户业务协议
     campus-synjones：慧新E校协议（一卡通 / 电费，M3 新增）
```

- **协议单点**：`crates/campus-auth/src/lib.rs:1` 明示「无 Tauri 依赖，安卓可复用」；`crates/campus-schedule` 同理仅依赖 serde/chrono/thiserror（各自 Cargo.toml）；`crates/campus-portal`（2026-09-18 M2 批次 1 新增）复用 campus-auth 的已登录 `CasClient`（clone 共享 Cookie jar），门户业务接口调用与解析全在此（[[modules/campus-portal|门户业务协议核心]]）。安卓端届时以 Cargo path 依赖引用这些 crate（`PLAN.md:15`），协议只写一份。**`crates/campus-synjones`（2026-09-19 M3 新增）**同样无 Tauri 依赖：慧新E校（内网 `10.3.100.110`）的 lyCas 桥换 token、一卡通卡信息与流水、电费三级级联全在此 crate，鉴权头组与三套响应信封与门户/教务完全隔离（[[modules/campus-synjones|慧新E校协议核心]]）。
- **平台外壳**：`tauri-app/src-tauri/src/lib.rs:2` 明示「协议核心全部在 campus-auth crate，本 crate 只做 IPC 接线与本地持久化」。
- **前端**：不感知协议细节，只消费 [[modules/campus-hub-tauri|接线层]] 的命令面 DTO。

## IPC 契约（冻结）

- 所有命令返回 `CommandResult<T> = { success, message?, data? }`，Rust 侧定义在 `tauri-app/src-tauri/src/commands/auth.rs:38-45`，`skip_serializing_if = "Option::is_none"` 使缺省字段与 TS `?:` 可选语义一致；TS 侧镜像定义在 `tauri-app/frontend/src/shared/types.ts:1`。
- 前端**唯一出口** `invokeCommand`（`tauri-app/frontend/src/shared/tauriApi.ts:8-17`）：所有 `invoke` 必须经过它；invoke 抛错（框架层错误）被包装为 `{ success:false, message:String(e) }`，前端拿到的永远是 CommandResult 形态，不处理异常分支。
- 业务失败走 `Ok(CommandResult::err)`，`Err(String)` 仅限 IPC 框架层错误（`commands/auth.rs:212` 注释冻结此口径）。
- 全部 DTO camelCase（`#[serde(rename_all = "camelCase")]`），与前端字段逐字对齐。

## 进程/线程模型与锁纪律

- Tauri 2 同进程双端：前端逻辑在 WebView，Rust 命令在 tokio 运行时上执行（`async fn` 命令）。
- `AppState.session` 是 `tokio::sync::Mutex<Option<CasSession>>`（`infra/state.rs:20-23`）——std Mutex 守卫非 Send、跨 await 编译不过（state.rs:3-5 记录此教训，源自 Wxxy-CampusLogin `commands/login.rs:87`）。
- **锁纪律**：锁内只 clone 出 client（`reqwest::Client` 为 Arc 包装，clone 廉价），drop guard 后再 await（`infra/state.rs:4-5`；落地在 `commands/auth.rs:357-364` 的 `session_client`）。唯一整段持锁写是 `finish_login` 的单条同步赋值，guard 在语句末 drop，此后无 await（`commands/auth.rs:341-344`）。
- `CasClient` 本身 `#[derive(Clone)]`（`crates/campus-auth/src/cas.rs:29`），clone 共享同一 RecordingJar，保证同一会话 cookie 连续性。

## 会话流（登录 → 捕获 → 持久化 → 重启回填）

1. **登录**：前端 `login` → `run_login`（自动验证码识别，重试 ≤3）→ `finish_login`（`commands/auth.rs:300-351`）。
2. **捕获**：登录用全新 CasClient（干净 jar，`commands/auth.rs:228`）；`sso_follow(PORTAL_SERVICE)` 建门户会话，302 链落点的 Set-Cookie 全部种进 RecordingJar（`cas.rs:183-231`）。
3. **持久化**：`jar.snapshot()` 取 `(名, 值)` 快照 → 逐条 DPAPI 加密 → `%APPDATA%/campushub/session.json`（`infra/state.rs:60-80`）；密码另经 DPAPI 存 `accounts.json`（`account/store.rs:43-62`）。落盘内容不含任何明文凭据（`state.rs:141-143` 单测断言）。
4. **重启回填**：`run()` 在 `manage` 之前跑 `restore_session()`（`lib.rs:13-15`）：读 session.json → DPAPI 解密 → `jar.restore` 按门户域 `https://my.cwxu.edu.cn/` 回填（`jar.rs:15,45-61`）。
5. **启动检测**：App 挂载后非阻塞调 `checkSession`（`void` 发出不挡渲染，`App.tsx:8-12`）→ Rust 侧 `portal_probe`：jar 有 `customsid` + 门户首页未被弹回 CAS 域 + **`tryLoginUserInfo` 响应信封 `meta.success==true`** = Alive（第三条判据 2026-09-19 补：门户用 **HTTP 200 + `data:null` + `meta.statusCode=302`** 表达会话失效，前两条对死会话同样成立，曾导致「显示已登录却点什么都报解析失败」，详见 [[learnings/portal-session-expiry-200-envelope|门户失效是 200 信封]]）；失败即清 AppState 与 session.json（`commands/auth.rs:443-457`）。判据取舍详见 [[decisions/recording-jar-session|RecordingJar 与会话持久化决策]]。**外壳为游客优先**：未登录不渲染登录页，壳与面板恒渲染、登录收进弹层与账号菜单（[[decisions/guest-mode-account-shell|游客优先决策]]）。

## 命令面与模块地图

命令面共 **77 条**（2026-09-19 M4.5 实测：auth 8 / profile 5 / portal 12 / timetable 24 / synjones 3 / ecard 7 / electricity 12 / electricity_history 6），全部注册于 `lib.rs` 的 `generate_handler!`。登录/账号 8 条（`get_captcha` / `login` / `login_manual` / `login_saved` / `check_session` / `logout` / `list_accounts` / `remove_account`）+ 头像 5 条（`get_avatar` / `set_avatar` / `clear_avatar` / `sync_official_avatar` / `upload_official_avatar`——最后一条 2026-09-18 新增，把裁切后的头像经门户 `portraitChange` 上传回学校，协议细节见 [[learnings/portal-avatar-upload-protocol|门户头像上传协议]]）+ 门户数据 12 条（`get_portal_overview` 聚合学期/钱包/下一节课且子字段失败互不阻塞，批次 1；批次 2 追加资讯/待办 5 条 `get_info_columns` / `get_info_list` / `get_info_detail` / `get_todo_tabs` / `get_todo_list` 单接口透传 + `open_in_browser` 走官方 `tauri-plugin-opener`、域名白名单 helper 与正文抓取同源；批次 3 追加应用/日程 4 条 `get_app_catalog`（图标后端代拉为 data URL）/ `get_schedule_classify` / `get_schedule_month` / `open_app`（**协议白名单** `is_http_url`，与抓取路径的域名白名单分工——见 [[learnings/portal-app-catalog-and-icons|应用目录、图标代拉与 appLink 校验分工]]）；M2 遗留项追加 `get_schedule_day_counts`（月视图角标）并扩展 `get_schedule_month` 并入校级会议（DJZ 按教学周次构造标题、失败降级空贡献 + `[meeting-diag]` 可观测，见 [[learnings/meeting-proxy-week-title-and-observable-degradation|会议代理端点与静默降级可观测化]]）——协议细节见 [[modules/campus-portal|门户业务协议核心]]，`needsBrowser` 三分类背景见 [[learnings/cwxu-official-site-content-extraction|官网正文抓取与鉴权门降级]]）；**M3 追加慧新E校 10 条**——一卡通 3 条（`get_ecard` / `get_ecard_transactions` / `get_wallet_cards`，最后一条是首页钱包卡的**跨源聚合**：一卡通先取实时、失败静默回落门户快照并带 `source` 标注）与电费 7 条（`list_feeitems` **免登录** / `query_electricity` 三级级联 / 常用房间 3 条本地 CRUD / `open_recharge_in_browser` 兜底）；**M3.1 再追加充值 6 条**（`recharge_create` / `recharge_pay_methods` / `recharge_query_account` / `recharge_submit` / `recharge_status` / `recharge_cancel`——客户端直调官方 App 口径支付链路，原内嵌官方页方案已废弃移除）；**M4 批 2 再追加电费历史 6 条**（新模块 `commands/electricity_history.rs`：`get_electricity_bills` / `get_electricity_monthly` / `get_electricity_orders` / `get_electricity_history` / `bind_electricity_room` / `run_electricity_snapshot`——账单·月度·订单三源全为**只读 GET**，历史与绑定落本地 `electricity_history.json` / `electricity_rooms.json`；`lib.rs` 同时新增 `.setup()` 做**启动补采**（今日未采 + 有内存会话才采一次，不弹窗、不阻塞、失败只记日志），见 [[decisions/electricity-daily-snapshot-and-merge|电费日快照与多端合并决策]]）；**M4.5 再追加一卡通 7 条**（新模块 `commands/ecard.rs`：`get_ecard_overview` / `get_ecard_types` / `get_ecard_stats_summary` / `get_ecard_stats_series` / `get_ecard_stats_assort` / `get_ecard_transfer_accounts` / `get_ecard_secure_keyboard`，全部**只读**；`get_ecard_transactions` 原地扩参；同期原「钱包」「电费」两面板合并为 `ecard` 面板，`PanelId` 9→8，见 [[decisions/ecard-panel-merge|一卡通面板合并决策]]）。详见 [[modules/campus-hub-tauri|接线层 campus-hub-tauri]] 与 [[modules/campus-synjones|慧新E校协议核心]]。

| 目录 | 职责 | 文章 |
|---|---|---|
| `crates/campus-auth/` | CAS 协议：textbook RSA、登录客户端、RecordingJar、验证码识别 | [[modules/campus-auth|CAS 协议核心]] |
| `crates/campus-schedule/` | 课表模型、周次/网格算法、正方教务解析 | [[modules/campus-schedule|课表核心]] |
| `crates/campus-portal/` | 门户业务协议：学期/钱包/周课表/资讯（含官网正文抓取与清洗）/待办/应用（图标带会话代拉 data URL、可达性元数据表）/日程（bs-schedule 独立信封 + 会议并入）调用与解析、校本大节表 | [[modules/campus-portal|门户业务协议核心]] |
| `crates/campus-synjones/` | 慧新E校协议：lyCas 桥换 token（单实例缓存）、一卡通卡信息与流水（`berserker-app` + `berserker-search`）、统计四端点与安全键盘（`ecard_stats.rs` / `ecard_ops.rs`，M4.5）、电费三级级联与**结构化余额**（`charge` 系；末级 `ElectricityView{ fields, money, balanceYuan, tip }`——`balanceYuan` 是后端提取好的余额数值，**前端读它、不解析自由文本**，`None` = 无数据且绝不用 0 代替）、缴费历史只读取数（`turnover.rs`：账单/月度/累计/订单/片区配置）、`synAccessSource=app` 双份携带 | [[modules/campus-synjones|慧新E校协议核心]] |
| `tauri-app/src-tauri/` | 命令面、AppState、DPAPI 存储 | [[modules/campus-hub-tauri|接线层]] |
| `tauri-app/frontend/src/` | 外壳组件、账号系统与头像、Dock 导航、8 面板（M4.5：原「钱包」「电费」合并为 `ecard`）、域色 token | [[modules/frontend-shell|前端外壳]] |

安全基线：CSP 收紧（`tauri.conf.json:26`，`connect-src 'self' ipc://localhost`；M2 批次 2 起 `img-src` 额外放行 `https/http://*.cwxu.edu.cn` 供内嵌正文官网图片）；capabilities 仅 `core:default`（`capabilities/default.json`；opener 插件仅 Rust 侧调用，不开放前端 invoke）；密码只在内存中存续、立即 RSA 加密，日志用户名打码、密码绝不入日志（`commands/auth.rs:10-11,157-165`）；门户 csrf 密钥常量只存在于 campus-auth 源码内（`cas.rs:26`，明文不入 wiki/文档），头像上传日志只打码用户名、绝不打印 data URL（`profile.rs:219-262`）；门户网关 JWT 与邮箱 `loginUrl`（内含 authkey）只在 campus-portal 内存缓存中使用，不落盘、不记日志、不返回前端；**URL 校验两套白名单分工（M2 批次 3 定案）**——「后端要抓取的 URL」（资讯正文抓取）与 `open_in_browser` 强制 `*.cwxu.edu.cn` 域名白名单（`is_allowed_info_url`，防 SSRF/钓鱼），「只在系统浏览器打开、后端不抓取」的 `open_app` appLink 走协议白名单 `is_http_url`（仅 http/https，受信目录来源、无 SSRF 面），且正文抓取用裸 HTTP client 不带门户鉴权头（JWT 只发门户同源）、图标代拉与 data URL 编码不落盘（[[learnings/portal-app-catalog-and-icons|应用目录、图标代拉与 appLink 校验分工]]）；**应用可达性元数据（M2 遗留项）**按附录 A 实测表（access.rs）推导 `AppItem.access`，**不信任门户 isCas 字段**——表是打开前的信息与提示层，不替代上述两种校验（WebVPN 包装未做：网关对未登录请求一律回落、明文包装无法验证，归 M4）；头像等非凭据明文落盘、凭据必 DPAPI（[[decisions/guest-mode-account-shell|游客优先决策]] D3）；**内嵌第三方页的边界（M3 定案）**：`open_recharge_page` 打开的官方缴费页**不被任何 capability 覆盖** ⇒ 其 JS 虽能访问 `__TAURI_INTERNALS__`（Tauri 内部对象无法隐藏），但 `invoke` 一律被 ACL 拒绝（已真机验证拒绝态），因此**不要**为它写 capability（写了就等于把命令面暴露给学校页面）；注入的 token 只在 Rust 内存中拼进 `initialization_script`，**不经过前端 JS API**（[[modules/campus-synjones|慧新E校协议核心]]）。

## 与参考项目 Wxxy-CampusLogin 的关系

- 同构：技术栈、目录骨架（`src-tauri/{commands,infra,account}` + `frontend/src/{shared,components,panels,stores}`）、IPC 契约、DPAPI 模式均沿用其架构（`PLAN.md:71,81,149`）。
- 有据可查的拷贝：DPAPI 裸 FFI 拷贝自其 `src-tauri/src/account/crypto.rs`（`account/crypto.rs:3-4`）；tokio Mutex 锁纪律教训引自其 `commands/login.rs:87`（`infra/state.rs:4`）。
- 独立部分：CAS 协议按本校实测重写（`docs/cas-recon/REPORT.md`，非 Wxxy-CampusLogin 的深澜协议）；课表核心移植自 shiguangschedule（Apache-2.0，[[modules/campus-schedule|课表核心]]）。
