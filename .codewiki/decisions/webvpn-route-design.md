---
title: WebVPN 路由设计决策（包装范围、会话 TTL 与写路径红线）
type: decision
source_files:
  - crates/campus-webvpn/src/route.rs
  - crates/campus-webvpn/src/session.rs
  - tauri-app/src-tauri/src/infra/net_zone.rs
  - tauri-app/src-tauri/src/commands/synjones.rs
  - tauri-app/src-tauri/src/commands/electricity.rs
tags:
  - webvpn
  - routing
  - security
  - offcampus
  - decision
---

# WebVPN 路由设计决策（包装范围、会话 TTL 与写路径红线）

2026-09-20（M4 批 2-4）。校外使用一卡通/电费需要把内网请求经深澜 WebVPN 网关代理，
本决策冻结四个边界：**包装哪些目标、会话怎么持有、探测不确定时怎么走、写路径怎么兜底**。
架构与注入点见 [[modules/webvpn-routing|WebVPN 校外路由]]。

## D1 包装范围：只包内网目标，公网域名永不包装

- **内网判据保守**（`route.rs:63-82`）：目标 host 是 IP 字面量且 ∈ 10/8，或命中
  `INTERNAL_HOSTS` 表（当前仅 `10.3.100.110`，列出是为显式钉住唯一目标）。**不搞**
  `.cwxu.edu.cn` 通配——公网校名域名校外本就可达，包装了收益为 0，反而引入网关
  cookie 域隔离与 service 白名单复杂度。
- **决策方向由「可达性」驱动而非「归属」**：`my.cwxu.edu.cn`（门户）、
  `wxcas.cwxu.edu.cn`（CAS）校外直接可达，进包装范围只会让 CAS 换票 service 白名单
  和 cookie 语义复杂化。将来真出现「公网域名但校外被墙」的服务，再单独加表。

## D2 Unknown 按直连处理，不做「直连失败改走 WebVPN」的二次尝试

- 校内是主场景，探测不明（`net_zone::Unknown`）时误拦用户（明明校内却要求 WebVPN）
  的成本高于误放；直连失败由上层中文报错兜底（`route.rs:12-14` 模块头注）。
- **不做自动二次尝试是刻意的**：读路径一次失败重试可承受，但写路径（充值建单/提交）
  的二次尝试触碰**资金安全红线**——失败原因若是「已受理但响应超时」，重试就是重复
  扣款。宁可不自动化（`route.rs:13-14`）。
- `route()` 包装失败（畸形 URL）保守 `Direct` 并交上层报错，不 panic 不 NeedLogin
  （`route.rs:98-101`）；`Unreachable` 枚举位预留不产生（「网关不可达」类判定需要
  网络观测，属上层职责，route 只做静态决策）。

## D3 会话：不落盘、内存持有、TTL 30 分钟静默重登

- **为什么不持久化 WebVPN cookie**（`session.rs:13-25` 头注）：登录态由网关一次性
  token 换发，持久化恢复价值低；而静默重登 = 一次 `sso_ticket` 换票 + 一次 302 链，
  **无验证码、无用户交互**，成本约等于一次请求。把 `RecordingJar` 的 (名, 值) 快照
  模型扩成 (名, 值, 域) 三元组以支持多域恢复，会牵动 M1 门户 `session.json` 格式与
  单域回填语义（`jar.rs:15`）——破坏面大于收益。
- **TTL 30 分钟**（`VPN_SESSION_TTL`，`commands/synjones.rs`）：过期静默重登而非
  每次 `is_alive` 探活——探活本身也是一次网络请求，TTL 内信任、过期重登的期望成本
  更低，且省一次串行请求。
- **login 失败 60 秒负缓存**（`VPN_LOGIN_BACKOFF`）：网关不可达 / TGT 过期时，
  高频命令（通知轮询、面板刷新）若每次都撞整条 SSO 链会把失败放大；窗口内直接回
  `ERR_OFFCAMPUS_RELOGIN` 可操作文案。换 TGT 重新登录后最多再等满 60 秒即恢复。
- **锁纪律**：`WEBVPN_SESSION` 是 tokio Mutex 且 guard 跨 await 持有是设计而非
  违例——login 是网络 IO，持锁排队正是「绝不并发 login」（网关侧一次性 token 换发）
  的实现方式；区别于项目常规「锁内只 clone」纪律的适用场景（后者针对瞬时同步赋值）。
  `net_zone::detect`（netsh 同步阻塞）绝不持任何锁执行。

## D4 写路径红线：校外失败绝不自动重试

- 支付类写路径（`recharge_create`/`recharge_submit`）校外失败由 `pay_err_text`
  追加固定提示「本操作不会自动重试，请改用『去官网充值』或连回校园网」
  （`commands/electricity.rs`）；`cancel`（清理未支付单）不追加——清理动作无害且
  必要，提示反而阻止用户收尾。
- 校外 + 无 WebVPN 会话（TGT 过期）→ `RouteDecision::NeedLogin` → 一律
  `ERR_OFFCAMPUS_RELOGIN`（「校外模式：登录态已过期，请重新登录后再试」），不引导
  用户反复撞网关。
- 匿名端点例外处理（`list_feeitems`）：免登录不等于免网关会话——校外经网关请求
  仍需网关 cookie，故校外未登录（无 TGT）时直接给可操作文案而不是尝试直连。

## D5 探测三层判据与缓存

- 主判据 UDP connect 选路源 IP（不发包）∈ 10/8；辅判据 A 接口地址枚举（覆盖代理
  TUN 抢路由，实测坑见 [[learnings/srun-webvpn-crypto|深澜 WebVPN 加密算法与网络归属探测]]）；
  辅判据 B SSID 单向佐证（不匹配不降级——有线时查不到 SSID 属正常，不能因查不到
  就改判校外）。
- 判据选 10/8 而非更紧的网段：10/8 是 RFC1918 私网，校外网络不可能给主机分配 10/8
  源地址，放宽不引入误判面，还能覆盖宿舍区/办公区不同 10.x 段。
- 60s TTL 缓存：归属在分钟尺度不变，高频命令不必每次探测。

## 未决项

- 带 `synjones-auth` 头的 berserker API 经网关透传未验证（live 只验证了匿名
  feeitem 端点 200）；校外真机端到端未验证（开发机在校园网内）。
- `.cwxu.edu.cn` 通配判据与「直连失败读路径自动重试」留待实锤服务出现再评估。
