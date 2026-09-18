---
title: "Tauri/WebView2 真机 UI 验收：vite 供旧模块与点击注入失效的可用替代路径"
type: learning
source_files:
  - tauri-app/frontend/src/panels/TimetablePanel.tsx
  - tauri-app/frontend/src/components/DockNav.tsx
  - tauri-app/frontend/src/shared/types.ts
tags: ["验收", "WebView2", "vite", "computer-use", "M2.5"]
---

# Tauri/WebView2 真机 UI 验收：两个坑与可用路径

2026-09-18 M2.5 课表真机验收时连续踩到两个坑，各花了不少轮次。记下判据与替代方案，
下次做「窗口内 UI 验收」直接照此排查，不要重新摸索。

## 坑一：vite dev server 会静默提供**旧模块**

**现象**：磁盘上 `DockNav.tsx` 已是 9 项（含课表），真机 Dock 仍只画 8 项；`ctrl+r` 刷新无效。

**判据（一条命令定案）**：
```bash
curl -s http://localhost:1420/src/components/DockNav.tsx | grep -o 'id: *"[a-z]*"'
# 磁盘有 9 项、这里只回 8 项 → dev server 的转译缓存未失效（watcher 漏掉了子代理写入的文件）
```

**处置**：`touch <该文件>` 后 dev server 立刻重新转译（实测 `touch` 后再 curl 即含 `timetable`）。
注意**不要**用 `tsc`/构建报错来判断，缓存陈旧时编译是正常的。

**连带结论（刷新语义）**：
- `touch src/shared/types.ts`（纯类型模块）→ vite 发 **page reload**（整页重载，组件重新挂载、`useEffect` 重跑）
- `touch src/panels/XxxPanel.tsx`（组件模块）→ 只发 **Fast Refresh**（保留 state，**`useEffect` 不会重跑**）
- 所以「让挂载时逻辑再跑一次」必须触发整页 reload 的那一类文件
- `key ctrl+r` 在窗口非前台时会被 CUA 拒绝（`frontmost_pid_mismatch`），不如 touch 稳

## 坑二：鼠标/键盘注入**进不了** Tauri 窗口

实测证据链（2026-09-18 本机）：

| 尝试 | 结果 |
|---|---|
| CUA `left_click` 坐标 event | 落点被 ZCode 窗口接收（`GetForegroundWindow()` = ZCode pid 18772）；`open_application(activate=true)` 报告 `active=true` 也拦不住 ZCode 在处理下一次工具调用时抢回前台 |
| PowerShell `SendInput` 真实点击 | `SetCursorPos` 生效（光标可见移动），但 `WindowFromPoint(1207,716)` 返回 **explorer 的 `FolderView`**（桌面层），而非 WebView2 子窗口（`hwnd=131738`） |
| UIA `InvokePattern.Invoke()` / CUA `AXPress` | 返回 OK、元素变 `focused`，**但 Chromium/WebView2 不派发 DOM click**；`onClick` 不执行 |
| 例外 | `DockNav` 的面板切换按钮**有效**（点 element 就能切页），说明不是全盘失效，而是页面内 React 按钮这类元素不派发 |

**结论**：本机环境下「自动化点击页面内按钮」这条路走不通，别再把轮次花在换注入方式上。

## 可用的替代验收路径（本轮采用，有效）

1. **临时验收代码**：在目标面板挂载处插一个 `useEffect`（用 `useRef` 保证只跑一次），按序调用
   `loginSaved`（会话过期时）→ `add_course_manual` → `import_timetable` → `parse_notice` → `apply_override` → `export_ics`，
   把每步结果拼成文本塞进已有的 `setImportMsg`。
2. **结果回读通道**：摘要条在真机上可能被视口遮挡且不进 AX 树前 200 项，故再把结果写进一条临时课程名
   （`add_course_manual` 的 `name = "TEMPLOG|" + log`），用 `grep -o "TEMPLOG|[^\"]*" "$APPDATA/campushub/timetable.json"` 读回。
3. **收尾必须清理**：
   - `git checkout -- <改动文件>` 还原临时代码，并 `git status --porcelain` 确认未混入提交
   - 删除 `%APPDATA%/campushub/timetable.json` 里的验收产物（本轮直接删文件并备份到 `%TEMP%/`）

## 仍然可靠的真机证据

- **窗口渲染**：`get_app_state(include_screenshot=true)` 截图 + AX 树（元素名可读：`row 信息安全周二 5-6 节 · 第 1-16 周 · …导入编辑删除`、
  `button 信息隐藏与取证技术，周四第2至2大节`、【导】/【调】角标文本）——「渲染是否正确」这类验收不受影响
- **落库结果**：直接读 `%APPDATA%/campushub/*.json`（`timetable.json` 的课程数/override/`autoApplied`，`session.json` 的 `tgtB64` 是否存在）
- **协议层**：`CAMPUS_HUB_CREDS=<凭据文件> cargo test -p campus-auth -- --ignored jwglxt`（真实账号 SSO + 拉课表）

受影响**只有**「鼠标点击 → IPC 命令」这一环；该环只能用源码级证据（dev server 下发的模块中确认存在
`onClick: doImport`）加上命令本身的单测覆盖，须在汇报里如实标注未点验。
