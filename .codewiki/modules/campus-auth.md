---
title: CAS 协议核心（campus-auth）
type: module
source_files:
  - crates/campus-auth/src/cas.rs
  - crates/campus-auth/src/rsa.rs
  - crates/campus-auth/src/jar.rs
  - crates/campus-auth/src/captcha.rs
  - crates/campus-auth/src/error.rs
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
| `cas.rs` | CasClient：kaptcha → login → sso_follow → portal_probe |
| `jar.rs` | RecordingJar：记录型 CookieStore（会话检测与持久化的数据源） |
| `captcha.rs` | 算术验证码识别（颜色不变强度图 + NCC 最近邻） |
| `error.rs` | `CampusAuthError { Rsa, Http, Parse }`（`error.rs:5-12`） |

## 公开 API

**rsa.rs**
- `rsa_encrypt_hex(plaintext) -> Result<String>`（`rsa.rs:28-49`）：CAS 线上 textbook RSA，公钥 e=0x010001、n 为固化常量 `CAS_RSA_N_HEX`（`rsa.rs:16`）；chunkSize=126 字节（`rsa.rs:22`），块尾补 0x00 后整块反转（等价 JS little-endian 组块），密文 hex 小写不补前导零、块间直接拼接。**非 ASCII 明文显式拒绝**（`rsa.rs:29-32`）——JS `charCodeAt` 语义对非 ASCII 不等价，为避免静默产出不一致密文（计划 P2-11 修正项）。
- `cas_token_header(now_ms)`（`rsa.rs:52-54`）：请求头 `token` = RSA(`"lyasp" + 毫秒时间戳`)，TAG=lyasp。

**cas.rs**（`CasClient`，`#[derive(Clone)]`，clone 共享同一 jar）
- `new()`（`cas.rs:86-105`）：构建两个 reqwest Client——`http`（默认重定向策略）与 `http_manual`（`Policy::none`，手动跟随），共享同一 `Arc<RecordingJar>`；UA 为 cas.js 同款 Chrome 126 串（`cas.rs:25`）。
- `kaptcha()`（`cas.rs:113-122`）：`GET {CAS_BASE}/kaptcha` → `CaptchaInfo { uid, png_base64, kaptcha_type }`；png_base64 已剥掉 `data:image/png;base64,` 前缀（`parse_kaptcha`，`cas.rs:294-308`）。
- `login(username, password_rsa_hex, captcha_uid, captcha_code) -> Result<CasLoginOk, CasLoginError>`（`cas.rs:128-169`）：`POST {CAS_BASE}/v1/tickets`，表单体 7 字段（username/password/service/loginType/id/code/otpcode，`build_login_body` `cas.rs:253-274`），头 `token` + Referer + Origin。成功响应顶层即 `{tgt, ticket}`（兼容 data 包裹与 data 为纯字符串，`parse_login_response` `cas.rs:314-344`）。
- `sso_follow(service_url, ticket)`（`cas.rs:174-220`）：`GET service?ticket=` 手动跟随 302 链（上限 10 跳），返回落点 URL。原因见下文「关键实现要点」。
- `portal_probe()`（`cas.rs:229-245`）：会话探测 → `SessionState::{Alive, Expired}`，判据取舍见 [[decisions/recording-jar-session|RecordingJar 决策]]。
- `jar()`（`cas.rs:108-110`）：jar 句柄，供 check_session 与持久化。

**jar.rs**（`RecordingJar`，实现 `reqwest::cookie::CookieStore`，`jar.rs:73-103`）
- `snapshot() -> Vec<(String, String)>`（`jar.rs:39-41`）：全部已见 cookie（不含域信息，计划冻结接口）。
- `restore(&[(name, value)])`（`jar.rs:45-61`）：按门户域 `https://my.cwxu.edu.cn/` 回填进内置 Jar（`RESTORE_URL` `jar.rs:15`），同名覆盖、幂等。
- `has(name)`（`jar.rs:64-66`）：浅检测（check_session 用 `customsid`）。

**captcha.rs**
- `solve(png_bytes, &KaptchaTemplates) -> Option<String>`（`captcha.rs:76-79`）：PNG → 算式答案（如 `"63"`）；None = 无法识别，上层刷新重试，绝不硬猜。
- 模板集编译期内嵌自 `templates/kaptcha-templates.json`（`include_str!`，`captcha.rs:56`），由 `tests/captcha_solve.rs::build_templates` 从标注样本生成。

## 错误码映射表（16 项，`map_error_code` `cas.rs:348-356`）

失败响应形态 `{"meta":...,"data":{"code":"..."}}`。全集与映射以 REPORT.md:30 为准：

| 错误码 | CasLoginError 变体 | 语义 |
|---|---|---|
| `NOUSER` | `WrongUserOrPwd` | 账号或密码错误 |
| `CODEFALSE` | `WrongCaptcha` | 验证码错误 |
| `USERLOCK` | `UserLocked` | 账号锁定 |
| `TWOVERIFY` | `NeedTwoVerify(data_raw)` | 二次验证（携带 data 原文，含「已连续错误N次，阈值M」计数提示） |
| 其余 12 码 | `Unknown(原样码)` | `USERDISABLED` / `PASSERROR` / `USERNOTONLY` / `PEOPLEMOREACCOUNT` / `ISBINDOTP` / `ISBINDWX` / `ISMODIFYPASS` / `NETWORKCOMMITMENT` / `NOREGISTER` / `NOAUTHORIZATION` / `OTPERROR` / `PENDINGACTIVATE` |

另有 `Network(String)` 变体承接网络层错误（`cas.rs:67`）。16 码全覆盖测试见 `tests/cas_parse.rs:25-52`。

## 关键实现要点

**sso_follow 为何手动跟随重定向**（`cas.rs:92-94,171-173`）：该 302 链中存在响应 body 中断的跳（hyper `IncompleteMessage`），reqwest 默认重定向策略会因读 body 失败使整链报错；手动跟随只要拿到 Location 与 Set-Cookie 即可继续，body 异常可忽略（`cas.rs:200` 尽力读、错误丢弃）。附带两处协议修正：每跳前把 cwxu 域名的 http 升级为 https（只升不降，`cas.rs:183-189`）——REPORT.md 实测门户 302 落点是 http:// 明文而服务端对明文请求直接断连；结束后落点若仍为 http 同样替换（`cas.rs:211-218`）。

**portal_probe 判据取舍**（`cas.rs:222-228` 注释，逐条实测）：正文匹配不可用（门户首页 HTML 恒含 `lyuapServer/login` 常量 2 处，会把已登录判成 Expired）；`/shiro-cas` 端点不可用（无 ticket 访问触发服务端断连）；最终判据 = jar 有 `customsid` 且访问门户首页未被弹回 CAS 域，未登录场景由 `customsid` 缺失挡住（首页未登录也返回 200 外壳）。精确过期检测待 M2 接鉴权 API 后替代（`cas.rs:228`）。

**验证码识别管线**（题面恒为「一位数 运算符(+/-/*) 一位数 =」，PNG 100×25）：垂直投影切分（`SEG_MIN=0.12` 相对峰值阈值，`captcha.rs:28,220`）→ 片数 ≠4 直接 None（字符断裂/粘连时片序不可信，错位片可能自信匹配出错误答案消耗 CAS 连续错误计数，`captcha.rs:85-91`）→ bbox 左上角锚定 16×14 画布（渲染位置逐位确定，缩放会引入形变噪声）→ 颜色不变强度图（`765-Σrgb` 除以本图峰值，`captcha.rs:207-217`）→ NCC + ±1px 位移补偿匹配（`captcha.rs:163-181`）。阈值 `NCC_MIN=0.88` 与平局间隙 `NCC_GAP=0.03` 按样本标定（类内 min 0.931 / 跨类 max 0.839，`captcha.rs:29-31,146-160`）；同类多模板先聚合为每类一个最大分再跨类比较，否则正常类内差异会被平局判定误杀（`captcha.rs:145-147`）。

## 测试清单

| 测试文件 | 内容 | 运行条件 |
|---|---|---|
| `tests/rsa_golden.rs` | golden 对拍（独立 BigInt 实现固化，`rsa_golden.rs:1-16`）+ 非 ASCII 拒绝（`rsa_golden.rs:27-34`） | 离线常跑 |
| `tests/cas_parse.rs` | 16 码映射全覆盖、成功响应三形态解析、login body 构造、RecordingJar 记录行为 | 离线常跑 |
| `tests/captcha_solve.rs` | 合成图 roundtrip（全链路）+ `build_templates` 模板生成 + 100 张样本三分类评测（门槛：正确率 ≥98% 且自信错误 =0） | 评测部分 `#[ignore]` |
| `tests/cas_live.rs` | 全流程 live 冒烟（凭据经 `CAMPUS_HUB_CREDS` 环境变量指向文件，真实凭据绝不入代码，`cas_live.rs:1-11`） | `#[ignore]`，主智能体验收时跑 |
