---
title: "应用内浏览器（inapp-browser）"
type: "module"
source_files:
  - tauri-app/src-tauri/src/commands/browser.rs
  - tauri-app/src-tauri/src/browser_inject.js
  - tauri-app/src-tauri/src/commands/electricity.rs
  - tauri-app/frontend/src/stores/browserStore.ts
  - tauri-app/frontend/src/components/browser/BrowserOverlay.tsx
  - tauri-app/frontend/src/components/browser/BrowserToolbar.tsx
  - tauri-app/frontend/src/components/browser/BrowserStatusView.tsx
  - tauri-app/frontend/src/components/browser/BrowserEventsBridge.tsx
  - tauri-app/frontend/src/shared/constants.ts
tags:
  - browser
  - webview
  - multiwebview
  - inapp-browser
---

# 应用内浏览器（2026-09-21 第一批）

三个"系统浏览器打开"入口（通知原文 `open_in_browser` / 门户应用目录 `open_app` / 电费充值兜底 `open_recharge_in_browser`）改为应用内 WebView 打开。域外应用（知网/万方/超星等 `access: "external"`）保持系统浏览器。**免密直达（CAS TGT 换 ST）属第二批**，本批在 webview 内手动登录一次后由 WebView2 profile 持久化自持。

## 布局模型（核心机制）

主窗口内嵌第二 webview（Tauri 2 multiwebview，**需 Cargo.toml `unstable` feature**）：

- 打开：主 webview（React）`set_bounds` 缩到顶部 48 逻辑px（React UI 被视口裁剪成顶栏条，BrowserToolbar 在这条内渲染）；副 webview label `app-browser` 占剩余区域加载校方页面。**不依赖 webview 控件 z-order**。
- 关闭：`Webview::close()` + 主 webview bounds 复原整窗。`WindowEvent::Resized` → `relayout` 重算两 webview bounds（有 browser 则分割、无则主全窗，幂等）。
- API 事实与六问真机结论见 [[learnings/inapp-webview-spike|多webview spike 结论]]：`add_child(builder, position, size)` 三参、无 `remove_webview`、close 异步落定（open 侧 close_stale 轮询等待 + 1s 超时放行）。

## 打开决策与命令面

- `decide_open(url) -> OpenDecision{InApp, External, Blocked}`（纯函数，单测含 `cwxu.edu.cn.evil.com` 域后缀攻击）：InApp 判据 = `campus_portal::is_allowed_info_url`（*.cwxu.edu.cn，**红线勿放宽**）或 host == `10.3.100.110`（充值内网 IP，SSRF 面与既有 open_app 等价：前端无法借道开 10.3.100.110 之外的地址）；External → 前端降级旧 `open_app` 系统浏览器；Blocked → 中文报错。
- 命令六个：`open_in_app_browser(url)` / `close_app_browser` / `app_browser_navigate` / `reload` / `back` / `forward`（后四者本批无 UI 调用方，接口先行冻结；`Webview` 无 navigate API，用 eval 实现）。
- **store 感知契约（C1 教训）**：凡 Rust 侧不经前端 `browserOpen` 直接调 `open_url_inapp` 建副 webview 的入口（电费 `open_recharge_in_browser` 转调），**必须让前端感知**——`open_url_inapp` 成功路径 emit `browser://opened {"url"}`，`BrowserEventsBridge`（挂 AppShell、常驻不随 overlay 卸载）listen 后调 `storeSyncOpen(url)` 同步 store。新增此类入口时零额外接线，漏接就是"无工具栏/无关闭按钮的应用内死路"。

## 事件契约（Rust → 前端，逐字冻结）

`browser://load {phase:"started"|"finished"}`（on_page_load）｜`browser://nav {url}`（on_navigation 放行）｜`browser://blocked {url}`（白名单外导航拒绝，副 webview 无 IPC capability，页面不能 invoke）。前端 listen 分工：三事件由 BrowserOverlay 挂（open 时）；`browser://opened` 由 BrowserEventsBridge 常驻挂。

## store 状态机（browserStore.ts）

`open/nav/loading/blockedUrl/errorMsg`（弹层态，不落盘）+ `history`（persist 键 `campushub-browser` v1，上限 20 去重置顶）。canBack/canForward 首批恒 false（无导航栈事件源）。**30s watchdog**：所有置 `loading:true` 的路径（browserOpen inApp / browserNav / storeSyncOpen / **setLoading(true) 的 started 回写**）都 arm，`setLoading(false)` 与 browserClose 撤——`setLoading(true)` 分支是 scoped re-review 抓出的缺口（页内导航/刷新丢 finished 时 loading 永久卡死），修复语义：兜底对全部 loading 场景生效。

## 注入脚本（browser_inject.js，include_str!）

IIFE + `__campushub_injected` 防重复，按 host 分桶：表格/img 防溢出、`:focus-visible` 焦点环（全部站点）；`my.cwxu.edu.cn` 正文居中；`10.3.100.110` min-width 防移动布局糊；`target=_blank`/`window.open` 改当前 webview 内导航；alert 仅 console 打点不改行为（弹窗治理本批只收集）。**三禁**：不自动填表提交、不 hook 官方 JS 校验、不伪造跨域 cookie。弹窗隐藏名单灰度后再上。

## 设计语义（防后续误改）

- overlay 开着时点域外应用：External 降级开系统浏览器、**overlay 保持打开**（保留浏览上下文，本批 UI 上该路径不可达——48px 视口内点不到入口）。
- `StatusBar`（CAS 登录页提示"该系统需登录"）覆盖工具栏整条而非 EmptyState 整区替换：React 视口被裁到 48px，整区替换物理上不可见。
- 工具栏高度唯一常量：前端 `shared/constants.ts` `BROWSER_TOOLBAR_H=48`，Rust 侧 `TOPBAR_LOGICAL` 同值（下批补 Rust→前端方向互指注释）。

## 已知问题 / 下批清单

免密直达（CAS TGT 换 ST 导航落点 + WebVPN 会话预置，spike 六问遗留：CAS cookie HttpOnly 与 user data folder 隔离）；Esc 不让位 palette/loginDialog（多层双关）；`includes("/login")` 对 query 误报；errorMsg 已有弹层外渲染口但面板内仍无 toast；host 提取重复 ~5 行与 NoticeBar 抽取；browserOpen in-flight 锁；导航栈事件（启用 back/forward）；下载链接处理（WebView2 原生行为待观察）；slow 10s 提示与 30s watchdog 的文案合并。
