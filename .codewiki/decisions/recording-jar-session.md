---
title: 决策记录 - RecordingJar 自实现与会话持久化
type: decision
source_files:
  - crates/campus-auth/src/jar.rs
  - crates/campus-auth/src/cas.rs
  - tauri-app/src-tauri/src/infra/state.rs
  - tauri-app/src-tauri/src/account/crypto.rs
  - tauri-app/src-tauri/src/commands/auth.rs
tags:
  - session
  - cookie
  - dpapi
  - reqwest
  - decision
---

# 决策记录：RecordingJar 自实现与会话持久化

状态：已实施（M1）；未决项 1 个（见文末）。关联模块：[[modules/campus-auth|CAS 协议核心]]、[[modules/campus-hub-tauri|接线层]]。

## D1：为什么自实现 RecordingJar

**问题**：会话检测与持久化需要读回 client 当前持有的全部 cookie，但 reqwest 0.12 的 `Client` 无法读回内部 CookieStore——内部 Jar 是私有类型，公开的 `reqwest::cookie::Jar` 只能写不能枚举（`jar.rs:1-6` 模块注释记录此事实）。

**决策**：实现 `RecordingJar`，实现 reqwest 0.12 的 `cookie::CookieStore` trait（`set_cookies` / `cookies` 两方法，0.12.28 均为 `&self`，`jar.rs:73-103`）：

- `set_cookies`：委托内置 `reqwest::cookie::Jar` 记录真实 cookie 语义（域、路径、过期），同时把 `Set-Cookie` 第一个 `;` 前的键值对按第一个 `=` 切分（值可含 `=`，如 base64 形态的 rememberMe）记入 `Mutex<Vec<(String, String)>>`（`jar.rs:79-97`）；同名后到覆盖先到（与浏览器单域语义一致）。
- `cookies`：纯委托内置 Jar（`jar.rs:100-102`）。

**备选与放弃理由**：换 `reqwest` 底层自行管理 cookie 头（丢失标准语义、易错）；升级/换用可枚举 cookie 的第三方 jar（引新依赖且 trait 兼容性无保证）。委托式 RecordingJar 改动最小、语义全保留。

**接口取舍**：`(名, 值)` 快照**不含域信息**（`jar.rs:13-14`，计划冻结接口）。恢复时统一按门户域 `RESTORE_URL = https://my.cwxu.edu.cn/` 回填——M1 会话恢复的核心是门户会话（customsid / Authorization / rememberMe 实测均种在 my.cwxu.edu.cn）；多域恢复（WebVPN 等）待 M4 按需扩展。`restore` 同名覆盖、幂等，重复 restore 不累积（`jar.rs:45-61`）。

## D2：会话持久化方案（session.json + DPAPI）

**问题**：P0-3「重启保持登录」要求 cookie 落盘，但 cookie 即凭据，明文落盘不可接受。

**决策**（`infra/state.rs:60-114`）：

- 登录成功收尾时 `jar.snapshot()` → 每条 cookie 值经 `dpapi_protect`（DPAPI CurrentUser 作用域加密 → base64）→ 写 `%APPDATA%/campushub/session.json`，结构 `{ username, cookies: [{ name, valueB64 }] }`（`state.rs:44-80`）。
- DPAPI 用裸 FFI（crypt32 的 `CryptProtectData/CryptUnprotectData`，零新依赖），拷贝自 Wxxy-CampusLogin（`account/crypto.rs:3-37`）；CurrentUser 作用域保证跨用户不可解密。
- 启动时 `restore_session()` 在 `manage` 之前执行（避免 setup 内碰 tokio Mutex，`lib.rs:11-15`）：解密 → `jar.restore` 回填 → 作为初始 AppState。
- 容错：文件缺失/损坏 → None（无会话，正常走登录）；**单个** cookie 解密失败跳过而非整体失败（`state.rs:86-94`）。
- 纪律：落盘内容不含明文有单测断言（`state.rs:141-143`）；密码同规则存 `accounts.json` 的 `passwordB64`（`account/store.rs:13-23`）。

**为什么不用 Windows 凭据管理器/tarPCSC 等专用存储**：多账号结构（accounts.json）与 cookie 列表（session.json）是 JSON 文档形态，DPAPI 加密字段 + JSON 文件保留可读结构与可迁移性；Wxxy-CampusLogin 已验证该模式。

## D3：check_session 判据的三次演进

需求：启动时判断「已有会话是否仍然有效」。三次取舍均来自实测（`cas.rs:222-228` 注释，`portal_probe` 实现 `cas.rs:229-245`）：

1. **门户首页正文匹配**（否决）：门户首页 HTML 恒含 `lyuapServer/login` 常量（2 处），无论是否登录都命中，会把已登录判成 Expired——正文不可作判据。
2. **`/shiro-cas` 端点探测**（否决）：无 ticket 访问该端点会触发服务端断连（hyper `IncompleteMessage`），请求本身就失败。
3. **最终方案：customsid 存在性 + 首页域判定**（采纳）：
   - 前置：`jar.has("customsid")` 为假直接 Expired——未登录场景由它挡住（首页未登录也返回 200 外壳，前端路由才跳登录，光看首页状态码会误判）；
   - 再访问门户首页 `https://my.cwxu.edu.cn/`，若最终 URL 被弹回 `wxcas.cwxu.edu.cn` 域（CAS 登录页重定向）= 会话过期，否则 Alive；
   - 探测请求本身失败（网络等）按安全侧报 Expired（`cas.rs:233-235`），前端随之走登录页，可接受。

配合的派生命令行为：`check_session` 返回 false 时同步清 AppState 会话与 session.json（`commands/auth.rs:411-416`），避免带着死会话继续跑。

## 未决项

**精确过期检测**：当前判据只能区分「有门户会话 cookie 且首页不弹回」与否则，无法感知服务端单方面吊销后 cookie 仍留存、以及 CAS TGT 过期但门户 cookie 残留的窗口。既定方向：M2 接入门户鉴权 API 后，改用真实鉴权接口探测（401/302 即过期）替代首页域判定（`cas.rs:228` ponytail 注记）。
