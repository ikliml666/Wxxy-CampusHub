---
title: 前端外壳（frontend-shell）
type: module
source_files:
  - tauri-app/frontend/src/App.tsx
  - tauri-app/frontend/src/main.tsx
  - tauri-app/frontend/src/shared/tauriApi.ts
  - tauri-app/frontend/src/shared/types.ts
  - tauri-app/frontend/src/shared/cn.ts
  - tauri-app/frontend/src/stores/uiStore.ts
  - tauri-app/frontend/src/components/AppShell.tsx
  - tauri-app/frontend/src/components/DockNav.tsx
  - tauri-app/frontend/src/panels/LoginPanel.tsx
  - tauri-app/frontend/src/panels/TodayPanel.tsx
  - tauri-app/frontend/src/panels/InfoPanel.tsx
  - tauri-app/frontend/src/panels/TodoPanel.tsx
  - tauri-app/frontend/src/panels/SchedulePanel.tsx
  - tauri-app/frontend/src/panels/AppsPanel.tsx
  - tauri-app/frontend/src/panels/WalletPanel.tsx
  - tauri-app/frontend/src/panels/PowerPanel.tsx
  - tauri-app/frontend/src/panels/SettingsPanel.tsx
tags:
  - react
  - zustand
  - navigation
  - login
  - gsap
---

# 前端外壳（frontend-shell）

`tauri-app/frontend/src`：React 19 + TypeScript + Tailwind v4。分层：`shared/`（IPC 契约与工具）、`stores/`（zustand）、`components/`（外壳与 shadcn 源码入库的 `ui/*`）、`panels/`（8 面板）。设计语言与域色 token 见 [[concepts/domain-color-system|域色编码系统]]。

## IPC 出口与契约类型（shared/）

- `tauriApi.ts:8-17`：`invokeCommand<T>(cmd, args)` 是**唯一 IPC 出口**，invoke 抛错包装为 `{ success:false, message:String(e) }`，调用方只处理 CommandResult（契约详见 [[modules/campus-hub-tauri|接线层]] 与 [[_architecture|架构总览]]）。
- `types.ts:1`：`CommandResult<T>`；`types.ts:4-12`：`PanelId` 冻结 8 项（today/info/todo/schedule/apps/wallet/power/settings），注释明示 M2.5 追加 `"timetable"` 时须同步改 types + DOCK_ITEMS + persist 兼容。
- `cn.ts:3-5`：clsx + tailwind-merge 的 `cn()`。

## uiStore（zustand + persist）

`stores/uiStore.ts:7-22`：仅两个状态——`activePanel`（当前面板，默认 "today"）与 `displayName`（问候语用，非凭据）。整体经 `persist` 中间件存 localStorage（key `campushub-ui`）。displayName 需要 persist 的原因：重启后 `check_session` 保持登录但命令面不返回显示名，问候语从 persist 恢复（`uiStore.ts:5-6`）；**密码绝不进 localStorage**（仅存在于表单 state，`LoginPanel.tsx:29-30`）。

## App.tsx：登录态门卫

`AppPhase = "checking" | "in" | "out"`（`App.tsx:7`）。挂载后调 `check_session`：`loggedIn` 则直接进 AppShell（重启保持会话），否则进登录页（`App.tsx:14-23`）。登出走乐观更新——UI 立即切回登录页，`logout` 命令后台执行，重启时 check_session 兜底（`App.tsx:26-31`）。

## AppShell：顶条 + 面板容器 + Dock

`components/AppShell.tsx`：

- 顶条（`AppShell.tsx:60-117`）：搜索按钮（⌘K 占位）、通知（占位）、深浅主题切换（`documentElement.classList.toggle("dark")`，`AppShell.tsx:38-41`）、头像下拉（极简实现不引 DropdownMenu 依赖；打开期间 `mousedown` 点外关闭，`AppShell.tsx:44-56`），菜单仅一项「退出登录」。
- 面板注册表 `PANEL_MAP: Record<PanelId, ComponentType>`（`AppShell.tsx:18-27`），8 面板静态 import。
- **切换动画**：`useDeferredValue(activePanel)` 保证快速连切时只渲染最终面板、AnimatePresence 不闪烁（`AppShell.tsx:31-33`）；`<AnimatePresence mode="wait">` 包 `motion.div`（key=deferredPanel，进入 spring stiffness 400 / damping 40，退出 0.04s 淡出，`AppShell.tsx:119-132`）。

## DockNav：悬浮 Dock 导航

`components/DockNav.tsx`，macOS Dock 式底部悬浮导航：

- **8 项配置** `DOCK_ITEMS`（`DockNav.tsx:25-39`）：每项 `{ id, label, icon, color }`，color 引用域色 CSS 变量（今日=brand、资讯=info、待办=todo、日程/应用=sched、钱包/电费=wallet、设置=text-2）。
- **gsap 磁吸**：注册每项 `gsap.quickTo(btn, "scale"/"y")`（duration 0.35，ease expo.out，`DockNav.tsx:68-79`）；容器 `onMouseMove` 以 RAF 节流，按指针到各项中心的距离插值——80px 半径内 scale 最高 1.35、上浮最高 -14px（`MAGNETIC_RANGE/MAX_SCALE/MAX_LIFT` `DockNav.tsx:41-43`；插值逻辑 `DockNav.tsx:107-124`）。
- **reduced-motion 降级**：`prefers-reduced-motion: reduce` 命中则不注册磁吸（`magnetEnabled=false` 直接短路，`DockNav.tsx:61-63`）；清理时 `gsap.killTweensOf` + 移除 resize 监听（`DockNav.tsx:95-103`）。
- **域色胶囊**：激活项用 `layoutId="dock-pill"` 的 motion.span 共享布局动画（spring 500/34），背景 `color-mix(in srgb, 域色 14%, transparent)`；底部 `layoutId="dock-dot"` 同域色圆点（`DockNav.tsx:159-187`）。两个 layoutId 使胶囊/圆点在项间平滑滑动。
- Tooltip（shadcn 源码入库 `ui/tooltip.tsx`）250ms 延迟显示项名。

## LoginPanel：登录页四态状态机

`panels/LoginPanel.tsx:8-10`：`idle → logging-in → success(切 AppShell，面板即卸载) | manual | error`。

- 提交 `login`；若返回 `message === "CAPTCHA_MANUAL" && "uid" in data` → `manual` 态：展示验证码图（裸 base64 拼 data URI）+ 答案输入，重试走 `login_manual`（`LoginPanel.tsx:57-67,84-102`）；其余失败 → `error` 红字提示（`role="alert"`）。
- 已保存账号：mount 时 `list_accounts` 拉取，空列表/失败整块隐藏（无占位，`LoginPanel.tsx:39-44,206-226`）；点胶囊走 `login_saved` 免密重登。
- 双形态 data 按 `"uid" in data` 收窄（对应 Rust 侧 `LoginResultData` untagged）。

## TodayPanel：今日页骨架（M1 骨架，数据 M2+ 接入）

`panels/TodayPanel.tsx`：分时段问候（5-11 早 / 11-18 午 / 其余晚，`TodayPanel.tsx:13-17`）+ 钱包三卡（一卡通/邮箱/图书借阅，数值以 `--` 占位待 M3 慧新E校实时数据，`TodayPanel.tsx:19-24`）+ 下一节课横幅（恒 null 以验证「无数据整行隐藏」，`TodayPanel.tsx:26-33`）+ 快捷动作（查电费/卡片充值可切面板，其余占位，`TodayPanel.tsx:34-46`）。卡片左上角 2×2 域色角标是全站卡片统一视觉记号（wallet 卡 `bg-wallet`、下一节课 `bg-sched`，`TodayPanel.tsx:64-67,79-81`）。

其余 7 面板（Info/Todo/Schedule/Apps/Wallet/Power/Settings）目前均为 12 行占位骨架，M2+ 逐个填充。

## shadcn 源码入库

`components/ui/{button,card,input,tooltip}.tsx` 为 shadcn 组件源码直接入库（非 CLI 生成依赖），消费 `index.css` 里映射好的 shadcn 语义变量（--primary/--ring/--destructive 等，见 [[concepts/domain-color-system|域色系统]]）。
