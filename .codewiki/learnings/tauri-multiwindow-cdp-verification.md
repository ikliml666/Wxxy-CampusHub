---
title: Tauri 多窗口应用的 CDP 点验：必须按 URL 选 target（附非主窗口 IPC 权限验证法）
type: learning
source_files:
  - tauri-app/src-tauri/src/commands/electricity.rs
  - tauri-app/frontend/src/components/CommandPalette.tsx
tags:
  - 验收
  - CDP
  - WebView2
  - 多窗口
  - m3
---

# Tauri 多窗口应用的 CDP 点验：必须按 URL 选 target

2026-09-19 M3 收尾轮真机点验时踩到，与 [[learnings/tauri-webview-ui-verification|既有 CDP 方法]] 配套看。

## 坑：开第二个窗口后，脚本会跑进「错的窗口」

`/json/list` 会同时列出多个 `type: "page"` 的 target。原 `cdp.ps1` 用 `Select-Object -First 1` 取第一个，**不保证是主窗口**——实测打开内嵌充值页后，主窗口脚本（点 Dock 项）实际跑进了充值窗口，表现为「元素找不到」且完全看不出原因（因为脚本本身没错，只是跑错地方）。

**处置**：用带 `-UrlFilter` 的版本 `%TEMP%/cdp2.ps1`，按 URL 片段定位：
- 主窗口：`-UrlFilter "localhost:1420"`（vite dev 地址）
- 内嵌第三方页：`-UrlFilter "charge-pc"`（或目标站点的路径片段）

**规则**：只要应用可能开多窗口，点验脚本一律显式指定 `-UrlFilter`，不要依赖 target 顺序。

## 顺带得到的安全边界验证法

非主窗口里的页面能否调我们后端？实测一条命令即可判定：

```js
await window.__TAURI_INTERNALS__.invoke('list_feeitems')
// → 抛错：DENIED: list_feeitems not allowed. Plugin not found
```

**能拿到 `__TAURI_INTERNALS__` 不等于能调命令**——它由 Tauri 内部注入、无法隐藏，真正拦截的是 capability（ACL）。不被任何 capability 覆盖的窗口（本项目内嵌充值页）=> 调用一律被拒。因此：

- 别把「页面能看到 `__TAURI_INTERNALS__`」当成漏洞，也别为此去关 `withGlobalTauri`（该选项管的是 `window.__TAURI__` 便捷封装，与本对象无关）。
- 新增内嵌窗口时**不需要**为它写 capability；反过来说，**一旦给它写了 capability，就等于把命令面暴露给第三方页面**，要极其慎重。

## 本轮据此点验通过（M3）

电费三级级联（片区→校区→楼栋→**手输房间号**→查询，结果为单键自由文本、负数行标红）；保存常用房间并确认落盘 `%APPDATA%/campushub/electricity_rooms.json`；钱包页（电子账户余额、1042 条流水分页、收入 `+`/支出 `-`）；首页钱包卡实时余额 + 「实时」来源标注；命令面板（触发条打开、模糊搜索命中页面项、Esc 关闭）；内嵌充值页为**登录态**（token 与 `localStorage.configs` 注入均生效，页面渲染出官方缴费表单而非登录页）。

未验项（如实标注）：**主窗口关闭时子窗口是否联动关闭**——`plugin:window|close` 被 ACL 拒（`core:window:allow-close` 未授予），CDP 无法触发窗口装饰按钮，只能人工点关闭按钮确认。
