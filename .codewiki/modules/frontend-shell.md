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
  - tauri-app/frontend/src/components/CommandPalette.tsx
  - tauri-app/frontend/src/components/AccountMenu.tsx
  - tauri-app/frontend/src/components/LoginDialog.tsx
  - tauri-app/frontend/src/components/Avatar.tsx
  - tauri-app/frontend/src/components/AvatarDialog.tsx
  - tauri-app/frontend/src/components/PanelHeader.tsx
  - tauri-app/frontend/src/components/EmptyState.tsx
  - tauri-app/frontend/src/components/Surface.tsx
  - tauri-app/frontend/src/components/DockNav.tsx
  - tauri-app/frontend/src/panels/TodayPanel.tsx
  - tauri-app/frontend/src/panels/TimetablePanel.tsx
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

`tauri-app/frontend/src`：React 19 + TypeScript + Tailwind v4。分层：`shared/`（IPC 契约与工具）、`stores/`（zustand：authStore 登录态 + uiStore 界面态）、`components/`（外壳与共享组件 + shadcn 源码入库的 `ui/*`）、`panels/`（9 面板）。设计语言与域色 token 见 [[concepts/domain-color-system|域色编码系统]]；游客模式与账号系统的取舍见 [[decisions/guest-mode-account-shell|游客优先与账号外壳决策]]。

## IPC 出口与契约类型（shared/）

- `tauriApi.ts:8-17`：`invokeCommand<T>(cmd, args)` 是**唯一 IPC 出口**，invoke 抛错包装为 `{ success:false, message:String(e) }`，调用方只处理 CommandResult（契约详见 [[modules/campus-hub-tauri|接线层]] 与 [[_architecture|架构总览]]）。
- `types.ts:1`：`CommandResult<T>`；`types.ts:3-41`：M2 批次 1 门户契约 4 接口（`SemesterInfo` / `WalletSummary` / `CourseBrief` / `PortalOverview`，镜像 Rust `commands/portal.rs` DTO，camelCase，子项均可 null）；`types.ts:43-114`：M2 批次 2 契约 7 接口（`InfoColumn` / `InfoItem` / `InfoPage` / `InfoDetail` / `TodoTab` / `TodoItem` / `TodoPage`；`InfoDetail` 含 `needsBrowser` 三分类——`true` 时前端引导浏览器打开、不显示错误态）；`types.ts:127-177`：M2 批次 3 契约 7 接口（`AppAccess` 四值联合类型 + `AppItem`（含 `access`，M2 遗留项新增）/ `AppGroup` / `AppCatalog`（pinned = 收藏钉选）/ `ScheduleClassify` / `ScheduleEvent`（毫秒时间戳，classifyName/color 后端按 code 映射补全，`extra` 承载会议附加信息）/ `ScheduleDayCount`）；`types.ts:116` 起：`PanelId` 9 项（today/**timetable**/info/todo/schedule/apps/wallet/power/settings，M2.5 批次 4 追加 "timetable"，注释明示三处同步：本类型 + DockNav DOCK_ITEMS + AppShell PANEL_MAP）；`types.ts:188` 起：M2.5 课表契约 12 接口（镜像 `campus-schedule::model` 与 `commands/timetable.rs` 的 camelCase 序列化：`CourseSource` / `Course`（`startSection/endSection/classId/remark` 为 `| null` 恒存在；`colorIndex` 导入课程是**课名哈希大数**，取色必须 `% 色板长度`）/ `CourseTableConfig` / `OverrideKind` / `CourseOverride` / `Timetable` / `TimeSlot` / `TimetableView`（批次 4 修订契约）/ `ImportResult` / `ManualCourseInput` / `NoticeConfidence` / `NoticeCandidate`——⚠️ `NoticeCandidate` 的 Option 字段 Rust 侧 `skip_serializing_if` 缺省省略 → TS 用**可选属性**（非 `| null`），与 Course/Override 的恒存在 null 字段区分）。
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

`stores/uiStore.ts:11-58`：`activePanel` + `theme`（两者持久化，键 `campushub-ui`，persist `version: 1`）+ `loginDialogOpen` / `avatarDialogOpen`（一次性 UI 状态，`partialize` 排除，`uiStore.ts:55`）。displayName 已迁往 authStore。登录/头像弹层的**唯一开关**在此：`openLoginDialog()` / `openAvatarDialog()` 供任意入口调用，弹层组件挂载在 App 根部监听同一状态。M2.5 批次 4 追加 `"timetable"` 面板时加了 `migrate`（`uiStore.ts:59-69`）：旧持久化值（无 version 字段按 0 处理触发迁移）原样保留、非法 `activePanel` 兜底回 `"today"`，防止 `PANEL_MAP` 查空白屏。

## App.tsx：游客模式（壳恒渲染）

`App.tsx:8-12`：挂载后 `void checkSession()` 非阻塞启动探测——`status: "unknown"` 期间壳与面板照常可交互，落定 guest/authed 后各组件自行响应（游客空态 / 登录态内容）。渲染恒为 `<AppShell /> + <LoginDialog /> + <AvatarDialog />`（`App.tsx:14-20`），**没有全屏登录门禁**：原「未登录即渲染登录页」的 `AppPhase` 门卫已删除（旧 `panels/LoginPanel.tsx` 整文件移除，逻辑迁入 LoginDialog）。登录入口收敛为三处，全部经 `uiStore.openLoginDialog()` 打开同一弹层：AccountMenu、TodayPanel 引导条（`TodayPanel.tsx:102-116`）、各面板 EmptyState 的「登录」按钮。

## AppShell：顶栏 + 面板容器 + Dock

`components/AppShell.tsx`：

- **新顶栏**（`AppShell.tsx:51-112`）：sticky、`bg-bg/85` 毛玻璃。左：品牌块（渐变方标「锡」+ 双行标识，`AppShell.tsx:53-69`）；中：搜索胶囊（**命令面板已实现**（2026-09-19 补做，原为 M2 遗留的 `aria-disabled` 占位）：点击或 `Cmd/Ctrl+K` 呼起，`AppShell.tsx:75-97` 触发条、`:137` 挂载 `<CommandPalette />`；面板本体 `CommandPalette.tsx`，数据源 = DockNav `DOCK_ITEMS` 页面项 + 现有 store 动作 + 设置跳转，键盘 `↑/↓/Enter/Esc` 可全程操作）；右：铃铛占位（M5 接入）+ AccountMenu 账号胶囊（`AppShell.tsx:94-110`）。
- **主题单向同步**：`useEffect` 把 uiStore `theme` 同步到 `documentElement.classList.toggle("dark")`（`AppShell.tsx:43-46`）；开关入口在 AccountMenu 的「深色模式」行。
- **面板注册表** `PANEL_MAP: Record<PanelId, ComponentType>`（`AppShell.tsx:26-36`），9 面板静态 import。
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

8→9 面板统一节奏：`PanelHeader` 页头 + `Surface` 卡片 + `EmptyState` 空态。**游客**（status=guest）显示「登录后查看×××」+ 登录按钮（经 `openLoginDialog`）；**已登录未接线**显示「数据接入中」（如 WalletPanel.tsx:50-54）。已接真实数据的面板：TodayPanel（M2 批次 1，见下节）、InfoPanel 与 TodoPanel（M2 批次 2，见下两节）、AppsPanel 与 SchedulePanel（M2 批次 3，见下两节）、TimetablePanel（M2.5 批次 4，见下节）。TodayPanel 游客态保留可关闭的登录引导条（本地 state 不持久化，`TodayPanel.tsx:153-166`）与分时段问候；快捷动作为游客点已落地项先登录、未落地项禁用 + tooltip（`TodayPanel.tsx:231` 起）。

## TodayPanel：今日页真实数据（M2 批次 1）

登录态下经 `invokeCommand` 调 `get_portal_overview` 单命令取数（`TodayPanel.tsx:104-128`），`OverviewState` 表达 UI 四态（`TodayPanel.tsx:36-40`；游客不发起取数、直接进 ready 空数据）：

- **加载中**：钱包三卡与横幅位显示骨架（`phase:"loading"`，`TodayPanel.tsx:176` 起）。
- **有数据**：钱包三卡真实数字（一卡通余额 / 未读邮件 / 在借图书），**单项接口失败该卡回落 "—"**（子字段可 null，`TodayPanel.tsx:131-132`）；「下一节课」横幅（课程 + 教室 + 大节起始时刻）仅 `nextCourse` 非 null 时渲染，无课/取失败整条隐藏。
- **空**：ready + 子字段 null（含游客态占位 `TodayPanel.tsx:115`）。
- **出错**：总览命令失败 → 错误条 + **重试按钮**（`reloadTick` 自增触发 effect 重拉，`TodayPanel.tsx:105,119-128,201-218`），不白屏、不伪造数据。

「下一节课」的大节起始时刻口径由后端 [[modules/campus-portal|门户业务协议核心]] 的校本大节表决定，前端不自算时间（教训见 [[learnings/portal-block-periods-and-school-timetable|门户大节语义与校本作息]]）。

## InfoPanel：资讯页真实数据（M2 批次 2）

三个独立 state 机各管一层（`InfoPanel.tsx:16-32`）：`ColumnsState`（栏目 rail：loading/ready/error）、`ListState`（列表四态）、`DetailState`（正文视图三态，`null` = 列表视图）。

- **栏目 rail**：登录后取一次 `get_info_columns`（后端固定 7 栏），首个栏目自动选中（`InfoPanel.tsx:72-89`）；失败整条错误 + 重试、加载中骨架胶囊；`role="tablist"` 语义；切栏目回第 1 页。
- **列表**：`get_info_list`（pageSize=10），条目行 = 栏目名（info 域色）+ 标题 + 部门（`sm:` 以上显示）+ 日期（`dateOf` 截取 `publishTime` 前 10 字符、不做时区换算，`InfoPanel.tsx:35-37`）；四态完整；**「下一页」按 `items.length == PAGE_SIZE` 满页判断**（服务端 total/pageCount 不可靠，`InfoPanel.tsx:144` 注释），不伪造页码。
- **内嵌正文**：`fetchDetail` 打开即带条目标题渲染骨架（`InfoPanel.tsx:114-125`）；后端清洗 HTML 经 `dangerouslySetInnerHTML` 直接渲染，**前端不二次清洗**（`InfoPanel.tsx:281`）；正文容器 `onClick={stopLinkNav}` 统一拦截 `<a>` 导航——WebView 不随正文跳转外站（`InfoPanel.tsx:40-42,278`）；已切走（返回列表/打开另一条）后的过期响应按 URL 比对丢弃（`InfoPanel.tsx:119-120`）；「返回列表」只置 `detail=null`，列表状态保留。
- **needsBrowser 分支**（`InfoPanel.tsx:249-271`）：显示「正文需在浏览器中查看 / 该栏目正文由学校官网鉴权保护，无法在应用内展示」+「在浏览器打开原文」（调 `open_in_browser`，失败内联文案）+「返回列表」——**无错误态/重试**（官网鉴权拦截与网络无关，重试无效）。
- 正文排版 `ARTICLE_CLASS`（`InfoPanel.tsx:45-55`）：段落/标题/表格/列表/图片/引用的最小样式，全部走 token 与 `[_a]` 域色选择器；外链 `<a>` 语义保留但点击被拦截。

## TodoPanel：待办页真实数据（M2 批次 2）

- **三栏 rail**：契约冻结三栏 `TODO_TAB_IDS = ["todo", "done", "apply"]`（`TodoPanel.tsx:17`；接口实际返回 6 个 tab，unread/read/focus 不在契约内不展示）；名称接口优先、失败回落 `TODO_TAB_FALLBACK` 兜底文案不阻塞列表（`TodoPanel.tsx:18-22,88-89`）；`count > 0` 显示待办数徽标（`TodoPanel.tsx:129-131`）。
- **列表**：`get_todo_list`（pageSize=10），标题 + 副行元信息 `metaLine`（申请人 · 申请时间 · 节点 · 紧急度，空段省略、全空回落 source 或 "—"，`TodoPanel.tsx:32-34,178-188`）；空态「暂无事项」（账号无待办时的常态）、错误可重试；分页同资讯页满页判断。
- 分栏名称与待办数取 `get_todo_tabs`（增强信息），与列表取数解耦——分栏接口失败只影响 rail 文案与徽标，列表照常。

## AppsPanel：应用页真实数据（M2 批次 3 + 遗留项可达性）

- **四态**：`CatalogState`（`AppsPanel.tsx:14-19`，loading/ready/empty/error），`get_app_catalog` 单命令取数；empty = 分组与钉选全空。
- **常用钉选区**：`AppCatalog.pinned` 横向置顶，为空时整区不渲染。
- **按部门分组网格**：`AppCatalog.groups` 按服务端组序渲染，组名即部门名；卡片 = 图标 + 应用名（+ 可达性徽标）。
- **图标**：`AppIcon` 有 `iconUrl`（后端代拉的 data URL）用 `<img>`，否则占位图标——不伪造图片。
- **可达性徽标与分级点击**（M2 遗留项，2026-09-18）：`accessBadge`（`AppsPanel.tsx:42`）按后端附录 A 实测表推导的 `item.access` 显示徽标——`webvpn` → 「需校园网/WebVPN」、`unavailable` → 「暂不可用」，cas/external 不标注；点击策略 `openApp`（`AppsPanel.tsx:147`）：**webvpn 提示后仍打开原链接**（校内无感直达、校外失败有解释）、**unavailable 只提示不打开**（实测死链/自有登录，打开无意义）、cas/external 直开不变；提示经 `accessHint` 内联条呈现（`AppsPanel.tsx:115,185`），与打开失败提示分开。
- **打开**：点击卡片经 `open_app`（`url` + `isCas`）在系统浏览器打开，协议白名单与打开策略在后端强制；失败内联红字，不弹错误态。

## SchedulePanel：日程页真实数据（M2 批次 3 + 遗留项月视图/会议）

- **周区间计算**：`weekRange(offset)`（`SchedulePanel.tsx:81`）取第 offset 周的 `[周一 00:00.000, 周日 23:59.999]` 本地毫秒区间；列序周一为 0（`getDay()` 周日=0 折算到第 6 列）。周切换器头部显示日期区间（如 `9.14 – 9.20`）。
- **周/月双视图**（M2 遗留项）：头部 `periodSwitcher`（`SchedulePanel.tsx:272`）= 周/月 toggle + 箭头（`shiftPeriod` 按视图切周/切月，月标题 `YYYY年M月`）；切视图清详情。
- **月视图与角标**：`monthGrid(offset)`（`SchedulePanel.tsx:63`）自然月网格（首格 = 当月 1 日所在周的周一，列序与周视图一致）；角标数据 `get_schedule_day_counts`（`SchedulePanel.tsx:213`，独立 `CountsState` 四态）——⚠️ **角标为当日全量日程数**（bs-schedule 计数接口无分类过滤参数，会议不计入，如实呈现不伪造）；今日格高亮；`gotoWeek`（`SchedulePanel.tsx:253`）点击某天跳到该天所在周（明细按当前过滤取数）；全不选分类时月视图同样不发请求（与周视图一致）；计数失败只降级角标（错误条 + 重试），日历不受影响。
- **取数**：切周 / 切分类过滤触发 `get_schedule_month`（`startMs/endMs/codes`，周视图 effect `SchedulePanel.tsx:180`；**月视图不发明细请求**）；**全不选分类时不发请求**（服务端空 codes 语义未实测，前端规避）；事件四态与计数四态各自独立。
- **5 类彩色过滤 chips**：`get_schedule_classify` 取分类与**服务端色值**（chip 与日程块色标同源）；点击 toggle；chips 自带加载骨架与出错重试。
- **会议块与单时刻**：会议由后端并入 `get_schedule_month` 响应（`classifyCode=Default-Meeting`）；**服务端无结束时刻 → `endMs=startMs`**，`fmtTimeRange`（`SchedulePanel.tsx:98`）对等值显示单时刻（不伪造时间段）；详情卡 `extra` 非空时追加一行附加信息（主持人/参会人员/承办单位）。
- **落列与高亮**：事件按开始时间落列（`dayIndexOf`，跨周事件忽略）；**今日列/今日格高亮仅当前周/当月生效**。

## TimetablePanel：课表页（M2.5 批次 4）

- **契约**：单命令 `get_timetable` → `TimetableView`（见 [[decisions/timetable-view-contract|课表视图契约]]）；`slots`（校本 5 大节作息）是时间标签唯一事实源，**前端不硬编码时间**；小节→大节换算 `blockOf = ceil(小节/2)` 与后端 ICS 展开一致。
- **色板**：`COURSE_PALETTE` 8 档全走 token（6 个既有域色 + index.css 新增 `--color-aqua` / `--color-rose`，深浅主题各一档）；导入课程 `colorIndex` 是课名哈希 → **取色一律 `% 8`**。
- **周视图**：`buildWeekBlocks` 纯函数产出一周块 —— 按视图周过滤 `course.weeks`；停课 override（cancelled）→ 原时段虚线占位「已停」；调课 override（rescheduled，新时间≠原时间）→ 原时段虚线「已调出」+ 新时段实体块；仅换教室 → 原位渲染新教室；补课（extra）→ 新增实体块。同列重叠做轻量分列（连通簇 + 贪心占道，对齐后端 `grid::merge_courses` 语义的前端重写）。override 匹配用**逆序取最后一条**（后采纳覆盖先采纳）。
- **角标**：【导】= `source==="import"`；【调】= 该块关联了生效 override；ghost 块无角标。
- **样式对齐（P0，2026-09-19）**：时间列 = 节号 + 起止时间两行；课程块三要素 = 课名 → 教师（空不渲染）→ `@教室`；天列每大节行铺空位按钮（渲染在课程块之下、不遮挡块点击），点击 `openForm` 预填该格星期/大节（小节 `2k-1..2k`）/展示周；`currentWeek===null`（未设开学日）时提示「请在学期内添加课程」。粒度保持大节行，否决小节行方案见 [[decisions/timetable-block-granularity|课表网格粒度决策]]。
- **P1-P5 全量补齐（2026-09-19，批 1-9；多课表/多方案/全局课程管理经用户裁决不做——单校定位）**：① 列头按 `firstDayOfWeek` 旋转、`displayDays` 裁剪周末列（`buildWeekBlocks` 内部恒 7 天，渲染层 `displayDayOf` 映射）、设置弹层 `save_semester_config`（当前周 hint 反推放后端）；② 跳过日列「休」徽标 + 课程不渲染（`config.skippedDates`）；③ `effective_slots_at(config, date)` 区间命中 slot_rules（rules→config.slots→内置三段回落）；④ 网格拖拽改课（4px 阈值 + pointerId 防多指 + 跳过日列无效回弹；多周/导入课 → `drag:<courseId>:<week>` 幂等 override、单周手动课直改；跨度按小节差守恒）；⑤ custom 课 `customBlockRange` 相交大节落块（无相交仅列表可见、不可拖）；⑥ 周次选择弹窗、顶栏 `weekState` 四态（unset/before/vacation/normal）、非本周课降级显示（`showNonCurrentWeek`）、色板合成 `coursePalette()`（8 固定 + uiStore 自定义段，导入哈希 `% 合成长度`）；⑦ `move_day_courses`（批量 override 整批可撤销）与 `quick_delete`（删空删整条）；⑧ `export_timetable_json`/`import_timetable_json`（一次落盘、config 非空才覆盖）与 ICS VALARM。今日页经 `get_today_courses` 接本地课表（本地优先/教务兜底，见 TodayPanel 段）。
- **详情浮层**：fixed 定位（视口边缘 clamp、下方放不下上翻），Esc / 点击浮层与课程块以外关闭；展示教师/教学班（classId）/周次/教室，`remark` **按来源区分标签**——导入课程 = 「性质 · 考核」（正方 `kcxz·khfsmc`），手动课程 = 「备注」；列出该课全部 override（可撤销）；按钮「手动添加同款」（预填表单）与「编辑」（`update_course` 任意来源可编辑，提示会被下次导入覆盖）。
- **手动表单**：CourseForm（新增 `add_course_manual` / 编辑 `update_course`），周次文本输入（`1-8,10` 混排，`parseWeeksInput`）+ 全部/单周/双周快捷 chips；`key` 重挂重置内部 state。
- **调课通知区**：粘贴文本 → `parse_notice` → 候选列表（high = 绿标「可自动应用」、low = 琥珀 reason + excerpt 引文）→「采纳」`apply_override`；已生效 override 平铺列表「撤销此通知调整」= `revoke_notice(sourceNoticeId)` 整批撤销。
- **导入/同步**：`import_timetable` 成功 → 摘要条（新增/更新/停开/共 N 门 + `changes[]` 逐条）+ 回到当前教学周；失败红字。**导出 ICS**：`export_ics` → Blob 下载「课表.ics」，失败内联红字。
- **全部课程列表**：所有课程平铺（按星期/节次排序），`disabled=true` 灰显 +「已停开」徽标（不画进网格），行内编辑/删除（删除带 confirm）。
- **四态**：guest → 登录空态；loading 骨架；ready 且 0 门 → 「导入课表/手动添加」引导；error 可重试。另：`currentWeek === null`（未设开学日）时显示琥珀提示「按第 N 周展示，导入可自动设置」。


## DockNav：悬浮 Dock 导航（M2.5 批次 4 起 9 项）

`components/DockNav.tsx`，macOS Dock 式底部悬浮导航：

- **9 项配置** `DOCK_ITEMS`（`DockNav.tsx:26-44`）：每项 `{ id, label, icon, color }`，color 引用域色 CSS 变量（今日=brand、**课表=sched**（`CalendarRange` 图标，2026-09-18 M2.5 批次 4 追加，置于「今日」之后；9 项总宽约 416px，1280px 默认窗口不溢出，磁吸/胶囊动画按 DOCK_ITEMS 遍历注册无需额外适配）、资讯=info、待办=todo、日程/应用=sched、钱包/电费=wallet、设置=text-2）。
- **gsap 磁吸**：注册每项 `gsap.quickTo(btn, "scale"/"y")`（duration 0.35，ease expo.out，`DockNav.tsx:68-79`）；容器 `onMouseMove` 以 RAF 节流，按指针到各项中心的距离插值——80px 半径内 scale 最高 1.35、上浮最高 -14px（`MAGNETIC_RANGE/MAX_SCALE/MAX_LIFT` `DockNav.tsx:41-43`；插值逻辑 `DockNav.tsx:107-124`）。
- **reduced-motion 降级**：`prefers-reduced-motion: reduce` 命中则不注册磁吸（`magnetEnabled=false` 直接短路，`DockNav.tsx:61-63`）；清理时 `gsap.killTweensOf` + 移除 resize 监听（`DockNav.tsx:95-103`）。
- **域色胶囊**：激活项用 `layoutId="dock-pill"` 的 motion.span 共享布局动画（spring 500/34），背景 `color-mix(in srgb, 域色 14%, transparent)`；底部 `layoutId="dock-dot"` 同域色圆点（`DockNav.tsx:159-187`）。两个 layoutId 使胶囊/圆点在项间平滑滑动。
- Tooltip（shadcn 源码入库 `ui/tooltip.tsx`）250ms 延迟显示项名。

## shadcn 源码入库

`components/ui/{button,card,input,tooltip}.tsx` 为 shadcn 组件源码直接入库（非 CLI 生成依赖），消费 `index.css` 里映射好的 shadcn 语义工具类（`bg-primary`/`bg-card`/`border-border` 等）。**注意**：Tailwind v4 下语义工具类必须经 `@theme inline {}` 映射生成（`:root` 里的普通 CSS 变量不会生成工具类，曾导致主按钮长期无底色），根因与修法见 [[learnings/tailwind-v4-shadcn-token-mapping|Tailwind v4 语义 token 映射]] 与 [[concepts/domain-color-system|域色系统]]。
