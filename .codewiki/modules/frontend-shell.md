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
  - tauri-app/frontend/src/stores/authStore.ts
  - tauri-app/frontend/src/components/AppShell.tsx
  - tauri-app/frontend/src/components/AccountMenu.tsx
  - tauri-app/frontend/src/components/LoginDialog.tsx
  - tauri-app/frontend/src/components/Avatar.tsx
  - tauri-app/frontend/src/components/AvatarDialog.tsx
  - tauri-app/frontend/src/components/PanelHeader.tsx
  - tauri-app/frontend/src/components/EmptyState.tsx
  - tauri-app/frontend/src/components/Surface.tsx
  - tauri-app/frontend/src/components/DockNav.tsx
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
  - avatar
  - portal
  - gsap
---

# 前端外壳（frontend-shell）

`tauri-app/frontend/src`：React 19 + TypeScript + Tailwind v4。分层：`shared/`（IPC 契约与工具）、`stores/`（zustand：authStore 登录态 + uiStore 界面态）、`components/`（外壳与共享组件 + shadcn 源码入库的 `ui/*`）、`panels/`（8 面板）。设计语言与域色 token 见 [[concepts/domain-color-system|域色编码系统]]；游客模式与账号系统的取舍见 [[decisions/guest-mode-account-shell|游客优先与账号外壳决策]]。

## IPC 出口与契约类型（shared/）

- `tauriApi.ts:8-17`：`invokeCommand<T>(cmd, args)` 是**唯一 IPC 出口**，invoke 抛错包装为 `{ success:false, message:String(e) }`，调用方只处理 CommandResult（契约详见 [[modules/campus-hub-tauri|接线层]] 与 [[_architecture|架构总览]]）。
- `types.ts:1`：`CommandResult<T>`；`types.ts:3-41`：M2 批次 1 门户契约 4 接口（`SemesterInfo` / `WalletSummary` / `CourseBrief` / `PortalOverview`，镜像 Rust `commands/portal.rs` DTO，camelCase，子项均可 null）；`types.ts:43-55`：`PanelId` 冻结 8 项（today/info/todo/schedule/apps/wallet/power/settings），注释明示 M2.5 追加 `"timetable"` 时须同步改 types + DOCK_ITEMS + persist 兼容。
- `cn.ts:3-5`：clsx + tailwind-merge 的 `cn()`。

## authStore：登录态单一来源（zustand + persist）

`stores/authStore.ts`：

- **三态模型** `AuthStatus = "unknown" | "guest" | "authed"`（`authStore.ts:9`）：unknown = 启动探测中、guest = 未登录、authed = 已登录。所有登录态消费方（AccountMenu、面板空态、AvatarDialog）只读 `status`，不再各自探测。
- **字段**：`username` / `displayName`（身份）、`avatarBase64` / `avatarSource`（头像，`"local" | "official" | null`）、`accounts`（已保存账号列表）。persist 键 `campushub-auth`，`partialize` **只持久化 username+displayName**（`authStore.ts:232-235`）——重启后先显示上次身份，`check_session` 兜底纠正；头像与账号列表由命令面拉取，凭据与 DPAPI 密文永不进 localStorage。
- **游客不展示账号头像**：`refreshAvatar` 在 `status !== "authed"` 时直接清空 `avatarBase64/avatarSource`（`authStore.ts:121-127`）——本地与官方头像都属账号资产，退出后回落「锡」占位。
- **动作**：`checkSession`（非阻塞探测，落定后顺带 `refreshAccounts` + `refreshAvatar`，`authStore.ts:97-115`）；`login` / `loginManual` / `loginSaved` 三入口成功后统一 set authed + 刷新账号与头像，且**首次登录且本地无头像时后台补一次官方头像**（失败静默，`authStore.ts:149,182`）；`logout` 本地清空五字段（`authStore.ts:187-196`）；头像四动作 `uploadAvatar/uploadOfficialAvatar/syncOfficialAvatar/clearAvatar` 经 `applyAvatar` 统一收口（成功且带 data 才覆盖，`authStore.ts:73-84,198-222`）；`removeAccount` 成功后刷新列表（`authStore.ts:224-230`）。
- **`uploadOfficialAvatar(imageDataUrl)`**（2026-09-18 新增，`authStore.ts:66,206-212`）：调后端 `upload_official_avatar` 把裁切后的头像上传回学校系统；返回的 `AvatarData` 即服务端回读的最新头像，`applyAvatar` 直接落 store——学校与本机头像一次动作同步更新。
- **untagged 双形态收窄**：`isLoginOk`（`"username" in data`）/ `isCaptchaPayload`（`"uid" in data`）类型守卫对应 Rust 侧 `LoginResultData` untagged（`authStore.ts:36-43`）。

## uiStore：界面态（面板路由 / 主题 / 弹层开关）

`stores/uiStore.ts:11-42`：`activePanel` + `theme`（两者持久化，键 `campushub-ui`）+ `loginDialogOpen` / `avatarDialogOpen`（一次性 UI 状态，`partialize` 排除，`uiStore.ts:39`）。displayName 已迁往 authStore。登录/头像弹层的**唯一开关**在此：`openLoginDialog()` / `openAvatarDialog()` 供任意入口调用，弹层组件挂载在 App 根部监听同一状态。

## App.tsx：游客模式（壳恒渲染）

`App.tsx:8-12`：挂载后 `void checkSession()` 非阻塞启动探测——`status: "unknown"` 期间壳与面板照常可交互，落定 guest/authed 后各组件自行响应（游客空态 / 登录态内容）。渲染恒为 `<AppShell /> + <LoginDialog /> + <AvatarDialog />`（`App.tsx:14-20`），**没有全屏登录门禁**：原「未登录即渲染登录页」的 `AppPhase` 门卫已删除（旧 `panels/LoginPanel.tsx` 整文件移除，逻辑迁入 LoginDialog）。登录入口收敛为三处，全部经 `uiStore.openLoginDialog()` 打开同一弹层：AccountMenu、TodayPanel 引导条（`TodayPanel.tsx:102-116`）、各面板 EmptyState 的「登录」按钮。

## AppShell：顶栏 + 面板容器 + Dock

`components/AppShell.tsx`：

- **新顶栏**（`AppShell.tsx:51-112`）：sticky、`bg-bg/85` 毛玻璃。左：品牌块（渐变方标「锡」+ 双行标识，`AppShell.tsx:53-69`）；中：搜索胶囊（命令面板 M2 占位，`aria-disabled` 不做假交互，`AppShell.tsx:71-92`）；右：铃铛占位（M5 接入）+ AccountMenu 账号胶囊（`AppShell.tsx:94-110`）。
- **主题单向同步**：`useEffect` 把 uiStore `theme` 同步到 `documentElement.classList.toggle("dark")`（`AppShell.tsx:43-46`）；开关入口在 AccountMenu 的「深色模式」行。
- **面板注册表** `PANEL_MAP: Record<PanelId, ComponentType>`（`AppShell.tsx:25-34`），8 面板静态 import。
- **切换动画**：`useDeferredValue(activePanel)` 保证快速连切时只渲染最终面板、AnimatePresence 不闪烁（`AppShell.tsx:39-41`）；`<AnimatePresence mode="wait">` 包 `motion.div`（key=deferredPanel，进入 spring stiffness 400 / damping 40，退出 0.04s 淡出，`AppShell.tsx:114-129`）。

## AccountMenu：账号系统（顶栏右上角）

`components/AccountMenu.tsx`，36px 胶囊触发器（Avatar + 名字 + chevron，`AccountMenu.tsx:299-319`）+ 右对齐 motion 菜单（点外/Esc 关闭、reduced-motion 降级，`AccountMenu.tsx:231-242,321-344`）。**按 `status` 分两套菜单**：

- **authed**（`AccountMenu.tsx:346-498`）：头部（姓名 + 学号去重显示 + 头像来源标注「学校头像」）→ 上传头像（开 AvatarDialog）→ 同步学校头像（busy 转圈 + 内联结果文案，`AccountMenu.tsx:273-284`）→ 折叠式切换账号（当前账号 ✓ 禁用，其余一键免密 `loginSaved`，pending 就地转圈、失败内联红字不关菜单，`AccountMenu.tsx:396-468`；删除账号带 `window.confirm` 确认，`AccountMenu.tsx:258-270`）→ 外观行（深色 Switch）→ 设置 → 退出登录。
- **guest**（`AccountMenu.tsx:499-568`）：占位头像「锡」+ 登录引导文案 → 「登录」主按钮（开 LoginDialog）→ 已保存账号快捷登录列表（hover 出 X 删除，`AccountMenu.tsx:126-179,526-543`）→ 外观/设置。
- 上次登录时间格式化 `fmtLastLogin`：epoch 毫秒串 → `YYYY-MM-DD HH:mm`，缺失/非数字不显示、不回退原文（`AccountMenu.tsx:30-38`）。

## LoginDialog：登录弹层（原 LoginPanel 迁移）

`components/LoginDialog.tsx`，受 `uiStore.loginDialogOpen` 控制的居中弹层：遮罩点空白关闭、Esc 关闭、弹层期间锁 body 滚动（`LoginDialog.tsx:67-79,181-185`）。状态机 `idle → logging-in | manual | error`（`LoginDialog.tsx:14`），成功即关弹窗：

- **登录结果统一收口** `applyAuthResult`（`LoginDialog.tsx:84-107`）：成功 → 清密码关弹窗；`message === "CAPTCHA_MANUAL"` → 先 `get_captcha` 取新图（失败用响应自带 payload 兜底，`manualPayloadFrom` `LoginDialog.tsx:24-30`）进手动模式；其余 → 红字。手动模式展示验证码图 + 答案输入 + 「换一张」（`LoginDialog.tsx:256-301`），重试走 `loginManual`。
- **已保存账号胶囊**：点击免密 `loginSaved`；若该账号也被要求验证码则回落表单手动模式并自动补学号（`LoginDialog.tsx:153-177`）。
- 密码仅表单 state（`LoginDialog.tsx:43`），绝不进 localStorage。

## 头像三态：Avatar + AvatarDialog

**三态优先级：本地 > 官方 > 首字默认**（后端裁决见 [[modules/campus-hub-tauri|接线层]] profile 一节）。

- `components/Avatar.tsx`：四档尺寸 sm/md/lg/xl（`Avatar.tsx:3-8`）；有 `src` 渲染 `<img>`，无 src 渲染品牌渐变（`linear-gradient(140deg, brand, info)`）+ 姓名首字、空则「锡」占位（`Avatar.tsx:34-50`）。
- `components/AvatarDialog.tsx`（2026-09-18 重写）：受 `uiStore.avatarDialogOpen` 控制，选图 → `react-easy-crop` 1:1 取景（拖拽/滚轮/滑杆缩放，另有圆/方形状切换与重置）→ Canvas 导出。**必须** `import Cropper, { type Area } from "react-easy-crop"` 并 `import "react-easy-crop/react-easy-crop.css"`（`AvatarDialog.tsx:1-3`），漏掉 css 裁切器无样式。选图管线 `handleFile`（`AvatarDialog.tsx:221-242`）：objectURL + `img.decode()`，旧的由 revoke effect 释放（`AvatarDialog.tsx:176-178`）。
- **两条保存路径**（与后端阈值对齐，详见 [[modules/campus-hub-tauri|接线层]] 与 [[learnings/portal-avatar-upload-protocol|门户头像上传协议]]）：
  - `仅保存到本机`（`handleSaveLocal` `AvatarDialog.tsx:245-264`）→ `renderLocal`（`AvatarDialog.tsx:77-95`）：边长 `min(LOCAL_MAX_SIDE=1024, 裁切边长)` 不放大，有 alpha 出 PNG、否则 JPEG q0.92，PNG 超 `LOCAL_MAX_BYTES=2MB` 回退白底 JPEG → `set_avatar`。
  - `保存并上传学校`（`handleSaveSchool` `AvatarDialog.tsx:266-289`，仅 authed）→ `findSchoolImage`（`AvatarDialog.tsx:97-114`）：尺寸阶梯 `SIZE_LADDER=[1024…320]` 外层 × 质量阶梯 `QUALITY_LADDER=[0.95…0.6]` 内层取第一个 ≤`SCHOOL_MAX_BYTES=200KB` 的组合（保留最大尺寸，源分辨率不足 clamp 不放大）→ `uploadOfficialAvatar(dataUrl)` 成功后**同一份图再写本机**并提示「已上传学校 · WxH · N KB」，1.2 秒后自动关闭。白底只在 JPEG 路径铺（`drawCrop` `AvatarDialog.tsx:27`，防透明区转 JPEG 发黑）。
- **体积预估与落盘同源**：裁切变化后防抖 250ms 用 `renderLocal`/`findSchoolImage` 真实重编码（`AvatarDialog.tsx:191-215`），保存时以同一函数重算——预估与落盘必然一致（真机实测预估 139 KB ≈ 落盘 141385 字节）。
- **Hook 顺序约束**：`onCropComplete` 的 `useCallback` 必须在 `if (!open) return null;`（`AvatarDialog.tsx:219`）之前声明——写在之后会因 `open` 变化改变 Hook 数量，React 19 直接卸载整个根节点（打开弹窗即白屏），详见 [[learnings/portal-avatar-upload-protocol|门户头像上传协议]] 第 5 节。
- 「同步学校头像」与「移除本地头像」按钮仅 authed 可见（`AvatarDialog.tsx:545-566`）；「保存并上传学校」对游客禁用并以 tooltip「登录后可上传到学校」提示（`AvatarDialog.tsx:507`）。

## 共享组件：PanelHeader / EmptyState / Surface

- `components/PanelHeader.tsx`：面板页头（域色圆点 + 标题 + 描述 + 右侧动作位）。`PanelDomain` 类型与 `domainVar` 域色 → CSS 变量映射在此定义，EmptyState/Surface 复用（`PanelHeader.tsx:4-20`）。
- `components/EmptyState.tsx`：面板空态（图标域色染色圆底 `color-mix 12%` + 标题 + 提示 + 动作位，compact 两档密度，`EmptyState.tsx:6-54`）。
- `components/Surface.tsx`：统一卡片容器（`rounded-card border-line bg-surface shadow-card`，可选域色左上角 8px 角块与 hover 上浮 `shadow-lift`，`Surface.tsx:5-36`）——卡片域色角标的组件化形态。

## 面板：游客空态与数据接入中

8 面板统一节奏：`PanelHeader` 页头 + `Surface` 卡片 + `EmptyState` 空态。**游客**（status=guest）显示「登录后查看×××」+ 登录按钮（经 `openLoginDialog`）；**已登录未接线**显示「数据接入中」（如 WalletPanel.tsx:50-54）。TodayPanel 自 2026-09-18 M2 批次 1 起接真实数据（见下节），游客态保留可关闭的登录引导条（本地 state 不持久化，`TodayPanel.tsx:153-166`）与分时段问候；快捷动作为游客点已落地项先登录、未落地项禁用 + tooltip（`TodayPanel.tsx:231` 起）。

## TodayPanel：今日页真实数据（M2 批次 1）

登录态下经 `invokeCommand` 调 `get_portal_overview` 单命令取数（`TodayPanel.tsx:104-128`），`OverviewState` 表达 UI 四态（`TodayPanel.tsx:36-40`；游客不发起取数、直接进 ready 空数据）：

- **加载中**：钱包三卡与横幅位显示骨架（`phase:"loading"`，`TodayPanel.tsx:176` 起）。
- **有数据**：钱包三卡真实数字（一卡通余额 / 未读邮件 / 在借图书），**单项接口失败该卡回落 "—"**（子字段可 null，`TodayPanel.tsx:131-132`）；「下一节课」横幅（课程 + 教室 + 大节起始时刻）仅 `nextCourse` 非 null 时渲染，无课/取失败整条隐藏。
- **空**：ready + 子字段 null（含游客态占位 `TodayPanel.tsx:115`）。
- **出错**：总览命令失败 → 错误条 + **重试按钮**（`reloadTick` 自增触发 effect 重拉，`TodayPanel.tsx:105,119-128,201-218`），不白屏、不伪造数据。

「下一节课」的大节起始时刻口径由后端 [[modules/campus-portal|门户业务协议核心]] 的校本大节表决定，前端不自算时间（教训见 [[learnings/portal-block-periods-and-school-timetable|门户大节语义与校本作息]]）。

## DockNav：悬浮 Dock 导航（未改）

`components/DockNav.tsx`，macOS Dock 式底部悬浮导航：

- **8 项配置** `DOCK_ITEMS`（`DockNav.tsx:25-39`）：每项 `{ id, label, icon, color }`，color 引用域色 CSS 变量（今日=brand、资讯=info、待办=todo、日程/应用=sched、钱包/电费=wallet、设置=text-2）。
- **gsap 磁吸**：注册每项 `gsap.quickTo(btn, "scale"/"y")`（duration 0.35，ease expo.out，`DockNav.tsx:68-79`）；容器 `onMouseMove` 以 RAF 节流，按指针到各项中心的距离插值——80px 半径内 scale 最高 1.35、上浮最高 -14px（`MAGNETIC_RANGE/MAX_SCALE/MAX_LIFT` `DockNav.tsx:41-43`；插值逻辑 `DockNav.tsx:107-124`）。
- **reduced-motion 降级**：`prefers-reduced-motion: reduce` 命中则不注册磁吸（`magnetEnabled=false` 直接短路，`DockNav.tsx:61-63`）；清理时 `gsap.killTweensOf` + 移除 resize 监听（`DockNav.tsx:95-103`）。
- **域色胶囊**：激活项用 `layoutId="dock-pill"` 的 motion.span 共享布局动画（spring 500/34），背景 `color-mix(in srgb, 域色 14%, transparent)`；底部 `layoutId="dock-dot"` 同域色圆点（`DockNav.tsx:159-187`）。两个 layoutId 使胶囊/圆点在项间平滑滑动。
- Tooltip（shadcn 源码入库 `ui/tooltip.tsx`）250ms 延迟显示项名。

## shadcn 源码入库

`components/ui/{button,card,input,tooltip}.tsx` 为 shadcn 组件源码直接入库（非 CLI 生成依赖），消费 `index.css` 里映射好的 shadcn 语义工具类（`bg-primary`/`bg-card`/`border-border` 等）。**注意**：Tailwind v4 下语义工具类必须经 `@theme inline {}` 映射生成（`:root` 里的普通 CSS 变量不会生成工具类，曾导致主按钮长期无底色），根因与修法见 [[learnings/tailwind-v4-shadcn-token-mapping|Tailwind v4 语义 token 映射]] 与 [[concepts/domain-color-system|域色系统]]。
