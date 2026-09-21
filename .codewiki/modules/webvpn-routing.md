---
title: WebVPN 校外路由（campus-webvpn 与注入点全景）
type: module
source_files:
  - crates/campus-webvpn/src/lib.rs
  - crates/campus-webvpn/src/crypto.rs
  - crates/campus-webvpn/src/wrap.rs
  - crates/campus-webvpn/src/route.rs
  - crates/campus-webvpn/src/session.rs
  - crates/campus-webvpn/tests/webvpn_session_live.rs
  - crates/campus-synjones/src/lib.rs
  - crates/campus-synjones/src/client.rs
  - crates/campus-synjones/src/sso.rs
  - crates/campus-synjones/src/charge.rs
  - crates/campus-synjones/src/recharge.rs
  - crates/campus-synjones/src/ecard_face.rs
  - tauri-app/src-tauri/src/infra/net_zone.rs
  - tauri-app/src-tauri/src/commands/synjones.rs
  - tauri-app/src-tauri/src/commands/electricity.rs
tags:
  - webvpn
  - srun
  - routing
  - offcampus
  - m4
---

# WebVPN 校外路由（campus-webvpn 与注入点全景）

2026-09-20（M4 批 2-4）。校外也能用一卡通/电费：新增协议 crate `crates/campus-webvpn`（深澜 Srun WebVPN 的 URL 包装 + 会话层，无 Tauri 依赖，与 [[modules/campus-auth|CAS 协议核心]] 同「协议单点」纪律），`crates/campus-synjones` 与命令层做**注入式**接入——校内路径一行未改，校外路径由同一份代码自动改走网关。设计取舍见 [[decisions/webvpn-route-design|WebVPN 路由设计决策]]，加密算法与探测坑见 [[learnings/srun-webvpn-crypto|深澜 WebVPN 加密算法与网络归属探测]]。

## 一、campus-webvpn crate（三模块 + 路由）

- **`crypto.rs`**：AES-128-CFB128 host 加解密。key=IV=`wrdvpnisthebest!`（`crypto.rs:9`），`encrypt_host` 输出 `hex(IV)+hex(密文)`（`crypto.rs:17-22`）；**前缀由 key 派生计算，调用方禁止硬编码 `7772…` 字符串**。流式无 padding，密文长度=明文字节长度。6 条 golden 向量含本校活样本（`crypto.rs:46-71`，`10.3.100.110 → …a1a70fcf696138003059d8fc`）。算法细节与破解过程见 [[learnings/srun-webvpn-crypto|深澜 WebVPN 加密算法与网络归属探测]]。
- **`wrap.rs`**：`wrap_url(raw, gateway)` 拼装 `{gateway}/{scheme}/{hex(host)}{path?query#fragment}`（`wrap.rs:16-53`）。要点：scheme 段跟随目标（https → `/https/`）；非默认端口写 `{scheme}-{port}`（`/http-8080/`，`wrap.rs:30-34`）；**host 已是网关域时幂等原样返回**（`wrap.rs:26-28`）——「先判后包」不二次包装的地基。`GATEWAY = "https://webvpn.cwxu.edu.cn"`（`wrap.rs:8`），勿在别处复制。
- **`route.rs`**：纯函数决策表 `route(raw_url, zone, has_vpn_session)`（`route.rs:85-104`），无 IO 可全量单测：

  | zone | 目标 | 有会话 | 决策 |
  |---|---|---|---|
  | Campus / Unknown | 任意 | 任意 | `Direct` |
  | OffCampus | 公网域名 | 任意 | `Direct` |
  | OffCampus | 内网目标 | true | `Wrapped(wrap_url 结果)` |
  | OffCampus | 内网目标 | false | `NeedLogin` |

  内网判据**保守**：IP 字面量 ∈ 10/8 或命中 `INTERNAL_HOSTS`（`route.rs:57-82`）；`.cwxu.edu.cn` 通配等宽判据留待实锤再放。`Unreachable` 枚举位预留、本函数不产生（`route.rs:51-53`）。
- **`session.rs`**：`WebVpnSession::login(cas, tgt)` 三步（`session.rs:89-101`）：① GET 网关 `/` 种初始 cookie `wengine_vpn_ticketwebvpn_cwxu_edu_cn`（`session.rs:36`）；② `CasClient::sso_ticket(tgt, WEBVPN_SERVICE)` 用现有 CAS TGT 换 WebVPN 的 ST（复用 [[modules/campus-auth|CAS 协议核心]]，无验证码无交互）；③ ticket 回跳手动跟随 302 链（`follow_wrap`：**每一跳 Location 先 join 成绝对 URL 再过 `wrap_url`**，`session.rs:142-180`）。`is_alive`（`session.rs:108-121`）：GET `/` 落 `/login` = 失效；`wrapped_client()` 供业务请求（自动跟随，与登录共用 jar）。协议事实源 `docs/cas-recon/REPORT.md` 三·二节。

## 二、网络归属探测（`infra/net_zone.rs`）

`NetZone::{Campus, OffCampus, Unknown}` 三态，与 crate 侧 `route::NetZone` 同名同义、由命令层逐变体映射（`to_route_zone`，`commands/synjones.rs`，两侧任一改名编译期暴露）。

- **主判据**：`UdpSocket::connect("10.3.100.110:80")` 后取 `local_addr()` 源 IP（**UDP connect 不实际发包**，仅内核路由选路，微秒级；`net_zone.rs:48`），源 IP ∈ 10/8 → Campus。
- **辅判据 A**：`netsh interface ip show addresses` 枚举全部接口 IPv4，任一 ∈ 10/8 → Campus（覆盖代理 TUN 抢路由，见 [[learnings/srun-webvpn-crypto|深澜 WebVPN 加密算法与网络归属探测]] 的 TUN 坑）。
- **辅判据 B**：SSID 含 "wxxy" 单向佐证（不匹配不降级）。
- 60s TTL 缓存（`net_zone.rs:55-58`）；netsh 是同步子进程，async 侧必须 `spawn_blocking`（`detect_zone`，`commands/synjones.rs`）。

## 三、注入点全景（campus-synjones 与命令层）

`SynjonesClient` 加两个字段（`client.rs`）：`base_override: Option<String>` 与 `vpn: Option<WebVpnSession>`，经 `set_webvpn(base, vpn)` 注入、`(None, None)` 复位直连。**全部 base 拼接收口三处**：

| 收口点 | 位置 | 说明 |
|---|---|---|
| `base_url()` | `client.rs` | `get`/`post_form`/`post_json` 三条业务路径全部经它拼 base（grep `10.3.100.110` 无遗漏） |
| `effective_http()` | `client.rs` | 业务 HTTP 句柄：WebVPN 模式返回网关会话 client（网关 cookie jar），否则 CAS 共享 client；`recharge::json_post`（`recharge.rs:622-635`）与 fapi 人脸（`ecard_face.rs:27` `fapi_base_of`）等旁路请求一律经它 |
| `sso_token_via` | `sso.rs:178` | 静默重进桥的 WebVPN 链：换票 service 保持 CAS 注册的内网原值（CAS 公网可达），**回跳入口才包装**（`vpn_entry_url`，`sso.rs:165`），302 链逐跳先判后包；token 仍从落点 query 提取 |

命令层接线（`commands/synjones.rs`）：

- `synjones_session_routed`（替代原 `synjones_session` 主路径）：锁外做路由决策 → 校外 `Wrapped` 时 `ensure_webvpn_session` 建/复用网关会话并 `set_webvpn` 注入；**非重建路径同样刷新路由态**（zone 在 client 生命周期内可变化）。
- 进程级 `static WEBVPN_SESSION`（TTL 30 分钟，过期静默重登）+ **login 失败 60 秒负缓存**（`VPN_LOGIN_BACKOFF`）：网关不可达时高频命令不再反复撞整条 SSO 链。锁纪律：`WEBVPN_SESSION` 用 tokio Mutex 且 guard 允许跨 await（login 是网络 IO，持锁排队正是串行化）；`net_zone::detect` 绝不持任何锁执行。
- 旧入口 `synjones_session` 保留为兼容签名（`ecard.rs`/`electricity_history.rs` 调用方）：路由照走，校外失败降级 `None`（文案欠精确但不阻塞界外模块）。
- 电费匿名端点 `list_feeitems` 单独处理：**匿名也要网关会话**（网关 cookie），校外未登录（无 TGT）直接给可操作文案（`commands/electricity.rs`）。
- `open_recharge_in_browser` 走 `browser_recharge_url`：校外把内网官网页包装成网关 URL（浏览器自身持有 WebVPN 登录态）。

## 四、live 验证状态（2026-09-21 更新）

- **已验证**：WebVPN 登录链真实走通（深澜 5 cookie、`is_alive=true`，`tests/webvpn_session_live.rs`）；`http://10.3.100.110/charge/feeitem` 经包装 URL 由网关返回 HTTP 200 真实 feeitemList（16101 字节）——**「网关是否代理内网 IP」这一最大风险已排除**。
- **已验证（2026-09-21 校外全链路探针 PASS，`tauri-app/src-tauri/tests/webvpn_offcampus_live.rs`）**：CAS 登录 → `WebVpnSession::login` → **`sso_token_via` 经网关跑 SSO 桥换到慧新E校 token（307 字节 bearer）** → `wrapped_client` GET `queryCurrentCard` 经网关 HTTP 200，业务信封 code=200 且 `data.retcode="0"`（网关换的 token 被一卡通业务层完整接受），真实余额与批 14 真机记录吻合。此前「berserker API 带 `synjones-auth` 头经网关透传」缺口就此实锤。顺带固化响应形态：`data={account,card,errmsg,retcode,sno}`（`retcode` 双层判定的第二层在业务层）。
- **未验证**：`net_zone` 判 OffCampus 后的自动触发（应用内接线）需校外真机端到端——当前开发机在校园网内，route 决策表由单测覆盖。

相关：[[modules/campus-synjones|慧新E校协议核心]]、[[modules/campus-hub-tauri|接线层 campus-hub-tauri]]、[[decisions/webvpn-route-design|WebVPN 路由设计决策]]、[[learnings/srun-webvpn-crypto|深澜 WebVPN 加密算法与网络归属探测]]。
