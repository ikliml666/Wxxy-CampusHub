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

受影响**只有**「鼠标点击 → IPC 命令」这一环。**该环已可用 CDP 远程调试点验**（见文末「用 CDP 绕过点击注入限制」）——
2026-09-18 M2.5 收尾轮已实测走通，不必再退化成"只凭源码级证据 + 标注未点验"。

## WebView2 不处理下载：不要用 `a[download]` 交付文件（2026-09-18 M2.5 收尾轮）

**现象**：课表页「导出 ICS」点击后完全无反应——无提示、`%USERPROFILE%\Downloads` 无文件、全盘搜不到新 `.ics`、页面无报错。用 CDP `Input.dispatchMouseEvent`（真实鼠标事件、带用户手势）再点一次仍无文件，**排除「缺用户手势」**。

**根因**：Tauri/WebView2 默认不接管下载（`DownloadStarting` 事件未被处理），前端 `URL.createObjectURL(new Blob(...))` + `a[download]` + `a.click()` 这条浏览器惯用路在桌面端 WebView2 里**静默失效**——不报错、不落盘、无任何反馈。

**修法（已落地）**：文件交付改由后端命令落盘——`export_ics` 生成 ICS 后用 `dirs::download_dir()` 写入「课表.ics」（覆盖写，`std::fs::write` 失败透出系统错误），命令返回写入的完整路径，前端只把路径显示在既有提示位。写盘逻辑抽成纯函数 `write_ics_to(dir, text)`，用 `%TEMP%` 临时目录单测覆盖（不依赖真实下载目录）。

**通用规则**：Tauri 桌面端凡「前端生成文件交给用户」的场景（ICS / CSV / 报告导出等），不要走 Blob 下载——在后端命令里写 `dirs::download_dir()`（或用 tauri-plugin-dialog 让用户选目录）并回显路径。验收时看文件是否真的出现在下载目录，不要只看前端无报错。

## 用 CDP 绕过点击注入限制（2026-09-18 M2.5 收尾轮，实测可用）

**一句话**：给 WebView2 开远程调试端口，用 CDP 在页面内触发真实事件——绕开「坐标点击进不来、UIA 不派发 click」的死局。

**启动**（不改仓库代码，只在启动 dev 时带环境变量）：
```bash
cd tauri-app && WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222" npx tauri dev
```
实测 `msedgewebview2.exe` 会带上该参数，`http://127.0.0.1:9222/json/version` 立即可用。

**两个必知的坑**：
1. **`/json/list` 在应用刚起时是空的**——page target 要等页面导航完成才注册；刚看到 `Running campus-hub.exe` 就查会误判"没有目标"（本次连踩两次），等 30~60 秒再查
2. **dev 因 Rust 改动重启后 target 也会短暂消失**；持续为空就彻底重启 dev（kill `campus-hub.exe` + 命令行含项目路径的 `node.exe`/`cargo.exe`）

**客户端**（本次落地于 `%TEMP%/cdp.ps1`，可直接复用）：`System.Net.WebSockets.ClientWebSocket` 连 `webSocketDebuggerUrl`，三种用法——
- `-ExprFile <js>`：`Runtime.evaluate`（`returnByValue` + `awaitPromise`，可跑 async 动作序列）
- `-Method <name> -ParamsFile <json>`：任意 CDP 方法
- `-RealClickExpr <js>`：先 evaluate 取元素中心坐标（表达式返回 `{x,y}`），再发 `Input.dispatchMouseEvent` 的 `mousePressed`/`mouseReleased`（**带用户手势**语义）

**页面内工具（提效关键）**：中文经 Bash→pwsh 传参会乱码，故把中文常量与工具**一次性注入页面**（`%TEMP%/inject.js`，中文写在文件里、由 `-Encoding UTF8` 读入），之后每次只传 ASCII：`window.__v.click("import")`。工具含：
- 按键名点击：`click`（精确）/ `clickContains` / `clickNth`
- **React 受控组件赋值**：`setNative`（native setter + `input`/`change` 事件；直接 `el.value=` React 感知不到）、`setSelect`（`HTMLSelectElement` 原生 setter）
- 断言读取：`body()` / `blocks()`（按 `aria-label` 取课程块）/ `noticePanel()` / `inputs()`
- 页面重载后 `window.__v` 丢失，需重新注入

**三个实战技巧**：
1. **索引错位坑**：`items = els.map(...).filter(...)` 之后 `els[i]` 不再对应 `items[i]`——必须保留 `[element, text]` 配对再筛（本次因此点错过菜单项）
2. **原生对话框阻塞 evaluate**：`window.confirm` 弹出会挂起 JS 线程、`Runtime.evaluate` 不返回（脚本超时）；点会弹 confirm 的按钮前先 `window.confirm = () => true`
3. **`element.click()` 足以触发 React onClick**（React 19 事件委托到 root，程序化 click 冒泡即命中），不必上 `Input.dispatchMouseEvent`；后者只在需要"真实用户手势"语义时用（本次用于排除「下载失败是因缺手势」的假设）

**本次据此点验通过**：导入/同步、课程详情浮层、手动添加（受控表单填值 + 提交）、调课通知解析与采纳、停课两档渲染、撤销通知调整、删除课程（含 confirm）、作息弹层编辑与保存/恢复默认、导出 ICS、深色模式切换与视觉。仅「旧 localStorage 8 面板持久化值的升级场景」仍未实测（需清掉现有 `campushub-ui` 再打开）。
