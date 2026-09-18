---
title: 系统架构总览
type: architecture
source_files:
  - tauri-app/src-tauri/src/lib.rs
  - tauri-app/src-tauri/src/commands/auth.rs
  - tauri-app/src-tauri/src/infra/state.rs
  - tauri-app/src-tauri/src/account/crypto.rs
  - tauri-app/src-tauri/src/account/store.rs
  - crates/campus-auth/src/lib.rs
  - crates/campus-schedule/src/lib.rs
  - tauri-app/frontend/src/shared/tauriApi.ts
  - tauri-app/frontend/src/shared/types.ts
  - tauri-app/frontend/src/stores/authStore.ts
  - tauri-app/frontend/src/App.tsx
tags:
  - architecture
  - tauri
  - ipc
  - session
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
     campus-auth：CAS 登录协议  campus-schedule：课表领域核心
```

- **协议单点**：`crates/campus-auth/src/lib.rs:1` 明示「无 Tauri 依赖，安卓可复用」；`crates/campus-schedule` 同理仅依赖 serde/chrono/thiserror（各自 Cargo.toml）。安卓端届时以 Cargo path 依赖引用这两个 crate（`PLAN.md:15`），协议只写一份。
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
5. **启动检测**：App 挂载后非阻塞调 `checkSession`（`void` 发出不挡渲染，`App.tsx:8-12`）→ Rust 侧 `portal_probe`：jar 有 `customsid` + 门户首页未被弹回 CAS 域 = Alive；失败即清 AppState 与 session.json（`commands/auth.rs:443-457`）。判据取舍详见 [[decisions/recording-jar-session|RecordingJar 与会话持久化决策]]。**外壳为游客优先**：未登录不渲染登录页，壳与面板恒渲染、登录收进弹层与账号菜单（[[decisions/guest-mode-account-shell|游客优先决策]]）。

## 命令面与模块地图

12 条命令注册于 `lib.rs:16-29`：登录/账号 8 条（`get_captcha` / `login` / `login_manual` / `login_saved` / `check_session` / `logout` / `list_accounts` / `remove_account`）+ 头像 4 条（`get_avatar` / `set_avatar` / `clear_avatar` / `sync_official_avatar`）。详见 [[modules/campus-hub-tauri|接线层 campus-hub-tauri]]。

| 目录 | 职责 | 文章 |
|---|---|---|
| `crates/campus-auth/` | CAS 协议：textbook RSA、登录客户端、RecordingJar、验证码识别 | [[modules/campus-auth|CAS 协议核心]] |
| `crates/campus-schedule/` | 课表模型、周次/网格算法、正方教务解析 | [[modules/campus-schedule|课表核心]] |
| `tauri-app/src-tauri/` | 命令面、AppState、DPAPI 存储 | [[modules/campus-hub-tauri|接线层]] |
| `tauri-app/frontend/src/` | 外壳组件、账号系统与头像、Dock 导航、8 面板、域色 token | [[modules/frontend-shell|前端外壳]] |

安全基线：CSP 收紧（`tauri.conf.json:26`，`connect-src 'self' ipc://localhost`）；capabilities 仅 `core:default`（`capabilities/default.json`）；密码只在内存中存续、立即 RSA 加密，日志用户名打码、密码绝不入日志（`commands/auth.rs:10-11,157-165`）；头像等非凭据明文落盘、凭据必 DPAPI（[[decisions/guest-mode-account-shell|游客优先决策]] D3）。

## 与参考项目 Wxxy-CampusLogin 的关系

- 同构：技术栈、目录骨架（`src-tauri/{commands,infra,account}` + `frontend/src/{shared,components,panels,stores}`）、IPC 契约、DPAPI 模式均沿用其架构（`PLAN.md:71,81,149`）。
- 有据可查的拷贝：DPAPI 裸 FFI 拷贝自其 `src-tauri/src/account/crypto.rs`（`account/crypto.rs:3-4`）；tokio Mutex 锁纪律教训引自其 `commands/login.rs:87`（`infra/state.rs:4`）。
- 独立部分：CAS 协议按本校实测重写（`docs/cas-recon/REPORT.md`，非 Wxxy-CampusLogin 的深澜协议）；课表核心移植自 shiguangschedule（Apache-2.0，[[modules/campus-schedule|课表核心]]）。
