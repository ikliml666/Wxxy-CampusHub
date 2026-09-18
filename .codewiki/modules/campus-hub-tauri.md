---
title: 接线层（campus-hub src-tauri）
type: module
source_files:
  - tauri-app/src-tauri/src/lib.rs
  - tauri-app/src-tauri/src/commands/auth.rs
  - tauri-app/src-tauri/src/commands/profile.rs
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
---

# 接线层（campus-hub src-tauri）

`tauri-app/src-tauri`（crate 名 `campus-hub`）是协议核心与前端之间的 IPC 接线层：命令面、AppState、DPAPI 持久化。协议逻辑零实现——「协议核心全部在 campus-auth crate，本 crate 只做 IPC 接线与本地持久化」（`src/lib.rs:2`）。登录/账号命令定义在 `src/commands/auth.rs`，头像/资料命令定义在 `src/commands/profile.rs`，全部注册于 `lib.rs:16-29`。

## 12 条命令面

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
| `get_avatar` | — | `{ imageBase64: string\|null, source: "local"\|"official"\|null }` | `profile.rs:140-143` |
| `set_avatar` | `imageBase64: String` | 同 AvatarData（空串/超 512KB → err） | `profile.rs:147-156` |
| `clear_avatar` | — | 同 AvatarData（只清本地，官方保留） | `profile.rs:160-166` |
| `sync_official_avatar` | — | 同 AvatarData（无会话 → err「请先登录」） | `profile.rs:171-187` |

约定：业务失败一律 `Ok(CommandResult::err(中文消息))`，`Err(String)` 仅限 IPC 框架层错误（`auth.rs` 注释冻结此口径）。头像四命令统一返回 `AvatarData`（键恒在、值可 null，`profile.rs:26-31`）。

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
| `profile.json` | `{ localBase64?, officialBase64?, officialFetchedAt? }`（camelCase，字段缺省即不存在，`profile.rs:34-44`）——**明文 base64，不走 DPAPI** | `store_local_avatar` / `clear_local_avatar` / `store_official_avatar`（`profile.rs:104-126`）；读 `read_profile`（文件缺失/损坏按空档处理，`profile.rs:57-62`） |

启动回填 `restore_session()`（`state.rs:105-114`）：`run()` 在 `manage` 之前调用（避免 setup 内碰 tokio Mutex，`lib.rs:11-13`），读 session.json → 解密 → `jar.restore` 回填；文件缺失/损坏/cookies 空 → None。落盘内容不含明文凭据有单测断言（`state.rs:141-143`）。

## 头像存取与官方同步（`commands/profile.rs`）

头像四命令全部只做接线与本地存取，协议拉取复用 `CasClient::portal_login_info`（见 [[modules/campus-auth|CAS 协议核心]]「门户资料接口」）：

- **生效优先级：本地 > 官方 > 无**，纯函数 `current_avatar` 统一裁决（`profile.rs:73-90`）；`clear_avatar` 只清本地、官方保留（回落展示，`profile.rs:112-117`）。
- **落盘即明文**：头像不是凭据，base64 明文写 `profile.json`，不经 DPAPI（`profile.rs:5-6` 注释；取舍见 [[decisions/guest-mode-account-shell|游客优先与账号外壳决策]]）。
- **体积守卫**：`set_avatar` 空串/超 `AVATAR_MAX_B64=512KB` 拒绝，冻结文案「头像文件过大（上限 512KB）」（`profile.rs:21,93-101`）；前端 AvatarDialog 同阈值预检内联报错。
- **`sync_official_avatar`**：无会话 → 约定错误文案「请先登录」（`ERR_NO_SESSION`，前端据此引导登录，`profile.rs:23,130-134`）；有会话 → 锁纪律 `session_client` clone 出 client 后发请求，成功落盘 `officialBase64 + officialFetchedAt`，网络/解析失败不落盘、不清已有头像（`profile.rs:171-187`）。
- 单测覆盖优先级轮转（官方 → 本地覆盖 → 清本地回落官方 → 全空双 null）、超限拒绝、无会话文案契约（`profile.rs:191-277`）。

## 最小权限

`capabilities/default.json` 仅声明 `permissions: ["core:default"]`、仅 `main` 窗口——M0/M1 无文件系统/剪贴板/通知等插件需求，不给多余能力。CSP 收紧为 `connect-src 'self' ipc://localhost`、`img-src 'self' data:`（验证码 base64 图需要 data:）等（`tauri.conf.json:26`）。

## 离线单测（`commands/auth.rs:506-626` + `commands/profile.rs:191-277`）

错误码→中文消息映射、重试状态机、计数提示解析与接近阈值文案、`login_saved` 解密失败路径（坏密文/账号不存在，不发起网络请求）、用户名打码（`mask_username`：前 2 位 + 末位，`auth.rs:157-165`）；profile 侧头像优先级轮转、体积校验、无会话文案契约。`cargo test --workspace` 全量 47 passed（本次新增头像/账号/门户解析单测前为 35）。
