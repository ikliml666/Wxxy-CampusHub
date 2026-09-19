---
title: 门户会话失效是 HTTP 200 信封而非错误码（误分类成「解析失败」的连锁）
type: learning
source_files:
  - "crates/campus-auth/src/cas.rs"
  - "crates/campus-auth/src/error.rs"
  - "crates/campus-portal/src/client.rs"
  - "crates/campus-portal/tests/portal_diag_live.rs"
tags:
  - portal
  - cas
  - session
  - error-classification
  - recon
---

# 门户会话失效是 HTTP 200 信封而非错误码（误分类成「解析失败」的连锁）

2026-09-19 真实账号实测定位（`crates/campus-portal/tests/portal_diag_live.rs`，用 `session.json` 复原的隔夜会话逐字复现用户报错）。用户看到的是「各界面资讯获取失败：获取登录信息失败: 响应解析失败: tryLoginUserInfo 缺少 userName」，**根因不是门户改版，而是「会话已过期」被两个既有缺口放大成了误导性文案**。

## 坑一：失效不用 HTTP 状态码表达，而是 200 + `meta`

`POST /tryLoginUserInfo` 在会话失效时返回 **HTTP 200**、`content-type: application/json`，body 是：

```json
{"meta":{"success":false,"statusCode":302,"message":"未登录或会话已过期，请重新登录！"},"data":null}
```

它与**完全匿名**请求的响应逐字段相同（都 `len=116`、`data:null`、`statusCode:302`）。也就是说 **HTTP 层一切正常，到期信息只在 JSON 信封里**——任何用 HTTP 状态码判会话的做法在这里都会失效。

**修**：先解信封再取字段。`extract_user_profile`（`cas.rs:445`）改为「`data` 缺失/为 `null` 或 `meta.success==false` → `PortalNotLogin`」，JSON 解析失败才仍是 `Parse`。

## 坑二：首页可达 ≠ 会话有效

`portal_probe` 原判据只有「jar 里有 `customsid`」+「GET 门户首页未被弹回 CAS 域」——实测**死会话两条都满足**，于是返回 `Alive`，客户端以为自己还登录着，`check_session` 也不清会话，用户进入「显示已登录、点什么都报错」的死状态。

**修**：在首页判据之后**追加一次鉴权信封判定**（`cas.rs:260`），`meta.success==true` 才算 `Alive`。代价是启动多一次请求，换来的是失效会话在启动时就被发现并引导重新登录。

## 坑三：错误分类口径不足会把「要求重新登录」伪装成「解析 bug」

链路 `cas.rs:417` → `error.rs` → `client.rs` 的 `profile_err` 原本把**所有非 HTTP 错误**一律归 `PortalError::Parse`，于是 `PortalError::NotLogin`（文案「请先登录」）在这条链路上**永远不会被触发**，用户和开发者都被指向「响应结构解析有问题」这个错误方向（甚至怀疑学校改了接口）。

**教训（可复用）**：解析层要分清「对端语义上的否定」（未登录/无权限/风控）与「结构不符合预期」（真解析失败）；对端明确给出的 `message`/状态位必须**优先**于我们对字段的期待。文案上，「缺少 X 字段」只应在结构真的不符合预期时出现，否则它会掩盖真正的业务状态。

暴露面也值得一提：`auth_head` 是**所有门户接口的公共前置**，所以一处鉴权失败会同时打挂资讯/待办/应用/日程——排查时先看公共前置，别逐页找。
