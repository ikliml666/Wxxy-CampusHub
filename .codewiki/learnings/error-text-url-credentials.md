---
title: "错误文案回显 URL 凭据——reqwest 错误串泄票据与 redact_secrets 脱敏"
type: learning
source_files:
  - crates/campus-synjones/src/lib.rs
  - crates/campus-synjones/src/client.rs
  - crates/campus-synjones/src/charge.rs
  - crates/campus-synjones/src/recharge.rs
  - crates/campus-synjones/src/sso.rs
  - tauri-app/src-tauri/src/commands/synjones.rs
tags:
  - security
  - error-handling
  - reqwest
  - cas
  - m4.5
---

# 错误文案回显 URL 凭据（2026-09-19 真机点验发现，M4.5 批 4 修复）

## 现象

真机点验时（校园网断开、SSO 换票失败），一卡通页的错误文案里带出了**完整 URL**，其中 `ticket=ST-232555-…`（CAS 服务票据）原样可见。这不是日志泄露——是**给用户看的 UI 文案**本身就把票据显示了出来；诊断日志与 live 探针同走 `Display` 文案，三条路径一起中招。

## 根因（两层叠加）

1. **reqwest 的请求错误会把完整 URL 拼进错误串**（`error.send()` 类错误的 Display 就是 `error sending request for url (…)`）。网络层失败（断网/超时/DNS）恰恰是最容易走到这条路径的场景。
2. **本系统的 URL query 里就带凭据**：CAS 换票是 `GET /berserker-auth/cas/login/lyCas?targetUrl=…&ticket=ST-…`（见 [[modules/campus-synjones|慧新E校协议核心]] §二），`sso.rs` 的 SSO 链路把这个 URL 原样发请求——失败时 `SsoFailed(format!(…含 url…))` 就把票据带出来了。其余键同样危险：`synjones-auth`（bearer token）、`token` / `access_token`、`password` / `pwd`、`vercode`（短信验证码）都可能以 query/form 形态出现在某个请求 URL 上。

即：**「错误透传 `e.to_string()`」在 URL 带凭据的系统里就是凭据泄露**，与是否打日志无关。

## 修法：错误构造点统一脱敏

`crates/campus-synjones/src/lib.rs:145` 新增 `pub fn redact_secrets(&str) -> String`：把上述**凭据键的值**抹成 `***`（键名大小写不敏感），其余文本原样保留。

- **落点在错误构造点**（不是命令层一处兜底）：
  - `client.rs:240` / `charge.rs:466` / `recharge.rs:635` 的 `Http(e.to_string())` → `Http(crate::redact_secrets(&e.to_string()))`；
  - `sso.rs:137,141` 两处 `SsoFailed(format!(…))` → 内层包一层 `redact_secrets`；
  - 命令层 `commands/synjones.rs:92` 的 `err_text` 兜底再过一遍（防新增错误变体漏改）。
- **三条消费路径同时受益**：UI 错误文案（`CommandResult.message`）、诊断日志（stderr）、live 探针输出——它们共享同一批 `Display` 字符串，源头干净则全链路干净。
- 脱敏键表：`ticket` / `synjones-auth` / `token` / `access_token` / `password` / `pwd` / `vercode`。新增带凭据的参数名时要**同步扩这张表**。

## 实现踩坑（两个，都是测试当场抓的）

1. **字节边界 panic**：第一版用 `rfind(键名).map(|p| p + 1)` 回看键名定位值起点——`rfind` 返回**字节**下标，错误文案里常见**多字节分隔符（中文括号「）」）**，`p + 1` 落在多字节字符中间直接 panic（`byte index N is not a char boundary`），被既有单测 `elec_err_messages_are_actionable`（错误文案必须可读、不得损坏）当场抓到。正确做法：键名回看用 `char_indices().rev() … .map(|(p, c)| p + c.len_utf8())` 拿**字符边界**，或干脆只在「值允许字符集」内推进。
2. **终止符列表吞全角标点**：第二版把「值的结束」定为终止符列表（`&` `?` 空格等），结果全角右括号「）」不在表里，被当成值的一部分一起吞掉，文案变成「…（ticket=***」缺了右括号。正确做法：**值用允许字符集界定**（值只由 URL 安全字符组成，遇到集合外字符即结束）——白名单界定天然兼容任意分隔符，永远不会吞错。

## 可复用结论

> **凡把外部错误原文透出给前端的路径，都要按凭据参数名脱敏。** `e.to_string()` / `format!("{}", e)` 在错误构造点出现时，先想想错误里会不会嵌 URL；URL 带凭据（票据、token、密码参数）的系统里，这条防线必须在**错误构造点**统一过，而不能指望每个下游消费点各自处理。脱敏实现里，字符串边界一律走 `char_indices()` 字符边界 + 白名单字符集，**不要用字节偏移 + 终止符列表**（两个坑都实测踩过）。
