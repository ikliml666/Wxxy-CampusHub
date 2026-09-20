---
title: 深澜 WebVPN 加密算法与网络归属探测（IV 派生与 TUN 抢路由坑）
type: learning
source_files:
  - crates/campus-webvpn/src/crypto.rs
  - crates/campus-webvpn/src/wrap.rs
  - tauri-app/src-tauri/src/infra/net_zone.rs
tags:
  - webvpn
  - srun
  - crypto
  - netzone
  - recon
---

# 深澜 WebVPN 加密算法与网络归属探测（2026-09-20 破解并 live 验证）

## 一、算法：AES-128-CFB128，key = IV = 固定 16 字节

深澜（Srun）WebVPN 网关的 host 段编码（逆向自 `webvpn.cwxu.edu.cn`，活样本
`docs/cas-recon/REPORT.md:68-69` 两条逐字节吻合）：

```
包装形态：{gateway}/{scheme}/hex(IV) + hex(AES-128-CFB128(key, IV, host)) + 原 path?query
key = IV = "wrdvpnisthebest!"（16 字节 ASCII 常量，crypto.rs:9）
```

- **CFB 是流式模式**：密文长度恒等于明文字节长度，**无 padding**（`crypto.rs:15-16`
  与长度单测 `crypto.rs:86-90` 钉死）。这解释了为何活样本 hex 段长度恰为
  `32 + 2×len(host)`——见到 padding 类实现（PKCS#7 等）即偏离官方行为。
- **本校 key/IV 为社区通行默认值**：活样本（`my.cwxu.edu.cn`、`10.3.100.110`）与
  社区通行的深澜算法向量共用同一 key，无需针对本校重推。
- **加解密对称**：CFB 加解密同为 keystream 异或，`decrypt_host` 可直接还原网关
  URL 中抠出的 `hex(IV)+hex(密文)` 整段（测试与网关落点解析复用，`crypto.rs:27-35`；
  IV 段不符报 `InvalidIv` 而不是解出乱码）。
- **golden 向量 6 条**（`crypto.rs:46-71`）：两条本校活样本 + 社区通行向量
  （`202.204.48.66`/`space.bilibili.com`/`219.216.96.4`）+ 本项目回归基线
  （`10.3.100.110 → 77726476706e69737468656265737421a1a70fcf696138003059d8fc`）。

## 二、实现纪律：前缀必须由 KEY 派生，禁止硬编码 `7772…`

`encrypt_host` 的输出前缀 `hex(IV)` 由 `hex::encode(KEY)` 现算（`crypto.rs:21`），
**任何调用方不得出现硬编码的 `77726476706e69737468656265737421` 字符串**（测试里
作 golden 期望值除外）。理由：key 与 IV 恒等是「本校现状」这一事实结论——将来
若遇到改了 key 的网关，只需改 `KEY` 常量一处；硬编码前缀则把事实抄进了两处，
改漏一处即产生静默的坏 URL。`wrap.rs` 测试里的 `HEX_103`/`HEX_MY` 常量是单测
期望值，不是运行时拼接来源。

## 三、探测坑：代理 TUN 网卡抢占默认路由

**本机实测（2026-09-20）**：主判据「UDP connect 到 10.3.100.110 取源 IP」在校内
真机上**误判校外**——代理工具的 TUN 虚拟网卡（Meta Tunnel，198.18.0.0/15 网段）
默认路由跃点数为 0，抢占了 UDP 选路，`local_addr()` 返回 TUN 网段地址而非真实
校园网 10.x 地址（`net_zone.rs:13-17` 头注）。

修复：辅判据 A——`netsh interface ip show addresses` 枚举**全部**接口 IPv4，任一
接口 ∈ 10/8 判 Campus（`net_zone.rs:88-92`）。成立性依据：10/8 是 RFC1918 私网，
家用宽带/VMware/TUN（198.18.0.0/15）网段都不会出现 10/8 接口地址，所以「任一接口
有 10/8」是校内的高置信信号；反过来 TUN 模式下用户通常同时在校园网物理网卡上
保持着 10.x 地址。教训：**「路由选路结果」反映的是最终出口而非物理归属**，凡是
本机可能挂着全局代理的场景，单一选路判据都会被 TUN 劫持，必须叠加接口枚举类
证据。

SSID 辅判据 B 只做单向佐证（含 "wxxy" 升 Campus，不匹配不降级）——有线连接时
`netsh wlan` 查不到 SSID 属正常，不能把「查不到」当「校外」。

## 四、live 验证与遗留

- 登录链（GET `/` 种 cookie → `sso_ticket(TGT, WEBVPN_SERVICE)` → ticket 回跳 302
  链）真机走通，深澜 5 cookie、`is_alive=true`（`tests/webvpn_session_live.rs`）。
- 包装 URL `…/http/<hex(10.3.100.110)>/charge/feeitem` 经网关返回 HTTP 200 真实
  feeitemList——网关代理内网 IP 已实锤。
- 未验证：`synjones-auth` 鉴权头经网关透传；校外真机端到端。

相关：[[modules/webvpn-routing|WebVPN 校外路由]]、[[decisions/webvpn-route-design|WebVPN 路由设计决策]]、[[learnings/cas-sso-plaintext-redirect|CAS SSO 回跳的坑]]（302 链 body 中断同款教训在 `session.rs:66-71`）。
