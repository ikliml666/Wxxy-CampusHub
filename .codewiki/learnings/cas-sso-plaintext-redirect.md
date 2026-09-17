---
title: CAS SSO 回跳的三处坑：重定向中断、明文落点、探测误判
type: learning
source_files:
  - "crates/campus-auth/src/cas.rs"
  - "crates/campus-auth/tests/cas_live.rs"
  - "docs/cas-recon/REPORT.md"
  - "docs/cas-recon/probe.js"
tags:
  - cas
  - sso
  - http
  - debug
---

# CAS SSO 回跳的三处坑：重定向中断、明文落点、探测误判

真实账号 live 验证时，`POST /lyuapServer/v1/tickets` 已成功签发 `ST-`/`TGT-`，但紧接着的 SSO 回跳（`GET {service}?ticket=ST-…`）连续报错。三处独立问题，全部实测定位。

## 坑一：默认重定向策略在中间跳 body 中断时整链失败

症状：`hyper_util::client::legacy::Error(SendRequest, hyper::Error(IncompleteMessage))`。

门户 302 链里存在响应体被中断的跳；reqwest 默认重定向策略会读 body 再跟随，读到中断即整链报错。

**修**：`sso_follow` 改为**手动跟随**（独立 client 用 `redirect::Policy::none()`，与主 client 共享同一 cookie jar）：每跳只取 `Location` 与 `Set-Cookie`（cookie 由 jar 自动吸收），body 用 `let _ = resp.bytes().await;` 尽力读、失败忽略（`cas.rs:169` 一带）。

**通用教训**：REST 会话建立类流程里，「拿到 Location/Set-Cookie 就能继续」的场景，不要依赖自动重定向——它对中间跳的容错为零。

## 坑二：302 落点是 http 明文，服务端对明文请求直接断连

REPORT.md 早已记录「门户 302 落点是 `http://my.cwxu.edu.cn/#/index` 明文」。手动跟随循环里继续用 http 请求该 URL 时，服务端直接中断连接（同一个 `IncompleteMessage`）。

**修**：循环内**每次请求前**把 cwxu 域名的 http 升级为 https（只升不降，避免来回跳），`cas.rs:183` 一带的 `upgrade` 闭包。这一步必须在循环内，不能只在循环结束后处理（否则请求已经发出去了）。

## 坑三：会话存活探测的两次误判

`portal_probe()` 最初判定「正文含 `lyuapServer/login` 即过期」——**门户首页 HTML 本身恒含该字符串 2 次**（前端路由常量，curl 实测下载量 12206 字节），已登录状态也会被判 Expired。

改用 `/shiro-cas` 端点探测——**无 ticket 访问该端点会触发服务端断连**，请求直接 Err。

**最终判据**（`cas.rs:222` 一带）：`jar.has("customsid")` + 请求门户首页、最终 URL 的 host 不在 `wxcas.cwxu.edu.cn`。未登录场景由 `customsid` 缺失挡住（门户首页未登录也返回 200 外壳，前端路由才跳登录页，故单纯 200 不足以判定）。

**通用教训**：SPA 站点的「正文特征匹配」极不可靠（常量池污染）；探测端点要选**行为差异化**的，且先手工验证两种状态下的响应。

## live 验收结果

`CAMPUS_HUB_CREDS=<凭据文件> cargo test -p campus-auth -- --ignored cas_live` 通过：验证码自动识别 → RSA 加密 → CAS 签发 ST/TGT → 手动跟随 302 链 → 门户种下 `customsid`/`rememberMe`/`Authorization` 三 cookie → `portal_probe() == Alive`。

## 相关

- [[modules/campus-auth|CAS 协议核心]]
- [[decisions/recording-jar-session|RecordingJar 与会话持久化决策]]
- [[learnings/kaptcha-arithmetic-five-roots|验证码识别五根因]]
