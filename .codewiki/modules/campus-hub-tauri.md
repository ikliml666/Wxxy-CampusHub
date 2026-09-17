---
title: 接线层（campus-hub src-tauri）
type: module
source_files:
  - tauri-app/src-tauri/src/lib.rs
  - tauri-app/src-tauri/src/commands/auth.rs
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
---

# 接线层（campus-hub src-tauri）

`tauri-app/src-tauri`（crate 名 `campus-hub`）是协议核心与前端之间的 IPC 接线层：命令面、AppState、DPAPI 持久化。协议逻辑零实现——「协议核心全部在 campus-auth crate，本 crate 只做 IPC 接线与本地持久化」（`src/lib.rs:2`）。命令全部定义在 `src/commands/auth.rs`，注册于 `lib.rs:16-24`。

## 7 条命令面

| 命令 | 参数（camelCase） | data 形态 | 位置 |
|---|---|---|---|
| `get_captcha` | — | `{ uid, pngBase64 }`（pngBase64 为裸 base64，前端自行拼 data URI，`auth.rs:74-80`） | `auth.rs:342-357` |
| `login` | `account: { username, password }` | 成功 `{ username, displayName }`；三次识别穷尽 `{ uid, pngBase64 }` + message=`CAPTCHA_MANUAL` | `auth.rs:361-366` |
| `login_manual` | `account: { username, password, captchaUid, captchaCode }` | 同 login（手动单次提交不重试） | `auth.rs:370-384` |
| `login_saved` | `account: { username }` | 同 login（DPAPI 解密已存密码后复用自动流程，`auth.rs:388-399`） | `auth.rs:388-399` |
| `check_session` | — | `{ loggedIn: bool }` | `auth.rs:404-418` |
| `logout` | — | 无（`CommandResult::empty()`） | `auth.rs:422-436` |
| `list_accounts` | — | `{ accounts: [{ username, lastLogin }] }`（密码绝不出现在返回里，`auth.rs:439-454`） | `auth.rs:440-454` |

约定：业务失败一律 `Ok(CommandResult::err(中文消息))`，`Err(String)` 仅限 IPC 框架层错误（`auth.rs:212`）。

## 登录重试状态机

`run_login` 内核三命令共享（`auth.rs:213-288`），登录用**全新 CasClient**（干净 jar，避免旧会话 cookie 干扰，`auth.rs:221`），密码仅在内存中存续、立即 RSA 加密（`auth.rs:222-223`）。

- **自动模式**（`auth.rs:233-287`）：`kaptcha → captcha::solve → login` 循环，总提交 ≤`MAX_LOGIN_ATTEMPTS=3`（`auth.rs:27`）。重试决策纯函数 `should_retry`（`auth.rs:199-201`）：**仅 `WrongCaptcha` 重试**；`WrongUserOrPwd`（防 CAS 连续错误计数锁号）、`UserLocked`、`NeedTwoVerify`、`Unknown`、`Network` 一律立即终止（单测 `auth.rs:493-505`）。识别失败（solve 返回 None）未提交到 CAS、不消耗连续错误计数，刷新重试不计提交（`auth.rs:245-252`）。
- **穷尽转手动**（`auth.rs:269-286`）：返回 `success:false + message="CAPTCHA_MANUAL" + data={uid, pngBase64}`，前端据此切手动模式；穷尽前最后一次提交的 uid 可能已被 CAS 消耗，故先尽力刷新一张新验证码图、失败用最后一张兜底。
- **手动模式**（`auth.rs:226-231`）：单次提交不自动重试，验证码错误由用户刷新重输。
- **接近阈值保护**：`parse_error_count_hint` 从 TWOVERIFY 的 data 原文解析「已连续错误N次，阈值M」（`auth.rs:166-177`）；`n+1 >= m` 时错误消息明确提示已停止自动重试（`auth.rs:187-191`）。

## LoginResultData：untagged 双形态

`#[serde(untagged)] enum LoginResultData { LoggedIn(LoginData), ManualNeeded(CaptchaData) }`（`auth.rs:99-104`）——序列化时内联，前端拿到裸对象：成功 = `{ username, displayName }`，穷尽 = `{ uid, pngBase64 }`。前端按 `"uid" in data` 收窄分支（`LoginPanel.tsx:46-58`）。

## 登录成功收尾（`finish_login` `auth.rs:291-323`）

`sso_follow(PORTAL_SERVICE)` 建门户会话 → `save_account`(DPAPI，失败不阻断会话、降级日志 `auth.rs:303-306`) → `jar.snapshot()` 逐条 DPAPI 加密写 session.json（`auth.rs:307-311`）→ AppState 锁内同步赋值（guard 语句末 drop、此后无 await，`auth.rs:312-316`）。`displayName` 无来源时用 username（`auth.rs:321`）。

`check_session` 返回 false（无会话/已过期/探测失败）时清 AppState 会话与 session.json（`auth.rs:411-416`）。`logout`：`GET {CAS_BASE}/logout` 尽力而为（5s 超时，复用 `sso_follow` 发 GET；REST 登录无 CASTGC，服务端可能本就无全局会话），随后本地清理必然执行（`auth.rs:422-436`）。

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

启动回填 `restore_session()`（`state.rs:105-114`）：`run()` 在 `manage` 之前调用（避免 setup 内碰 tokio Mutex，`lib.rs:11-12`），读 session.json → 解密 → `jar.restore` 回填；文件缺失/损坏/cookies 空 → None。落盘内容不含明文凭据有单测断言（`state.rs:141-143`）。

## 最小权限

`capabilities/default.json` 仅声明 `permissions: ["core:default"]`、仅 `main` 窗口——M0/M1 无文件系统/剪贴板/通知等插件需求，不给多余能力。CSP 收紧为 `connect-src 'self' ipc://localhost`、`img-src 'self' data:`（验证码 base64 图需要 data:）等（`tauri.conf.json:26`）。

## 离线单测（`commands/auth.rs:458-546`）

错误码→中文消息映射、重试状态机、计数提示解析与接近阈值文案、`login_saved` 解密失败路径（坏密文/账号不存在，不发起网络请求）、用户名打码（`mask_username`：前 2 位 + 末位，`auth.rs:154-161`）。
