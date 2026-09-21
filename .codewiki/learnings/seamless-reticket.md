---
title: "校 CAS 无 cookie 设计与应用内免密补票"
type: "learning"
source_files:
  - tauri-app/src-tauri/src/commands/browser.rs
  - crates/campus-auth/src/jwglxt.rs
tags:
  - cas
  - sso
  - inapp-browser
  - webview
---

# 校 CAS 无 cookie 设计与应用内免密补票（2026-09-21）

**核心事实**：校 CAS（lyuapServer）为**无 cookie 设计**——`CasClient::sso_ticket` 实测登录后 jar 为空，会话复用全靠客户端保存 TGT（`crates/campus-auth/src/jwglxt.rs:46` 注释钉死）。推论：Rust jar 的登录态**不存在**以 cookie 形态进入 WebView 的路径，也不存在"种 TGC"——各应用的免密只能**各自换票**。

## 免密补票机制（browser.rs on_navigation）

1. WebView 内应用 302 到 CAS 登录页（真机实证该 URL 携带 `service` 参数经过 on_navigation）。
2. `on_navigation` 拦下（return false），解析 query 的 `service`，`sso_ticket(tgt, service)` 现换 ST。
3. 带票回跳 `{service}&ticket={ST}`（service 自带 query 用 `&`，session.rs 同款）——应用验票后在**自己域**种会话 cookie（正常进 WebView cookie store），后续自动自持。
4. on_navigation 是同步回调而换票是 async → spawn 里换+导航（clone CasClient/TGT 进 future）；失败送回登录页手登降级。
5. **防循环护栏**：ST 被拒时应用再 302 回登录页会无限换票 → 同 service 8s 冷却窗。

**真机实锤**（2026-09-21 dev）：教务系统 `jwgl` 拦票换票后 302 链自动进学生主界面（`index_initMenu.html?jsdm=xs`）；whall 深层 URL 直接作 service 验票通过（金智框架接受任意深度 URL）。TGT 过期 → 换票失败 → 手登降级，与现状一致。

**边界**：校外场景的免密未接线——B 类目标需先 wrap、`service=内网 IP`（一卡通桥）校外不可达，属下一批路由集成。课程类分类剔除（SchedulePanel chip）判据：name 含「课表」或 code 含 course，与 isCourseEvent 一致。
