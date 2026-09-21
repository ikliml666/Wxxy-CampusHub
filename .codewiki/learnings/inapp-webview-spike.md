---
title: "应用内多webview spike 结论（Task 1）"
type: "learning"
source_files:
  - tauri-app/src-tauri/src/commands/browser.rs
tags:
  - tauri
  - webview
  - spike
  - inapp-browser
---

# 应用内多 webview spike 结论（2026-09-21 真机验证）

Tauri 2（tauri 2.11.5，Windows WebView2）主窗口内嵌第二 webview 的 API 面与六问验证结论。spike 代码 `commands/browser.rs`（Task 3 会重写为正式命令，API 事实保留于本文）。

## API 事实（编译器钉死）

- `Window::add_child(builder, position, size)` **三参数**（不是 brief 骨架的单参+事后 set_bounds；position/size 可传 `tauri::Rect`），返回 `Webview`。
- **无 `Window::remove_webview`**；销毁用 `Webview::close()`（异步落定，close 后立即重建同 label 有竞态窗口，需先 close_stale + 打点观察）。
- 多 webview 相关 API 全挂 **`unstable` feature 门**（`Manager::get_window/get_webview` 也在内）——`tauri-app/src-tauri/Cargo.toml` 的 tauri 依赖必须保留 `features = ["unstable"]`，正式功能绕不开。
- `Webview::set_bounds(tauri::Rect)` 可用；`Rect { position: Position, size: Size }` 用逻辑坐标铺排。
- `Webview` 无 `title()`，Rust 侧读不到页面标题；注入验证用 DOM 标记（红条）+ `window.__inapp` 旁证。

## 六问结论（真机 tauri dev 实测）

1. **布局** ✅ 截图实锤：主 webview 缩顶 48 逻辑px（React UI 被视口裁剪在条内、不崩），副 webview `add_child` 占剩余区正常渲染外站（example.com / my.cwxu.edu.cn）。
2. **initialization_script** ✅ 注入执行（`[spike] injected OK __inapp=1` 红条实锤），每次文档起始注入可用。
3. **on_navigation** ✅ 回调挂载成功且放行事件正常（`allowed=true` 打点）；拦截（返回 false）语义为纯函数行为，Task 3 冒烟补 baidu 拦截验证。
4. **close 复原** ✅ 日志：`close_child: ok` + `main_restore rect=(0,0 1920x1290): ok`，复原后 run#2 再次成功，React UI 无残留异常。
5. **resize** ⚠️ bounds API 可用，窗口拖拽交互未测——Task 3 正式实现挂 `WindowEvent::Resized` 重算（relayout），Task 7 集成验证补测。
6. **profile 持久化** ⚠️ 结构成立：run#2 载入 my.cwxu.edu.cn 正常走重定向链（my → /auth → wxcas CAS 登录页，与逆向分析的鉴权断点一致）；「登录一次后自持」需人工登录验证，留 Task 7/用户验收。

## 对后续任务的约束

- 正式 open/close 必须串行化保护：close 后 WebView2 销毁异步未落定时，立即同 label `add_child` 可能撞 label 冲突（spike 已加 close_stale 兜底，正式实现沿用）。
- 副 webview 页面默认无 IPC capability（不能 invoke Tauri 命令）——注入脚本不得依赖页面侧 invoke，宿主通信走 `on_navigation` / `on_page_load` 回调。
- dev 启动：`tauri-app` 目录 `npm run tauri dev`（根 package.json 装 @tauri-apps/cli；frontend 另有自己的 npm install）。端口 1420 冲突时先查孤儿 vite（`netstat -ano | grep 1420`）。

## Task 3 正式实现落盘（2026-09-21）

正式命令落在 `tauri-app/src-tauri/src/commands/browser.rs`（spike 代码已移除），冒烟真机验证通过：

- **API 签名补钉**：`on_page_load(Fn(Webview<R>, PageLoadPayload<'_>))`——payload 有 `.url()` 与 `.event()`，事件枚举 `tauri::webview::PageLoadEvent::Started/Finished`；`Webview::reload()` / `Webview::eval()` 真机可用；确认无 `Webview::navigate`，导航用 eval `location.href`（url 经 `serde_json::to_string` 成 JS 字符串字面量防拆串）。
- **设计决策（open 侧重建而非导航复用）**：brief 原写「已有 app-browser 则导航复用」，但 `Webview::close()` 异步销毁且无存活态判定——`get_webview` 返回 Some 无法区分「活的」与「销毁中」，复用会 eval 到悬空 webview。故 open 侧统一 `close_stale_child`（close + 每 50ms 轮询 `get_webview` 变 None，20 次 = 1s 超时放行）→ 重建，「单 webview、无多标签」契约语义不变。
- **失败回滚**：`add_child` 失败时主 webview 必须复原整窗，不留下 48px 裁剪态。
- **冒烟结论（2026-09-21 dev 真机）**：open my.cwxu.edu.cn → CAS 重定向链（my → /auth → wxcas.cwxu.edu.cn）全部 `allowed=true` 放行 + on_page_load started/finished 正常；eval 强跳 baidu → `allowed=false` 拦截（`browser://blocked` 路径执行）；close 复原成功。bounds 计算 `1280x812` 逻辑尺寸正确。
- **遗留观察（Task 7）**：注入 style 标签（`campushub-inject`）在副 webview 内不可远程观测（无 devtools 通道），本次以 initialization_script 机制沿用 spike 六问 2 已验证结论旁证；`WindowEvent::Resized` 高频 relayout 的拖拽流畅度未人工验证。
