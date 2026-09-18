---
title: CAS 协议核心（campus-auth）
type: module
source_files:
  - crates/campus-auth/src/cas.rs
  - crates/campus-auth/src/rsa.rs
  - crates/campus-auth/src/jar.rs
  - crates/campus-auth/src/captcha.rs
  - crates/campus-auth/src/error.rs
  - crates/campus-auth/src/jwglxt.rs
  - crates/campus-auth/src/lib.rs
  - crates/campus-auth/Cargo.toml
tags:
  - cas
  - rsa
  - cookie
  - captcha
  - reqwest
---

# CAS 协议核心（campus-auth）

`crates/campus-auth`：锡院助手 CAS 统一身份认证协议核心，无 Tauri 依赖、安卓可复用（`src/lib.rs:1`）。协议事实全部来自实测打通的 `docs/cas-recon/REPORT.md`（2026-09-17，真实账号端到端已验证，`src/cas.rs:3-6`）。依赖仅 num-bigint-dig / reqwest(0.12, rustls-tls) / image / serde / base64 / thiserror（`Cargo.toml:10-19`）。

## 模块组成

| 文件 | 职责 |
|---|---|
| `rsa.rs` | textbook RSA 加密（Shapiro RSA.js 系等价实现）+ `token` 头构造 |
| `cas.rs` | CasClient：kaptcha → login → sso_follow → portal_probe + 门户资料接口（头像/姓名/头像上传） |
| `jar.rs` | RecordingJar：记录型 CookieStore（会话检测与持久化的数据源） |
| `captcha.rs` | 算术验证码识别（颜色不变强度图 + NCC 最近邻） |
| `jwglxt.rs` | 正方教务 SSO：`sso_ticket`（CAS REST 换票）+ `jwglxt_sso`（一键进教务）+ `fetch_timetable_json`（确保教务会话 + 拉课表 JSON 原文） |
| `error.rs` | `CampusAuthError { Rsa, Http, Parse, JwglNotLogin }`（`error.rs:5-17`；`JwglNotLogin` = 教务 901 会话失效信号，M2.5 批次 1 新增） |

## 公开 API

**rsa.rs**
- `rsa_encrypt_hex(plaintext) -> Result<String>`（`rsa.rs:28-49`）：CAS 线上 textbook RSA，公钥 e=0x010001、n 为固化常量 `CAS_RSA_N_HEX`（`rsa.rs:16`）；chunkSize=126 字节（`rsa.rs:22`），块尾补 0x00 后整块反转（等价 JS little-endian 组块），密文 hex 小写不补前导零、块间直接拼接。**非 ASCII 明文显式拒绝**（`rsa.rs:29-32`）——JS `charCodeAt` 语义对非 ASCII 不等价，为避免静默产出不一致密文（计划 P2-11 修正项）。
- `cas_token_header(now_ms)`（`rsa.rs:52-54`）：请求头 `token` = RSA(`"lyasp" + 毫秒时间戳`)，TAG=lyasp。

**cas.rs**（`CasClient`，`#[derive(Clone)]`，clone 共享同一 jar）
- `new()`（`cas.rs:106-125`）：构建两个 reqwest Client——`http`（默认重定向策略）与 `http_manual`（`Policy::none`，手动跟随），共享同一 `Arc<RecordingJar>`；UA 为 cas.js 同款 Chrome 126 串（`cas.rs:29`）。
- `kaptcha()`（`cas.rs:133-142`）：`GET {CAS_BASE}/kaptcha` → `CaptchaInfo { uid, png_base64, kaptcha_type }`；png_base64 已剥掉 `data:image/png;base64,` 前缀（`parse_kaptcha`，`cas.rs:518-531`）。
- `login(username, password_rsa_hex, captcha_uid, captcha_code) -> Result<CasLoginOk, CasLoginError>`（`cas.rs:148-189`）：`POST {CAS_BASE}/v1/tickets`，表单体 7 字段（username/password/service/loginType/id/code/otpcode，`build_login_body` `cas.rs:477-497`），头 `token` + Referer + Origin。成功响应顶层即 `{tgt, ticket}`（兼容 data 包裹与 data 为纯字符串，`parse_login_response` `cas.rs:538-570`）。
- `sso_follow(service_url, ticket)`（`cas.rs:193-241`）：`GET service?ticket=` 手动跟随 302 链（上限 10 跳），返回落点 URL。原因见下文「关键实现要点」。
- `portal_probe()`（`cas.rs:250-266`）：会话探测 → `SessionState::{Alive, Expired}`，判据取舍见 [[decisions/recording-jar-session|RecordingJar 决策]]。
- `portal_login_info()`（`cas.rs:273-283`）：`GET 门户 /api/upp/userControl/getLoginInfo` → `data.headPortrait` 裸 base64（官方头像）。门户 base 由 `PORTAL_PROBE` 派生不另设常量；解析见 `extract_head_portrait`。复用 `http`（与登录/探测同 jar、同 UA）；会话过期时服务端返回非预期结构 → Parse 错误。
- `portal_user_profile()`（`cas.rs:291-303`）：`POST 门户 /tryLoginUserInfo` → `PortalProfile { name, department, user_id, org_id, token_id }`（真实姓名、院系、学号、组织 id 与网关 JWT，struct `cas.rs:60-71`）。**2026-09-18 实测：GET 返回 405，须 POST JSON `{}`**；解析见 `extract_user_profile`。
- `portal_change_portrait(data_url)`（`cas.rs:316-364`，2026-09-18 实机取证新增）：`POST 门户 /api/authc/users/portraitChange`，body `{"displayPhoto": "<完整 data URL>"}`，把裁切后的头像**上传回学校系统**。鉴权头方案（除会话 Cookie 外全部必需）：`Authorization` = `tokenId`（JWT，**无 Bearer 前缀**）、`loginUserId`/`loginUserName` = `userId`（学号同值两处）、`loginUserOrgId` = `orgId`（学生实测 `"-1"`，缺失兜底）、`csrfTimestamp` = 当前毫秒、`csrfToken` = `csrf_token` 现算，另有 `X-Requested-With: XMLHttpRequest` 与门户前端一致。每次调用先现取一次 `portal_user_profile`（JWT 随取随用最新）再发请求；服务端**原样存储不压缩**，体积守卫归调用方（本方法只校验 `data:image/` 前缀形态）；`meta.success=false` 属业务失败，暂以 Parse 透传服务端原文（error.rs 增加 Unknown 变体后分流，`cas.rs:325-327`）。协议细节与踩坑见 [[learnings/portal-avatar-upload-protocol|门户头像上传协议]]。
- `csrf_token(ts_ms) -> String`（`cas.rs:434-437`）：门户网关 csrf = `md5("timestamp=<毫秒>,key=<GATEWAY_KEY>")` 小写 hex；密钥常量 `GATEWAY_KEY` 固化在 `cas.rs:26`（来自门户 app bundle 实机取证，**明文不入 wiki/文档**）；`to_hex` 手写 3 行（`cas.rs:440-444`）不为此引 hex crate。金标向量测试防算法串格式漂移。
- `parse_portrait_change_response(body)`（`cas.rs:453-475`）：上传响应解析——`meta.success=true` 通过；`false` 取 `meta` 内消息为错；`data`/`meta` 缺失或非 JSON 一律 Parse 错。
- `jar()`（`cas.rs:128-130`）：jar 句柄，供 check_session 与持久化。
- 解析纯函数（离线单测覆盖）：`extract_head_portrait`（`cas.rs:373-397`，容忍完整 data URL——实测形态——与裸 base64 两种输入，剥 `data:` 前缀取首个 `,` 之后；空/缺失/非法 JSON 一律 Parse 错）与 `extract_user_profile`（`cas.rs:400-430`，`userName` trim 后为空即错、`departmentName` 缺失/空为 None；姓名与院系不进日志）。

**jwglxt.rs**（2026-09-18 实测新增，正方教务 SSO，协议细节见 [[learnings/jwglxt-sso-chain|教务 SSO 链与课表接口取证]]）
- `sso_ticket(tgt, service)`（`jwglxt.rs:56-77`）：`POST {CAS_BASE}/v1/tickets/{tgt}`（form `service=<service>`）→ 响应体纯文本即新 ST；非 `ST-` 开头 → Parse 错。**TGT 必须随会话持久化**——CAS 服务端不种任何登录 cookie（jar 实测登录后为空），会话复用全靠客户端保存 TGT。
- `jwglxt_sso(tgt)`（`jwglxt.rs:88-91`）：换教务 ST（service=`JWGL_SERVICE`）→ 复用 `sso_follow` 走 `/sso/lyiotlogin` 302 链，返回落点 URL（实测 `…/jwglxt/xtgl/index_initMenu.html?jsdm=xs…` 学生主界面）；成功后 jar 种下教务域 `route`/`JSESSIONID`（+`rememberMe`），同 jar 的 `http_client()` 即可调 `/jwglxt/` 业务接口。
- `fetch_timetable_json(tgt, xnm, xqm)`（`jwglxt.rs:124-146`，M2.5 批次 1）：**确保教务会话 + 拉课表 JSON 原文**一步到位——先直接 POST [`KBCX_XSKBCX_URL`]（`gnmkdm=N2151` 固定在 query，body 仅 `xnm=<学年起始年>&xqm=<学期代码 3/12/16>`，头组 `Content-Type: …charset=UTF-8` + `X-Requested-With: XMLHttpRequest`——XRW 必带，让会话失效表现为明确的 901 而非 200 登录页）；901 时 `tgt=Some` → `jwglxt_sso` 静默重进（换新 ST 走 5 跳链）后**重试一次**，重进失败或重试仍 901 → `JwglNotLogin`（TGT 失效 = 静默续期不可用，内部换票错误归一为该变体，不向用户暴露）；`tgt=None`（旧会话文件无 TGT）→ 直接 `JwglNotLogin`。判定逻辑抽纯函数 `interpret_kbcx_response`（`jwglxt.rs:35-49`，离线单测）：901 → `JwglNotLogin`；200 且 body 以 `{` 开头 → JSON 原文透传（**解析交上层** `campus_schedule::parse_kb_response`，本 crate 不依赖 campus-schedule 运行时依赖）；200 HTML（登录页形态）与其他状态码 → `Parse`，错误消息只含长度/状态码、不含响应原文。

**jar.rs**（`RecordingJar`，实现 `reqwest::cookie::CookieStore`，`jar.rs:73-103`）
- `snapshot() -> Vec<(String, String)>`（`jar.rs:39-41`）：全部已见 cookie（不含域信息，计划冻结接口）。
- `restore(&[(name, value)])`（`jar.rs:45-61`）：按门户域 `https://my.cwxu.edu.cn/` 回填进内置 Jar（`RESTORE_URL` `jar.rs:15`），同名覆盖、幂等。
- `has(name)`（`jar.rs:64-66`）：浅检测（check_session 用 `customsid`）。

**captcha.rs**
- `solve(png_bytes, &KaptchaTemplates) -> Option<String>`（`captcha.rs:76-79`）：PNG → 算式答案（如 `"63"`）；None = 无法识别，上层刷新重试，绝不硬猜。
- 模板集编译期内嵌自 `templates/kaptcha-templates.json`（`include_str!`，`captcha.rs:56`），由 `tests/captcha_solve.rs::build_templates` 从标注样本生成。

## 错误码映射表（16 项，`map_error_code` `cas.rs:572-580`）

失败响应形态 `{"meta":...,"data":{"code":"..."}}`。全集与映射以 REPORT.md:30 为准：

| 错误码 | CasLoginError 变体 | 语义 |
|---|---|---|
| `NOUSER` | `WrongUserOrPwd` | 账号或密码错误 |
| `CODEFALSE` | `WrongCaptcha` | 验证码错误 |
| `USERLOCK` | `UserLocked` | 账号锁定 |
| `TWOVERIFY` | `NeedTwoVerify(data_raw)` | 二次验证（携带 data 原文，含「已连续错误N次，阈值M」计数提示） |
| 其余 12 码 | `Unknown(原样码)` | `USERDISABLED` / `PASSERROR` / `USERNOTONLY` / `PEOPLEMOREACCOUNT` / `ISBINDOTP` / `ISBINDWX` / `ISMODIFYPASS` / `NETWORKCOMMITMENT` / `NOREGISTER` / `NOAUTHORIZATION` / `OTPERROR` / `PENDINGACTIVATE` |

另有 `Network(String)` 变体承接网络层错误（`cas.rs:87`）。16 码全覆盖测试见 `tests/cas_parse.rs:19-52`。

## 门户资料接口（2026-09-18 新增，含头像上传）

门户在 CAS 之外还暴露三个**会话内**资料端点，均复用登录后的 RecordingJar（cookie 连续性由 CasClient clone 语义保证），门户 base 从 `PORTAL_PROBE`（`cas.rs:23`）派生：

| 方法 | 端点 | 提取字段/效果 | 消费方 |
|---|---|---|---|
| `portal_login_info` | `GET /api/upp/userControl/getLoginInfo` | `data.headPortrait`（官方头像） | `commands/profile.rs::sync_official_avatar`、`upload_official_avatar`（上传后回读） |
| `portal_user_profile` | `POST /tryLoginUserInfo`（body `{}`，GET 405） | `data.userName` / `data.departmentName` / `data.userId` / `data.orgId` / `data.tokenId` | `commands/auth.rs::finish_login`（真实姓名落 displayName）；`portal_change_portrait`（JWT 与 ids 作鉴权头） |
| `portal_change_portrait` | `POST /api/authc/users/portraitChange` | 覆盖学校头像（`displayPhoto` data URL），响应 `meta.success` 校验 | `commands/profile.rs::upload_official_avatar` |

设计约束：两个方法**只负责拉取与解析**（解析拆成纯函数便于离线单测），落盘/降级策略归调用方——头像失败不落盘不清旧值（`profile.rs:171-187`），姓名失败回退学号且不抹旧值（`auth.rs:312-330`）。会话过期的表现是响应结构不合预期 → Parse 错误，与网络错误走同一条降级路径。

## 关键实现要点

**sso_follow 为何手动跟随重定向**（`cas.rs:110-119,193-195`）：该 302 链中存在响应 body 中断的跳（hyper `IncompleteMessage`），reqwest 默认重定向策略会因读 body 失败使整链报错；手动跟随只要拿到 Location 与 Set-Cookie 即可继续，body 异常可忽略（`cas.rs:219` 尽力读、错误丢弃）。附带两处协议修正：每跳前把 cwxu 域名的 http 升级为 https（只升不降，`cas.rs:202-208`）——REPORT.md 实测门户 302 落点是 http:// 明文而服务端对明文请求直接断连；结束后落点若仍为 http 同样替换（`cas.rs:230-237`）。

**portal_probe 判据取舍**（`cas.rs:241-249` 注释，逐条实测）：正文匹配不可用（门户首页 HTML 恒含 `lyuapServer/login` 常量 2 处，会把已登录判成 Expired）；`/shiro-cas` 端点不可用（无 ticket 访问触发服务端断连）；最终判据 = jar 有 `customsid` 且访问门户首页未被弹回 CAS 域，未登录场景由 `customsid` 缺失挡住（首页未登录也返回 200 外壳）。精确过期检测待 M2 接鉴权 API 后替代（`cas.rs:248`）。

**验证码识别管线**（题面恒为「一位数 运算符(+/-/*) 一位数 =」，PNG 100×25）：垂直投影切分（`SEG_MIN=0.12` 相对峰值阈值，`captcha.rs:28,220`）→ 片数 ≠4 直接 None（字符断裂/粘连时片序不可信，错位片可能自信匹配出错误答案消耗 CAS 连续错误计数，`captcha.rs:85-91`）→ bbox 左上角锚定 16×14 画布（渲染位置逐位确定，缩放会引入形变噪声）→ 颜色不变强度图（`765-Σrgb` 除以本图峰值，`captcha.rs:207-217`）→ NCC + ±1px 位移补偿匹配（`captcha.rs:163-181`）。阈值 `NCC_MIN=0.88` 与平局间隙 `NCC_GAP=0.03` 按样本标定（类内 min 0.931 / 跨类 max 0.839，`captcha.rs:29-31,146-160`）；同类多模板先聚合为每类一个最大分再跨类比较，否则正常类内差异会被平局判定误杀（`captcha.rs:145-147`）。

## 测试清单

| 测试文件 | 内容 | 运行条件 |
|---|---|---|
| `tests/rsa_golden.rs` | golden 对拍（独立 BigInt 实现固化）+ 非 ASCII 拒绝 + `cas_token_header` 时间戳金标，3 个常跑测试 | 离线常跑 |
| `tests/cas_parse.rs` | 16 码映射全覆盖、成功响应三形态解析、login body 构造、RecordingJar 记录行为 | 离线常跑 |
| `cas.rs` 内嵌 `#[cfg(test)]`（`cas.rs:582-738`，14 个） | csrf 金标向量 `csrf_token_golden_vector`（防算法串漂移，`cas.rs:682-684`）+ 小写 hex 与时变性 `csrf_token_lowercase_hex_varies`；`portrait_change_success` / `portrait_change_failure_keeps_server_message` / `portrait_change_invalid_or_missing_meta` 上传响应解析三分支；`extract_head_portrait`（data URL/裸 base64/缺失/空串/非法 JSON）与 `extract_user_profile`（正常/无院系/空姓名/非法输入）解析单测 | 离线常跑 |
| `jwglxt.rs` 内嵌 `#[cfg(test)]`（4 个，M2.5 批次 1） | `interpret_kbcx_response` 三分支（901 → `JwglNotLogin`；200 JSON 透传/前导空白容忍；200 HTML 拒绝且错误不含原文）+ 其他状态码 → Parse + `kbcx_url_same_origin_as_jwgl_service` 常量防漂移（课表端点与 JWGL_SERVICE 同源、`gnmkdm=N2151` 在 query） | 离线常跑 |
| `tests/captcha_solve.rs` | 合成图 roundtrip（全链路，常跑）+ `build_templates` 模板生成 + 100 张样本三分类评测（门槛：正确率 ≥98% 且自信错误 =0） | 评测部分 `#[ignore]`（2 个） |
| `tests/cas_live.rs` | 全流程 live 冒烟（凭据经 `CAMPUS_HUB_CREDS` 环境变量指向文件，真实凭据绝不入代码，`cas_live.rs:1-11`） | `#[ignore]`，主智能体验收时跑 |
| `tests/jwglxt_live.rs` | 教务全链 live：`jwglxt_sso` → 课表接口 200 JSON 含 kbList → `parse_kb_response` >0 门课 → 头组/参数对照 → 失败模式断言（未登录+XRW=901 空 body；非 ajax=200 登录页 HTML）；`jwgl_service_shape` 常量防漂移为离线常跑 | live 部分 `#[ignore]` |

`cargo test -p campus-auth` 全绿：lib 18（cas 14 + jwglxt 4）+ rsa_golden 3 + cas_parse 8 + captcha_solve 1 + jwglxt_live 常量防漂移 1 常跑（另有 4 个 ignored 待真机样本/凭据）。
